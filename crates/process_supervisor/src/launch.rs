use std::{
    io,
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    sync::OnceLock,
    thread,
    time::{Duration, Instant},
};

use tokio::{sync::oneshot, time::sleep};
use tokio_util::sync::CancellationToken;

use self::admission::Permit;
pub(crate) use self::{admission::Admission, hooks::Hooks};
use crate::{
    options::{ProcessOptions, ProcessSpec},
    process::Owner,
    result::{Interruption, InterruptionReason, PartialState, Stage},
};

static ADMISSION: OnceLock<Admission> = OnceLock::new();

mod admission {
    use std::sync::Arc;

    use tokio::sync::{OwnedSemaphorePermit, Semaphore};

    // Neither the semaphore nor a permit's semaphore() accessor escapes this capability.
    #[derive(Clone)]
    pub(crate) struct Admission(Arc<Semaphore>);

    pub(crate) struct Permit {
        _permit: OwnedSemaphorePermit,
    }

    impl Admission {
        pub fn new(limit: usize) -> Self {
            Self(Arc::new(Semaphore::new(limit)))
        }

        pub async fn acquire(&self) -> Permit {
            Permit {
                _permit: self
                    .0
                    .clone()
                    .acquire_owned()
                    .await
                    .expect("admission semaphore is never closed"),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::super::*;

        impl Admission {
            pub fn available_permits(&self) -> usize {
                self.0.available_permits()
            }
        }
    }
}

pub(crate) fn interruption(
    token: &CancellationToken,
    start: Instant,
    options: &ProcessOptions,
) -> Option<InterruptionReason> {
    if token.is_cancelled() {
        Some(InterruptionReason::Cancelled)
    } else if start.elapsed() >= options.startup_timeout {
        Some(InterruptionReason::TimedOut)
    } else {
        None
    }
}

async fn startup_deadline(start: Instant, budget: Duration) {
    loop {
        let remaining = budget.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            return;
        }
        // Duration::MAX need not fit Instant or Tokio's timer horizon. Long sleeps
        // are only wakeups; elapsed time remains the authority for expiration.
        sleep(remaining.min(Duration::from_secs(24 * 60 * 60))).await;
    }
}

// No runtime backend or test configuration survives in release builds.
#[cfg(not(test))]
pub(crate) mod hooks {
    use super::*;

    #[derive(Default)]
    pub(crate) struct Hooks {
        _private: (),
    }

