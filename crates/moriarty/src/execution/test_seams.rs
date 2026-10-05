use std::{
    future::{Future, ready},
    sync::{Arc, Mutex},
};

use tokio_util::sync::CancellationToken;

use super::{SUBMISSION, Scope, interrupted};
use process_supervisor::{ProcessReport, ProcessSpec};

/// Uses the production host owner while substituting only the shutdown event.
pub(crate) async fn run_until(
    work: impl Future<Output = miette::Result<()>> + Send + 'static,
    stop: impl Future<Output = miette::Result<&'static str>>,
) -> miette::Result<()> {
    super::run_until(work, stop).await
}

// Inject only the operation result: these tests exercise callers, not supervisor internals.
/// A single-operation fixture: additional submissions exhaust the injected result and fail.
/// Use `with_operations` when work legitimately submits more than once.
pub(crate) async fn with_result<T>(
    result: Result<ProcessReport, &str>,
    work: impl Future<Output = T>,
) -> T {
    let mut result = Some(result.map_err(interrupted));
    with_operations(
        move |_, _| ready(result.take().expect("unexpected additional operation")),
        work,
    )
    .await
}

/// Replaces only process results; admission, linked cancellation, retained owners and shutdown
/// stay real. Injected futures must honor cancellation and eventually settle to allow drain.
pub(crate) async fn with_operations<F, Fut, T>(handler: F, work: impl Future<Output = T>) -> T
where
    F: FnMut(ProcessSpec, CancellationToken) -> Fut + Send + 'static,
    Fut: Future<Output = miette::Result<ProcessReport>> + Send + 'static,
{
    let mut scope = Scope::new();
    let handler = Arc::new(Mutex::new(handler));
    Arc::get_mut(&mut scope).unwrap().process = Arc::new(move |spec, cancellation| {
        let outcome = {
            let mut handler = handler.lock().unwrap();
            handler(spec, cancellation)
        };
        Box::pin(outcome)
    });
    let value = SUBMISSION.scope(scope.clone(), work).await;
    // Injected failures belong to the caller-policy assertion, not this fixture's return type.
    let _ = scope.shutdown().await;
    value
}
