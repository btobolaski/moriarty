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

use clap::Subcommand;

use git_read_only::GitReadOnly;
use jj_read_only::JjReadOnly;
use miette::IntoDiagnostic;
use rmcp::{ServiceExt, model::ProtocolVersion, transport::stdio};
use tool_runner::ToolRunner;

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
            Self::GitReadOnly => {
                let server = GitReadOnly;
                let service = server.serve(stdio()).await.into_diagnostic()?;
                service.waiting().await.into_diagnostic()?;
                Ok(())
            }
            Self::JjReadOnly => {
                let server = JjReadOnly;
                let service = server.serve(stdio()).await.into_diagnostic()?;
                service.waiting().await.into_diagnostic()?;
                Ok(())
            }
            Self::ProjectTools => {
                let server = ToolRunner;

                let service = server.serve(stdio()).await.into_diagnostic()?;

                service.waiting().await.into_diagnostic()?;
                Ok(())
            }
            Self::Install => install_mcp_server().await,
        }
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
mod tests {
    use std::{collections::BTreeSet, time::Duration};

    use rmcp::{ServerHandler, ServiceExt};
    use serde_json::{Value, json};
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf, split},
        task::JoinHandle,
        time::timeout,
    };

    use super::{GitReadOnly, JjReadOnly, ToolRunner};
    use crate::{
        project_config::approvals,
        test_helpers::{setup_isolated_xdg_config, setup_project_dir_with_config},
    };

    const TEST_TIMEOUT: Duration = Duration::from_secs(5);

    struct TestClient {
        reader: BufReader<ReadHalf<DuplexStream>>,
        writer: WriteHalf<DuplexStream>,
        server_task: JoinHandle<()>,
    }

    impl TestClient {
        async fn start<S: ServerHandler>(server: S) -> Self {
            let (server_io, client_io) = tokio::io::duplex(64 * 1024);
            let server_task = tokio::spawn(async move {
                let service = server.serve(server_io).await.unwrap();
                service.waiting().await.unwrap();
            });
            let (reader, writer) = split(client_io);

            Self {
                reader: BufReader::new(reader),
                writer,
                server_task,
            }
        }

        async fn send(&mut self, message: Value) {
            let mut encoded = serde_json::to_vec(&message).unwrap();
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

            let mut response = String::new();
            let bytes_read = timeout(TEST_TIMEOUT, self.reader.read_line(&mut response))
                .await
                .expect("timed out reading MCP response")
                .unwrap();
            assert_ne!(bytes_read, 0, "MCP server closed without a response");
            serde_json::from_str(&response).unwrap()
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
                .unwrap();
        }
    }

    async fn initialize<S: ServerHandler>(
        server: S,
        requested_version: &str,
    ) -> (TestClient, Value) {
        let mut client = TestClient::start(server).await;
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
        (client, response)
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

    async fn request_with_newer_protocol(client: &mut TestClient) -> Value {
        client
            .request(json!({
                "jsonrpc": "2.0",
                "id": 99,
                "method": "tools/list",
                "params": {
                    "_meta": {
                        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                        "io.modelcontextprotocol/clientCapabilities": {},
                        "io.modelcontextprotocol/clientInfo": {
                            "name": "moriarty-protocol-test",
                            "version": "0"
                        }
                    }
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
                .unwrap()
                .iter()
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
                ("status", &["project_dir", "args"]),
                ("diff", &["project_dir", "args"]),
                ("log", &["project_dir", "args"]),
                ("show", &["project_dir", "args"]),
            ],
        );
        client.close().await;
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

        let project = tempfile::tempdir().unwrap();
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
        assert_tool_contract(&tools, &[("run", &["project_dir", "command", "args"])]);
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
                ("run_lint", &["project_dir"]),
                ("run_build", &["project_dir"]),
                ("run_formatter", &["project_dir"]),
                ("run_tests", &["project_dir"]),
                ("run_checks", &["project_dir"]),
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