    #[rustfmt::skip]
    impl Hooks {
        pub(super) fn admission(&self, fallback: Admission) -> Admission { fallback }
        pub(super) fn thread_failure(&self) -> Option<io::Error> { None }
        pub(super) fn take_before_receive(&mut self) -> Option<oneshot::Receiver<()>> { None }
        pub(super) fn take_receiver_closed(&mut self) -> Option<oneshot::Sender<()>> { None }
        pub(super) fn before_spawn(&mut self, _: &mut Command) {}
        pub(super) fn child_created(&mut self, _: &io::Result<Owner>) {}
        pub(super) fn before_send(&mut self) {}
        pub(super) fn after_send(&mut self) {}
        pub(crate) fn pipe_setup(&self) -> Option<fn(&CancellationToken) -> io::Result<()>> { None }
        pub(crate) fn before_allocate(&mut self, _: usize) -> io::Result<()> { Ok(()) }
    }
}

pub(crate) async fn admit(
    options: &ProcessOptions,
    token: &CancellationToken,
    start: Instant,
    state: &mut PartialState,
    hooks: &mut Hooks,
) -> Option<Permit> {
    let admission = hooks.admission(ADMISSION.get_or_init(|| Admission::new(4)).clone());
    let acquire = admission.acquire();
    let deadline = startup_deadline(start, options.startup_timeout);
    tokio::pin!(acquire, deadline);
    let permit = loop {
        if let Some(reason) = interruption(token, start, options) {
            state.interruption = Some(Interruption::Startup(reason));
            token.cancel();
            return None;
        }
        tokio::select! {
            biased;
            _ = token.cancelled() => {},
            _ = &mut deadline => {},
            permit = &mut acquire => break permit,
        }
    };
    Some(permit)
}

/// Terminal outcomes describe creation certainty even when no owner was handed off.
pub(crate) enum LaunchOutcome {
    /// Pipe registration and final startup checks still belong to the caller.
    Ready(Owner),
    /// The caller owns cleanup before it may report startup interruption.
    Interrupted(Owner, InterruptionReason),
    /// Creation failed or was skipped; interruption may coexist with a spawn diagnostic.
    Failed(Option<InterruptionReason>),
    /// A launcher may still create or clean a child; no handoff is not proof of absence.
    Unconfirmed(Option<InterruptionReason>),
}

pub(crate) async fn launch(
    spec: ProcessSpec,
    options: &ProcessOptions,
    token: &CancellationToken,
    start: Instant,
    state: &mut PartialState,
    permit: Permit,
    mut hooks: Hooks,
) -> LaunchOutcome {
    if let Some(reason) = interruption(token, start, options) {
        token.cancel();
        return LaunchOutcome::Failed(Some(reason));
    }
    if let Some(error) = hooks.thread_failure() {
        state.record(Stage::Launcher, error);
        return LaunchOutcome::Failed(None);
    }
    let before_receive = hooks.take_before_receive();
    let receiver_closed = hooks.take_receiver_closed();
    let (sender, mut receiver) = oneshot::channel();
    let launch_token = token.clone();
    let post_kill = options.post_kill_grace;
    if let Err(error) = thread::Builder::new()
        .name("process-launch".into())
        .spawn(move || {
            let _permit = permit;
            let result = if launch_token.is_cancelled() {
                Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "launch cancelled before OS spawn",
                ))
            } else {
                let mut command = Command::new(spec.program);
                command
                    .args(spec.args)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .process_group(0);
                if let Some(cwd) = spec.cwd {
                    command.current_dir(cwd);
                }
                hooks.before_spawn(&mut command);
                // Install ownership before any channel send or other fallible handoff.
                command.spawn().map(Owner::new)
            };
            hooks.child_created(&result);
            hooks.before_send();
            if let Err(Ok(owner)) = sender.send(result) {
                owner.late_cleanup(post_kill);
            }
            hooks.after_send();
        })
    {
        state.record(Stage::Launcher, error);
        return LaunchOutcome::Failed(None);
    }
    if let Some(before_receive) = before_receive {
        let _ = before_receive.await;
    }
    let deadline = startup_deadline(start, options.startup_timeout);
    tokio::pin!(deadline);
    loop {
        if let Some(reason) = interruption(token, start, options) {
            token.cancel();
            // close alone does not remove a value sent just before closure.
            receiver.close();
            if let Some(closed) = receiver_closed {
                let _ = closed.send(());
            }
            return match receiver.try_recv() {
                Ok(Ok(owner)) => LaunchOutcome::Interrupted(owner, reason),
                Ok(Err(error)) => {
                    state.record(Stage::Spawn, error);
                    LaunchOutcome::Failed(Some(reason))
                }
                // Closing an empty channel cannot distinguish a blocked launcher
                // from one concurrently finishing; do not promise no child exists.
                Err(_) => LaunchOutcome::Unconfirmed(Some(reason)),
            };
        }
        tokio::select! {
            biased;
            _ = token.cancelled() => {},
            _ = &mut deadline => {},
            result = &mut receiver => {
                let interrupted = interruption(token, start, options);
                if interrupted.is_some() {
                    token.cancel();
                }
                return match result {
                    Ok(Ok(owner)) => match interrupted {
                        Some(reason) => LaunchOutcome::Interrupted(owner, reason),
                        None => LaunchOutcome::Ready(owner),
                    },
                    Ok(Err(error)) => {
                        state.record(Stage::Spawn, error);
                        LaunchOutcome::Failed(interrupted)
                    },
                    Err(error) => {
                        state.record(Stage::Launcher, io::Error::other(error));
                        LaunchOutcome::Unconfirmed(interrupted)
                    },
                };
            },
        }
    }
}

#[cfg(test)]
pub(crate) mod hooks {
    use super::*;

    type CommandHook = Box<dyn FnOnce(&mut Command) + Send>;

    #[derive(Default)]
    pub(crate) struct Hooks {
        pub admission: Option<Admission>,
        pub before_spawn: Option<CommandHook>,
        pub before_send: Option<Box<dyn FnOnce() + Send>>,
        pub child_created: Option<oneshot::Sender<u32>>,
        pub after_send: Option<Box<dyn FnOnce() + Send>>,
        pub before_receive: Option<oneshot::Receiver<()>>,
        pub receiver_closed: Option<oneshot::Sender<()>>,
        pub pipe_setup: Option<fn(&CancellationToken) -> io::Result<()>>,
        pub before_allocate: Option<Box<dyn FnMut(usize) -> io::Result<()> + Send>>,
        pub fail_thread: bool,
    }

    #[rustfmt::skip]
    impl Hooks {
        pub(super) fn admission(&self, fallback: Admission) -> Admission { self.admission.clone().unwrap_or(fallback) }
        pub(super) fn thread_failure(&self) -> Option<io::Error> { self.fail_thread.then(|| io::Error::other("injected thread creation failure")) }
        pub(super) fn take_before_receive(&mut self) -> Option<oneshot::Receiver<()>> { self.before_receive.take() }
        pub(super) fn take_receiver_closed(&mut self) -> Option<oneshot::Sender<()>> { self.receiver_closed.take() }
        pub(super) fn before_spawn(&mut self, command: &mut Command) {
            if let Some(hook) = self.before_spawn.take() { hook(command); }
        }
        pub(super) fn child_created(&mut self, result: &io::Result<Owner>) {
            if let (Ok(owner), Some(created)) = (result, self.child_created.take()) { let _ = created.send(owner.id()); }
        }
        pub(super) fn before_send(&mut self) {
            if let Some(hook) = self.before_send.take() { hook(); }
        }
        pub(super) fn after_send(&mut self) {
            if let Some(hook) = self.after_send.take() { hook(); }
        }
        pub(crate) fn pipe_setup(&self) -> Option<fn(&CancellationToken) -> io::Result<()>> { self.pipe_setup }
        pub(crate) fn before_allocate(&mut self, limit: usize) -> io::Result<()> { self.before_allocate.as_mut().map_or(Ok(()), |hook| hook(limit)) }
    }
}
