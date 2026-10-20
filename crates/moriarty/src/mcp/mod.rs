//! MCP (Model Context Protocol) servers for Moriarty.
//!
//! This module provides three MCP servers:
//!
//! - [`git_read_only`]: Read-only git operations (status, diff, log, show)
//! - [`jj_read_only`]: Read-only jj operations (status, diff, log, show, op log, file show, file list)
//! - [`tool_runner`]: Project-configured tool execution (lint, test, build, format, checks)
//!
//! # Usage
//!
//! Servers can be run via the CLI:
//!
//! ```bash
//! moriarty mcp git-read-only
//! moriarty mcp jj-read-only
//! moriarty mcp project-tools
//! moriarty mcp install  # Install all servers to Claude Code
//! ```

use std::{
    borrow::Cow,
    fmt::Display,
    future::Future,
    io,
    path::PathBuf,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use clap::Subcommand;
use futures::poll;
use miette::{IntoDiagnostic, WrapErr};
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, Service, ServiceExt,
    model::{
        ClientJsonRpcMessage, ClientNotification, ClientRequest, ContentBlock, GetMeta,
        PingRequestMethod, ProtocolVersion, RequestMetaObject, ServerConfig, ServerJsonRpcMessage,
        ServerResult,
    },
    service::{NotificationContext, QuitReason, RequestContext},
    transport::{IntoTransport, Transport, stdio},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::mpsc,
    task::{JoinError, JoinHandle},
};
use tokio_util::{
    sync::{CancellationToken, WaitForCancellationFutureOwned},
    task::TaskTracker,
};

use self::{git_read_only::GitReadOnly, jj_read_only::JjReadOnly, tool_runner::ToolRunner};
use crate::execution::{self, Scope, ShutdownSignals};

// Dependency upgrades must not expand Moriarty's supported MCP revisions.
const MCP_LEGACY_PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::V_2025_11_25;
const MCP_MODERN_PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::V_2026_07_28;
const MCP_SUPPORTED_PROTOCOL_VERSIONS: &[ProtocolVersion] = &[
    ProtocolVersion::V_2024_11_05,
    ProtocolVersion::V_2025_03_26,
    ProtocolVersion::V_2025_06_18,
    MCP_LEGACY_PROTOCOL_VERSION,
    MCP_MODERN_PROTOCOL_VERSION,
];

pub mod git_read_only;
pub mod jj_read_only;
pub mod read_only;
pub mod tool_runner;

const MAX_PENDING_REJECTIONS: usize = 64;
type RejectionSend<E> = Pin<Box<dyn Future<Output = Result<(), E>> + Send>>;

// rmcp 3.6 skips bootstrap ping validation and checks discovery completeness first.
// Intercept unsupported discovery and unsupported/modern-context ping without choosing
// a lifecycle. Supported legacy-version ping passes through even with context keys;
// context-free ping (including empty/extension-only metadata) also stays upstream-owned.
struct ProtocolGuardTransport<T: Transport<RoleServer>> {
    inner: T,
    rejections: Option<mpsc::Sender<RejectionSend<T::Error>>>,
    rejection_tasks: TaskTracker,
    execution: Arc<Scope>,
    failures: Arc<Mutex<Option<String>>>,
    service_ct: CancellationToken,
}

impl<T: Transport<RoleServer>> ProtocolGuardTransport<T> {
    fn new(
        inner: T,
        execution: &Arc<Scope>,
        failures: &Arc<Mutex<Option<String>>>,
        service_ct: &CancellationToken,
    ) -> (Self, JoinHandle<()>) {
        let (sender, mut receiver) =
            mpsc::channel::<RejectionSend<T::Error>>(MAX_PENDING_REJECTIONS);
        // rmcp close needs a cancellable wait; the host also retains the join handle
        // to settle failed/cancelled startup and report sender panics.
        let rejection_tasks = TaskTracker::new();
        let task = rejection_tasks.spawn({
            let execution = execution.clone();
            let failures = failures.clone();
            let service_ct = service_ct.clone();
            async move {
                let _cancel_on_exit = service_ct.clone().drop_guard();
                while let Some(reply) = receiver.recv().await {
                    if let Err(error) = reply.await {
                        rejection_failed(&execution, &failures, &service_ct, error);
                        break;
                    }
                }
            }
        });
        rejection_tasks.close();
        (
            Self {
                inner,
                rejections: Some(sender),
                rejection_tasks,
                execution: execution.clone(),
                failures: failures.clone(),
                service_ct: service_ct.clone(),
            },
            task,
        )
    }
}

