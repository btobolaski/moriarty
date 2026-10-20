use std::{
    collections::BTreeSet,
    env::{current_dir, set_current_dir},
    fs,
    future::{Future, pending, poll_fn},
    io,
    pin::Pin,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Waker},
    time::Duration,
};

use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ServerInfo,
        ServerResult,
    },
    service::{QuitReason, RequestContext},
    transport::{OneshotTransport, Transport},
};
use serde_json::{Value, from_str, from_value, json, to_value, to_vec};
use tempfile::tempdir;
use tokio::{
    io::{
        AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, DuplexStream, ReadBuf,
        ReadHalf, WriteHalf, split,
    },
    sync::{Notify, oneshot},
    task::JoinHandle,
    time::timeout,
};

use super::{
    DiscoveryFallbackTransport, GitReadOnly, JjReadOnly, ObservedIo, ToolRunner,
    git_read_only::StatusArgs, jj_read_only::JjArgs, serve_until, service_exit,
    settle_tool_response, tool_runner::RunArgs,
};
use crate::{
    execution::Scope,
    project_config::approvals,
    test_helpers::{
        setup_git_repo_with_commit, setup_isolated_xdg_config, setup_project_dir_with_config,
    },
};
use process_supervisor::CancellationToken;

fn observed<T>(inner: T) -> ObservedIo<T> {
    let service_ct = CancellationToken::new();
    ObservedIo::new(
        inner,
        &Scope::new(),
        &Arc::new(Mutex::new(None)),
        &service_ct,
    )
}

