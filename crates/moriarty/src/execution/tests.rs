use std::{
    ffi::OsString,
    future::{Ready, pending, poll_fn, ready},
    os::unix::process::ExitStatusExt,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::sync_channel,
    },
    thread,
    time::Duration,
};

use miette::WrapErr;
use tokio::sync::Notify;

use super::*;
use crate::test_helpers::test_report;
use process_supervisor::PartialState;

async fn poll_once(mut future: Pin<&mut (impl Future + ?Sized)>) {
    poll_fn(|context| {
        assert!(
            future.as_mut().poll(context).is_pending(),
            "must retain ownership while pending"
        );
        Poll::Ready(())
    })
    .await;
}

fn assert_error<T>(result: miette::Result<T>, expected: &[&str]) {
    let error = result.err().expect("must fail");
    let rendered = format!("{error:#}");
    for text in expected {
        assert!(rendered.contains(text), "{rendered}");
    }
}

fn pending_process(
    cancellation: &CancellationToken,
) -> (
    CancellationToken,
    oneshot::Sender<()>,
    BoxFuture<'static, miette::Result<ProcessReport>>,
) {
    let running = CancellationToken::new();
    let token = running.clone();
    let (finish, finished) = oneshot::channel();
    let owner = own_process(
        ready(Ok((token, async move {
            finished.await.unwrap();
            Ok(test_report(Completion::Cancelled(ExitStatus::from_raw(0))))
        }))),
        cancellation.clone(),
    );
    (running, finish, Box::pin(owner))
}

fn queued_failure(
    scope: &Scope,
    error: miette::Report,
) -> oneshot::Receiver<miette::Result<ProcessReport>> {
    let (sender, receiver) = oneshot::channel();
    let failures = scope.failures.clone();
    scope.tracker.spawn(async move {
        deliver_job(Err(error), sender, &failures);
    });
    receiver
}

fn spec(program: impl Into<OsString>) -> ProcessSpec {
    ProcessSpec {
        program: program.into(),
        args: Vec::new(),
        cwd: None,
    }
}

#[test]
fn supervisor_errors_never_become_ordinary_discovery_absence() {
    assert!(is_execution_error(
        &ProcessError::CleanupUnconfirmed {
            state: Box::<PartialState>::default(),
        }
        .into()
    ));
    assert!(!is_execution_error(&miette::miette!(
        "ordinary discovery absence"
    )));
}

#[test]
fn only_ordinary_exit_can_succeed() {
    let status = ExitStatus::from_raw(0);
    assert!(
        exited(&test_report(Completion::Exited(status)))
            .unwrap()
            .success()
    );
    for (completion, expected) in [
        (Completion::Cancelled(status), "cancelled"),
        (Completion::TimedOut(status), "timed out"),
    ] {
        assert_error(exited(&test_report(completion)), &[expected]);
    }
}

#[test]
fn presentation_discloses_truncation_without_discard_counts() {
    let mut report = test_report(Completion::Exited(ExitStatus::from_raw(0)));
    report.stdout.bytes = vec![b'x', 0xff];
    report.stdout.truncated = true;
    assert_eq!(present(&report.stdout, None), "x�\n[output truncated]");
    assert_error(complete_output(&report), &["Incomplete"]);
    assert!(exited(&report).unwrap().success());
}

fn assert_sender_dropped(result: Result<(), oneshot::error::RecvError>) {
    let Err(_closed) = result else {
        panic!("pending work must drop its sender")
    };
}

fn pending_work<T>(
    cleanup: bool,
) -> (
    impl Future<Output = T>,
    oneshot::Receiver<()>,
    oneshot::Receiver<()>,
) {
    let (started, ready) = oneshot::channel();
    let (dropped, dropped_rx) = oneshot::channel();
    let work = async move {
        let _notice = dropped;
        if cleanup {
            SUBMISSION
                .with(|scope| record_failure(&scope.failures, &interrupted("cleanup unconfirmed")));
        }
        started.send(()).unwrap();
        pending::<T>().await
    };
    (work, ready, dropped_rx)
}

#[tokio::test]
async fn cli_signal_or_signal_error_drops_work_before_return() {
    for reason in [Ok("SIGINT"), Err("Signal stream failed")] {
        let (work, ready, dropped) = pending_work(false);
        let stop = async {
            ready.await.unwrap();
            reason.map_err(interrupted)
        };
        assert_error(
            run_until(work, stop).await,
            &[reason.unwrap_or("Signal stream failed")],
        );
        assert_sender_dropped(dropped.await);
    }
}