fn rejection_failed(
    execution: &Scope,
    failures: &Mutex<Option<String>>,
    service_ct: &CancellationToken,
    error: impl Display,
) {
    tracing::warn!(%error, "failed to reject MCP request");
    failures
        .lock()
        .unwrap()
        .get_or_insert_with(|| error.to_string());
    execution.close();
    service_ct.cancel();
}

impl<T: Transport<RoleServer>> Transport<RoleServer> for ProtocolGuardTransport<T> {
    type Error = T::Error;

    fn send(
        &mut self,
        message: ServerJsonRpcMessage,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.inner.send(message)
    }

    async fn receive(&mut self) -> Option<ClientJsonRpcMessage> {
        loop {
            let message = tokio::select! {
                biased;
                () = self.rejection_tasks.wait() => return None,
                message = self.inner.receive() => message?,
            };
            if let ClientJsonRpcMessage::Request(request) = &message
                && matches!(
                    request.request,
                    ClientRequest::PingRequest(_) | ClientRequest::DiscoverRequest(_)
                )
            {
                let meta = request.request.get_meta();
                let error = match meta.protocol_version() {
                    // A revision outside our policy is rejected even if its context is incomplete.
                    Some(version) if !MCP_SUPPORTED_PROTOCOL_VERSIONS.contains(&version) => {
                        McpError::unsupported_protocol_version(
                            version,
                            MCP_SUPPORTED_PROTOCOL_VERSIONS,
                        )
                    }
                    _ if matches!(request.request, ClientRequest::DiscoverRequest(_)) => {
                        return Some(message);
                    }
                    Some(version) if version.has_initialize() => return Some(message),
                    // rmcp's RequestMetaObject keeps META_KEY_CLIENT_INFO and
                    // META_KEY_LOG_LEVEL private. Mirror them so optional-only modern
                    // context cannot bypass required metadata validation.
                    _ if !RequestMetaObject::DRAFT_REQUIRED_KEYS
                        .into_iter()
                        .chain([
                            "io.modelcontextprotocol/clientInfo",
                            "io.modelcontextprotocol/logLevel",
                        ])
                        .any(|key| meta.contains_key(key)) =>
                    {
                        return Some(message);
                    }
                    _ => {
                        let missing = meta.missing_required_keys(&MCP_MODERN_PROTOCOL_VERSION);
                        if missing.is_empty() {
                            McpError::method_not_found::<PingRequestMethod>()
                        } else {
                            // Keep ping errors consistent with rmcp's private
                            // service::server::missing_request_metadata_error.
                            McpError::invalid_params(
                                format!(
                                    "request _meta is missing or has malformed required fields: {}",
                                    missing.join(", ")
                                ),
                                None,
                            )
                        }
                    }
                };
                let mut reply = Box::pin(
                    self.inner
                        .send(ServerJsonRpcMessage::error(error, Some(request.id.clone()))),
                );
                // Preserve immediate write failures before forwarding another request.
                // A parked send moves, still pinned, to the host-owned bounded sender
                // before another await; cancellation of receive cannot drop the reply.
                match poll!(reply.as_mut()) {
                    Poll::Ready(Ok(())) => {}
                    Poll::Ready(Err(error)) => {
                        rejection_failed(&self.execution, &self.failures, &self.service_ct, error);
                        return None;
                    }
                    Poll::Pending => {
                        if let Err(error) = self.rejections.as_ref()?.try_send(reply) {
                            rejection_failed(
                                &self.execution,
                                &self.failures,
                                &self.service_ct,
                                format!("MCP rejection queue failed: {error}"),
                            );
                            return None;
                        }
                    }
                }
                continue;
            }
            return Some(message);
        }
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        self.execution.close();
        self.service_ct.cancel();
        self.rejections.take();
        self.rejection_tasks.wait().await;
        self.inner.close().await
    }
}

// Resolving `.` at use time keeps omitted project directories relative to the server's cwd.
fn default_project_dir() -> PathBuf {
    PathBuf::from(".")
}

#[derive(Debug, Subcommand)]
pub enum McpServers {
    /// runs the git read only server as stdin / stdout server
    GitReadOnly,
    /// runs the jj read only server as stdin / stdout server
    JjReadOnly,
    /// runs the project tools server as stdin / stdout server
    ProjectTools,
    /// installs the MCP servers into Claude Code
    Install,
}

impl McpServers {
    pub async fn run(self) -> miette::Result<()> {
        match self {
            Self::GitReadOnly => serve_owned(GitReadOnly, stdio()).await,
            Self::JjReadOnly => serve_owned(JjReadOnly, stdio()).await,
            Self::ProjectTools => serve_owned(ToolRunner, stdio()).await,
            Self::Install => install_mcp_server().await,
        }
    }
}

