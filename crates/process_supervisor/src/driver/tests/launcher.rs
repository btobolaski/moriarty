use std::{
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    process::Stdio,
    sync::{Arc, Mutex, mpsc},
};

use rustix::{
    io::Errno,
    process::{Pid, WaitId, WaitIdOptions, waitid},
};
use tempfile::TempDir;
use tokio::time::error::Elapsed;

use super::*;
use crate::launch::Admission;

fn startup_options(reason: InterruptionReason) -> ProcessOptions {
    options().with_startup_timeout(match reason {
        InterruptionReason::Cancelled => Duration::MAX,
        InterruptionReason::TimedOut => SHORT_STARTUP,
    })
}

#[tokio::test]
async fn dropping_unpolled_startup_never_launches() {
    let allocations = Arc::new(Mutex::new(Vec::new()));
    let allocated = allocations.clone();
    drop(spawn_with(
        spec(),
        options(),
        Hooks {
            before_allocate: Some(Box::new(move |limit| {
                allocated.lock().unwrap().push(limit);
                Ok(())
            })),
            before_spawn: Some(Box::new(|_| panic!("unpolled startup launched"))),
            ..Hooks::default()
        },
    ));
    tokio::task::yield_now().await;
    assert!(allocations.lock().unwrap().is_empty());
}

#[tokio::test]
async fn interrupted_startup_never_launches() {
    for reason in [InterruptionReason::Cancelled, InterruptionReason::TimedOut] {
        let token = CancellationToken::new();
        let mut options = options();
        match reason {
            InterruptionReason::Cancelled => token.cancel(),
            InterruptionReason::TimedOut => options = options.with_startup_timeout(Duration::ZERO),
        }
        let error = within(prepare(
            spec(),
            &options,
            &token,
            Instant::now(),
            Hooks {
                before_spawn: Some(Box::new(|_| panic!("interrupted startup launched"))),
                ..Hooks::default()
            },
        ))
        .await
        .err()
        .unwrap();
        assert!(matches!(error, ProcessError::StartupInterrupted { .. }));
        let state = error.partial();
        assert_eq!(state.interruption, Some(Interruption::Startup(reason)));
    }
}

#[tokio::test]
async fn blocked_admission_is_cancellable() {
    let token = CancellationToken::new();
    let mut hooks = Hooks {
        admission: Some(Admission::new(0)),
        ..Hooks::default()
    };
    let huge = options().with_startup_timeout(Duration::MAX);
    {
        let pending = launch::admit(&huge, &token, Instant::now(), &mut hooks);
        tokio::pin!(pending);
        let blocked = timeout(Duration::from_millis(5), &mut pending).await;
        let _: Elapsed = blocked
            .err()
            .expect("admission must remain blocked until cancellation");
        token.cancel();
        assert_eq!(
            within(&mut pending).await.err(),
            Some(InterruptionReason::Cancelled)
        );
    }
}

#[tokio::test]
async fn interruption_after_admission_never_launches() {
    let admission = Admission::new(1);
    let token = CancellationToken::new();
    let mut state = PartialState::default();
    let mut hooks = Hooks {
        admission: Some(admission.clone()),
        before_spawn: Some(Box::new(|_| panic!("cancelled launch spawned"))),
        ..Hooks::default()
    };
    let options = options();
    let start = Instant::now();
    let permit = launch::admit(&options, &token, start, &mut hooks)
        .await
        .unwrap();
    token.cancel();
    let outcome = launch::launch(spec(), &options, &token, start, &mut state, permit, hooks).await;
    assert!(matches!(
        outcome,
        LaunchOutcome::Failure {
            unconfirmed: false,
            reason: Some(InterruptionReason::Cancelled)
        }
    ));
    assert_eq!(state.failures().count(), 0);
    assert_eq!(admission.available_permits(), 1);
}

