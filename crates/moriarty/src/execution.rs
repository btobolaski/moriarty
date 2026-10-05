//! Completion owners outlive callers so receiver loss requests cancellation, not task abortion.
//! Submission helpers are token-free; operation tokens inherit their owning scope's cancellation.
//! Admission closes before broadcast cancellation and concurrent tracker drain. Default supervision
//! keeps 1 MiB raw bytes per stream, 5 s startup, no execution deadline, and independent 2 s TERM /
//! 1 s post-KILL budgets. Startup cancellation relinquishes a pending launch without a join;
//! creation/late cleanup uncertainty stays explicit. Runtime destruction is not graceful settlement.
//! Blocking stdout from `moriarty hooks exec`, `moriarty test checks` and `moriarty test project-tools`
//! can prevent cooperative polling; no output timeout or graceful signal exit is promised then.
//! SIGKILL may be required and proves no cleanup/persistence. That owner-accepted exception does not
//! waive MCP output shutdown, cancellation/admission or owned-work settlement.
//! OS identity/signaling/reaping/capture assurance belongs solely to `process_supervisor` tests;
//! Moriarty verifies routing, ownership, and caller policy. The inherited-stdio installer is excluded.

use std::{
    error::Error,
    fmt::{self, Display, Formatter},
    future::{Future, pending},
    panic::AssertUnwindSafe,
    pin::Pin,
    process::ExitStatus,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use futures::{FutureExt, future::BoxFuture};
use miette::IntoDiagnostic;
use tokio::{
    signal::unix::{Signal, SignalKind, signal},
    sync::oneshot,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::error;

use process_supervisor::{
    Capture, Completion, ProcessError, ProcessOptions, ProcessReport, ProcessSpec, RunningProcess,
};

pub(crate) const OUTPUT_TRUNCATED_NOTICE: &str = "[output truncated]";

/// Cancellation remains an execution failure for callers, but confirmed no-launch cancellation
/// must not contaminate cleanup evidence or be mistaken for ordinary discovery absence.
#[derive(Debug, miette::Diagnostic)]
pub(crate) enum ExecutionError {
    CancelledBeforeLaunch,
    Interrupted(String),
}

impl Display for ExecutionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::CancelledBeforeLaunch => formatter.write_str("Startup cancelled before launch"),
            Self::Interrupted(message) => message.fmt(formatter),
        }
    }
}

impl Error for ExecutionError {}

/// Retains the intended CLI exit code until execution drain and terminal restoration finish.
#[derive(Debug, miette::Diagnostic)]
pub(crate) struct CliExit(pub(crate) i32);

impl Display for CliExit {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "Command failed with exit code {}", self.0)
    }
}

impl Error for CliExit {}

/// Only owning CLI/MCP boundaries consume Unix shutdown signals; a closed stream is failure,
/// not evidence of successful graceful shutdown.
pub(crate) struct ShutdownSignals {
    interrupt: Signal,
    terminate: Signal,
}

impl ShutdownSignals {
    pub(crate) fn new() -> miette::Result<Self> {
        Ok(Self {
            interrupt: signal(SignalKind::interrupt()).into_diagnostic()?,
            terminate: signal(SignalKind::terminate()).into_diagnostic()?,
        })
    }

    pub(crate) async fn recv(&mut self) -> miette::Result<&'static str> {
        let received = tokio::select! {
            value = self.interrupt.recv() => value.map(|_| "SIGINT"),
            value = self.terminate.recv() => value.map(|_| "SIGTERM"),
        };
        received.ok_or_else(|| interrupted("Shutdown signal stream closed"))
    }
}

/// Keeps the initiating error first and appends cleanup failure; successful work cannot hide
/// uncertain settlement. A clean drain preserves typed errors such as `CliExit`.
pub(crate) fn settled<T>(
    outcome: miette::Result<T>,
    cleanup: miette::Result<()>,
) -> miette::Result<T> {
    match (outcome, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => {
            Err(interrupted(format!("{error:#}\nSettlement: {cleanup:#}")))
        }
    }
}

