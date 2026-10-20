use std::{
    collections::BTreeSet,
    env::{current_dir, set_current_dir},
    fs,
    future::{Future, pending, poll_fn},
    io,
    os::unix::process::ExitStatusExt,
    pin::Pin,
    process::{Command, ExitStatus},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
    time::Duration,
};

use futures::poll;
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, CancelTaskParams,
        ClientCapabilities, ContentBlock, GetTaskParams, Implementation, RequestMetaObject,
        ServerConfig, ServerResult, UpdateTaskParams,
    },
    service::{QuitReason, RequestContext},
    transport::{IntoTransport, OneshotTransport, Transport},
};
use serde_json::{Value, from_str, from_value, json, to_value, to_vec};
use tempfile::tempdir;
use tokio::{
    io::{
        AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
        DuplexStream, ReadBuf, ReadHalf, WriteHalf, split,
    },
    sync::{Notify, oneshot},
    task::JoinHandle,
    time::timeout,
};
use tokio_util::sync::WaitForCancellationFutureOwned;

use super::{
    GitReadOnly, JjReadOnly, MAX_PENDING_REJECTIONS, ObservedIo, ProtocolGuardTransport,
    ToolRunner, git_read_only::StatusArgs, jj_read_only::JjArgs, serve_until, service_exit,
    settle_tool_response, tool_runner::RunArgs,
};
use crate::{
    execution::{
        self, Scope,
        test_seams::{current_scope, scope_with_operations},
    },
    project_config::approvals,
    test_helpers::{
        setup_git_repo_with_commit, setup_isolated_xdg_config, setup_project_dir_with_config,
        test_report,
    },
};
use process_supervisor::{CancellationToken, Completion, PartialState, ProcessError, ProcessSpec};

mod contracts;
mod lifecycle;
mod settlement;
mod transport;

// Independent expectations catch accidental changes to the production revision policy.
pub(super) const EXPECTED_PROTOCOL_VERSIONS: [&str; 5] = [
    "2024-11-05",
    "2025-03-26",
    "2025-06-18",
    "2025-11-25",
    "2026-07-28",
];

const TEST_TIMEOUT: Duration = Duration::from_secs(30);

struct TestClient {
    modern: bool,
    reader: BufReader<ReadHalf<DuplexStream>>,
    writer: WriteHalf<DuplexStream>,
    server_task: JoinHandle<miette::Result<()>>,
}

impl TestClient {
    async fn start<S: ServerHandler + Clone>(server: S) -> Self {
        Self::start_until(server, pending()).await
    }

    async fn start_until<S: ServerHandler + Clone>(
        server: S,
        stop: impl Future<Output = miette::Result<&'static str>> + Send + 'static,
    ) -> Self {
        Self::start_in_scope(server, stop, Scope::new(), false).await
    }

    async fn start_in_scope<S: ServerHandler + Clone>(
        server: S,
        stop: impl Future<Output = miette::Result<&'static str>> + Send + 'static,
        scope: Arc<Scope>,
        modern: bool,
    ) -> Self {
        let (server_io, client_io) = tokio::io::duplex(64 * 1024);
        let server_task = tokio::spawn(serve_until(server, split(server_io), stop, scope));
        let (reader, writer) = split(client_io);

        Self {
            modern,
            reader: BufReader::new(reader),
            writer,
            server_task,
        }
    }

    async fn connect<S: ServerHandler + Clone>(server: S, modern: bool) -> Self {
        let mut client = Self::start_in_scope(server, pending(), Scope::new(), modern).await;
        if !modern {
            let initialized = initialize_client(&mut client, "2025-11-25").await;
            assert_eq!(initialized["result"]["protocolVersion"], "2025-11-25");
        }
        client
    }

    async fn send(&mut self, message: Value) {
        let mut encoded = to_vec(&message).unwrap();
        encoded.push(b'\n');
        timeout(TEST_TIMEOUT, self.writer.write_all(&encoded))
            .await
            .expect("timed out writing MCP message")
            .unwrap();
        timeout(TEST_TIMEOUT, self.writer.flush())
            .await
            .expect("timed out flushing MCP message")
            .unwrap();
    }

    async fn send_request(&mut self, mut request: Value) {
        if self.modern {
            request["params"]
                .as_object_mut()
                .unwrap()
                .entry("_meta")
                .or_insert_with(|| protocol_meta("2026-07-28"));
        }
        self.send(request).await;
    }

    async fn request(&mut self, request: Value) -> Value {
        self.send_request(request).await;
        self.response().await
    }

    // Negative metadata tests must bypass the lifecycle fixture's valid context.
    async fn raw_request(&mut self, request: Value) -> Value {
        self.send(request).await;
        self.response().await
    }

    async fn response(&mut self) -> Value {
        let mut response = String::new();
        let bytes_read = timeout(TEST_TIMEOUT, self.reader.read_line(&mut response))
            .await
            .expect("timed out reading MCP response")
            .unwrap();
        assert_ne!(bytes_read, 0, "MCP server closed without a response");
        from_str(&response).unwrap()
    }

    async fn close(self) {
        let Self {
            modern: _,
            reader,
            writer,
            server_task,
        } = self;
        drop(reader);
        drop(writer);
        timeout(TEST_TIMEOUT, server_task)
            .await
            .expect("MCP server did not stop after client disconnect")
            .unwrap()
            .unwrap();
    }
}