#[tokio::test]
async fn cancelled_empty_handoff_retains_launcher_ownership() {
    let admission = Admission::new(1);
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (done_tx, done_rx) = oneshot::channel();
    let token = CancellationToken::new();
    let driver_token = token.clone();
    let hooks = Hooks {
        admission: Some(admission.clone()),
        before_spawn: Some(Box::new(move |_| {
            let _ = entered_tx.send(());
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        })),
        after_send: Some(Box::new(move || {
            let _ = done_tx.send(());
        })),
        ..Hooks::default()
    };
    let task = tokio::spawn(async move {
        prepare(
            spec(),
            &ProcessOptions::default().with_startup_timeout(Duration::MAX),
            &driver_token,
            Instant::now(),
            hooks,
        )
        .await
        .err()
        .expect("cancelled startup")
    });
    within(entered_rx).await.unwrap();
    token.cancel();
    let error = within(task).await.unwrap();
    assert!(matches!(error, ProcessError::StartupInterrupted { .. }));
    assert!(error.partial().launch_unconfirmed);
    assert_eq!(admission.available_permits(), 0);
    let blocked = returned_error(
        options().with_startup_timeout(SHORT_STARTUP),
        Hooks {
            admission: Some(admission.clone()),
            before_spawn: Some(Box::new(|_| panic!("occupied admission launched"))),
            ..Hooks::default()
        },
    )
    .await;
    assert!(!blocked.partial().launch_unconfirmed);
    release_tx.send(()).unwrap();
    within(done_rx).await.unwrap();
    within(async {
        while admission.available_permits() != 1 {
            sleep(POLL).await;
        }
    })
    .await;
}

#[tokio::test]
async fn capture_allocations_fail_fast_before_launch() {
    for failing_limit in [11, 23] {
        let allocations = Arc::new(Mutex::new(Vec::new()));
        let allocated = allocations.clone();
        let error = within(spawn_with(
            spec(),
            options().with_capture_limits(11, 23).unwrap(),
            Hooks {
                before_allocate: Some(Box::new(move |limit| {
                    allocated.lock().unwrap().push(limit);
                    if limit == failing_limit {
                        Err(io::Error::other(format!(
                            "injected allocation failure: {limit}"
                        )))
                    } else {
                        Ok(())
                    }
                })),
                before_spawn: Some(Box::new(|_| panic!("allocation failure launched"))),
                ..Hooks::default()
            },
        ))
        .await
        .err()
        .expect("allocation must fail");
        let error = error.downcast_ref::<ProcessError>().unwrap();
        assert!(matches!(error, ProcessError::StartFailed { .. }));
        let state = error.partial();
        assert_eq!(
            failure(state, Stage::Allocation).to_string(),
            format!("injected allocation failure: {failing_limit}")
        );
        let expected = if failing_limit == 11 {
            vec![11]
        } else {
            vec![11, 23]
        };
        assert_eq!(*allocations.lock().unwrap(), expected);
    }
}

#[tokio::test]
async fn missing_program_preserves_spawn_stage() {
    let mut missing = spec();
    missing.program = "/nonexistent-process-supervisor-test/program".into();
    missing.cwd = Some("/nonexistent-process-supervisor-test/cwd".into());
    let expected_cwd = missing.cwd.clone();
    let raw = OsString::from_vec(vec![0xff]);
    missing.args.push(raw.clone());
    let error = spawn_with(
        missing,
        options(),
        Hooks {
            before_spawn: Some(Box::new(move |command| {
                assert_eq!(command.get_args().last(), Some(raw.as_os_str()));
                assert_eq!(command.get_current_dir(), expected_cwd.as_deref());
                assert_eq!(command.get_envs().count(), 0);
            })),
            ..Hooks::default()
        },
    )
    .await
    .err()
    .unwrap();
    let error = error.downcast_ref::<ProcessError>().unwrap();
    assert!(matches!(error, ProcessError::StartFailed { .. }));
    assert_eq!(
        failure(error.partial(), Stage::Spawn).raw_os_error(),
        Some(Errno::NOENT.raw_os_error())
    );
}

#[tokio::test]
async fn dropped_handoff_is_unconfirmed() {
    let error = returned_error(
        options(),
        Hooks {
            before_spawn: Some(Box::new(|_| panic!("injected launcher panic"))),
            ..Hooks::default()
        },
    )
    .await;
    assert!(matches!(error, ProcessError::CleanupUnconfirmed { .. }));
    assert!(error.partial().launch_unconfirmed);
    failure(error.partial(), Stage::Launcher);
}