pub(crate) async fn run_with_signals(
    work: impl Future<Output = miette::Result<()>> + Send + 'static,
) -> miette::Result<()> {
    let mut signals = ShutdownSignals::new()?;
    run_until(work, signals.recv()).await
}

async fn run_until(
    work: impl Future<Output = miette::Result<()>> + Send + 'static,
    stop: impl Future<Output = miette::Result<&'static str>>,
) -> miette::Result<()> {
    let scope = Scope::new();
    // CLI signals cancel the host scope; an unused request trigger keeps the same owner path as MCP.
    let operation = async {
        let outcome = scope.run(CancellationToken::new(), work).await?;
        settled(outcome.value, outcome.cleanup)
    };
    tokio::pin!(operation);
    let outcome = tokio::select! {
        biased;
        reason = stop => {
            scope.close();
            let completion = operation.await;
            let interruption = match reason {
                Ok(reason) => Err(interrupted(format!("Interrupted by {reason}"))),
                Err(error) => Err(error),
            };
            settled(interruption, completion.map(|_| ()))
        },
        result = &mut operation => result,
    };
    settled(outcome, scope.shutdown().await)
}

// Policy fixtures vary only the process result; admission, cancellation, and drain stay real.
type StartProcess = dyn Fn(ProcessSpec, CancellationToken) -> BoxFuture<'static, miette::Result<ProcessReport>>
    + Send
    + Sync;

tokio::task_local! {
    static SUBMISSION: Arc<Scope>;
}

/// Completed work is not proof of cleanup; boundaries must retain `value` while reporting
/// `cleanup` uncertainty rather than letting one replace the other.
pub(crate) struct ScopedOutcome<T> {
    pub(crate) value: T,
    pub(crate) cleanup: miette::Result<()>,
}

/// Retains completion owners after caller loss. Drop requests cancellation; awaited shutdown
/// reports settlement, requiring a live runtime and retaining the supervisor's late-launch limits.
pub(crate) struct Scope {
    open: Mutex<bool>,
    tracker: TaskTracker,
    cancellation: CancellationToken,
    process: Arc<StartProcess>,
    request_cancellation: Option<CancellationToken>,
    failures: Arc<Mutex<Option<String>>>,
}

impl Scope {
    pub(crate) fn new() -> Arc<Self> {
        Self::with_process(CancellationToken::new(), Arc::new(start_process), None)
    }

    fn with_process(
        cancellation: CancellationToken,
        process: Arc<StartProcess>,
        request_cancellation: Option<CancellationToken>,
    ) -> Arc<Self> {
        Arc::new(Self {
            open: Mutex::new(true),
            tracker: TaskTracker::new(),
            cancellation,
            process,
            request_cancellation,
            failures: Arc::new(Mutex::new(None)),
        })
    }

    fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
            || self
                .request_cancellation
                .as_ref()
                .is_some_and(CancellationToken::is_cancelled)
    }

    /// Admission/startup inspect the actual request flag even inside a work poll; the outer select
    /// alone cannot prevent later work. Receiver loss drops work, not its completion owner, which
    /// settles the request scope before delivery and keeps work and cleanup evidence separate.
    pub(crate) async fn run<F, T>(
        self: &Arc<Self>,
        cancellation: CancellationToken,
        work: F,
    ) -> miette::Result<ScopedOutcome<T>>
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let (mut sender, receiver) = oneshot::channel();
        {
            let open = self.open.lock().unwrap();
            if !*open || cancellation.is_cancelled() || self.is_cancelled() {
                return Err(interrupted("Operation cancelled before admission"));
            }
            let child = Self::with_process(
                self.cancellation.child_token(),
                self.process.clone(),
                Some(cancellation.clone()),
            );
            let failures = self.failures.clone();
            self.tracker.spawn(async move {
                // Contain work unwinds so the owner still reaches child drain; never render the panic payload.
                let outcome = AssertUnwindSafe(async {
                    let work = SUBMISSION.scope(child.clone(), work);
                    tokio::pin!(work);
                    tokio::select! {
                        biased;
                        _ = cancellation.cancelled() => Err(interrupted("Operation cancelled")),
                        _ = child.cancellation.cancelled() => Err(interrupted("Execution scope cancelled")),
                        _ = sender.closed() => Err(interrupted("Operation receiver dropped")),
                        value = &mut work => {
                            if cancellation.is_cancelled() { Err(interrupted("Operation cancelled")) }
                            else if child.is_cancelled() { Err(interrupted("Execution scope cancelled")) }
                            else { Ok(value) }
                        },
                    }
                }).catch_unwind().await;
                let panicked = outcome.is_err();
                let outcome = outcome.unwrap_or_else(|_| Err(interrupted("Operation panicked")));
                let cleanup = child.shutdown().await;
                let cleanup_failure = cleanup.as_ref().err().map(|error| interrupted(format!("{error:#}")));
                let outcome = match outcome {
                    Ok(value) => Ok(ScopedOutcome { value, cleanup }),
                    Err(error) => settled(Err(error), cleanup),
                };
                // A lost caller needs both panic and cleanup evidence, not just the cleanup-only fallback.
                if let Err(Err(error)) = sender.send(outcome) && panicked {
                    record_failure(&failures, &error);
                } else if let Some(error) = cleanup_failure {
                    record_failure(&failures, &error);
                }
            });
        }
        receiver
            .await
            .map_err(|_| interrupted("Operation owner lost; cleanup unconfirmed"))?
    }

    /// Closing admission and broadcasting cancellation do not themselves prove settlement.
    pub(crate) fn close(&self) {
        *self.open.lock().unwrap() = false;
        self.cancellation.cancel();
        self.tracker.close();
    }

    /// Waits rather than aborting owners, preserving retained failure evidence after task drain.
    /// Runtime destruction cannot provide this guarantee; late launches retain supervisor limits.
    pub(crate) async fn shutdown(&self) -> miette::Result<()> {
        self.close();
        self.tracker.wait().await;
        match self.failures.lock().unwrap().as_ref() {
            Some(message) => Err(interrupted(message.clone())),
            None => Ok(()),
        }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        self.close();
    }
}

fn record_failure(failures: &Mutex<Option<String>>, error: &miette::Report) {
    if matches!(
        error.downcast_ref::<ExecutionError>(),
        Some(ExecutionError::CancelledBeforeLaunch)
    ) {
        return;
    }
    // Alternate Display retains I/O causes but never Debug-prints the supervisor's sensitive captures.
    error!(error = %format_args!("{error:#}"), "Execution settlement failure");
    failures
        .lock()
        .unwrap()
        .get_or_insert_with(|| format!("{error:#}"));
}

fn deliver_job(
    outcome: miette::Result<ProcessReport>,
    result: oneshot::Sender<miette::Result<ProcessReport>>,
    failures: &Mutex<Option<String>>,
) {
    if let Err(error) = &outcome
        && (matches!(
            error.downcast_ref::<ProcessError>(),
            Some(
                ProcessError::CleanupUnconfirmed { .. }
                    | ProcessError::StartupInterrupted { .. }
                    | ProcessError::SupervisionFailed { .. }
            )
        ) || error.downcast_ref::<ExecutionError>().is_some())
    {
        record_failure(failures, error);
    }
    if let Err(Err(error)) = result.send(outcome) {
        record_failure(failures, &error);
    }
}

pub(crate) fn interrupted(message: impl Into<String>) -> miette::Report {
    ExecutionError::Interrupted(message.into()).into()
}