// Wrap Service rather than routers: one owning seam preserves every handler's protocol behavior.
struct OwnedService<S> {
    server: S,
    execution: Arc<Scope>,
}

impl<S: ServerHandler + Clone> Service<RoleServer> for OwnedService<S> {
    async fn handle_request(
        &self,
        request: ClientRequest,
        context: RequestContext<RoleServer>,
    ) -> Result<ServerResult, McpError> {
        if !matches!(request, ClientRequest::CallToolRequest(_)) {
            return self.server.handle_request(request, context).await;
        }
        let server = self.server.clone();
        let outcome = self
            .execution
            .run(context.ct.clone(), async move {
                server.handle_request(request, context).await
            })
            .await
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        settle_tool_response(outcome.value, outcome.cleanup)
    }

    async fn handle_notification(
        &self,
        notification: ClientNotification,
        context: NotificationContext<RoleServer>,
    ) -> Result<(), McpError> {
        self.server.handle_notification(notification, context).await
    }

    fn get_info(&self) -> ServerConfig {
        self.server.get_info()
    }
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        self.server.supported_protocol_versions()
    }
}

fn settle_tool_response(
    response: Result<ServerResult, McpError>,
    cleanup: miette::Result<()>,
) -> Result<ServerResult, McpError> {
    let Err(cleanup) = cleanup else {
        return response;
    };
    match response {
        Ok(ServerResult::CallToolResult(mut result)) => {
            result.is_error = Some(true);
            result
                .content
                .push(ContentBlock::text(format!("Settlement: {cleanup}")));
            Ok(ServerResult::CallToolResult(result))
        }
        Err(primary) => Err(McpError::internal_error(
            format!("{primary}\nSettlement: {cleanup}"),
            None,
        )),
        Ok(_) => Err(McpError::internal_error(cleanup.to_string(), None)),
    }
}

async fn serve_owned<S, R, W>(server: S, transport: (R, W)) -> miette::Result<()>
where
    S: ServerHandler + Clone,
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let mut signals = ShutdownSignals::new()?;
    serve_until(server, transport, signals.recv(), Scope::new()).await
}

async fn serve_until<S, R, W>(
    server: S,
    (reader, writer): (R, W),
    stop: impl Future<Output = miette::Result<&'static str>>,
    execution: Arc<Scope>,
) -> miette::Result<()>
where
    S: ServerHandler + Clone,
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let failures = Arc::new(Mutex::new(None));
    let service_ct = CancellationToken::new();
    let (transport, rejection_task) = ProtocolGuardTransport::new(
        (
            ObservedIo::new(reader, &execution, &failures, &service_ct),
            ObservedIo::new(writer, &execution, &failures, &service_ct),
        )
            .into_transport(),
        &execution,
        &failures,
        &service_ct,
    );
    tokio::pin!(stop);
    // Early returns and `?` must exit only this block so admission close, cancellation
    // and awaited drain below still run.
    let outcome = async {
        let server = OwnedService { server, execution: execution.clone() };
        let service = tokio::select! {
            biased;
            reason = &mut stop => {
                execution.close();
                service_ct.cancel();
                return interrupted_host(reason);
            },
            result = server.serve_with_ct(transport, service_ct.clone()) => result.into_diagnostic()?,
        };
        let waiting = service.waiting();
        tokio::pin!(waiting);
        tokio::select! {
            biased;
            reason = &mut stop => {
                execution.close();
                service_ct.cancel();
                execution::settled(interrupted_host(reason), service_exit(waiting.await))
            },
            result = &mut waiting => service_exit(result),
        }
    }.await;
    execution.close();
    service_ct.cancel();
    let outcome = execution::settled(
        outcome,
        rejection_task
            .await
            .into_diagnostic()
            .wrap_err("MCP rejection sender failed"),
    );
    let outcome = execution::settled(outcome, execution.shutdown().await);
    let transport_failure = failures.lock().unwrap().take();
    execution::settled(
        outcome,
        transport_failure.map_or(Ok(()), |error| {
            Err(miette::miette!("MCP transport failed: {error}"))
        }),
    )
}

fn interrupted_host(reason: miette::Result<&'static str>) -> miette::Result<()> {
    match reason {
        Ok(reason) => Err(miette::miette!("Interrupted by {reason}")),
        Err(error) => Err(error),
    }
}

fn service_exit(result: Result<QuitReason, JoinError>) -> miette::Result<()> {
    match result.into_diagnostic()? {
        QuitReason::Closed | QuitReason::Cancelled => Ok(()),
        QuitReason::JoinError(error) => Err(error).into_diagnostic(),
        _ => Err(miette::miette!("Unexpected MCP service termination")),
    }
}

