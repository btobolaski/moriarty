#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("process_supervisor supports only Linux and macOS");

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) use platform::*;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod platform {
    use std::{
        io,
        process::{Child, ChildStderr, ChildStdout, ExitStatus},
        thread,
        time::{Duration, Instant},
    };

    use rustix::{
        io::Errno,
        process::{
            Pid, Signal, WaitId, WaitIdOptions, getpgrp, kill_process_group,
            test_kill_process_group, waitid,
        },
    };

    #[cfg(not(test))]
    type Operations = System;
    #[cfg(test)]
    type Operations = tests::Script;

    use crate::result::{PartialState, Stage, StageFailure};

    pub(crate) const POLL: Duration = Duration::from_millis(20);

    pub(crate) struct Owner {
        child: Child,
        group: Option<Pid>,
        armed: bool,
        ops: Operations,
    }

    // The borrow excludes signaling access, and revocation survives the view's lifetime.
    pub(crate) struct Reaper<'a> {
        child: &'a mut Child,
        ops: &'a Operations,
    }

    impl Owner {
        pub fn id(&self) -> u32 {
            self.child.id()
        }

        pub fn take_stdout(&mut self) -> Option<ChildStdout> {
            self.child.stdout.take()
        }

        pub fn take_stderr(&mut self) -> Option<ChildStderr> {
            self.child.stderr.take()
        }

        pub fn disarm(&mut self) -> Reaper<'_> {
            self.armed = false;
            Reaper {
                child: &mut self.child,
                ops: &self.ops,
            }
        }

        pub fn new(child: Child) -> Self {
            let group = i32::try_from(child.id())
                .ok()
                .and_then(Pid::from_raw)
                .filter(|pid| pid.as_raw_pid() > 1 && *pid != getpgrp());
            Self {
                child,
                group,
                armed: group.is_some(),
                ops: Operations::default(),
            }
        }

        fn identity(&self) -> io::Result<Pid> {
            self.group
                .ok_or_else(|| io::Error::other("invalid or host process-group identity"))
        }

        // NOWAIT pins the leader's identity until the last destructive signal. Using
        // Child::try_wait here would reap it and permit an unrelated PGID to be reused.
        pub fn observe(&mut self) -> io::Result<bool> {
            let pid = self.identity()?;
            let result = bounded_retry(|| self.ops.observe(pid));
            if matches!(result, Err(Errno::CHILD)) {
                self.armed = false;
            }
            result.map_err(Into::into)
        }

        fn preflight(&mut self) -> Result<(), StageFailure> {
            let observation = if self.armed {
                self.observe().map(|_| ())
            } else {
                Err(io::Error::other("signal authority revoked"))
            };
            observation.map_err(|source| StageFailure {
                stage: Stage::Observation,
                source,
            })
        }

        pub fn term(&mut self) -> Result<(), StageFailure> {
            self.preflight()?;
            self.signal(Signal::TERM).map_err(|source| StageFailure {
                stage: Stage::Term,
                source,
            })
        }

        pub fn final_kill(&mut self) -> Result<(), StageFailure> {
            let observation = self.preflight();
            // Revocation is permanent, including when observation or KILL fails.
            self.armed = false;
            observation?;
            self.signal(Signal::KILL).map_err(|source| StageFailure {
                stage: Stage::Kill,
                source,
            })
        }

        pub fn group_absent(&self) -> io::Result<bool> {
            let group = self.identity()?;
            match self.ops.group_absent(group) {
                Ok(()) => Ok(false),
                Err(Errno::SRCH) => Ok(true),
                Err(error) => Err(error.into()),
            }
        }

        fn signal(&self, signal: Signal) -> io::Result<()> {
            let group = self.identity()?;
            match bounded_retry(|| self.ops.signal(group, signal)) {
                Ok(()) | Err(Errno::SRCH) => Ok(()),
                Err(error) => Err(error.into()),
            }
        }

        pub fn record_group_absence(&self, state: &mut PartialState) {
            if !state.group_absent {
                match self.group_absent() {
                    Ok(absent) => state.group_absent = absent,
                    Err(error) => state.record(Stage::GroupProbe, error),
                }
            }
        }

        pub fn late_cleanup(mut self, budget: Duration) {
            let mut state = PartialState::default();
            state.child_id = Some(self.child.id());
            state.kill_attempted = true;
            if let Err(error) = self.final_kill() {
                state.record(error.stage, error.source);
            }
            let start = Instant::now();
            self.child.stdout.take();
            self.child.stderr.take();
            loop {
                if state.status.is_none() {
                    match self.disarm().reap() {
                        Ok(status) => state.status = status,
                        Err(error) => state.record(Stage::Reap, error),
                    }
                }
                self.record_group_absence(&mut state);
                if state.status.is_some() && state.group_absent {
                    break;
                }
                if start.elapsed() >= budget {
                    break;
                }
                thread::sleep(POLL.min(budget.saturating_sub(start.elapsed())));
            }
            if state.failed() || state.status.is_none() || !state.group_absent {
                report_late_cleanup(&state);
            }
        }
    }

    fn report_late_cleanup(state: &PartialState) {
        // No receiver remains. Subscriber dispatch can block; the launcher must
        // retain its admission permit until this call and the thread actually finish.
        tracing::warn!(state = %state, "process_supervisor late cleanup unconfirmed");
    }

    fn bounded_retry<T>(
        mut operation: impl FnMut() -> rustix::io::Result<T>,
    ) -> rustix::io::Result<T> {
        for _ in 0..2 {
            match operation() {
                Err(Errno::INTR) => continue,
                result => return result,
            }
        }
        operation()
    }

    #[derive(Default)]
    pub(crate) struct System;

    impl System {
        fn observe(&self, pid: Pid) -> rustix::io::Result<bool> {
            waitid(
                WaitId::Pid(pid),
                WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
            )
            .map(|status| status.is_some())
        }

        fn group_absent(&self, group: Pid) -> rustix::io::Result<()> {
            bounded_retry(|| test_kill_process_group(group))
        }

        fn signal(&self, group: Pid, signal: Signal) -> rustix::io::Result<()> {
            kill_process_group(group, signal)
        }

        fn reap(&self, child: &mut Child) -> io::Result<Option<ExitStatus>> {
            child.try_wait()
        }

        fn reap_on_drop(&self, child: &mut Child) {
            let _ = child.try_wait();
        }
    }

    impl Reaper<'_> {
        pub fn reap(&mut self) -> io::Result<Option<ExitStatus>> {
            self.ops.reap(self.child)
        }

        fn reap_on_drop(&mut self) {
            self.ops.reap_on_drop(self.child);
        }
    }

    impl Drop for Owner {
        fn drop(&mut self) {
            if self.armed {
                let _ = self.final_kill();
            }
            self.disarm().reap_on_drop();
        }
    }

    #[cfg(test)]
    mod tests {
        use std::{
            env, fmt,
            os::unix::process::CommandExt,
            process::{Command, Stdio},
            sync::{Arc, Mutex},
        };

        use tracing::{
            Event, Metadata, Subscriber,
            field::Field,
            span::{Attributes, Id, Record},
            subscriber::with_default,
        };

        use super::*;

        impl Owner {
            pub(crate) fn ops(&mut self) -> &mut Operations {
                &mut self.ops
            }
        }

        #[derive(Clone, Default)]
        pub(crate) struct Trace(Arc<Mutex<Vec<Stage>>>);

        #[rustfmt::skip]
        impl Trace {
            fn push(&self, stage: Stage) { self.0.lock().unwrap().push(stage); }
            pub fn clear(&self) { self.0.lock().unwrap().clear(); }
            pub fn snapshot(&self) -> Vec<Stage> { self.0.lock().unwrap().clone() }
        }

        #[derive(Default)]
        pub(crate) struct Script {
            errors: Vec<(Stage, Errno)>,
            exited: Option<bool>,
            status: Option<Option<ExitStatus>>,
            absent: Option<bool>,
            trace: Trace,
        }

        #[rustfmt::skip]
        impl Script {
            pub fn fail(&mut self, stage: Stage, error: Errno) { self.errors.push((stage, error)); }
            pub fn observe_as(&mut self, exited: bool) { self.exited = Some(exited); }
            pub fn stall_cleanup(&mut self) { self.status = Some(None); self.absent = Some(false); }
            pub fn group_absent_as(&mut self, absent: bool) { self.absent = Some(absent); }
            pub fn trace_handle(&self) -> Trace { self.trace.clone() }
        }

        impl Script {
            fn check(&self, stage: Stage) -> rustix::io::Result<()> {
                self.trace.push(stage);
                self.errors
                    .iter()
                    .find(|(at, _)| *at == stage)
                    .map_or(Ok(()), |(_, error)| Err(*error))
            }

            pub(super) fn observe(&self, pid: Pid) -> rustix::io::Result<bool> {
                self.check(Stage::Observation)?;
                self.exited.map_or_else(|| System.observe(pid), Ok)
            }

            pub(super) fn group_absent(&self, group: Pid) -> rustix::io::Result<()> {
                self.check(Stage::GroupProbe)?;
                match self.absent {
                    Some(false) => Ok(()),
                    Some(true) => Err(Errno::SRCH),
                    None => System.group_absent(group),
                }
            }

            pub(super) fn signal(&self, group: Pid, signal: Signal) -> rustix::io::Result<()> {
                self.check(if signal == Signal::TERM {
                    Stage::Term
                } else {
                    Stage::Kill
                })?;
                System.signal(group, signal)
            }

            pub(super) fn reap(&self, child: &mut Child) -> io::Result<Option<ExitStatus>> {
                self.check(Stage::Reap)?;
                self.status.map_or_else(|| System.reap(child), Ok)
            }

            pub(super) fn reap_on_drop(&self, child: &mut Child) {
                // Scripted failures must not suppress the real child's last nonblocking reap.
                self.trace.push(Stage::Reap);
                System.reap_on_drop(child);
            }
        }

        struct Events(Arc<Mutex<Vec<String>>>);

        impl Subscriber for Events {
            fn enabled(&self, _: &Metadata<'_>) -> bool {
                true
            }
            fn new_span(&self, _: &Attributes<'_>) -> Id {
                Id::from_u64(1)
            }
            fn record(&self, _: &Id, _: &Record<'_>) {}
            fn record_follows_from(&self, _: &Id, _: &Id) {}
            fn enter(&self, _: &Id) {}
            fn exit(&self, _: &Id) {}
            fn event(&self, event: &Event<'_>) {
                event.record(&mut |field: &Field, value: &dyn fmt::Debug| {
                    self.0.lock().unwrap().push(format!("{field}={value:?}"));
                });
            }
        }

        #[test]
        fn late_diagnostic_event_excludes_captured_bytes() {
            let mut state = PartialState::default();
            state.child_id = Some(7);
            state.stdout.bytes = b"private stdout".to_vec();
            state.stderr.bytes = b"private stderr".to_vec();
            state.record(Stage::Reap, io::Error::other("reap failed"));
            let events = Arc::new(Mutex::new(Vec::new()));
            with_default(Events(events.clone()), || report_late_cleanup(&state));
            let events = events.lock().unwrap();
            assert_eq!(events.len(), 2);
            assert!(
                events
                    .iter()
                    .any(|field| field == &format!("state={state}"))
            );
            let rendered = events.join(" ");
            for capture in [&state.stdout, &state.stderr] {
                assert!(!rendered.contains(&format!("{:?}", capture.bytes)));
                assert!(!rendered.contains(&String::from_utf8_lossy(&capture.bytes).to_string()));
            }
        }

        #[test]
        fn retries_interrupted_operation() {
            let mut attempts = 0;
            assert_eq!(
                bounded_retry(|| {
                    attempts += 1;
                    (attempts > 1).then_some(()).ok_or(Errno::INTR)
                }),
                Ok(())
            );
            assert_eq!(attempts, 2);
        }

        fn exited_owner() -> Owner {
            let child = Command::new(env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "driver::tests::fixture",
                    "--nocapture",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()
                .unwrap();
            let mut owner = Owner::new(child);
            let start = Instant::now();
            while !owner.observe().unwrap() {
                assert!(start.elapsed() < Duration::from_secs(5));
                thread::sleep(POLL);
            }
            owner.ops().trace_handle().clear();
            owner
        }

        #[test]
        fn late_cleanup_records_signal_failures() {
            for (stage, errno) in [
                (Stage::Observation, Errno::CHILD),
                (Stage::Kill, Errno::PERM),
            ] {
                let mut owner = exited_owner();
                owner.ops().fail(stage, errno);
                let trace = owner.ops().trace_handle();
                let events = Arc::new(Mutex::new(Vec::new()));
                with_default(Events(events.clone()), || {
                    owner.late_cleanup(Duration::ZERO)
                });
                let rendered = events.lock().unwrap().join(" ");
                assert!(
                    rendered.contains(&format!(
                        "{stage:?}: {}",
                        io::Error::from_raw_os_error(errno.raw_os_error())
                    )),
                    "{rendered}"
                );
                let trace = trace.snapshot();
                assert!(!trace.contains(&Stage::Term));
                assert_eq!(trace.contains(&Stage::Kill), stage == Stage::Kill);
                let reap = trace.iter().position(|seen| *seen == Stage::Reap).unwrap();
                assert!(
                    !trace[reap..]
                        .iter()
                        .any(|seen| matches!(seen, Stage::Term | Stage::Kill))
                );
            }
        }

        #[test]
        fn late_cleanup_signals_once() {
            let mut owner = exited_owner();
            owner.ops().stall_cleanup();
            let trace = owner.ops().trace_handle();
            owner.late_cleanup(Duration::ZERO);
            let trace = trace.snapshot();
            for (stage, count) in [(Stage::Kill, 1), (Stage::GroupProbe, 1), (Stage::Reap, 2)] {
                assert_eq!(trace.iter().filter(|seen| **seen == stage).count(), count);
            }
        }

        #[test]
        fn reaping_view_permanently_revokes_signal_authority() {
            let mut owner = exited_owner();
            let status = owner.disarm().reap().unwrap();
            assert!(status.is_some());
            for result in [owner.term(), owner.final_kill()] {
                let Err(error) = result else {
                    panic!("signal authority was restored")
                };
                assert_eq!(error.stage, Stage::Observation);
                assert_eq!(error.source.to_string(), "signal authority revoked");
            }
            let trace = owner.ops().trace_handle();
            drop(owner);
            assert!(trace.snapshot().iter().all(|stage| *stage == Stage::Reap));
        }

        #[test]
        fn invalid_identity_cannot_signal_even_in_drop() {
            let mut owner = exited_owner();
            owner.group = None;
            let mut state = PartialState::default();
            owner.record_group_absence(&mut state);
            let failure = state.failures().next().unwrap();
            assert_eq!(failure.stage, Stage::GroupProbe);
            assert_eq!(
                failure.source.to_string(),
                "invalid or host process-group identity"
            );
            assert!(!state.group_absent);
            let trace = owner.ops().trace_handle();
            trace.clear();
            drop(owner);
            assert_eq!(trace.snapshot(), vec![Stage::Reap]);
        }
    }
}