async fn initialize<S: ServerHandler + Clone>(
    server: S,
    requested_version: &str,
) -> (TestClient, Value) {
    let mut client = TestClient::start(server).await;
    let response = initialize_client(&mut client, requested_version).await;
    (client, response)
}

async fn initialize_client(client: &mut TestClient, requested_version: &str) -> Value {
    let response = client
        .request(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": requested_version,
                "capabilities": {},
                "clientInfo": {
                    "name": "moriarty-protocol-test",
                    "version": "0"
                }
            }
        }))
        .await;
    client
        .send(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }))
        .await;
    response
}

#[derive(Clone, Default)]
struct PendingTools {
    owned_work: bool,
    scopes: [Arc<Mutex<Option<Arc<Scope>>>>; 2],
    calls: Arc<AtomicUsize>,
    entered: [Arc<Notify>; 2],
    release: [Arc<Notify>; 2],
    done: [Arc<Notify>; 2],
    dropped: [Arc<AtomicBool>; 2],
}

struct WorkDrop {
    done: Arc<Notify>,
    dropped: Arc<AtomicBool>,
}
impl Drop for WorkDrop {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
        self.done.notify_one();
    }
}

impl ServerHandler for PendingTools {
    fn get_info(&self) -> ServerConfig {
        ServerHandler::get_info(&GitReadOnly)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if request.name == "large" {
            return Ok(CallToolResult::success(vec![ContentBlock::text(
                "unread".repeat(64 * 1024),
            )])
            .into());
        }
        let index = usize::from(request.name == "second");
        *self.scopes[index].lock().unwrap() = Some(current_scope());
        let _operation = self.owned_work.then(|| {
            execution::submit(ProcessSpec {
                program: request.name.clone().into_owned().into(),
                args: Vec::new(),
                cwd: None,
            })
            .unwrap()
        });
        let _guard = WorkDrop {
            done: self.done[index].clone(),
            dropped: self.dropped[index].clone(),
        };
        self.entered[index].notify_one();
        self.release[index].notified().await;
        Ok(CallToolResult::success(vec![ContentBlock::text("released")]).into())
    }
}

impl PendingTools {
    async fn start_call(&self, client: &mut TestClient, index: usize, id: i32) {
        client.send_request(json!({"jsonrpc":"2.0", "id":id, "method":"tools/call", "params":{"name":if index == 0 { "first" } else { "second" }, "arguments":{}}})).await;
        timeout(TEST_TIMEOUT, self.entered[index].notified())
            .await
            .unwrap();
    }
}

async fn assert_scope_rejects_admission(scope: &Arc<Scope>) {
    // Rejection must prevent polling the work, not merely return an error.
    let error = scope
        .run(CancellationToken::new(), async {
            panic!("closed scope must reject new work")
        })
        .await
        .err()
        .unwrap();
    assert!(
        error.to_string().contains("cancelled before admission"),
        "{error}"
    );
}

struct FailingWrite<W> {
    inner: W,
    fail: Arc<AtomicBool>,
    zero: bool,
}

impl<W: AsyncWrite + Unpin> AsyncWrite for FailingWrite<W> {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.fail.load(Ordering::SeqCst) {
            return if this.zero {
                Poll::Ready(Ok(0))
            } else {
                Poll::Ready(Err(io::Error::other("injected write failure")))
            };
        }
        Pin::new(&mut this.inner).poll_write(context, bytes)
    }
    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(context)
    }
    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(context)
    }
}

struct GatedWrite<W> {
    inner: W,
    blocked: Arc<Notify>,
    resume: Pin<Box<WaitForCancellationFutureOwned>>,
}

impl<W> GatedWrite<W> {
    fn new(inner: W) -> (Self, Arc<Notify>, CancellationToken) {
        let blocked = Arc::new(Notify::new());
        let resume = CancellationToken::new();
        (
            Self {
                inner,
                blocked: blocked.clone(),
                resume: Box::pin(resume.clone().cancelled_owned()),
            },
            blocked,
            resume,
        )
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for GatedWrite<W> {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.resume.as_mut().poll(context).is_pending() {
            this.blocked.notify_one();
            return Poll::Pending;
        }
        Pin::new(&mut this.inner).poll_write(context, bytes)
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(context)
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(context)
    }
}

async fn list_tools(client: &mut TestClient) -> Value {
    client
        .request(json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }))
        .await
}

fn protocol_meta(version: &str) -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": version,
        "io.modelcontextprotocol/clientCapabilities": {}
    })
}

fn assert_legacy_result_shape(response: &Value) {
    for field in ["resultType", "ttlMs", "cacheScope"] {
        assert!(
            response["result"].get(field).is_none(),
            "legacy response unexpectedly contains {field}: {response}"
        );
    }
}

fn assert_result_shape(response: &Value, modern: bool, cacheable: bool) {
    assert!(response.get("error").is_none(), "{response}");
    if !modern {
        assert_legacy_result_shape(response);
        return;
    }
    assert_eq!(response["result"]["resultType"], "complete", "{response}");
    if cacheable {
        assert!(response["result"]["ttlMs"].as_u64().is_some(), "{response}");
        assert!(
            matches!(
                response["result"]["cacheScope"].as_str(),
                Some("public" | "private")
            ),
            "{response}"
        );
    }
}