#[tokio::test]
async fn normal_cli_errors_survive_drain() {
    assert_error(
        run_until(
            async { Err(interrupted("injected CLI failure")) },
            pending(),
        )
        .await,
        &["injected CLI failure"],
    );
}

#[tokio::test]
async fn clean_exit_codes_survive_drain() {
    let error = run_until(async { Err(CliExit(1).into()) }, pending())
        .await
        .unwrap_err();
    assert_eq!(error.downcast_ref::<CliExit>().unwrap().0, 1);
}

#[test]
fn primary_and_settlement_causes_survive() {
    assert_error(
        settled::<()>(
            Err(interrupted("nested initiating source")).wrap_err("original blocking cause"),
            Err(interrupted("independent cleanup source")).wrap_err("cleanup unconfirmed"),
        ),
        &[
            "original blocking cause",
            "nested initiating source",
            "cleanup unconfirmed",
            "independent cleanup source",
        ],
    );
}

#[tokio::test]
async fn real_scope_preserves_completed_failure_alongside_settlement_failure() {
    let scope = Scope::new();
    let outcome = scope
        .run(CancellationToken::new(), async {
            SUBMISSION.with(|submission| {
                let cleanup: miette::Result<()> = Err(interrupted("independent cleanup source"));
                record_failure(
                    &submission.failures,
                    &cleanup.wrap_err("cleanup unconfirmed").unwrap_err(),
                )
            });
            Err::<(), _>(interrupted("nested initiating source")).wrap_err("CLI failed")
        })
        .await
        .unwrap();
    assert_error(
        settled(outcome.value, outcome.cleanup),
        &[
            "CLI failed",
            "nested initiating source",
            "cleanup unconfirmed",
            "independent cleanup source",
        ],
    );
    assert_error(
        scope.shutdown().await,
        &["cleanup unconfirmed", "independent cleanup source"],
    );
}

#[tokio::test]
async fn real_scope_preserves_cancellation_alongside_settlement_failure() {
    let scope = Scope::new();
    let (request, token, _) = pending_request(&scope, true).await;
    token.cancel();
    assert_error(
        request.await,
        &["Operation cancelled", "cleanup unconfirmed"],
    );
    assert_error(scope.shutdown().await, &["cleanup unconfirmed"]);
}

#[tokio::test]
async fn closed_submission_rejects_before_startup() {
    let scope = Scope::new();
    scope.close();
    assert_error(
        scope.submit(spec("must-not-start")),
        &["Execution scope closed before admission"],
    );
    scope.shutdown().await.unwrap();
}

#[tokio::test]
async fn accepted_owner_is_tracked_and_scope_cancelled_before_startup() {
    let scope = Scope::new();
    assert!(scope.tracker.is_empty());
    let operation = scope.submit(spec("must-not-start")).unwrap();
    assert_eq!(scope.tracker.len(), 1, "registration must precede startup");
    scope.close();
    assert!(
        operation.cancellation.is_cancelled(),
        "operation must inherit scope cancellation"
    );
    let error = operation.await.unwrap_err();
    assert_eq!(error.to_string(), "Execution scope closed before startup");
    scope.shutdown().await.unwrap();
    assert!(scope.tracker.is_empty());
}

type RequestResult = miette::Result<ScopedOutcome<()>>;

async fn pending_request(
    scope: &Arc<Scope>,
    cleanup: bool,
) -> (
    BoxFuture<'_, RequestResult>,
    CancellationToken,
    oneshot::Receiver<()>,
) {
    let token = CancellationToken::new();
    let (work, ready, dropped_rx) = pending_work(cleanup);
    let mut request = Box::pin(scope.run(token.clone(), work));
    tokio::select! { biased; _ = &mut request => panic!("work must stay pending"), result = ready => result.unwrap() }
    (request, token, dropped_rx)
}

#[tokio::test]
async fn dropped_request_receiver_drops_work_and_drains_real_scope() {
    let scope = Scope::new();
    let (request, _, dropped) = pending_request(&scope, false).await;
    drop(request);
    assert_sender_dropped(
        tokio::time::timeout(Duration::from_secs(2), dropped)
            .await
            .unwrap(),
    );
    scope.shutdown().await.unwrap();
    assert!(scope.tracker.is_empty());
}

