use super::*;

async fn assert_discovery_contract<S: ServerHandler + Clone>(server: S, prompts: bool) {
    let mut legacy_tools = None;
    for modern in [false, true] {
        let mut client = TestClient::connect(server.clone(), modern).await;
        if modern {
            let discovery = client
                .request(json!({
                    "jsonrpc": "2.0", "id": 0, "method": "server/discover", "params": {}
                }))
                .await;
            assert_result_shape(&discovery, true, true);
            let capabilities = &discovery["result"]["capabilities"];
            assert!(capabilities.get("tools").is_some(), "{discovery}");
            assert_eq!(
                capabilities.get("prompts").is_some(),
                prompts,
                "{discovery}"
            );
            assert!(
                capabilities["extensions"]
                    .get("io.modelcontextprotocol/tasks")
                    .is_none(),
                "{discovery}"
            );
        }
        let tools = list_tools(&mut client).await;
        assert_result_shape(&tools, modern, true);
        if let Some(legacy) = &legacy_tools {
            assert_eq!(&tools["result"]["tools"], legacy);
        } else {
            legacy_tools = Some(tools["result"]["tools"].clone());
        }
        client.close().await;
    }
}

#[tokio::test]
async fn discovery_capabilities_and_tool_schemas_match_both_lifecycles() {
    assert_discovery_contract(GitReadOnly, true).await;
    assert_discovery_contract(JjReadOnly, false).await;
    assert_discovery_contract(ToolRunner, false).await;
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
        let properties = tool["inputSchema"]["properties"].as_object().unwrap();
        let expected_properties: BTreeSet<_> =
            required.iter().copied().chain(["project_dir"]).collect();
        assert_eq!(
            properties
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            expected_properties
        );
        assert_eq!(properties["project_dir"]["type"], "string");
        assert_eq!(properties["project_dir"]["default"], ".");
        if let Some(args) = properties.get("args") {
            assert_eq!(args["type"], "array");
            assert_eq!(args["items"]["type"], "string");
        }
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
async fn unknown_older_protocol_falls_back_to_legacy() {
    assert_git_contract("1999-01-01", "2025-11-25").await;
}

#[tokio::test]
async fn git_prompt_and_tool_responses_keep_names_content_and_roles() {
    for modern in [false, true] {
        let mut client = TestClient::connect(GitReadOnly, modern).await;

        let prompts = client
            .request(json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "prompts/list",
                "params": {}
            }))
            .await;
        assert_result_shape(&prompts, modern, true);
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
        assert_result_shape(&prompt, modern, false);
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
        assert_result_shape(&status, modern, false);
        let assistant = prompt["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap();
        let user = prompt["result"]["messages"][1]["content"]["text"]
            .as_str()
            .unwrap();
        assert!(assistant.contains("status"), "{prompt}");
        assert!(user.contains("status"), "{prompt}");
        assert!(user.contains(project.path().to_str().unwrap()), "{prompt}");
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
}

#[tokio::test]
async fn jj_transport_preserves_protocol_and_tool_contracts() {
    for modern in [false, true] {
        let mut client = TestClient::connect(JjReadOnly, modern).await;

        let tools = list_tools(&mut client).await;
        assert_result_shape(&tools, modern, true);
        assert_tool_contract(&tools, &[("run", &["command", "args"])]);
        let command_values: BTreeSet<_> =
            find_tool(&tools, "run")["inputSchema"]["$defs"]["JjCommand"]["enum"]
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
        assert_result_shape(&status, modern, false);
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
}

#[tokio::test]
async fn tool_runner_transport_preserves_protocol_and_tool_contracts() {
    for modern in [false, true] {
        let mut client = TestClient::connect(ToolRunner, modern).await;

        let tools = list_tools(&mut client).await;
        assert_result_shape(&tools, modern, true);
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
}

#[tokio::test]
async fn tool_runner_responses_keep_content_and_error_shapes() {
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
    for modern in [false, true] {
        let mut client = TestClient::connect(ToolRunner, modern).await;

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
        assert_result_shape(&success, modern, false);
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
        assert_result_shape(&nonzero, modern, false);
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
        assert_result_shape(&defaulted, modern, false);
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
}