#[tokio::test]
async fn pending_output_wakes_on_service_cancellation_without_peer_reads() {
    let (writer, _unread) = tokio::io::duplex(1);
    let mut writer = observed(writer);
    let cancellation = writer.service_ct.clone();
    let mut write = Box::pin(writer.write_all(b"ab"));
    poll_fn(|context| {
        assert!(write.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    cancellation.cancel();
    let error = timeout(TEST_TIMEOUT, write)
        .await
        .expect("cancelled write must wake without a peer read")
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::ConnectionAborted);
    assert!(writer.failure.lock().unwrap().is_some());
    let Err(error) = writer
        .execution
        .run(CancellationToken::new(), async {})
        .await
    else {
        panic!("cancelled output must close admission")
    };
    assert!(
        error.to_string().contains("cancelled before admission"),
        "{error}"
    );
    writer.execution.shutdown().await.unwrap();
}

#[tokio::test]
async fn zero_capacity_read_keeps_admission_open() {
    let mut io = observed(tokio::io::empty());
    let mut context = Context::from_waker(Waker::noop());
    let mut buffer = ReadBuf::new(&mut []);
    assert!(matches!(
        Pin::new(&mut io).poll_read(&mut context, &mut buffer),
        Poll::Ready(Ok(()))
    ));
    assert!(!io.service_ct.is_cancelled());
    assert!(io.failure.lock().unwrap().is_none());
    assert_eq!(
        io.execution
            .run(CancellationToken::new(), async { 42 })
            .await
            .unwrap()
            .value,
        42
    );
    io.execution.shutdown().await.unwrap();
}

#[tokio::test]
async fn later_transport_failure_preserves_first_cause() {
    let io = observed(tokio::io::empty());
    for cause in ["first", "later"] {
        let Poll::Ready(Err(error)) = io.observe::<()>(Poll::Ready(Err(io::Error::other(cause))))
        else {
            panic!("must propagate failure")
        };
        assert_eq!(error.to_string(), cause);
    }
    assert_eq!(io.failure.lock().unwrap().as_deref(), Some("first"));
    assert!(io.service_ct.is_cancelled());
    let error = io
        .execution
        .run(CancellationToken::new(), async {})
        .await
        .err()
        .expect("closed admission must fail");
    assert!(
        error.to_string().contains("cancelled before admission"),
        "{error}"
    );
    io.execution.shutdown().await.unwrap();
}

fn settled_response(response: Result<ServerResult, McpError>) -> Result<Value, McpError> {
    settle_tool_response(response, Err(miette::miette!("cleanup")))
        .map(|result| to_value(result).unwrap())
}

#[test]
fn tool_result_retains_content_on_cleanup_failure() {
    for initial in [CallToolResult::success, CallToolResult::error] {
        let original = initial(vec![ContentBlock::text("primary")]);
        let result = settled_response(Ok(ServerResult::CallToolResult(original))).unwrap();
        assert_eq!(result["isError"], true);
        assert_eq!(result["content"][0]["text"], "primary");
        assert_eq!(result["content"][1]["text"], "Settlement: cleanup");
    }
}

#[test]
fn request_error_retains_both_causes() {
    let original = Err(McpError::invalid_request("primary", None));
    let error = settled_response(original).unwrap_err();
    assert!(error.message.contains("primary") && error.message.contains("cleanup"));
}

const TEST_TIMEOUT: Duration = Duration::from_secs(30);

struct TestClient {
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
        let (server_io, client_io) = tokio::io::duplex(64 * 1024);
        let server_task = tokio::spawn(serve_until(server, split(server_io), stop));
        let (reader, writer) = split(client_io);

        Self {
            reader: BufReader::new(reader),
            writer,
            server_task,
        }
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

    async fn request(&mut self, request: Value) -> Value {
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
    fn get_info(&self) -> ServerInfo {
        ServerHandler::get_info(&GitReadOnly)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        if request.name == "large" {
            return Ok(CallToolResult::success(vec![ContentBlock::text(
                "unread".repeat(64 * 1024),
            )])
            .into());
        }
        let index = usize::from(request.name == "second");
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
        client.send(json!({"jsonrpc":"2.0", "id":id, "method":"tools/call", "params":{"name":if index == 0 { "first" } else { "second" }, "arguments":{}}})).await;
        timeout(TEST_TIMEOUT, self.entered[index].notified())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn request_cancellation_is_local_and_drops_its_work() {
    let server = PendingTools::default();
    let (mut client, _) = initialize(server.clone(), "2025-11-25").await;
    server.start_call(&mut client, 0, 11).await;
    server.start_call(&mut client, 1, 22).await;
    client.send(json!({"jsonrpc":"2.0", "method":"notifications/cancelled", "params":{"requestId":11, "reason":"test cancellation"}})).await;
    timeout(TEST_TIMEOUT, server.done[0].notified())
        .await
        .unwrap();
    assert!(server.dropped[0].load(Ordering::SeqCst));
    assert!(
        !server.dropped[1].load(Ordering::SeqCst),
        "another request must remain active"
    );
    server.release[1].notify_one();
    loop {
        let response = client.response().await;
        if response["id"] == 22 {
            assert_eq!(response["result"]["content"][0]["text"], "released");
            break;
        }
        assert_eq!(response["id"], 11);
        assert!(response.get("error").is_some());
    }
    client.close().await;
}

#[tokio::test]
async fn disconnect_settles_every_dropped_handler_before_return() {
    for graceful_eof in [true, false] {
        let server = PendingTools::default();
        let (mut client, _) = initialize(server.clone(), "2025-11-25").await;
        server.start_call(&mut client, 0, 11).await;
        server.start_call(&mut client, 1, 22).await;
        let TestClient {
            reader,
            mut writer,
            server_task,
        } = client;
        let reader = if graceful_eof {
            writer.shutdown().await.unwrap();
            Some(reader)
        } else {
            drop(reader);
            None
        };
        drop(writer);
        let outcome = timeout(TEST_TIMEOUT, server_task).await.unwrap().unwrap();
        if graceful_eof {
            outcome.unwrap();
        } else if let Err(error) = outcome {
            assert!(
                error.to_string().contains("MCP transport failed"),
                "{error}"
            );
        }
        drop(reader);
        assert!(
            server
                .dropped
                .iter()
                .all(|done| done.load(Ordering::SeqCst))
        );
    }
}

async fn assert_host_failure(
    task: JoinHandle<miette::Result<()>>,
    causes: &[&str],
    server: &PendingTools,
) {
    let Err(error) = timeout(TEST_TIMEOUT, task).await.unwrap().unwrap() else {
        panic!("host must fail")
    };
    for cause in causes {
        assert!(error.to_string().contains(cause), "{error}");
    }
    assert!(server.dropped[0].load(Ordering::SeqCst));
}

#[tokio::test]
async fn host_signal_preserves_cause_and_awaits_request_settlement() {
    let server = PendingTools::default();
    let (trigger, signal) = oneshot::channel();
    let mut client = TestClient::start_until(server.clone(), async {
        signal.await.unwrap();
        Ok("SIGTERM")
    })
    .await;
    initialize_client(&mut client, "2025-11-25").await;
    server.start_call(&mut client, 0, 11).await;
    client.send(json!({"jsonrpc":"2.0", "id":12, "method":"tools/call", "params":{"name":"large", "arguments":{}}})).await;
    let prefix = timeout(TEST_TIMEOUT, client.reader.fill_buf())
        .await
        .unwrap()
        .unwrap();
    assert!(!prefix.is_empty());
    // Keep response reads stopped while a second request still owns work.
    trigger.send(()).unwrap();
    assert_host_failure(
        client.server_task,
        &["SIGTERM", "MCP transport failed"],
        &server,
    )
    .await;
}

async fn failure_client<R, W>(
    server: &PendingTools,
    wrap: impl FnOnce((ReadHalf<DuplexStream>, WriteHalf<DuplexStream>), Arc<AtomicBool>) -> (R, W),
) -> (TestClient, Arc<AtomicBool>)
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let fail = Arc::new(AtomicBool::new(false));
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let server_task = tokio::spawn(serve_until(
        server.clone(),
        wrap(split(server_io), fail.clone()),
        pending(),
    ));
    let (reader, writer) = split(client_io);
    let mut client = TestClient {
        reader: BufReader::new(reader),
        writer,
        server_task,
    };
    initialize_client(&mut client, "2025-11-25").await;
    server.start_call(&mut client, 0, 11).await;
    (client, fail)
}

struct FailingRead<R> {
    inner: R,
    fail: Arc<AtomicBool>,
}
impl<R: AsyncRead + Unpin> AsyncRead for FailingRead<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.fail.load(Ordering::SeqCst) {
            return Poll::Ready(Err(io::Error::other("injected transport failure")));
        }
        Pin::new(&mut this.inner).poll_read(context, buffer)
    }
}

#[tokio::test]
async fn transport_failure_is_not_successful_eof_and_settles_handlers() {
    let server = PendingTools::default();
    let (mut client, fail) = failure_client(&server, |(reader, writer), fail| {
        (
            FailingRead {
                inner: reader,
                fail,
            },
            writer,
        )
    })
    .await;
    fail.store(true, Ordering::SeqCst);
    client.writer.write_all(b"\n").await.unwrap();
    assert_host_failure(client.server_task, &["injected transport failure"], &server).await;
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

#[tokio::test]
async fn transport_write_failure_surfaces_and_settles_handlers() {
    for zero in [false, true] {
        let server = PendingTools::default();
        let (mut client, fail) = failure_client(&server, |(reader, writer), fail| {
            (
                reader,
                FailingWrite {
                    inner: writer,
                    fail,
                    zero,
                },
            )
        })
        .await;
        fail.store(true, Ordering::SeqCst);
        client
            .send(json!({"jsonrpc":"2.0", "id":12, "method":"tools/list"}))
            .await;
        let expected = if zero {
            io::Error::from(io::ErrorKind::WriteZero).to_string()
        } else {
            "injected write failure".into()
        };
        let causes = ["MCP transport failed", expected.as_str()];
        assert_host_failure(client.server_task, &causes, &server).await;
    }
}

#[tokio::test]
async fn service_join_failures_preserve_the_cause() {
    for nested in [false, true] {
        let task = tokio::spawn(pending::<()>());
        task.abort();
        let error = task.await.unwrap_err();
        assert!(error.is_cancelled());
        let expected = error.to_string();
        let result = if nested {
            Ok(QuitReason::JoinError(error))
        } else {
            Err(error)
        };
        assert_eq!(service_exit(result).unwrap_err().to_string(), expected);
    }
    service_exit(Ok(QuitReason::Closed)).unwrap();
    service_exit(Ok(QuitReason::Cancelled)).unwrap();
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
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {
            "name": "moriarty-protocol-test",
            "version": "0"
        }
    })
}

async fn request_with_newer_protocol(client: &mut TestClient) -> Value {
    client
        .request(json!({
            "jsonrpc": "2.0",
            "id": 99,
            "method": "tools/list",
            "params": {
                "_meta": protocol_meta("2026-07-28")
            }
        }))
        .await
}

fn assert_newer_protocol_rejected(response: &Value) {
    assert_eq!(response["error"]["code"], -32022);
    assert_eq!(response["error"]["data"]["requested"], "2026-07-28");
    let supported: BTreeSet<_> = response["error"]["data"]["supported"]
        .as_array()
        .unwrap()
        .iter()
        .map(|version| version.as_str().unwrap())
        .collect();
    assert_eq!(
        supported,
        ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"]
            .into_iter()
            .collect()
    );
    assert!(response.get("result").is_none());
}

async fn assert_server_rejects_newer_protocol<S: ServerHandler + Clone>(server: S) {
    let (mut initialized, response) = initialize(server.clone(), "2025-11-25").await;
    assert_eq!(response["result"]["protocolVersion"], "2025-11-25");
    assert_newer_protocol_rejected(&request_with_newer_protocol(&mut initialized).await);
    initialized.close().await;

    let mut fresh = TestClient::start(server).await;
    assert_newer_protocol_rejected(&request_with_newer_protocol(&mut fresh).await);
    fresh.close().await;
}

async fn assert_discover_fallback<S: ServerHandler + Clone>(server: S) {
    let mut client = TestClient::start(server).await;
    let discovery = client
        .request(json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "server/discover",
            "params": { "_meta": protocol_meta("2026-07-28") }
        }))
        .await;
    assert_eq!(discovery["id"], 0);
    assert_newer_protocol_rejected(&discovery);

    let initialized = initialize_client(&mut client, "2025-11-25").await;
    assert_eq!(initialized["result"]["protocolVersion"], "2025-11-25");
    let tools = list_tools(&mut client).await;
    assert!(
        !tools["result"]["tools"].as_array().unwrap().is_empty(),
        "{tools}"
    );
    assert_legacy_result_shape(&tools);

    assert_newer_protocol_rejected(&request_with_newer_protocol(&mut client).await);
    let tools = list_tools(&mut client).await;
    assert!(tools.get("error").is_none(), "{tools}");
    client.close().await;
}