impl Scope {
    fn submit(&self, spec: ProcessSpec) -> miette::Result<Operation> {
        let cancellation = self.cancellation.child_token();
        let receiver = {
            let open = self.open.lock().unwrap();
            if !*open || self.is_cancelled() {
                return Err(interrupted("Execution scope closed before admission"));
            }
            let (result, receiver) = oneshot::channel();
            let process = self.process.clone();
            let scope = self.cancellation.clone();
            let request = self.request_cancellation.clone();
            let operation = cancellation.clone();
            let failures = self.failures.clone();
            self.tracker.spawn(async move {
                // Startup has not been polled, so either rejection has no launch to settle.
                if scope.is_cancelled() {
                    let _ = result.send(Err(interrupted("Execution scope closed before startup")));
                    return;
                }
                if request
                    .as_ref()
                    .is_some_and(CancellationToken::is_cancelled)
                {
                    let _ = result.send(Err(ExecutionError::CancelledBeforeLaunch.into()));
                    return;
                }
                let requested = async {
                    match request {
                        Some(request) => request.cancelled().await,
                        None => pending().await,
                    }
                };
                let outcome = process(spec, operation.clone());
                tokio::pin!(outcome);
                let outcome = tokio::select! {
                    biased;
                    _ = requested => { operation.cancel(); outcome.await },
                    outcome = &mut outcome => outcome,
                };
                deliver_job(outcome, result, &failures);
            });
            receiver
        };
        Ok(Operation {
            receiver,
            cancellation,
        })
    }
}

/// Drop requests cancellation while the completion owner retains the wait. Unscoped handles
/// provide no awaitable host-drain guarantee after they are dropped.
pub(crate) struct Operation {
    receiver: oneshot::Receiver<miette::Result<ProcessReport>>,
    cancellation: CancellationToken,
}

impl Operation {
    /// A request, not cleanup evidence: await the operation or its owning scope's shutdown.
    pub(crate) fn cancel(&self) {
        self.cancellation.cancel();
    }
}

impl Future for Operation {
    type Output = miette::Result<ProcessReport>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.receiver).poll(context).map(|result| {
            result.unwrap_or_else(|_| Err(interrupted("Process owner lost; cleanup unconfirmed")))
        })
    }
}

impl Drop for Operation {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Scoped owners register before startup is polled; unscoped calls retain one completion owner.
/// Dropping the returned handle requests cancellation without aborting that owner.
pub(crate) fn submit(spec: ProcessSpec) -> miette::Result<Operation> {
    if let Ok(submission) = SUBMISSION.try_with(Clone::clone) {
        submission.submit(spec)
    } else {
        let cancellation = CancellationToken::new();
        let (result, receiver) = oneshot::channel();
        let operation = Operation {
            receiver,
            cancellation: cancellation.clone(),
        };
        // Unscoped callers need one completion owner, not a one-operation dispatcher and tracker.
        tokio::spawn(async move {
            let outcome = start_process(spec, cancellation).await;
            if let Err(Err(error)) = result.send(outcome)
                && !matches!(
                    error.downcast_ref::<ExecutionError>(),
                    Some(ExecutionError::CancelledBeforeLaunch)
                )
            {
                error!(error = %error, "Execution settlement failure");
            }
        });
        Ok(operation)
    }
}

/// Admission shares the host gate/tracker; later caller loss cannot release a transaction lock
/// while detached filesystem I/O continues. Graceful drain still requires a live runtime.
pub(crate) async fn retain_persistence<F>(work: F) -> miette::Result<()>
where
    F: Future<Output = miette::Result<()>> + Send + 'static,
{
    let scope = SUBMISSION
        .try_with(Arc::clone)
        .unwrap_or_else(|_| Scope::new());
    let (sender, receiver) = oneshot::channel();
    {
        let open = scope.open.lock().unwrap();
        if !*open || scope.is_cancelled() {
            return Err(interrupted("Persistence cancelled before admission"));
        }
        let failures = scope.failures.clone();
        scope.tracker.spawn(async move {
            let outcome = work.await;
            if let Err(error) = &outcome {
                record_failure(&failures, error);
            }
            let _ = sender.send(outcome);
        });
    }
    receiver
        .await
        .map_err(|_| interrupted("Persistence owner lost; completion unconfirmed"))?
}

/// An ambient scope retains the new request owner; otherwise this helper also drains its host.
/// Dropping this future leaves admitted owners tracked, not aborted.
pub(crate) async fn in_scope<F, T>(work: F) -> miette::Result<ScopedOutcome<T>>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    if let Ok(scope) = SUBMISSION.try_with(Clone::clone) {
        return scope
            .run(scope.request_cancellation.clone().unwrap_or_default(), work)
            .await;
    }
    let scope = Scope::new();
    let outcome = scope.run(CancellationToken::new(), work).await;
    let cleanup = scope.shutdown().await;
    match outcome {
        Ok(mut outcome) => {
            outcome.cleanup = settled(outcome.cleanup, cleanup);
            Ok(outcome)
        }
        Err(error) => settled(Err(error), cleanup),
    }
}

