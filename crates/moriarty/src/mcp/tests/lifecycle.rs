use super::*;

async fn request_with_newer_protocol(client: &mut TestClient) -> Value {
    client
        .request(json!({
            "jsonrpc": "2.0",
            "id": 99,
            "method": "tools/list",
            "params": {
                "_meta": protocol_meta("2099-01-01")
            }
        }))
        .await
}

fn assert_newer_protocol_rejected(response: &Value) {
    assert_eq!(response["error"]["code"], -32022);
    assert_eq!(response["error"]["data"]["requested"], "2099-01-01");
    let supported: BTreeSet<_> = response["error"]["data"]["supported"]
        .as_array()
        .unwrap()
        .iter()
        .map(|version| version.as_str().unwrap())
        .collect();
    assert_eq!(supported, EXPECTED_PROTOCOL_VERSIONS.into_iter().collect());
    assert!(response.get("result").is_none());
}

async fn assert_server_rejects_newer_protocol<S: ServerHandler + Clone>(server: S) {
    let (mut initialized, response) = initialize(server.clone(), "2025-11-25").await;
    assert_eq!(response["result"]["protocolVersion"], "2025-11-25");
    assert_newer_protocol_rejected(&request_with_newer_protocol(&mut initialized).await);
    initialized.close().await;

    let mut fresh = TestClient::start(server).await;
    assert_newer_protocol_rejected(&request_with_newer_protocol(&mut fresh).await);
    let recovered = initialize_client(&mut fresh, "2025-11-25").await;
    assert_eq!(recovered["result"]["protocolVersion"], "2025-11-25");
    fresh.close().await;
}

async fn assert_discover_fallback<S: ServerHandler + Clone>(server: S) {
    let mut client = TestClient::start(server).await;
    let discovery = client
        .request(json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "server/discover",
            "params": { "_meta": protocol_meta("2099-01-01") }
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

async fn assert_modern_discovery_recovers<S: ServerHandler + Clone>(server: S) {
    for version in ["2026-07-28", "2099-01-01"] {
        let mut client = TestClient::connect(server.clone(), true).await;
        let discovery = client
            .request(json!({
                "jsonrpc": "2.0", "id": 0, "method": "server/discover",
                "params": { "_meta": protocol_meta(version) }
            }))
            .await;
        if version == "2099-01-01" {
            assert_newer_protocol_rejected(&discovery);
        } else {
            assert_eq!(discovery["result"]["resultType"], "complete", "{discovery}");
        }
        let tools = list_tools(&mut client).await;
        assert_eq!(tools["result"]["resultType"], "complete", "{tools}");
        client.close().await;
    }
}

#[tokio::test]
async fn discovery_then_modern_work_on_all_servers() {
    assert_modern_discovery_recovers(GitReadOnly).await;
    assert_modern_discovery_recovers(JjReadOnly).await;
    assert_modern_discovery_recovers(ToolRunner).await;
}

async fn assert_incomplete_unsupported_discovery_recovers<S: ServerHandler + Clone>(server: S) {
    for modern in [false, true] {
        let mut client =
            TestClient::start_in_scope(server.clone(), pending(), Scope::new(), modern).await;
        let malformed = client
            .raw_request(json!({
                "jsonrpc":"2.0", "id":-1, "method":"server/discover",
                "params":{"_meta":{"io.modelcontextprotocol/protocolVersion":42}}
            }))
            .await;
        assert_eq!(malformed["error"]["code"], -32602, "{malformed}");
        let discovery = client
            .raw_request(json!({
                "jsonrpc":"2.0", "id":0, "method":"server/discover",
                "params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2099-01-01"}}
            }))
            .await;
        assert_eq!(discovery["id"], 0);
        assert_newer_protocol_rejected(&discovery);
        if !modern {
            let initialized = initialize_client(&mut client, "2025-11-25").await;
            assert_eq!(initialized["result"]["protocolVersion"], "2025-11-25");
        }
        let tools = list_tools(&mut client).await;
        assert!(tools.get("error").is_none(), "{tools}");
        assert_result_shape(&tools, modern, true);
        client.close().await;
    }
}

#[tokio::test]
async fn incomplete_unsupported_discovery_recovers_on_all_servers() {
    assert_incomplete_unsupported_discovery_recovers(GitReadOnly).await;
    assert_incomplete_unsupported_discovery_recovers(JjReadOnly).await;
    assert_incomplete_unsupported_discovery_recovers(ToolRunner).await;
}