fn assert_legacy_result_shape(response: &Value) {
    assert!(
        response["result"].get("resultType").is_none(),
        "legacy response unexpectedly contains resultType: {response}"
    );
}

fn find_tool<'a>(response: &'a Value, name: &str) -> &'a Value {
    response["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == name)
        .unwrap_or_else(|| panic!("missing tool {name}"))
}

fn assert_tool_contract(response: &Value, expected: &[(&str, &[&str])]) {
    let tools = response["result"]["tools"].as_array().unwrap();
    let actual_names: BTreeSet<_> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    let expected_names: BTreeSet<_> = expected.iter().map(|(name, _)| *name).collect();
    assert_eq!(actual_names, expected_names);

    for (name, required) in expected {
        let tool = find_tool(response, name);
        let actual_required: BTreeSet<_> = tool["inputSchema"]["required"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|field| field.as_str().unwrap())
            .collect();
        assert_eq!(actual_required, required.iter().copied().collect());
    }
}

async fn assert_git_contract(requested_version: &str, expected_version: &str) {
    let (mut client, response) = initialize(GitReadOnly, requested_version).await;
    assert_eq!(response["result"]["protocolVersion"], expected_version);

    let tools = list_tools(&mut client).await;
    assert_tool_contract(
        &tools,
        &[
            ("status", &["args"]),
            ("diff", &["args"]),
            ("log", &["args"]),
            ("show", &["args"]),
        ],
    );
    client.close().await;
}