/// Dropping this future requests cancellation; scoped shutdown still owns the drain. Unscoped
/// callers need a live runtime for the retained owner to finish.
pub(crate) async fn execute(spec: ProcessSpec) -> miette::Result<ProcessReport> {
    submit(spec)?.await
}

fn start_process(
    spec: ProcessSpec,
    cancellation: CancellationToken,
) -> BoxFuture<'static, miette::Result<ProcessReport>> {
    Box::pin(own_process(
        async move {
            let process = RunningProcess::spawn(spec, ProcessOptions::default()).await?;
            Ok((process.cancellation_handle(), process.wait()))
        },
        cancellation,
    ))
}

// This is the completion owner, not an ordinary helper chain. Its wait is never abandoned.
async fn own_process<W: Future<Output = miette::Result<ProcessReport>>>(
    startup: impl Future<Output = miette::Result<(CancellationToken, W)>>,
    cancellation: CancellationToken,
) -> miette::Result<ProcessReport> {
    if cancellation.is_cancelled() {
        return Err(ExecutionError::CancelledBeforeLaunch.into());
    }
    let (running, wait) = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(interrupted("Startup cancelled; child creation and late cleanup may be uncertain")),
        result = startup => result?,
    };
    tokio::pin!(wait);
    tokio::select! {
        result = &mut wait => result,
        _ = cancellation.cancelled() => { running.cancel(); wait.await },
    }
}

/// A zero child status after cancellation or timeout is not successful work.
pub(crate) fn exited(report: &ProcessReport) -> miette::Result<ExitStatus> {
    match report.completion {
        Completion::Exited(status) => Ok(status),
        Completion::Cancelled(_) => Err(interrupted("Process cancelled during execution")),
        Completion::TimedOut(_) => Err(interrupted("Process timed out during execution")),
    }
}

/// VCS/discovery callers must reject partial captures, never return partial diffs or infer roots
/// from incomplete output, even when the child exited successfully.
pub(crate) fn complete_output(report: &ProcessReport) -> miette::Result<ExitStatus> {
    let status = exited(report)?;
    if report.stdout.truncated || report.stderr.truncated {
        return Err(interrupted("Incomplete process output: capture truncated"));
    }
    Ok(status)
}

/// The optional cap bounds the lossy text prefix; a fixed notice remains outside that cap.
pub(crate) fn present(capture: &Capture, max_bytes: Option<usize>) -> String {
    let mut text = String::from_utf8_lossy(&capture.bytes).into_owned();
    let cap = max_bytes.filter(|&max| text.len() > max);
    if let Some(max) = cap {
        text.truncate(text.floor_char_boundary(max));
    }
    if capture.truncated || cap.is_some() {
        text.push('\n');
        text.push_str(OUTPUT_TRUNCATED_NOTICE);
    }
    text
}

/// Native supervision uncertainty and ownership interruptions must not become ordinary
/// discovery fallback or successful no-checks outcomes.
pub(crate) fn is_execution_error(error: &miette::Report) -> bool {
    error.downcast_ref::<ExecutionError>().is_some()
        || error.downcast_ref::<ProcessError>().is_some()
}

#[cfg(test)]
pub(crate) mod test_seams;

#[cfg(test)]
mod tests;