#[tokio::test]
async fn first_modern_tool_call_needs_neither_handshake_nor_client_info() {
    let server = PendingTools::default();
    let mut client = TestClient::connect(server.clone(), true).await;
    let response = client
        .request(json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "large", "arguments": {} }
        }))
        .await;
    assert_eq!(response["result"]["resultType"], "complete", "{response}");
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
    client.close().await;
}

#[tokio::test]
async fn modern_metadata_is_validated_before_tool_admission_on_every_request() {
    let valid = protocol_meta("2026-07-28");
    let mut missing_version = valid.clone();
    missing_version
        .as_object_mut()
        .unwrap()
        .remove("io.modelcontextprotocol/protocolVersion");
    let mut missing_capabilities = valid.clone();
    missing_capabilities
        .as_object_mut()
        .unwrap()
        .remove("io.modelcontextprotocol/clientCapabilities");
    let mut malformed_version = valid.clone();
    malformed_version["io.modelcontextprotocol/protocolVersion"] = json!(42);
    let mut malformed_capabilities = valid;
    malformed_capabilities["io.modelcontextprotocol/clientCapabilities"] = json!(42);
    for later in [false, true] {
        for (meta, code) in [
            (None, -32602),
            (Some(json!({})), -32602),
            (Some(missing_version.clone()), -32602),
            (Some(missing_capabilities.clone()), -32602),
            (Some(malformed_version.clone()), -32602),
            (Some(malformed_capabilities.clone()), -32602),
            (Some(protocol_meta("2099-01-01")), -32022),
        ] {
            let server = PendingTools::default();
            let mut client = TestClient::connect(server.clone(), true).await;
            if later {
                assert_eq!(
                    list_tools(&mut client).await["result"]["resultType"],
                    "complete"
                );
            }
            let mut params = json!({ "name": "large", "arguments": {} });
            if let Some(meta) = meta {
                params["_meta"] = meta;
            }
            let response = client
                .raw_request(json!({
                    "jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": params
                }))
                .await;
            assert_eq!(response["error"]["code"], code, "{response}");
            if code == -32022 {
                assert_newer_protocol_rejected(&response);
            }
            assert_eq!(server.calls.load(Ordering::SeqCst), 0);
            assert_eq!(
                list_tools(&mut client).await["result"]["resultType"],
                "complete"
            );
            client.close().await;
        }
    }
}

async fn assert_bootstrap_ping_policy<S: ServerHandler + Clone>(server: S) {
    let mut missing_capabilities = protocol_meta("2026-07-28");
    missing_capabilities
        .as_object_mut()
        .unwrap()
        .remove("io.modelcontextprotocol/clientCapabilities");
    let mut malformed_version = protocol_meta("2026-07-28");
    malformed_version["io.modelcontextprotocol/protocolVersion"] = json!(42);
    for (meta, code) in [
        (None, None),
        (Some(protocol_meta("2025-11-25")), None),
        (Some(protocol_meta("2026-07-28")), Some(-32601)),
        (Some(protocol_meta("2099-01-01")), Some(-32022)),
        (
            Some(json!({"io.modelcontextprotocol/protocolVersion":"2099-01-01"})),
            Some(-32022),
        ),
        (Some(missing_capabilities.clone()), Some(-32602)),
        (Some(malformed_version.clone()), Some(-32602)),
        (
            Some(json!({"io.modelcontextprotocol/clientInfo": {
                "name":"test", "version":"1"
            }})),
            Some(-32602),
        ),
        (
            Some(json!({"io.modelcontextprotocol/logLevel":"info"})),
            Some(-32602),
        ),
    ] {
        // Only policy rejections need the SDK discovery/recovery cross-product;
        // the remaining cases isolate the wrapper's context classification.
        let lifecycles: &[(bool, bool)] = if matches!(code, Some(-32601 | -32022)) {
            &[(false, false), (false, true), (true, false), (true, true)]
        } else {
            &[(false, false)]
        };
        for &(discovered, modern_recovery) in lifecycles {
            let mut client = TestClient::start(server.clone()).await;
            if discovered {
                let response = client
                    .request(json!({
                        "jsonrpc":"2.0", "id":0, "method":"server/discover",
                        "params":{"_meta":protocol_meta("2026-07-28")}
                    }))
                    .await;
                assert_result_shape(&response, true, true);
            }
            let params = meta
                .as_ref()
                .map_or(json!({}), |meta| json!({"_meta":meta}));
            let response = client
                .request(json!({
                    "jsonrpc":"2.0", "id":8, "method":"ping", "params":params
                }))
                .await;
            if let Some(code) = code {
                assert_eq!(response["error"]["code"], code, "{response}");
                assert!(response.get("result").is_none());
                if code == -32022 {
                    assert_newer_protocol_rejected(&response);
                }
            } else {
                assert_eq!(response["result"], json!({}), "{response}");
            }
            if modern_recovery {
                let tools = client
                    .request(json!({
                        "jsonrpc":"2.0", "id":9, "method":"tools/list",
                        "params":{"_meta":protocol_meta("2026-07-28")}
                    }))
                    .await;
                assert_result_shape(&tools, true, true);
            } else {
                let initialized = initialize_client(&mut client, "2025-11-25").await;
                assert_eq!(initialized["result"]["protocolVersion"], "2025-11-25");
                assert_result_shape(&list_tools(&mut client).await, false, true);
                let ping = client
                    .request(json!({
                        "jsonrpc":"2.0", "id":9, "method":"ping", "params":{}
                    }))
                    .await;
                assert_eq!(ping["result"], json!({}), "{ping}");
            }
            client.close().await;
        }
    }
}