#[test]
fn omitted_project_dir_defaults_to_server_cwd() {
    let git: StatusArgs = from_value(json!({ "args": [] })).unwrap();
    let jj: JjArgs = from_value(json!({ "command": "status", "args": [] })).unwrap();
    let tools: RunArgs = from_value(json!({})).unwrap();
    for dir in [git.project_dir, jj.project_dir, tools.project_dir] {
        assert_eq!(dir.canonicalize().unwrap(), current_dir().unwrap());
    }
}

#[tokio::test]
async fn git_transport_preserves_protocol_and_tool_contracts() {
    assert_git_contract("2025-06-18", "2025-06-18").await;
    assert_git_contract("2026-07-28", "2025-11-25").await;
}

#[tokio::test]
async fn unknown_older_protocol_falls_back_to_ceiling() {
    assert_git_contract("1999-01-01", "2025-11-25").await;
}

#[tokio::test]
async fn git_rejects_newer_request_protocol() {
    assert_server_rejects_newer_protocol(GitReadOnly).await;
}

#[tokio::test]
async fn jj_rejects_newer_request_protocol() {
    assert_server_rejects_newer_protocol(JjReadOnly).await;
}

#[tokio::test]
async fn tool_runner_rejects_newer_request_protocol() {
    assert_server_rejects_newer_protocol(ToolRunner).await;
}

