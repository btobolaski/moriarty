use std::os::fd::OwnedFd;

use rustix::io::Errno;

use super::*;

fn assert_signal_order(trace: &[Stage]) {
    let first_reap = trace
        .iter()
        .position(|stage| *stage == Stage::Reap)
        .expect("reap attempted");
    assert!(
        !trace[first_reap..]
            .iter()
            .any(|stage| matches!(stage, Stage::Term | Stage::Kill)),
        "{trace:?}"
    );
}

#[tokio::test]
async fn scripted_errors_preserve_initiating_failure_and_signal_order() {
    for (stage, errno) in [
        (Stage::Observation, Errno::CHILD),
        (Stage::Observation, Errno::INTR),
        (Stage::Term, Errno::PERM),
        (Stage::Kill, Errno::PERM),
        (Stage::Reap, Errno::IO),
        (Stage::GroupProbe, Errno::PERM),
    ] {
        let mut driver = model().await;
        driver.owner.ops().fail(stage, errno);

        driver.read_result(Pipe::Stdout, Err(io::Error::other("injected read failure")));
        let trace = driver.owner.ops().trace_handle();
        let options =
            options().with_cleanup_graces(Duration::from_millis(5), Duration::from_millis(30));
        let error = within(driver.run(options, CancellationToken::new()))
            .await
            .unwrap_err();
        let partial = error.partial();
        assert_eq!(
            failure(partial, Stage::Stdout).to_string(),
            "injected read failure"
        );
        assert!(
            partial.failures().any(|failure| failure.stage == stage
                && failure.source.raw_os_error() == Some(errno.raw_os_error())),
            "{error}"
        );
        let trace = trace.snapshot();
        assert_signal_order(&trace);
        if stage == Stage::Term {
            let term = trace
                .iter()
                .position(|stage| *stage == Stage::Term)
                .unwrap();
            let kill = trace
                .iter()
                .position(|stage| *stage == Stage::Kill)
                .unwrap();
            assert!(
                term < kill,
                "failed TERM must still be followed by KILL: {trace:?}"
            );
            assert_eq!(stage_count(&trace, Stage::Kill), 1);
        }
        if stage == Stage::Observation {
            assert!(
                !partial
                    .failures()
                    .any(|failure| matches!(failure.stage, Stage::Term | Stage::Kill))
            );
            assert!(
                !trace
                    .iter()
                    .any(|stage| matches!(stage, Stage::Term | Stage::Kill))
            );
        }
    }
}

#[tokio::test]
async fn failed_capture_settles_without_inventing_eof_or_hiding_cleanup_uncertainty() {
    for (stuck_process, stuck_stderr, stuck_group) in [
        (false, false, false),
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let mut driver = model().await;
        driver.stdout.bytes = b"\xff\0partial".to_vec();
        driver.stdout.truncated = true;
        if stuck_process {
            driver.owner.ops().stall_cleanup();
        }
        if stuck_group {
            driver.owner.ops().group_absent_as(false);
        }
        let (reader, _writer) = io::pipe().unwrap();
        if stuck_stderr {
            driver
                .stderr
                .register(ChildStderr::from_std(OwnedFd::from(reader).into()).unwrap());
        }
        driver.read_result(Pipe::Stdout, Err(io::Error::other("original read failure")));
        let trace = driver.owner.ops().trace_handle();
        let budget = Duration::from_millis(40);
        let start = Instant::now();
        let error = timeout(
            Duration::from_secs(2),
            driver.run(
                options().with_cleanup_graces(Duration::ZERO, budget),
                CancellationToken::new(),
            ),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert_eq!(
            matches!(error, ProcessError::CleanupUnconfirmed { .. }),
            stuck_process || stuck_stderr || stuck_group
        );
        if stuck_process || stuck_stderr || stuck_group {
            assert!(start.elapsed() >= budget);
        }
        let state = error.partial();
        assert_eq!(state.status.is_none(), stuck_process);
        assert_eq!(state.group_absent, !(stuck_process || stuck_group));
        assert_eq!(state.stdout.bytes, b"\xff\0partial");
        assert!(state.stdout.truncated && !state.stdout.eof);
        assert_eq!(state.stderr.eof, !stuck_stderr);
        assert_eq!(
            failure(state, Stage::Stdout).to_string(),
            "original read failure"
        );
        let trace = trace.snapshot();
        assert_signal_order(&trace);
        if !stuck_process && !stuck_group {
            assert_eq!(stage_count(&trace, Stage::GroupProbe), 1);
        }
    }
}

#[tokio::test]
async fn final_transition_reaps_without_another_poll() {
    let mut driver = model().await;
    let trace = driver.owner.ops().trace_handle();
    driver.observe();
    assert!(matches!(driver.phase, Phase::Final(_, Cause::Exited)));
    assert!(
        driver.complete(),
        "an exited, EOF-complete fixture needs no further poll"
    );
    let trace = trace.snapshot();
    assert_eq!(stage_count(&trace, Stage::Reap), 1);
    assert_signal_order(&trace);
}

#[tokio::test]
async fn confirmed_group_absence_is_monotonic() {
    let mut driver = model().await;
    driver.kill(Cause::Exited);
    driver.owner.ops().group_absent_as(true);
    driver.finalize();
    assert!(driver.state.group_absent);
    driver.owner.ops().group_absent_as(false);
    let trace = driver.owner.ops().trace_handle();
    trace.clear();
    driver.finalize();
    assert!(driver.state.group_absent);
    assert_eq!(stage_count(&trace.snapshot(), Stage::GroupProbe), 0);
}

#[tokio::test]
async fn cleanup_uncertainty_retains_the_interrupted_phase() {
    for interruption in [
        Interruption::Startup(InterruptionReason::Cancelled),
        Interruption::Startup(InterruptionReason::TimedOut),
        Interruption::Execution(InterruptionReason::Cancelled),
        Interruption::Execution(InterruptionReason::TimedOut),
    ] {
        let mut driver = model().await;
        if matches!(interruption, Interruption::Startup(_)) {
            driver.state.record(
                Stage::PipeSetup,
                io::Error::other("interrupted pipe setup failure"),
            );
            driver.stdout.close_failed();
        }
        driver.owner.ops().stall_cleanup();
        driver.kill(Cause::Interrupted(interruption));
        let error = driver
            .run(
                options().with_cleanup_graces(Duration::ZERO, Duration::ZERO),
                CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, ProcessError::CleanupUnconfirmed { .. }));
        assert_eq!(error.partial().interruption, Some(interruption));
        if matches!(interruption, Interruption::Startup(_)) {
            assert_eq!(
                failure(error.partial(), Stage::PipeSetup).to_string(),
                "interrupted pipe setup failure"
            );
            assert!(!error.partial().stdout.eof);
        }
    }
}