// rmcp drains handlers before reporting EOF; cancellation must happen at the I/O boundary.
// Read errors also look like EOF to rmcp, so retain their metadata outside its service loop.
struct ObservedIo<T> {
    inner: T,
    failure: Arc<Mutex<Option<String>>>,
    execution: Arc<Scope>,
    service_ct: CancellationToken,
    cancelled: Pin<Box<WaitForCancellationFutureOwned>>,
}

impl<T> ObservedIo<T> {
    fn new(
        inner: T,
        execution: &Arc<Scope>,
        failure: &Arc<Mutex<Option<String>>>,
        service_ct: &CancellationToken,
    ) -> Self {
        Self {
            inner,
            failure: failure.clone(),
            execution: execution.clone(),
            cancelled: Box::pin(service_ct.clone().cancelled_owned()),
            service_ct: service_ct.clone(),
        }
    }

    // Writable EOF replies can still drain normally. A parked output has no peer-driven wake,
    // so cancellation must wake it and abandon delivery rather than hold rmcp's writer mutex.
    fn observe_output<U>(
        &mut self,
        context: &mut Context<'_>,
        result: Poll<io::Result<U>>,
    ) -> Poll<io::Result<U>> {
        if result.is_pending()
            && (self.service_ct.is_cancelled() || self.cancelled.as_mut().poll(context).is_ready())
        {
            self.observe(Poll::Ready(Err(io::ErrorKind::ConnectionAborted.into())))
        } else {
            self.observe(result)
        }
    }

    fn stop(&self) {
        self.execution.close();
        self.service_ct.cancel();
    }

    fn observe<U>(&self, result: Poll<io::Result<U>>) -> Poll<io::Result<U>> {
        if let Poll::Ready(Err(error)) = &result {
            self.failure
                .lock()
                .unwrap()
                .get_or_insert_with(|| error.to_string());
            self.stop();
        }
        result
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for ObservedIo<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buffer.filled().len();
        let available = buffer.remaining();
        let result = Pin::new(&mut this.inner).poll_read(context, buffer);
        if available > 0 && matches!(result, Poll::Ready(Ok(()))) && buffer.filled().len() == before
        {
            this.stop();
        }
        this.observe(result)
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for ObservedIo<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.inner).poll_write(context, bytes);
        let result = if !bytes.is_empty() && matches!(result, Poll::Ready(Ok(0))) {
            Poll::Ready(Err(io::ErrorKind::WriteZero.into()))
        } else {
            result
        };
        this.observe_output(context, result)
    }
    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.inner).poll_flush(context);
        this.observe_output(context, result)
    }
    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.inner).poll_shutdown(context);
        this.observe_output(context, result)
    }
}

async fn install_mcp_server() -> miette::Result<()> {
    // Install all MCP servers, tracking results
    let servers = ["git-read-only", "jj-read-only", "project-tools"];
    let mut errors = Vec::new();

    for server in &servers {
        match install_single_mcp_server(server).await {
            Ok(_) => {}
            Err(e) => {
                eprintln!("Warning: Failed to install {} server: {}", server, e);
                errors.push((server, e));
            }
        }
    }

    if errors.is_empty() {
        eprintln!("\nSuccessfully installed all MCP servers!");
        Ok(())
    } else {
        Err(miette::miette!(
            "Failed to install {} server(s):\n{}",
            errors.len(),
            errors
                .iter()
                .map(|(name, err)| format!("  - {}: {}", name, err))
                .collect::<Vec<_>>()
                .join("\n")
        ))
    }
}

async fn install_single_mcp_server(server_name: &str) -> miette::Result<()> {
    use tokio::process::Command;

    // Errors are ignored because the server may not be installed yet, and other errors
    // (missing claude binary, permissions) will be caught by the subsequent add command.
    eprintln!(
        "Removing existing {} MCP server (if present)...",
        server_name
    );
    let _ = Command::new("claude")
        .args(["mcp", "remove", server_name])
        .status()
        .await;

    eprintln!("Adding {} MCP server...", server_name);
    let status = Command::new("claude")
        .args([
            "mcp",
            "add",
            "--scope",
            "user",
            "--transport",
            "stdio",
            server_name,
            "--",
            "moriarty",
            "mcp",
            server_name,
        ])
        .status()
        .await
        .into_diagnostic()?;

    if !status.success() {
        return Err(miette::miette!(
            "Failed to add {} MCP server. Exit code: {}",
            server_name,
            status.code().unwrap_or(-1)
        ));
    }

    eprintln!("Successfully installed {} MCP server!", server_name);
    Ok(())
}

#[cfg(test)]
mod tests;