#[tokio::test]
async fn guarded_ping_optional_context_errors_match_sdk() {
    let mut client_info = RequestMetaObject::new();
    client_info.set_client_info(Implementation::new("test", "1"));
    let mut log_level = RequestMetaObject::new();
    log_level.set_log_level(from_value(json!("info")).unwrap());
    for meta in [client_info, log_level] {
        let mut client = TestClient::connect(GitReadOnly, true).await;
        let sdk = client
            .raw_request(json!({
                "jsonrpc":"2.0", "id":1, "method":"tools/list", "params":{"_meta":meta}
            }))
            .await;
        let guarded = client
            .raw_request(json!({
                "jsonrpc":"2.0", "id":2, "method":"ping", "params":{"_meta":meta}
            }))
            .await;
        assert_eq!(sdk["error"]["code"], -32602, "{sdk}");
        assert_eq!(guarded["error"]["code"], -32602, "{guarded}");
        assert_eq!(
            sdk["error"]["message"].as_str().unwrap(),
            guarded["error"]["message"].as_str().unwrap(),
            "{meta:?}"
        );
        // The fixture expects normal shutdown, not EOF during recoverable bootstrap.
        let startup = client
            .raw_request(json!({
                "jsonrpc":"2.0", "id":3, "method":"tools/list",
                "params":{"_meta":protocol_meta("2026-07-28")}
            }))
            .await;
        assert!(startup.get("error").is_none(), "{startup}");
        client.close().await;
    }
}

#[tokio::test]
async fn bootstrap_ping_obeys_metadata_and_preserves_lifecycle_recovery_on_all_servers() {
    assert_bootstrap_ping_policy(GitReadOnly).await;
    assert_bootstrap_ping_policy(JjReadOnly).await;
    assert_bootstrap_ping_policy(ToolRunner).await;
}

async fn assert_task_methods_disabled<S: ServerHandler + Clone>(server: S) {
    let mut client = TestClient::connect(server, true).await;
    let capabilities = json!({ "extensions": { "io.modelcontextprotocol/tasks": {} } });
    assert!(
        from_value::<ClientCapabilities>(capabilities.clone())
            .unwrap()
            .supports_tasks()
    );
    let mut meta = protocol_meta("2026-07-28");
    meta["io.modelcontextprotocol/clientCapabilities"] = capabilities;
    let discovery = client
        .request(json!({
            "jsonrpc": "2.0", "id": 0, "method": "server/discover",
            "params": { "_meta": meta }
        }))
        .await;
    assert_result_shape(&discovery, true, true);
    assert!(
        discovery["result"]["capabilities"]["extensions"]
            .get("io.modelcontextprotocol/tasks")
            .is_none(),
        "{discovery}"
    );

    for (method, params) in [
        ("tasks/get", to_value(GetTaskParams::new("absent")).unwrap()),
        (
            "tasks/update",
            to_value(UpdateTaskParams::new("absent", Default::default())).unwrap(),
        ),
        (
            "tasks/cancel",
            to_value(CancelTaskParams::new("absent")).unwrap(),
        ),
        ("ping", json!({})),
    ] {
        let mut params = params;
        params["_meta"] = meta.clone();
        let response = client
            .request(json!({
                "jsonrpc": "2.0", "id": 9, "method": method, "params": params
            }))
            .await;
        assert_eq!(response["error"]["code"], -32601, "{method}: {response}");
        assert!(response.get("result").is_none());
    }
    client.close().await;
}