#[tokio::test]
async fn fresh_exit_observation_precedes_pending_control() {
    for cancelled in [false, true] {
        for observed in [Ok(false), Ok(true), Err(Errno::IO)] {
            let mut driver = model().await;
            driver.owner.ops().observe_as(false);
            driver.observe();
            match observed {
                Ok(exited) => driver.owner.ops().observe_as(exited),
                Err(error) => driver.owner.ops().fail(Stage::Observation, error),
            }
            let trace = driver.owner.ops().trace_handle();
            let token = CancellationToken::new();
            let timeout = if !cancelled && observed == Ok(false) {
                Duration::from_millis(50)
            } else {
                Duration::ZERO
            };
            driver.phase = Phase::Running(Instant::now());
            let start = Instant::now();
            if cancelled {
                token.cancel();
            }
            let result = driver
                .run(
                    options()
                        .with_execution_timeout(Some(timeout))
                        .with_cleanup_graces(Duration::ZERO, Duration::MAX),
                    token.clone(),
                )
                .await;
            match observed {
                Ok(true) => assert!(matches!(result.unwrap().completion, Completion::Exited(_))),
                Ok(false) if cancelled => assert!(matches!(
                    result.unwrap().completion,
                    Completion::Cancelled(_)
                )),
                Ok(false) => {
                    assert!(matches!(
                        result.unwrap().completion,
                        Completion::TimedOut(_)
                    ));
                    assert!(start.elapsed() >= timeout);
                }
                Err(errno) => {
                    let Err(error) = result else {
                        panic!("observation failure was hidden")
                    };
                    assert!(matches!(error, ProcessError::SupervisionFailed { .. }));
                    assert!(
                        error
                            .partial()
                            .failures()
                            .any(|failure| failure.stage == Stage::Observation
                                && failure.source.raw_os_error() == Some(errno.raw_os_error()))
                    );
                }
            }
            let snapshot = trace.snapshot();
            token.cancel();
            assert_eq!(trace.snapshot(), snapshot);
            assert_signal_order(&snapshot);
            if observed == Ok(true) {
                assert!(!snapshot.contains(&Stage::Term));
            }
        }
    }
}

#[tokio::test]
async fn term_phase_preserves_its_cause() {
    let mut driver = model().await;
    driver.owner.ops().observe_as(false);
    let cause = Cause::Interrupted(Interruption::Execution(InterruptionReason::TimedOut));
    driver.term(cause);
    driver.phase = Phase::Term(Instant::now() - Duration::from_secs(1), cause);
    driver.owner.ops().observe_as(true);
    let trace = driver.owner.ops().trace_handle();
    let token = CancellationToken::new();
    token.cancel();
    let report = driver
        .run(
            options()
                .with_execution_timeout(Some(Duration::ZERO))
                .with_cleanup_graces(Duration::ZERO, Duration::ZERO),
            token,
        )
        .await
        .unwrap();
    assert!(matches!(report.completion, Completion::TimedOut(_)));
    let trace = trace.snapshot();
    assert_eq!(stage_count(&trace, Stage::Term), 1);
    assert_eq!(stage_count(&trace, Stage::Kill), 1);
}
