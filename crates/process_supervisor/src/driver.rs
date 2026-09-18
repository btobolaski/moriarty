use std::{io, time::Instant};

use tokio::{
    io::AsyncRead,
    process::{ChildStderr, ChildStdout},
    sync::oneshot,
    time::sleep,
};
use tokio_util::sync::{CancellationToken, DropGuard};

use crate::{
    capture::Stream,
    launch::{self, Hooks, LaunchOutcome},
    options::{ProcessOptions, ProcessSpec},
    process::{Owner, POLL},
    result::{
        Completion, DriverChannel, Interruption, InterruptionReason, PartialState, ProcessError,
        ProcessReport, Stage,
    },
};

/// Scope ownership requests cancellation on Drop; the independent driver still owns
/// cleanup. See the [ownership contract](crate#usage-and-ownership).
pub struct RunningProcess {
    completion: oneshot::Receiver<Result<ProcessReport, ProcessError>>,
    token: CancellationToken,
    _guard: DropGuard,
}

impl RunningProcess {
    /// A clone can cancel while `wait(self)` owns this handle; dropping a clone does not cancel.
    pub fn cancellation_handle(&self) -> CancellationToken {
        self.token.clone()
    }

    /// Nonzero exits are reports, not supervision errors. Dropping this future cancels
    /// without aborting cleanup; see [result guarantees](crate#budgets-and-results).
    pub async fn wait(mut self) -> miette::Result<ProcessReport> {
        // Keep the owner (including its guard) in the future until this await ends.
        (&mut self.completion)
            .await
            .unwrap_or_else(|_| Err(ProcessError::driver_closed(DriverChannel::Completion)))
            .map_err(Into::into)
    }

    /// Awaiting cleanup, rather than aborting the driver, preserves the child's actual status.
    pub async fn cancel_and_wait(self) -> miette::Result<ProcessReport> {
        self.token.cancel();
        self.wait().await
    }
}

/// Success acknowledges both child ownership and registered capture pipes. Requires
/// Tokio I/O and time; [startup uncertainty](crate#stalled-startup-and-host-lifetime)
/// remains explicit if creation outlives its budget.
pub async fn spawn(spec: ProcessSpec, options: ProcessOptions) -> miette::Result<RunningProcess> {
    spawn_with(spec, options, Hooks::default()).await
}

async fn spawn_with(
    spec: ProcessSpec,
    options: ProcessOptions,
    hooks: Hooks,
) -> miette::Result<RunningProcess> {
    let start = Instant::now();
    let token = CancellationToken::new();
    let guard = token.clone().drop_guard();
    let (startup_tx, startup_rx) = oneshot::channel();
    let (completion_tx, completion) = oneshot::channel();
    let driver_token = token.clone();
    tokio::spawn(async move {
        match prepare(spec, &options, &driver_token, start, hooks).await {
            Ok(driver) => {
                if startup_tx.send(Ok(())).is_err() {
                    driver_token.cancel();
                }
                let _ = completion_tx.send(driver.run(options, driver_token).await);
            }
            Err(error) => {
                let _ = startup_tx.send(Err(error));
            }
        }
    });
    startup_rx
        .await
        .unwrap_or_else(|_| Err(ProcessError::driver_closed(DriverChannel::Startup)))?;
    Ok(RunningProcess {
        completion,
        token,
        _guard: guard,
    })
}

#[derive(Clone, Copy)]
enum Cause {
    Exited,
    Cancelled,
    TimedOut,
    Failed,
    StartFailed,
    Interrupted,
}

enum Pipe {
    Stdout,
    Stderr,
}

enum Phase {
    Running(Instant),
    Term(Instant, Cause),
    Final(Instant, Cause),
}

struct Driver {
    owner: Owner,
    stdout: Stream<ChildStdout>,
    stderr: Stream<ChildStderr>,
    state: PartialState,
    phase: Phase,
}

fn allocate_stream<R: AsyncRead + Unpin>(limit: usize, hooks: &mut Hooks) -> io::Result<Stream<R>> {
    // Per-boundary injection prevents allocation reordering from evading the failure test.
    hooks.before_allocate(limit)?;
    Stream::allocate(limit)
}