#[tokio::test]
async fn execution_clock_starts_after_pipe_setup() {
    static SETUP_END: Mutex<Option<Instant>> = Mutex::new(None);
    let token = CancellationToken::new();
    let options = options()
        .with_startup_timeout(Duration::MAX)
        .with_execution_timeout(Some(Duration::MAX));
    let driver = within(prepare(
        spec(),
        &options,
        &token,
        Instant::now(),
        Hooks {
            before_spawn: Some(Box::new(|_| thread::sleep(Duration::from_millis(30)))),
            pipe_setup: Some(|_| {
                thread::sleep(Duration::from_millis(30));
                *SETUP_END.lock().unwrap() = Some(Instant::now());
                Ok(())
            }),
            ..Hooks::default()
        },
    ))
    .await
    .unwrap();
    let Phase::Running(start) = driver.phase else {
        panic!("startup not complete")
    };
    assert!(start >= SETUP_END.lock().unwrap().unwrap());
    within(driver.run(options, token)).await.unwrap();
}

#[tokio::test]
async fn interrupted_pipe_setup_cleans_child() {
    let error = returned_error(
        options(),
        Hooks {
            pipe_setup: Some(|token| {
                token.cancel();
                Ok(())
            }),
            ..Hooks::default()
        },
    )
    .await;
    assert!(matches!(error, ProcessError::StartupInterrupted { .. }));
    let state = error.partial();
    assert_eq!(
        state.interruption,
        Some(Interruption::Startup(InterruptionReason::Cancelled))
    );
    assert!(state.child_id.is_some() && state.status.is_some() && state.kill_attempted);
    assert!(state.group_absent && state.stdout.eof && state.stderr.eof);
}

#[tokio::test]
async fn failed_pipe_setup_preserves_startup_interruption() {
    for reason in [InterruptionReason::Cancelled, InterruptionReason::TimedOut] {
        let hook: fn(&CancellationToken) -> io::Result<()> = match reason {
            InterruptionReason::Cancelled => |token| {
                token.cancel();
                Err(io::Error::other("interrupted pipe setup failure"))
            },
            InterruptionReason::TimedOut => |_| {
                thread::sleep(SHORT_STARTUP);
                Err(io::Error::other("interrupted pipe setup failure"))
            },
        };
        let options = startup_options(reason);
        let error = returned_error(
            options,
            Hooks {
                pipe_setup: Some(hook),
                ..Hooks::default()
            },
        )
        .await;
        assert!(
            matches!(error, ProcessError::StartupInterrupted { .. }),
            "{error}"
        );
        let state = error.partial();
        assert_eq!(state.interruption, Some(Interruption::Startup(reason)));
        assert_eq!(
            failure(state, Stage::PipeSetup).to_string(),
            "interrupted pipe setup failure"
        );
        assert!(state.status.is_some() && state.group_absent && state.kill_attempted);
        assert!(!state.stdout.eof && !state.stderr.eof && !state.term_attempted);
    }
}

#[tokio::test]
async fn pipe_setup_failures_preserve_stage() {
    for (hooks, source) in [
        (
            Hooks {
                pipe_setup: Some(|_| Err(io::Error::other("injected pipe setup failure"))),
                ..Hooks::default()
            },
            "injected pipe setup failure",
        ),
        (
            Hooks {
                before_spawn: Some(Box::new(|command| {
                    command.stderr(Stdio::null());
                })),
                ..Hooks::default()
            },
            "missing stderr pipe",
        ),
    ] {
        let error = returned_error(options(), hooks).await;
        assert!(matches!(error, ProcessError::StartFailed { .. }), "{error}");
        let failure = failure(error.partial(), Stage::PipeSetup).to_string();
        assert_eq!(failure, source);
        assert!(error.partial().kill_attempted && error.partial().group_absent);
    }
}