#[tokio::test]
async fn supported_and_versionless_discovery_pass_through_unchanged() {
    let supported = protocol_meta("2025-11-25");
    let mut versionless = supported.clone();
    versionless
        .as_object_mut()
        .unwrap()
        .remove("io.modelcontextprotocol/protocolVersion");
    for meta in [supported, versionless] {
        let expected = json!({
            "jsonrpc": "2.0",
            "id": "discovery",
            "method": "server/discover",
            "params": { "_meta": meta }
        });
        let (inner, _responses) = OneshotTransport::new(from_value(expected.clone()).unwrap());
        let mut transport = DiscoveryFallbackTransport(inner);
        let forwarded = timeout(TEST_TIMEOUT, transport.receive())
            .await
            .expect("timed out forwarding discovery")
            .expect("discovery was intercepted");
        assert_eq!(to_value(forwarded).unwrap(), expected);
    }
}

#[tokio::test]
async fn discovery_rejection_send_failure_closes_transport() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "server/discover",
        "params": { "_meta": protocol_meta("2026-07-28") }
    });
    let (inner, responses) = OneshotTransport::new(from_value(request).unwrap());
    drop(responses);
    let mut transport = DiscoveryFallbackTransport(inner);
    let received = timeout(TEST_TIMEOUT, transport.receive())
        .await
        .expect("timed out after discovery rejection send failure");
    assert!(received.is_none());
}

// Each handler has its own tool contract; keep a fallback regression per server
// so handler changes cannot silently break just one advertised MCP connection.
#[tokio::test]
async fn git_discover_fallback_preserves_legacy_tools() {
    assert_discover_fallback(GitReadOnly).await;
}

#[tokio::test]
async fn jj_discover_fallback_preserves_legacy_tools() {
    assert_discover_fallback(JjReadOnly).await;
}

#[tokio::test]
async fn tool_runner_discover_fallback_preserves_legacy_tools() {
    assert_discover_fallback(ToolRunner).await;
}

#[tokio::test]
async fn git_legacy_prompt_responses_keep_names_and_roles() {
    let (mut client, response) = initialize(GitReadOnly, "2025-11-25").await;
    assert_eq!(response["result"]["protocolVersion"], "2025-11-25");

    let prompts = client
        .request(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "prompts/list",
            "params": {}
        }))
        .await;
    assert_legacy_result_shape(&prompts);
    let prompt_names: BTreeSet<_> = prompts["result"]["prompts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|prompt| prompt["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        prompt_names,
        ["status", "diff", "log", "show"].into_iter().collect()
    );

    let project = tempdir().unwrap();
    let prompt = client
        .request(json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "prompts/get",
            "params": {
                "name": "status",
                "arguments": {
                    "project_dir": project.path().to_string_lossy(),
                    "args": []
                }
            }
        }))
        .await;
    assert_legacy_result_shape(&prompt);
    setup_git_repo_with_commit(project.path());
    fs::write(project.path().join("git-default-cwd-marker.txt"), "fixture").unwrap();
    // Nextest isolates each test process, so changing cwd cannot affect sibling tests.
    let original_dir = current_dir().unwrap();
    set_current_dir(project.path()).unwrap();
    let status = client
        .request(json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": { "name": "status", "arguments": { "args": [] } }
        }))
        .await;
    set_current_dir(original_dir).unwrap();
    assert_eq!(
        status["result"]["structuredContent"]["exit_code"], 0,
        "{status}"
    );
    assert!(
        status["result"]["structuredContent"]["stdout"]
            .as_str()
            .unwrap()
            .contains("git-default-cwd-marker.txt"),
        "{status}"
    );
    assert_eq!(
        prompt["result"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message["role"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["assistant", "user"]
    );
    client.close().await;
}

#[tokio::test]
async fn jj_transport_preserves_protocol_and_tool_contracts() {
    let (mut client, response) = initialize(JjReadOnly, "2025-11-25").await;
    assert_eq!(response["result"]["protocolVersion"], "2025-11-25");

    let tools = list_tools(&mut client).await;
    assert_tool_contract(&tools, &[("run", &["command", "args"])]);
    let command_values: BTreeSet<_> = find_tool(&tools, "run")["inputSchema"]["$defs"]["JjCommand"]
        ["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();
    assert_eq!(
        command_values,
        [
            "status",
            "diff",
            "log",
            "show",
            "op-log",
            "file-show",
            "file-list"
        ]
        .into_iter()
        .collect()
    );
    let project = tempdir().unwrap();
    setup_git_repo_with_commit(project.path());
    let initialized = Command::new("jj")
        .args(["git", "init", "--colocate"])
        .current_dir(project.path())
        .output()
        .unwrap();
    assert!(initialized.status.success(), "{initialized:?}");
    fs::write(project.path().join("jj-default-cwd-marker.txt"), "fixture").unwrap();
    // The read-only server ignores working-copy updates, so snapshot the fixture first.
    let snapshot = Command::new("jj")
        .arg("status")
        .current_dir(project.path())
        .output()
        .unwrap();
    assert!(snapshot.status.success(), "{snapshot:?}");
    let original_dir = current_dir().unwrap();
    set_current_dir(project.path()).unwrap();
    let status = client
        .request(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "run",
                "arguments": { "command": "status", "args": [] }
            }
        }))
        .await;
    set_current_dir(original_dir).unwrap();
    assert_eq!(
        status["result"]["structuredContent"]["exit_code"], 0,
        "{status}"
    );
    assert!(
        status["result"]["structuredContent"]["stdout"]
            .as_str()
            .unwrap()
            .contains("jj-default-cwd-marker.txt"),
        "{status}"
    );
    client.close().await;
}