async fn prepare(
    spec: ProcessSpec,
    options: &ProcessOptions,
    token: &CancellationToken,
    start: Instant,
    mut hooks: Hooks,
) -> Result<Driver, ProcessError> {
    let mut state = PartialState::default();
    let Some(permit) = launch::admit(options, token, start, &mut state, &mut hooks).await else {
        return Err(if state.interruption.is_some() {
            ProcessError::StartupInterrupted {
                state: Box::new(state),
            }
        } else {
            ProcessError::StartFailed {
                state: Box::new(state),
            }
        });
    };
    let streams: io::Result<_> = (|| {
        let stdout = allocate_stream(options.stdout_limit, &mut hooks)?;
        let stderr = allocate_stream(options.stderr_limit, &mut hooks)?;
        Ok((stdout, stderr))
    })();
    let (stdout, stderr) = match streams {
        Ok(streams) => streams,
        Err(error) => {
            state.record(Stage::Allocation, error);
            return Err(ProcessError::StartFailed {
                state: Box::new(state),
            });
        }
    };
    let pipe_setup = hooks.pipe_setup();
    let (owner, mut cause) =
        match launch::launch(spec, options, token, start, &mut state, permit, hooks).await {
            LaunchOutcome::Ready(owner) => (owner, None),
            LaunchOutcome::Interrupted(owner, reason) => {
                state.interruption = Some(Interruption::Startup(reason));
                (owner, Some(Cause::Interrupted))
            }
            LaunchOutcome::Failed(reason) => {
                state.interruption = reason.map(Interruption::Startup);
                return Err(match reason {
                    Some(_) => ProcessError::StartupInterrupted {
                        state: Box::new(state),
                    },
                    None => ProcessError::StartFailed {
                        state: Box::new(state),
                    },
                });
            }
            LaunchOutcome::Unconfirmed(reason) => {
                state.launch_unconfirmed = true;
                state.interruption = reason.map(Interruption::Startup);
                return Err(match reason {
                    Some(_) => ProcessError::StartupInterrupted {
                        state: Box::new(state),
                    },
                    None => ProcessError::CleanupUnconfirmed {
                        state: Box::new(state),
                    },
                });
            }
        };
    state.child_id = Some(owner.id());
    let mut driver = Driver {
        owner,
        stdout,
        stderr,
        state,
        phase: Phase::Running(Instant::now()),
    };
    let setup = driver
        .setup_pipes()
        .and_then(|()| pipe_setup.map_or(Ok(()), |hook| hook(token)));
    if let Err(error) = setup {
        driver.state.record(Stage::PipeSetup, error);
        driver.stdout.close_failed();
        driver.stderr.close_failed();
        cause.get_or_insert(Cause::StartFailed);
    }
    if cause.is_none()
        && let Some(reason) = launch::interruption(token, start, options)
    {
        driver.state.interruption = Some(Interruption::Startup(reason));
        token.cancel();
        cause = Some(Cause::Interrupted);
    }
    if let Some(cause) = cause {
        driver.kill(cause);
        let (driver, cause) = driver.settle(options, token).await;
        return Err(driver.into_error(cause));
    }
    driver.phase = Phase::Running(Instant::now());
    Ok(driver)
}

impl Driver {
    fn setup_pipes(&mut self) -> io::Result<()> {
        let stdout = self
            .owner
            .take_stdout()
            .ok_or_else(|| io::Error::other("missing stdout pipe"))?;
        self.stdout.reader = Some(ChildStdout::from_std(stdout)?);
        let stderr = self
            .owner
            .take_stderr()
            .ok_or_else(|| io::Error::other("missing stderr pipe"))?;
        self.stderr.reader = Some(ChildStderr::from_std(stderr)?);
        Ok(())
    }

    fn term(&mut self, cause: Cause) {
        self.phase = Phase::Term(Instant::now(), cause);
        self.state.interruption = match cause {
            Cause::Cancelled => Some(Interruption::Execution(InterruptionReason::Cancelled)),
            Cause::TimedOut => Some(Interruption::Execution(InterruptionReason::TimedOut)),
            _ => None,
        };
        self.state.term_attempted = true;
        if let Err(error) = self.owner.term() {
            self.state.record(error.stage, error.source);
        }
    }

    fn kill(&mut self, cause: Cause) {
        self.state.kill_attempted = true;
        if let Err(error) = self.owner.final_kill() {
            self.state.record(error.stage, error.source);
        }
        self.phase = Phase::Final(Instant::now(), cause);
        // Unconverted pipes must not keep a startup failure's cleanup open.
        self.owner.take_stdout();
        self.owner.take_stderr();
    }

    fn observe(&mut self) {
        match self.owner.observe() {
            Ok(true) => {
                if matches!(self.phase, Phase::Running(_)) {
                    self.kill(Cause::Exited);
                }
            }
            Ok(false) => {}
            Err(error) => {
                self.state.record(Stage::Observation, error);
                if matches!(self.phase, Phase::Running(_)) {
                    self.term(Cause::Failed);
                }
            }
        }
    }

    fn finalize(&mut self) {
        if self.state.status.is_none() {
            match self.owner.disarm().reap() {
                Ok(status) => self.state.status = status,
                Err(error) => self.state.record(Stage::Reap, error),
            }
        }
        self.owner.record_group_absence(&mut self.state);
    }