#[tokio::test]
async fn tasks_stay_disabled_for_both_lifecycles() {
    assert_task_methods_disabled(GitReadOnly).await;
    assert_task_methods_disabled(JjReadOnly).await;
    assert_task_methods_disabled(ToolRunner).await;
    for modern in [false, true] {
        let server = PendingTools::default();
        let mut client = TestClient::connect(server.clone(), modern).await;
        let mut meta = protocol_meta("2026-07-28");
        meta["io.modelcontextprotocol/clientCapabilities"] =
            json!({ "extensions": { "io.modelcontextprotocol/tasks": {} } });
        let mut params = json!({ "name": "large", "arguments": {} });
        if modern {
            params["_meta"] = meta;
        }
        let response = client
            .request(json!({
                "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": params
            }))
            .await;
        assert_result_shape(&response, modern, false);
        assert!(response["result"].get("task").is_none(), "{response}");
        assert!(response["result"].get("taskId").is_none(), "{response}");
        assert_eq!(response["result"]["content"][0]["type"], "text");
        assert_eq!(server.calls.load(Ordering::SeqCst), 1);
        client.close().await;
    }
}

async fn assert_version_policy<S: ServerHandler + Clone>(server: S) {
    for (requested, expected) in [
        ("2024-11-05", "2024-11-05"),
        ("2025-03-26", "2025-03-26"),
        ("2025-06-18", "2025-06-18"),
        ("2025-11-25", "2025-11-25"),
        ("2026-07-28", "2025-11-25"),
        ("2099-01-01", "2025-11-25"),
        ("1999-01-01", "2025-11-25"),
    ] {
        let (mut client, initialized) = initialize(server.clone(), requested).await;
        assert_eq!(initialized["result"]["protocolVersion"], expected);
        let tools = list_tools(&mut client).await;
        assert!(tools.get("error").is_none(), "{tools}");
        assert_legacy_result_shape(&tools);
        client.close().await;
    }
    let mut client = TestClient::start(server).await;
    let discovery = client
        .request(json!({
            "jsonrpc": "2.0", "id": 0, "method": "server/discover",
            "params": { "_meta": protocol_meta("2026-07-28") }
        }))
        .await;
    assert_eq!(
        discovery["result"]["supportedVersions"],
        json!(EXPECTED_PROTOCOL_VERSIONS),
        "{discovery}"
    );
    let initialized = initialize_client(&mut client, "2025-11-25").await;
    assert_eq!(initialized["result"]["protocolVersion"], "2025-11-25");
    assert_legacy_result_shape(&list_tools(&mut client).await);
    client.close().await;
}

#[tokio::test]
async fn all_servers_have_explicit_versions_and_legacy_fallback() {
    assert_version_policy(GitReadOnly).await;
    assert_version_policy(JjReadOnly).await;
    assert_version_policy(ToolRunner).await;
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

async fn assert_malformed_bootstrap_recovers<S: ServerHandler + Clone>(server: S) {
    for meta in [
        json!({}),
        json!({
            "io.modelcontextprotocol/protocolVersion": 42,
            "io.modelcontextprotocol/clientCapabilities": {}
        }),
    ] {
        let mut client = TestClient::start(server.clone()).await;
        let response = client
            .request(json!({
                "jsonrpc": "2.0", "id": 0, "method": "tools/list",
                "params": { "_meta": meta }
            }))
            .await;
        assert_eq!(response["error"]["code"], -32602, "{response}");
        let initialized = initialize_client(&mut client, "2025-11-25").await;
        assert_eq!(initialized["result"]["protocolVersion"], "2025-11-25");
        assert!(list_tools(&mut client).await.get("error").is_none());
        client.close().await;
    }
}

#[tokio::test]
async fn malformed_bootstrap_recovers_on_all_servers() {
    assert_malformed_bootstrap_recovers(GitReadOnly).await;
    assert_malformed_bootstrap_recovers(JjReadOnly).await;
    assert_malformed_bootstrap_recovers(ToolRunner).await;
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
