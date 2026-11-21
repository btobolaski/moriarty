use std::{borrow::Cow, future::ready, path::Path};

use rmcp::{
    model::{ClientNotification, ClientRequest, ProtocolVersion, RequestId},
    service::NotificationContext,
};
use tempfile::TempDir;
use tokio::{io::duplex, time::sleep};

use super::*;
use crate::test_helpers::task_meta;

const CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const TASKS: &str = "io.modelcontextprotocol/tasks";
const TASK_METHODS: [&str; 3] = ["tasks/get", "tasks/update", "tasks/cancel"];

fn request(id: i32, method: &str, mut params: Value, meta: Value) -> Value {
    params["_meta"] = meta;
    json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params})
}

fn task_call(id: i32, tool: &str, project: &Path) -> Value {
    request(
        id,
        "tools/call",
        json!({"name":tool,"arguments":{"project_dir":project}}),
        task_meta(),
    )
}

fn management(id: i32, method: &str, task: &str, meta: Value) -> Value {
    let mut params = json!({"taskId":task});
    if method == "tasks/update" {
        params = to_value(UpdateTaskParams::new(task, Default::default())).unwrap();
    }
    request(id, method, params, meta)
}

fn handle(response: &Value) -> &str {
    assert_eq!(response["result"]["resultType"], "task", "{response}");
    assert_eq!(response["result"].get("ttlMs"), Some(&Value::Null));
    response["result"]["taskId"].as_str().unwrap()
}

async fn get(client: &mut TestClient, id: &str) -> Value {
    let reply = client
        .request(management(70, "tasks/get", id, task_meta()))
        .await;
    assert_eq!(reply["result"]["resultType"], "complete", "{reply}");
    assert_eq!(reply["result"]["taskId"], id);
    reply["result"].clone()
}

