use clap::Subcommand;

use git_read_only::GitReadOnly;
use miette::IntoDiagnostic;
use rmcp::{transport::stdio, ServiceExt};
use tool_runner::ToolRunner;

pub mod git_read_only;
mod tool_runner;

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

                for tool in server.tool_router.list_all() {
                    eprintln!("\n{}: {}", tool.name, tool.description.unwrap_or_default());

                    if let Some(output_schema) = &tool.output_schema {
                        eprintln!(
                            " Output schema: {}",
                            serde_json::to_string_pretty(output_schema).unwrap()
                        );
                    } else {
                        eprintln!(" Output: Unstructured text");
                    }
                }

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
            "Failed to install {} server(s): {}",
            errors.len(),
            errors
                .iter()
                .map(|(name, _)| **name)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

async fn install_single_mcp_server(server_name: &str) -> miette::Result<()> {
    use tokio::process::Command;

    // Remove any existing installation first to ensure a clean reinstall.
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
