use clap::Subcommand;

use git_read_only::GitReadOnly;
use miette::IntoDiagnostic;
use rmcp::{transport::stdio, ServiceExt};

pub mod git_read_only;

const MCP_SERVER_NAME: &str = "git-read-only";

#[derive(Debug, Subcommand)]
pub enum McpServers {
    /// runs the git read only server as stdin / stdout server
    GitReadOnly,
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
            Self::Install => install_mcp_server().await,
        }
    }
}

async fn install_mcp_server() -> miette::Result<()> {
    use tokio::process::Command;

    // Remove any existing installation first to ensure a clean reinstall.
    // Errors are ignored because the server may not be installed yet, and other errors
    // (missing claude binary, permissions) will be caught by the subsequent add command.
    eprintln!("Removing existing {} MCP server (if present)...", MCP_SERVER_NAME);
    let _ = Command::new("claude")
        .args(["mcp", "rm", MCP_SERVER_NAME])
        .status()
        .await;

    eprintln!("Adding {} MCP server...", MCP_SERVER_NAME);
    let status = Command::new("claude")
        .args([
            "mcp",
            "add",
            "--scope",
            "user",
            "--transport",
            "stdio",
            MCP_SERVER_NAME,
            "--",
            "moriarty",
            "mcp",
            MCP_SERVER_NAME,
        ])
        .status()
        .await
        .into_diagnostic()?;

    if !status.success() {
        return Err(miette::miette!(
            "Failed to add {} MCP server. Exit code: {}",
            MCP_SERVER_NAME,
            status.code().unwrap_or(-1)
        ));
    }

    eprintln!("Successfully installed {} MCP server!", MCP_SERVER_NAME);
    Ok(())
}