async fn terminal(client: &mut TestClient, id: &str) -> Value {
    timeout(TEST_TIMEOUT, async {
        loop {
            let task = get(client, id).await;
            if task["status"] != "working" {
                return task;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn only_valid_opted_in_calls_create_tasks() {
    let config = r#"[commands]
test = ["echo", "dispatch"]
lint = ["echo", "dispatch"]
build = ["echo", "dispatch"]
format = ["echo", "dispatch"]
[[checks]]
name = "dispatch"
command = ["echo", "dispatch"]
"#;
    let (project, _xdg) = approved_jj_project(config).await;
    let starts = Arc::new(AtomicUsize::new(0));
    let scope = scope_with_operations({
        let starts = starts.clone();
        move |spec: ProcessSpec, _| {
            assert_eq!(spec.args, ["dispatch"]);
            starts.fetch_add(1, Ordering::SeqCst);
            ready(Ok(test_report(Completion::Exited(ExitStatus::from_raw(0)))))
        }
    });
    let mut client = TestClient::start_in_scope(ProjectTools, pending(), scope, true).await;
    for (index, tool) in [
        "run_lint",
        "run_build",
        "run_formatter",
        "run_tests",
        "run_checks",
    ]
    .into_iter()
    .enumerate()
    {
        let reply = client.request(task_call(2, tool, project.path())).await;
        let result = terminal(&mut client, handle(&reply)).await;
        assert_eq!(result["status"], "completed", "{result}");
        assert_eq!(result["result"]["isError"], false, "{result}");
        assert_eq!(starts.load(Ordering::SeqCst), index + 1, "{tool}");
    }
    let reply = client
        .request(request(
            3,
            "tools/call",
            json!({"name":"run_checks", "task":{}, "arguments":{"project_dir":project.path()}}),
            protocol_meta("2026-07-28"),
        ))
        .await;
    assert_result_shape(&reply, true, false);
    assert!(reply["result"].get("taskId").is_none(), "{reply}");

    let validated_starts = starts.load(Ordering::SeqCst);
    assert_eq!(
        validated_starts, 6,
        "the opted-out call still runs directly"
    );
    let reply = client
        .request(task_call(4, "no-such-tool", project.path()))
        .await;
    assert_eq!(reply["error"]["code"], -32602);
    assert!(reply.get("result").is_none());
    let reply = client
        .request(request(
            5,
            "tools/call",
            json!({"name":"run_tests","arguments":{"project_dir":52}}),
            task_meta(),
        ))
        .await;
    assert_result_shape(&reply, true, false);
    assert_eq!(reply["result"]["isError"], true);
    assert!(reply["result"].get("taskId").is_none());
    assert_eq!(starts.load(Ordering::SeqCst), validated_starts);
    client.close().await;
}

#[tokio::test]
async fn task_management_checks_capability_and_connection_before_lookup() {
    let (project, _xdg) = approved_jj_project("[commands]").await;
    let service = ProjectTools;
    let mut client = TestClient::connect(service, true).await;
    let reply = client
        .request(task_call(1, "run_checks", project.path()))
        .await;
    let id = handle(&reply);
    terminal(&mut client, id).await;
    let mut other = TestClient::connect(service, true).await;
    for method in TASK_METHODS {
        let reply = other.request(management(1, method, id, task_meta())).await;
        assert_eq!(reply["error"]["code"], -32602, "{reply}");
        let reply = client
            .request(management(2, method, "absent", task_meta()))
            .await;
        assert_eq!(reply["error"]["code"], -32602, "{reply}");
        let reply = client
            .request(management(3, method, "absent", protocol_meta("2026-07-28")))
            .await;
        assert_eq!(reply["error"]["code"], -32021, "{reply}");
        assert_eq!(
            reply["error"]["data"]["requiredCapabilities"]["extensions"][TASKS],
            json!({})
        );
    }
    other.close().await;
    client.close().await;
}

#[tokio::test]
async fn task_requests_preserve_sdk_revision_and_metadata_error_precedence() {
    let mut client = TestClient::connect(ProjectTools, true).await;
    list_tools(&mut client).await;
    let mut malformed = task_meta();
    malformed.as_object_mut().unwrap().remove(CAPABILITIES);
    for (meta, code) in [
        (malformed.clone(), -32602),
        (
            {
                malformed["io.modelcontextprotocol/protocolVersion"] = json!("2099-01-01");
                malformed
            },
            -32022,
        ),
    ] {
        let call = request(4, "tools/call", json!({"name":"run_tests"}), meta.clone());
        for message in [call, management(5, "tasks/get", "absent", meta)] {
            let reply = client.request(message).await;
            assert_eq!(reply["error"]["code"], code, "{reply}");
            if code == -32602 {
                assert!(
                    reply["error"]["message"]
                        .as_str()
                        .unwrap()
                        .contains(CAPABILITIES)
                );
            }
        }
    }
    client.close().await;
}

#[tokio::test]
async fn modern_discovery_does_not_enable_tasks_for_legacy_initialization() {
    let project = tempdir().unwrap();
    for declares_tasks in [false, true] {
        let mut legacy = TestClient::start(ProjectTools).await;
        let discovery = legacy
            .raw_request(request(1, "server/discover", json!({}), task_meta()))
            .await;
        assert_eq!(
            discovery["result"]["capabilities"]["extensions"][TASKS],
            json!({})
        );
        let caps = if declares_tasks {
            task_meta()[CAPABILITIES].clone()
        } else {
            json!({})
        };
        let initialized =
            initialize_client_with_capabilities(&mut legacy, "2025-11-25", caps.clone()).await;
        assert_eq!(initialized["result"]["protocolVersion"], "2025-11-25");
        assert!(
            initialized["result"]["capabilities"]["extensions"]
                .get(TASKS)
                .is_none()
        );
        let response = legacy.request(json!({"jsonrpc":"2.0", "id":3,"method":"tools/call", "params":{"name":"run_checks","arguments":{"project_dir":project.path()}}})).await;
        assert_legacy_result_shape(&response);
        assert!(response["result"].get("content").is_some(), "{response}");
        // The repeated initialize can update rmcp's stored peer metadata, but
        // that is not per-request opt-in to the draft.
        let repeated = initialize_client_with_capabilities(&mut legacy, "2026-07-28", caps).await;
        assert_eq!(repeated["result"]["protocolVersion"], "2025-11-25");
        for method in TASK_METHODS {
            let mut message = management(4, method, "absent", task_meta());
            message["params"].as_object_mut().unwrap().remove("_meta");
            let reply = legacy.request(message).await;
            assert_eq!(reply["error"]["code"], -32601, "{reply}");
        }
        legacy.close().await;
    }
}

#[derive(Default)]
struct HeldCommand {
    starts: AtomicUsize,
    entered: Notify,
    cancelled: Notify,
    release: Notify,
    finished: Notify,
    fail_cleanup: AtomicBool,
}

fn held_command_scope() -> (Arc<Scope>, [Arc<HeldCommand>; 2]) {
    let commands: [Arc<HeldCommand>; 2] = Default::default();
    let scope = scope_with_operations({
        let commands = commands.clone();
        move |spec: ProcessSpec, token| {
            let arg = spec.args.first().unwrap();
            let held = commands[usize::from(arg == "unrelated")].clone();
            async move {
                let Some(arg) = spec.args.first().and_then(|arg| arg.to_str()) else {
                    panic!("expected a command argument: {spec:?}");
                };
                let output = match arg {
                    "held" | "unrelated" => {
                        held.starts.fetch_add(1, Ordering::SeqCst);
                        held.entered.notify_one();
                        let completion = tokio::select! {
                            () = token.cancelled() => {
                                held.cancelled.notify_one();
                                held.release.notified().await;
                                Completion::Cancelled(ExitStatus::from_raw(0))
                            },
                            () = held.release.notified() => Completion::Exited(ExitStatus::from_raw(0)),
                        };
                        held.finished.notify_one();
                        if held.fail_cleanup.load(Ordering::SeqCst) {
                            return Err(ProcessError::CleanupUnconfirmed {
                                state: Box::default(),
                            }
                            .into());
                        }
                        let mut report = test_report(completion);
                        report.stdout.bytes = b"original result".to_vec();
                        return Ok(report);
                    }
                    "large" => vec![b'x'; 256 * 1024],
                    _ => panic!("unexpected command: {spec:?}"),
                };
                let mut report = test_report(Completion::Exited(ExitStatus::from_raw(0)));
                report.stdout.bytes = output;
                Ok(report)
            }
        }
    });
    (scope, commands)
}

const COMMANDS: &str = r#"[commands]
test = ["echo", "held"]
lint = ["echo", "unrelated"]
build = ["echo", "large"]
"#;

#[derive(Clone, Default)]
struct HeldPoll {
    server: ProjectTools,
    entered: Arc<Notify>,
    cancelled: Arc<Notify>,
}

impl Service<RoleServer> for HeldPoll {
    async fn handle_request(
        &self,
        request: ClientRequest,
        context: RequestContext<RoleServer>,
    ) -> Result<ServerResult, McpError> {
        let hold = matches!(request, ClientRequest::GetTaskRequest(_))
            && context.id == RequestId::Number(71);
        let token = context.ct.clone();
        let reply = self.server.handle_request(request, context).await?;
        if hold {
            self.entered.notify_one();
            token.cancelled().await;
            self.cancelled.notify_one();
        }
        Ok(reply)
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

async fn approved_jj_project(config: &str) -> (TempDir, TempDir) {
    let xdg = setup_isolated_xdg_config();
    let project = setup_project_dir_with_config(config);
    fs::create_dir_all(project.path().join(".jj/repo")).unwrap();
    approvals::approve_project_config(project.path(), config)
        .await
        .unwrap();
    (project, xdg)
}

#[tokio::test]
async fn task_handle_precedes_command_result_and_request_cancellation_does_not_own_work() {
    let (project, _xdg) = approved_jj_project(COMMANDS).await;
    let (scope, [held, _]) = held_command_scope();
    let poll = HeldPoll::default();
    let mut client = TestClient::start_in_scope(poll.clone(), pending(), scope, true).await;
    let reply = client
        .request(task_call(11, "run_tests", project.path()))
        .await;
    let id = handle(&reply);
    timeout(TEST_TIMEOUT, held.entered.notified())
        .await
        .unwrap();
    assert_eq!(get(&mut client, id).await["status"], "working");
    client
        .send_request(management(71, "tasks/get", id, task_meta()))
        .await;
    timeout(TEST_TIMEOUT, poll.entered.notified())
        .await
        .unwrap();
    for request_id in [11, 71] {
        client.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":request_id}})).await;
    }
    timeout(TEST_TIMEOUT, poll.cancelled.notified())
        .await
        .unwrap();
    let mut update = management(12, "tasks/update", id, task_meta());
    update["params"]["inputResponses"] =
        json!({"unsolicited":{"resultType":"complete", "action":"accept"}});
    client.send_request(update).await;
    loop {
        let reply = client.response().await;
        if reply["id"] == 12 {
            assert_eq!(reply["result"], json!({"resultType":"complete"}), "{reply}");
            break;
        }
        assert_eq!(reply["id"], 71);
    }
    assert_eq!(get(&mut client, id).await["status"], "working");
    held.release.notify_one();
    let result = terminal(&mut client, id).await;
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(result["result"]["isError"], false);
    assert_eq!(
        result["result"]["content"][0]["text"],
        "stdout: \n\n original result"
    );
    assert_eq!(held.starts.load(Ordering::SeqCst), 1);
    client.close().await;
}

#[tokio::test]
async fn task_cancel_acknowledges_while_cleanup_is_held() {
    let (project, _xdg) = approved_jj_project(COMMANDS).await;
    let (scope, [held, unrelated]) = held_command_scope();
    let mut client = TestClient::start_in_scope(ProjectTools, pending(), scope, true).await;
    let reply = client
        .request(task_call(11, "run_tests", project.path()))
        .await;
    let id = handle(&reply);
    timeout(TEST_TIMEOUT, held.entered.notified())
        .await
        .unwrap();
    let mut direct = task_call(13, "run_lint", project.path());
    direct["params"]["_meta"] = protocol_meta("2026-07-28");
    client.send_request(direct).await;
    timeout(TEST_TIMEOUT, unrelated.entered.notified())
        .await
        .unwrap();
    let reply = client
        .request(management(12, "tasks/cancel", id, task_meta()))
        .await;
    assert_eq!(reply["result"], json!({"resultType":"complete"}));
    timeout(TEST_TIMEOUT, held.cancelled.notified())
        .await
        .unwrap();
    assert_eq!(get(&mut client, id).await["status"], "working");

    unrelated.release.notify_one();
    let reply = client.response().await;
    assert_eq!(reply["id"], 13);
    assert_result_shape(&reply, true, false);
    assert_eq!(reply["result"]["isError"], false);

    held.release.notify_one();
    let task = terminal(&mut client, id).await;
    assert_eq!(task["status"], "cancelled");
    assert!(task.get("result").is_none() && task.get("error").is_none());
    client.close().await;
}

#[tokio::test]
async fn poll_reports_interruption_and_unconfirmed_cleanup() {
    let (project, _xdg) = approved_jj_project(COMMANDS).await;
    let (scope, [held, _]) = held_command_scope();
    held.fail_cleanup.store(true, Ordering::SeqCst);
    let mut client = TestClient::start_in_scope(ProjectTools, pending(), scope, true).await;
    let reply = client
        .request(task_call(1, "run_tests", project.path()))
        .await;
    let id = handle(&reply);
    timeout(TEST_TIMEOUT, held.entered.notified())
        .await
        .unwrap();
    client
        .request(management(2, "tasks/cancel", id, task_meta()))
        .await;
    timeout(TEST_TIMEOUT, held.cancelled.notified())
        .await
        .unwrap();
    held.release.notify_one();

    let result = terminal(&mut client, id).await;
    assert_eq!(result["status"], "failed", "{result}");
    assert_eq!(result["error"]["code"], -32603);
    assert!(result.get("result").is_none());
    let message = result["error"]["message"].as_str().unwrap();
    assert!(message.contains("Operation cancelled"), "{message}");
    assert!(
        message.contains("Settlement: process cleanup or capture settlement unconfirmed"),
        "{message}"
    );

    client.writer.shutdown().await.unwrap();
    let drain_error = timeout(TEST_TIMEOUT, client.server_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(
        drain_error.to_string().contains("settlement unconfirmed"),
        "{drain_error}"
    );
}

#[tokio::test]
async fn signal_under_output_backpressure_drains_task_without_a_poll() {
    assert_mixed_owner_drain(ShutdownCase::Signal).await;
}

#[tokio::test]
async fn stdin_eof_drains_task_without_a_poll() {
    assert_mixed_owner_drain(ShutdownCase::Eof).await;
}

#[tokio::test]
async fn failed_handle_delivery_drains_task_without_a_poll() {
    assert_mixed_owner_drain(ShutdownCase::WriteFailure).await;
}

#[derive(Clone, Copy)]
enum ShutdownCase {
    Signal,
    Eof,
    WriteFailure,
}

async fn assert_mixed_owner_drain(case: ShutdownCase) {
    let (project, _xdg) = approved_jj_project(COMMANDS).await;
    let (scope, owners) = held_command_scope();
    owners[0].fail_cleanup.store(true, Ordering::SeqCst);
    let (trigger, stop) = oneshot::channel();
    let (server_io, client_io) = duplex(64 * 1024);
    let (read, write) = split(server_io);
    let fail_write = matches!(case, ShutdownCase::WriteFailure);
    let (write, blocked, resume) = GatedWrite::new(FailingWrite {
        inner: write,
        fail: Arc::new(AtomicBool::new(fail_write)),
        zero: false,
    });
    if !fail_write {
        resume.cancel();
    }
    let server_task = tokio::spawn(serve_until(
        ProjectTools,
        (read, write),
        async {
            stop.await.unwrap();
            Ok("SIGTERM")
        },
        scope.clone(),
    ));
    let (reader, writer) = split(client_io);
    let mut client = TestClient {
        modern: true,
        reader: BufReader::new(reader),
        writer,
        server_task,
    };

    let mut lint = task_call(3, "run_lint", project.path());
    lint["params"]["_meta"] = protocol_meta("2026-07-28");
    client.send_request(lint).await;
    client
        .send_request(task_call(1, "run_tests", project.path()))
        .await;
    for owner in &owners {
        timeout(TEST_TIMEOUT, owner.entered.notified())
            .await
            .unwrap();
    }
    if !fail_write {
        handle(&client.response().await);
    }
    match case {
        ShutdownCase::Signal => {
            let mut build = task_call(2, "run_build", project.path());
            build["params"]["_meta"] = protocol_meta("2026-07-28");
            client.send_request(build).await;
            let prefix = timeout(TEST_TIMEOUT, client.reader.fill_buf())
                .await
                .unwrap()
                .unwrap();
            assert!(!prefix.is_empty());
            trigger.send(()).unwrap();
        }
        ShutdownCase::Eof => client.writer.shutdown().await.unwrap(),
        ShutdownCase::WriteFailure => {
            timeout(TEST_TIMEOUT, blocked.notified()).await.unwrap();
            resume.cancel();
        }
    }
    for owner in &owners {
        timeout(TEST_TIMEOUT, owner.cancelled.notified())
            .await
            .unwrap();
    }
    assert_scope_rejects_admission(&scope).await;
    for owner in owners.iter().rev() {
        assert!(
            poll!(Pin::new(&mut client.server_task)).is_pending(),
            "host returned before both owners settled"
        );
        owner.release.notify_one();
        timeout(TEST_TIMEOUT, owner.finished.notified())
            .await
            .unwrap();
    }
    let error = timeout(TEST_TIMEOUT, &mut client.server_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(
        error.to_string().contains("settlement unconfirmed"),
        "{error}"
    );
    match case {
        ShutdownCase::Signal => {
            for expected in ["SIGTERM", "MCP transport failed"] {
                assert!(error.to_string().contains(expected), "{error}");
            }
        }
        ShutdownCase::Eof => {}
        ShutdownCase::WriteFailure => {
            assert!(
                error.to_string().contains("injected write failure"),
                "{error}"
            );
            assert!(
                client.reader.fill_buf().await.unwrap().is_empty(),
                "handle must not have been delivered"
            );
        }
    }
}