#[tokio::test]
async fn tool_runner_transport_preserves_protocol_and_tool_contracts() {
    let (mut client, response) = initialize(ToolRunner, "2025-11-25").await;
    assert_eq!(response["result"]["protocolVersion"], "2025-11-25");

    let tools = list_tools(&mut client).await;
    assert_tool_contract(
        &tools,
        &[
            ("run_lint", &[]),
            ("run_build", &[]),
            ("run_formatter", &[]),
            ("run_tests", &[]),
            ("run_checks", &[]),
        ],
    );
    client.close().await;
}

#[tokio::test]
async fn tool_runner_legacy_responses_keep_content_and_error_shapes() {
    let _xdg_dir = setup_isolated_xdg_config();
    let config = r#"[commands]
test = ["sh", "-c", "printf success-out; printf success-err >&2"]
lint = ["sh", "-c", "printf failure-out; printf failure-err >&2; exit 7"]
"#;
    let project = setup_project_dir_with_config(config);
    approvals::approve_project_config(project.path(), config)
        .await
        .unwrap();
    let project_dir = project.path().to_string_lossy();
    let (mut client, response) = initialize(ToolRunner, "2025-11-25").await;
    assert_eq!(response["result"]["protocolVersion"], "2025-11-25");

    let success = client
        .request(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "run_tests",
                "arguments": { "project_dir": project_dir }
            }
        }))
        .await;
    assert_legacy_result_shape(&success);
    assert_eq!(success["result"]["isError"], false);
    assert_eq!(success["result"]["content"].as_array().unwrap().len(), 2);
    assert_eq!(success["result"]["content"][0]["type"], "text");
    assert_eq!(
        success["result"]["content"][0]["text"],
        "stdout: \n\n success-out"
    );
    assert_eq!(success["result"]["content"][1]["type"], "text");
    assert_eq!(
        success["result"]["content"][1]["text"],
        "stderr: \n\n success-err"
    );

    let nonzero = client
        .request(json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "run_lint",
                "arguments": { "project_dir": project_dir }
            }
        }))
        .await;
    assert_legacy_result_shape(&nonzero);
    assert_eq!(nonzero["result"]["isError"], true);

    let original_dir = current_dir().unwrap();
    set_current_dir(project.path()).unwrap();
    let defaulted = client
        .request(json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": { "name": "run_tests", "arguments": {} }
        }))
        .await;
    set_current_dir(original_dir).unwrap();
    assert_eq!(defaulted["result"]["isError"], false, "{defaulted}");
    assert_eq!(
        defaulted["result"]["content"][0]["text"],
        "stdout: \n\n success-out"
    );

    let absent = client
        .request(json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "run_build",
                "arguments": { "project_dir": project_dir }
            }
        }))
        .await;
    assert_eq!(absent["error"]["code"], -32002);
    assert!(absent.get("result").is_none());
    client.close().await;
}