#[tokio::test]
async fn real_scope_shutdown_surfaces_first_job_settlement_failure() {
    let state = Box::<PartialState>::default;
    for error in [
        ProcessError::CleanupUnconfirmed { state: state() },
        ProcessError::StartupInterrupted { state: state() },
        ProcessError::SupervisionFailed { state: state() },
    ] {
        let scope = Scope::new();
        let expected = error.to_string();
        let receiver = queued_failure(&scope, error.into());
        assert_eq!(receiver.await.unwrap().unwrap_err().to_string(), expected);
        assert_eq!(scope.shutdown().await.unwrap_err().to_string(), expected);
    }
}

#[tokio::test]
async fn lost_job_receiver_preserves_even_an_ordinary_start_failure() {
    let scope = Scope::new();
    let error = ProcessError::StartFailed {
        state: Box::<PartialState>::default(),
    };
    let expected = error.to_string();
    let failures = scope.failures.clone();
    let (result, receiver) = oneshot::channel();
    drop(receiver);
    scope.tracker.spawn(async move {
        deliver_job(Err(error.into()), result, &failures);
        let (later, _receiver) = oneshot::channel();
        deliver_job(
            Err(interrupted("later failure must not replace the first")),
            later,
            &failures,
        );
    });
    assert_eq!(scope.shutdown().await.unwrap_err().to_string(), expected);
}

#[tokio::test]
async fn closed_and_precancelled_scopes_reject_admission() {
    let scope = Scope::new();
    let token = CancellationToken::new();
    token.cancel();
    assert_error(
        scope.run(token, async { panic!("must not start") }).await,
        &["Operation cancelled before admission"],
    );
    scope.shutdown().await.unwrap();
    assert_error(
        scope.run(CancellationToken::new(), async { 1 }).await,
        &["Operation cancelled before admission"],
    );
}