#[tokio::test]
async fn thread_creation_failure_preserves_launcher_stage() {
    let error = returned_error(
        options(),
        Hooks {
            fail_thread: true,
            ..Hooks::default()
        },
    )
    .await;
    assert!(matches!(error, ProcessError::StartFailed { .. }), "{error}");
    assert_eq!(
        failure(error.partial(), Stage::Launcher).to_string(),
        "injected thread creation failure"
    );
}

#[tokio::test]
async fn interrupted_handoffs_preserve_known_spawn_and_cleanup_outcomes() {
    for spawn_fails in [false, true] {
        for reason in [InterruptionReason::Cancelled, InterruptionReason::TimedOut] {
            let dir = TempDir::new().unwrap();
            let mut spec = spec();
            if spawn_fails {
                spec.program = dir.path().join("absent").into();
            }
            let (sent_tx, sent_rx) = oneshot::channel();
            let (resume_tx, resume_rx) = oneshot::channel();
            let token = CancellationToken::new();
            let driver_token = token.clone();
            let options = startup_options(reason);
            let startup_budget = options.startup_timeout;
            let task = tokio::spawn(async move {
                prepare(
                    spec,
                    &options,
                    &driver_token,
                    Instant::now(),
                    Hooks {
                        after_send: Some(Box::new(move || {
                            let _ = sent_tx.send(());
                        })),
                        before_receive: Some(resume_rx),
                        ..Hooks::default()
                    },
                )
                .await
                .err()
                .expect("startup must be interrupted")
            });
            within(sent_rx).await.unwrap();
            match reason {
                InterruptionReason::Cancelled => token.cancel(),
                InterruptionReason::TimedOut => sleep(startup_budget).await,
            }
            resume_tx.send(()).unwrap();
            let error = within(task).await.unwrap();
            assert!(
                matches!(error, ProcessError::StartupInterrupted { .. }),
                "{error}"
            );
            let state = error.partial();
            assert_eq!(state.interruption, Some(Interruption::Startup(reason)));
            assert!(!state.launch_unconfirmed);
            if spawn_fails {
                assert_eq!(
                    failure(state, Stage::Spawn).raw_os_error(),
                    Some(Errno::NOENT.raw_os_error())
                );
            } else {
                assert_eq!(state.failures().count(), 0);
            }
        }
    }
}

#[tokio::test]
async fn dropped_spawn_cleans_late_children_during_creation_and_send() {
    for during_send in [false, true] {
        let admission = Admission::new(1);
        let (entered_tx, entered_rx) = oneshot::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (created_tx, created_rx) = oneshot::channel();
        let (closed_tx, closed_rx) = oneshot::channel();
        let (done_tx, done_rx) = oneshot::channel();
        let block = move || {
            let _ = entered_tx.send(());
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        };
        let mut hooks = Hooks {
            admission: Some(admission.clone()),
            child_created: Some(created_tx),
            receiver_closed: Some(closed_tx),
            after_send: Some(Box::new(move || {
                let _ = done_tx.send(());
            })),
            ..Hooks::default()
        };
        if during_send {
            hooks.before_send = Some(Box::new(block));
        } else {
            hooks.before_spawn = Some(Box::new(move |_| block()));
        }
        let task = tokio::spawn(spawn_with(
            spec(),
            ProcessOptions::default().with_startup_timeout(Duration::MAX),
            hooks,
        ));
        within(entered_rx).await.unwrap();
        task.abort();
        assert!(within(task).await.err().unwrap().is_cancelled());
        // Abort cancels the public future, not the driver. Closure must precede release
        // so the send fails and after_send acknowledges completed launcher cleanup.
        within(closed_rx).await.unwrap();
        assert_eq!(admission.available_permits(), 0);
        release_tx.send(()).unwrap();
        let pid = within(created_rx).await.unwrap();
        within(done_rx).await.unwrap();
        let observed = waitid(
            WaitId::Pid(Pid::from_raw(pid as i32).unwrap()),
            WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
        );
        assert!(
            matches!(observed, Err(Errno::CHILD)),
            "late child must already be reaped"
        );
        within(async {
            while admission.available_permits() != 1 {
                sleep(POLL).await;
            }
        })
        .await;
    }
}