    fn complete(&self) -> bool {
        self.state.status.is_some()
            && self.state.group_absent
            && self.stdout.settled()
            && self.stderr.settled()
    }

    fn into_error(mut self, cause: Cause) -> ProcessError {
        let confirmed = self.complete();
        self.state.stdout = self.stdout.capture;
        self.state.stderr = self.stderr.capture;
        let state = Box::new(self.state);
        if !confirmed {
            return ProcessError::CleanupUnconfirmed { state };
        }
        match cause {
            Cause::Interrupted => ProcessError::StartupInterrupted { state },
            Cause::StartFailed => ProcessError::StartFailed { state },
            _ => ProcessError::SupervisionFailed { state },
        }
    }

    fn finish(self, cause: Cause) -> Result<ProcessReport, ProcessError> {
        if !self.state.group_absent
            || self.state.failed()
            || !self.stdout.capture.eof
            || !self.stderr.capture.eof
        {
            return Err(self.into_error(cause));
        }
        let Some(status) = self.state.status else {
            return Err(self.into_error(cause));
        };
        let completion = match cause {
            Cause::Exited => Completion::Exited(status),
            Cause::Cancelled => Completion::Cancelled(status),
            Cause::TimedOut => Completion::TimedOut(status),
            _ => return Err(self.into_error(cause)),
        };
        Ok(ProcessReport {
            completion,
            stdout: self.stdout.capture,
            stderr: self.stderr.capture,
        })
    }

    async fn run(
        self,
        options: ProcessOptions,
        token: CancellationToken,
    ) -> Result<ProcessReport, ProcessError> {
        let (driver, cause) = self.settle(&options, &token).await;
        driver.finish(cause)
    }

    async fn settle(
        mut self,
        options: &ProcessOptions,
        token: &CancellationToken,
    ) -> (Self, Cause) {
        let mut last_poll = None;
        loop {
            // Control and fixed phase clocks always run before another ready chunk.
            let pending = match self.phase {
                Phase::Running(_) if token.is_cancelled() => Some(Cause::Cancelled),
                Phase::Running(start)
                    if options
                        .execution_timeout
                        .is_some_and(|budget| start.elapsed() >= budget) =>
                {
                    Some(Cause::TimedOut)
                }
                _ => None,
            };
            if let Some(cause) = pending {
                // This fresh observation defines precedence, not the kernel's historical exit time.
                self.observe();
                last_poll = Some(Instant::now());
                if matches!(self.phase, Phase::Running(_)) {
                    self.term(cause);
                }
            }
            if let Phase::Term(start, cause) = self.phase
                && start.elapsed() >= options.term_grace
            {
                self.kill(cause);
            }
            let due = last_poll.is_none_or(|last: Instant| last.elapsed() >= POLL);
            if due {
                last_poll = Some(Instant::now());
                if matches!(self.phase, Phase::Final(..)) {
                    self.finalize();
                } else {
                    self.observe();
                }
            }
            if let Phase::Final(start, cause) = self.phase {
                if self.complete() {
                    return (self, cause);
                }
                if start.elapsed() >= options.post_kill_grace {
                    self.finalize();
                    let out = self.stdout.probe().await;
                    let err = self.stderr.probe().await;
                    self.read_result(Pipe::Stdout, out);
                    self.read_result(Pipe::Stderr, err);
                    return (self, cause);
                }
            }
            let remaining = match self.phase {
                Phase::Running(start) => options
                    .execution_timeout
                    .map(|budget| budget.saturating_sub(start.elapsed()))
                    .unwrap_or(POLL),
                Phase::Term(start, _) => options.term_grace.saturating_sub(start.elapsed()),
                Phase::Final(start, _) => options.post_kill_grace.saturating_sub(start.elapsed()),
            }
            .min(POLL.saturating_sub(last_poll.map(|last| last.elapsed()).unwrap_or_default()));
            tokio::select! {
                _ = token.cancelled(), if matches!(self.phase, Phase::Running(_)) => {},
                result = self.stdout.read() => self.read_result(Pipe::Stdout, result),
                result = self.stderr.read() => self.read_result(Pipe::Stderr, result),
                _ = sleep(remaining) => {},
            }
            tokio::task::yield_now().await;
        }
    }

    fn read_result(&mut self, pipe: Pipe, result: io::Result<()>) {
        if let Err(error) = result {
            let stage = match pipe {
                Pipe::Stdout => {
                    self.stdout.close_failed();
                    Stage::Stdout
                }
                Pipe::Stderr => {
                    self.stderr.close_failed();
                    Stage::Stderr
                }
            };
            self.state.record(stage, error);
            if matches!(self.phase, Phase::Running(_)) {
                self.term(Cause::Failed);
            }
        }
    }
}

#[cfg(test)]
mod tests;