#[tokio::test]
async fn cancellation_during_work_poll_rejects_new_admission() {
    for (nested, label) in [(false, "direct"), (true, "nested")] {
        let scope = Scope::new();
        let token = CancellationToken::new();
        let cancelling = token.clone();
        let (entered, entering) = sync_channel(1);
        let (cancelled, cancelling_done) = sync_channel(1);
        let canceller = thread::spawn(move || {
            entering.recv_timeout(Duration::from_secs(2)).unwrap();
            cancelling.cancel();
            cancelled.send(()).unwrap();
        });
        let (observed, observation) = oneshot::channel();
        let mut observed = Some(observed);
        let writes = Arc::new(AtomicUsize::new(0));
        let admitted = writes.clone();
        let work = poll_fn(move |context| {
            entered.send(()).unwrap();
            cancelling_done
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
            // The owner cannot poll its outer select until this poll returns; a relay cannot enforce admission here.
            let submission_rejected = submit(spec("must-not-start")).is_err();
            let admitted = admitted.clone();
            let mut persistence = Box::pin(retain_persistence(async move {
                admitted.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }));
            let persistence_rejected =
                matches!(persistence.as_mut().poll(context), Poll::Ready(Err(_)));
            observed
                .take()
                .unwrap()
                .send((submission_rejected, persistence_rejected))
                .unwrap();
            Poll::<()>::Pending
        });
        let work = async move {
            if nested {
                in_scope(work).await.map(|_| ())
            } else {
                work.await;
                Ok(())
            }
        };
        let result = tokio::time::timeout(Duration::from_secs(2), scope.run(token, work))
            .await
            .unwrap();
        canceller.join().unwrap();
        assert_error(result, &["Operation cancelled"]);
        assert_eq!(
            observation.await.unwrap(),
            (true, true),
            "{label}: request cancellation must be visible before another admission"
        );
        assert_eq!(writes.load(Ordering::SeqCst), 0);
        scope.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn cancelled_startup_does_not_repoll_a_ready_launch() {
    let cancellation = CancellationToken::new();
    let (release, released) = oneshot::channel();
    let launches = Arc::new(AtomicUsize::new(0));
    let started = launches.clone();
    let startup = async move {
        released.await.unwrap();
        started.fetch_add(1, Ordering::SeqCst);
        Ok((
            CancellationToken::new(),
            ready(Ok(test_report(Completion::Exited(ExitStatus::from_raw(0))))),
        ))
    };
    let mut owner = Box::pin(own_process(startup, cancellation.clone()));
    poll_once(owner.as_mut()).await;
    cancellation.cancel();
    release.send(()).unwrap();
    assert_error(
        owner.await,
        &[
            "Startup cancelled",
            "creation and late cleanup may be uncertain",
        ],
    );
    assert_eq!(launches.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn request_cancellation_is_local_and_host_drain_waits() {
    let (started, ready) = oneshot::channel::<(CancellationToken, oneshot::Sender<()>)>();
    let mut started = Some(started);
    test_seams::with_operations(
        move |_, cancellation| {
            let (running, finish, owner) = pending_process(&cancellation);
            started.take().unwrap().send((running, finish)).unwrap();
            owner
        },
        async {
            let scope = SUBMISSION.with(Clone::clone);
            let token = CancellationToken::new();
            let mut first = Box::pin(scope.run(token.clone(), execute(spec("must-not-start"))));
            poll_once(first.as_mut()).await;
            let (running, finish) = ready.await.unwrap();
            token.cancel();
            running.cancelled().await;
            poll_once(first.as_mut()).await;
            let independent = CancellationToken::new();
            let completed = scope.run(independent.clone(), async { 42 }).await.unwrap();
            assert_eq!(completed.value, 42);
            completed.cleanup.unwrap();
            assert!(!independent.is_cancelled());
            let mut drain = Box::pin(scope.shutdown());
            poll_once(drain.as_mut()).await;
            poll_once(first.as_mut()).await;
            finish.send(()).unwrap();
            assert_error(first.await, &["Operation cancelled"]);
            drain.await.unwrap();
            assert!(scope.tracker.is_empty());
        },
    )
    .await;
}

#[tokio::test]
async fn panicking_work_drains_admitted_process_before_delivery() {
    let (started, ready) = oneshot::channel::<(CancellationToken, oneshot::Sender<()>)>();
    let mut started = Some(started);
    tokio::time::timeout(
        Duration::from_secs(2),
        test_seams::with_operations(
            move |_, cancellation| {
                let (running, finish, owner) = pending_process(&cancellation);
                started.take().unwrap().send((running, finish)).unwrap();
                owner
            },
            async {
                let scope = SUBMISSION.with(Clone::clone);
                let (panic, panicking) = oneshot::channel();
                let mut request = Box::pin(scope.run(CancellationToken::new(), async move {
                    let _operation = submit(spec("must-not-start")).unwrap();
                    panicking.await.unwrap();
                    panic!("injected work panic");
                }));
                poll_once(request.as_mut()).await;
                let (running, finish) = ready.await.unwrap();
                panic.send(()).unwrap();
                running.cancelled().await;
                poll_once(request.as_mut()).await;
                let mut drain = Box::pin(scope.shutdown());
                poll_once(drain.as_mut()).await;
                finish.send(()).unwrap();
                assert_error(request.await, &["Operation panicked"]);
                drain.await.unwrap();
                assert!(scope.tracker.is_empty());
            },
        ),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn panicking_work_drains_persistence_and_retains_failure_after_caller_loss() {
    tokio::time::timeout(Duration::from_secs(2), async {
        let scope = Scope::new();
        let (admitted, admission) = oneshot::channel();
        let (finish, finished) = oneshot::channel();
        let (panic, panicking) = oneshot::channel();
        let mut request = Box::pin(scope.run(CancellationToken::new(), async move {
            let cancellation = SUBMISSION.with(|scope| scope.cancellation.clone());
            let mut persistence = Box::pin(retain_persistence(async move {
                admitted.send(cancellation).unwrap();
                finished.await.unwrap();
                Err(interrupted("injected persistence cause")).wrap_err("approval write failed")
            }));
            poll_once(persistence.as_mut()).await;
            panicking.await.unwrap();
            panic!("injected work panic");
        }));
        poll_once(request.as_mut()).await;
        let cancellation = admission.await.unwrap();
        panic.send(()).unwrap();
        cancellation.cancelled().await;
        drop(request);
        let mut drain = Box::pin(scope.shutdown());
        poll_once(drain.as_mut()).await;
        finish.send(()).unwrap();
        assert_error(
            drain.await,
            &[
                "Operation panicked",
                "approval write failed",
                "injected persistence cause",
            ],
        );
        assert!(scope.tracker.is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dropped_unstarted_operation_has_clean_shutdown() {
    let completed = Arc::new(Notify::new());
    let observed = completed.clone();
    let scope = Scope::with_process(
        CancellationToken::new(),
        Arc::new(move |spec, cancellation| {
            let process = start_process(spec, cancellation);
            let completed = observed.clone();
            Box::pin(async move {
                let outcome = process.await;
                completed.notify_one();
                outcome
            })
        }),
        None,
    );
    drop(scope.submit(spec("must-not-start")).unwrap());
    tokio::time::timeout(Duration::from_secs(2), completed.notified())
        .await
        .unwrap();
    assert!(scope.tracker.is_empty());
    assert!(!scope.cancellation.is_cancelled());
    scope.shutdown().await.unwrap();
}

#[tokio::test]
async fn startup_handoff_preserves_report() {
    let completion = own_process(
        ready(Ok((
            CancellationToken::new(),
            ready(Ok(test_report(Completion::Exited(ExitStatus::from_raw(0))))),
        ))),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(matches!(completion.completion, Completion::Exited(_)));
}

#[tokio::test]
async fn precancelled_startup_is_never_polled() {
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let error = own_process::<Ready<_>>(async { panic!("must not poll startup") }, cancellation)
        .await
        .expect_err("precancelled startup must fail");
    assert!(matches!(
        error.downcast_ref::<ExecutionError>(),
        Some(ExecutionError::CancelledBeforeLaunch)
    ));
    assert!(is_execution_error(&error));
    let scope = Scope::new();
    let delivered = queued_failure(&scope, error);
    assert_error(
        delivered.await.unwrap(),
        &["Startup cancelled before launch"],
    );
    scope.shutdown().await.unwrap();
}

#[tokio::test]
async fn pending_startup_is_dropped_without_waiting_for_a_launcher() {
    let cancellation = CancellationToken::new();
    let (work, _ready, dropped) = pending_work(false);
    let mut startup = Box::pin(own_process::<Ready<_>>(work, cancellation.clone()));
    poll_once(startup.as_mut()).await;
    cancellation.cancel();
    assert_error(
        startup.await,
        &["creation and late cleanup may be uncertain"],
    );
    assert_sender_dropped(dropped.await);
}

#[tokio::test]
async fn all_running_cancellations_precede_concurrent_settlement() {
    let cancellation = CancellationToken::new();
    let (first_token, finish_first, first) = pending_process(&cancellation);
    let (second_token, finish_second, second) = pending_process(&cancellation);
    let mut waits = Box::pin(async { tokio::join!(first, second) });
    poll_once(waits.as_mut()).await;
    cancellation.cancel();
    poll_once(waits.as_mut()).await;
    assert!(first_token.is_cancelled() && second_token.is_cancelled());
    finish_second.send(()).unwrap();
    finish_first.send(()).unwrap();
    let (first, second) = waits.await;
    assert!(matches!(
        [first.unwrap().completion, second.unwrap().completion],
        [Completion::Cancelled(_), Completion::Cancelled(_)]
    ));
}

#[tokio::test]
async fn unscoped_execute_and_submit_share_completed_reports() {
    assert!(SUBMISSION.try_with(|_| ()).is_err());
    for direct in [false, true] {
        let mut spec = spec("printf");
        spec.args.push("ok".into());
        let report = if direct {
            execute(spec).await
        } else {
            submit(spec).unwrap().await
        }
        .unwrap();
        assert!(exited(&report).unwrap().success());
        assert_eq!(report.stdout.bytes, b"ok");
        complete_output(&report).unwrap();
    }
}

#[tokio::test]
async fn registration_precedes_startup_and_receiver_loss_settles() {
    let (started, ready) = oneshot::channel::<(CancellationToken, oneshot::Sender<()>)>();
    let mut started = Some(started);
    test_seams::with_operations(
        move |_, cancellation| {
            let (running, finish, owner) = pending_process(&cancellation);
            started.take().unwrap().send((running, finish)).unwrap();
            owner
        },
        async {
            let scope = SUBMISSION.with(Clone::clone);
            let operation = submit(spec("must-not-start")).unwrap();
            let (running, finish) = ready.await.unwrap();
            assert_eq!(scope.tracker.len(), 1);
            drop(operation);
            running.cancelled().await;
            let mut drain = Box::pin(scope.shutdown());
            poll_once(drain.as_mut()).await;
            finish.send(()).unwrap();
            drain.await.unwrap();
            assert!(scope.tracker.is_empty());
        },
    )
    .await;
}
