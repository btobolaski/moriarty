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
    future::Future,
    io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use clap::Subcommand;
use miette::IntoDiagnostic;
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, Service, ServiceExt,
    model::{
        ClientNotification, ClientRequest, ContentBlock, ProtocolVersion, ServerInfo, ServerResult,
    },
    service::{NotificationContext, QuitReason, RequestContext},
    transport::stdio,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    task::JoinError,
};
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};

use self::{git_read_only::GitReadOnly, jj_read_only::JjReadOnly, tool_runner::ToolRunner};
use crate::execution::{self, Scope, ShutdownSignals};

// Dependency upgrades must not expand Moriarty's supported MCP revisions.
const MCP_PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::V_2025_11_25;
const MCP_SUPPORTED_PROTOCOL_VERSIONS: &[ProtocolVersion] = &[
    ProtocolVersion::V_2024_11_05,
    ProtocolVersion::V_2025_03_26,
    ProtocolVersion::V_2025_06_18,
    MCP_PROTOCOL_VERSION,
];

pub mod git_read_only;
pub mod jj_read_only;
pub mod read_only;
pub mod tool_runner;

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

    fn get_info(&self) -> ServerInfo {
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
    serve_until(server, transport, signals.recv()).await
}

async fn serve_until<S, R, W>(
    server: S,
    (reader, writer): (R, W),
    stop: impl Future<Output = miette::Result<&'static str>>,
) -> miette::Result<()>
where
    S: ServerHandler + Clone,
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let execution = Scope::new();
    let failures = Arc::new(Mutex::new(None));
    let service_ct = CancellationToken::new();
    let transport = (
        ObservedIo::new(reader, &execution, &failures, &service_ct),
        ObservedIo::new(writer, &execution, &failures, &service_ct),
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
