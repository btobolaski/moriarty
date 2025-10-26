//! MCP (Model Context Protocol) servers for Moriarty.
//!
//! This module provides two MCP servers:
//!
//! - [`git_read_only`]: Read-only git operations (status, diff, log, show)
//! - [`tool_runner`]: Project-configured tool execution (lint, test, build, format)
//!
//! # Usage
//!
//! Servers can be run via the CLI:
//!
//! ```bash
//! moriarty mcp git-read-only
//! moriarty mcp project-tools
//! moriarty mcp install  # Install both servers to Claude Code
//! ```

use clap::Subcommand;

use git_read_only::GitReadOnly;
use miette::IntoDiagnostic;
use rmcp::{transport::stdio, ServiceExt};
use tool_runner::ToolRunner;

pub mod approvals;
pub mod git_read_only;
pub mod tool_runner;

#[derive(Debug, Subcommand)]
pub enum McpServers {
    /// runs the git read only server as stdin / stdout server
    GitReadOnly,
    /// runs the project tools server as stdin / stdout server
    ProjectTools,
    /// installs the MCP server into Claude Code
    Install,
}

impl McpServers {
    pub async fn run(self) -> miette::Result<()> {
        match self {
            Self::GitReadOnly => {
                let server = GitReadOnly::default();
                let service = server.serve(stdio()).await.into_diagnostic()?;
                service.waiting().await.into_diagnostic()?;
                Ok(())
            }
            Self::ProjectTools => {
                let server = ToolRunner::default();

                let service = server.serve(stdio()).await.into_diagnostic()?;

                service.waiting().await.into_diagnostic()?;
                Ok(())
            }
            Self::Install => install_mcp_server().await,
        }
    }
}

async fn install_mcp_server() -> miette::Result<()> {
    // Install both MCP servers, tracking results
    let servers = ["git-read-only", "project-tools"];
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
