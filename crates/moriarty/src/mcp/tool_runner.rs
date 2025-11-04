//! MCP server for running project-configured tools.
//!
//! This module provides an MCP server that executes project-specific commands
//! (lint, test, build, format) as configured in `.config/tools.toml`.
//!
//! # Configuration
//!
//! Projects must provide a `.config/tools.toml` file with command definitions:
//!
//! ```toml
//! [commands]
//! lint = ["cargo", "clippy", "--all-targets", "--", "--deny", "warnings"]
//! test = ["cargo", "nextest", "run"]
//! build = ["cargo", "build"]
//! format = ["cargo", "fmt"]
//! ```
//!
//! # Security Model
//!
//! **IMPORTANT**: This server executes arbitrary commands from `.config/tools.toml`
//! without validation or sandboxing. The security model assumes:
//!
//! - **Trusted configuration files**: Only use with project directories where you
//!   trust the contents of `.config/tools.toml`
//! - **No runtime validation**: Commands are executed as-is without checking for
//!   dangerous patterns or operations
//! - **Full filesystem access**: Commands run with the same permissions as the MCP
//!   server process
//!
//! ## Security Best Practices
//!
//! 1. **Restrict file permissions**: Set `.config/tools.toml` to read-only for the
//!    owner and inaccessible to other users:
//!    ```bash
//!    chmod 600 .config/tools.toml
//!    ```
//!
//! 2. **Review before use**: Always inspect `.config/tools.toml` in new projects
//!    before running tools
//!
//! 3. **Avoid shell execution**: While the server uses `Command::new()` to prevent
//!    shell metacharacter injection in arguments, the configuration itself can
//!    specify shell invocation:
//!
//!    ```toml
//!    # DANGEROUS - executes arbitrary shell commands
//!    test = ["sh", "-c", "rm -rf /tmp/* && cargo test"]
//!
//!    # SAFE - direct command execution
//!    test = ["cargo", "test"]
//!    ```
//!
//! ## Threat Model
//!
//! This server is designed to prevent:
//! - **Shell injection via arguments**: The server never invokes a shell, preventing
//!   injection through command-line arguments
//! - **Path traversal**: All project directories are canonicalized to prevent `../`
//!   escape sequences
//!
//! This server does NOT protect against:
//! - **Malicious configuration files**: If `.config/tools.toml` contains dangerous
//!   commands, they will be executed
//! - **Command output exfiltration**: Commands can write to arbitrary files or
//!   network locations
//! - **Resource exhaustion**: Commands can consume unlimited CPU, memory, or disk

use std::path::PathBuf;

use rmcp::{
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::*,
    tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::project_config::{load_project_settings, ProjectApprovals, VerificationResult};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunArgs {
    /// The project directory containing `.config/tools.toml`
    pub project_dir: PathBuf,
}

#[derive(Debug, Clone, Copy)]
enum ProjectCommand {
    Lint,
    Test,
    Build,
    Format,
}

impl ProjectCommand {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Lint => "lint",
            Self::Test => "test",
            Self::Build => "build",
            Self::Format => "format",
        }
    }
}

impl std::fmt::Display for ProjectCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_str().fmt(f)
    }
}

/// MCP server for executing project-configured tools.
///
/// This server provides four MCP tools:
/// - `run_lint`: Execute the configured linter
/// - `run_build`: Execute the configured build command
/// - `run_formatter`: Execute the configured formatter
/// - `run_tests`: Execute the configured test runner
///
/// Each tool reads its command from `.config/tools.toml` in the project directory.
///
/// # Example
///
/// ```no_run
/// use moriarty::mcp::tool_runner::ToolRunner;
///
/// let server = ToolRunner::default();
/// // Server can now be used with rmcp::ServiceExt::serve()
/// ```
#[derive(Clone)]
pub struct ToolRunner {
    tool_router: ToolRouter<Self>,
}

impl ToolRunner {
    async fn run_command(cmd: ProjectCommand, args: RunArgs) -> Result<CallToolResult, McpError> {
        // Canonicalize path to prevent traversal attacks
        let canonical_dir = args.project_dir.canonicalize().map_err(|e| McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: format!(
                "Invalid project directory: {} ({})",
                args.project_dir.display(),
                e
            )
            .into(),
            data: None,
        })?;

        let settings = match load_project_settings(canonical_dir.clone()).await {
            Ok(settings) => settings,
            Err(error) => {
                return Err(McpError {
                    code: ErrorCode::INTERNAL_ERROR,
                    message: format!("{error:?}").into(),
                    data: None,
                });
            }
        };

        // Verify approval before executing
        let command_name = cmd.as_str();
        let approvals = ProjectApprovals::load().await.map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: format!("Failed to load approvals: {}", e).into(),
            data: None,
        })?;

        let verification_result = approvals
            .verify_project(&canonical_dir, command_name)
            .await
            .map_err(|e| McpError {
                code: ErrorCode::INTERNAL_ERROR,
                message: format!("Failed to verify approvals: {}", e).into(),
                data: None,
            })?;

        match verification_result {
            VerificationResult::Approved => {
                // Continue with execution
            }
            VerificationResult::NotApproved => {
                return Err(McpError {
                    code: ErrorCode::INVALID_REQUEST,
                    message: format!(
                        "Project tools not approved. Run: moriarty approve-project {}",
                        canonical_dir.display()
                    )
                    .into(),
                    data: None,
                });
            }
            VerificationResult::ConfigHashMismatch { expected, actual } => {
                return Err(McpError {
                    code: ErrorCode::INVALID_REQUEST,
                    message: format!(
                        "tools.toml has been modified since approval. \
                        Run: moriarty approve-project {} \
                        (expected: {}, actual: {})",
                        canonical_dir.display(),
                        expected,
                        actual
                    )
                    .into(),
                    data: None,
                });
            }
            VerificationResult::BinaryHashMismatch {
                item,
                expected,
                actual,
            } => {
                return Err(McpError {
                    code: ErrorCode::INVALID_REQUEST,
                    message: format!(
                        "Binary for '{}' has been modified since approval. \
                        Run: moriarty approve-project {} \
                        (expected: {}, actual: {})",
                        item,
                        canonical_dir.display(),
                        expected,
                        actual
                    )
                    .into(),
                    data: None,
                });
            }
            VerificationResult::ItemNotApproved { item } => {
                return Err(McpError {
                    code: ErrorCode::INVALID_REQUEST,
                    message: format!(
                        "Tool '{}' not approved. Run: moriarty approve-project {}",
                        item,
                        canonical_dir.display()
                    )
                    .into(),
                    data: None,
                });
            }
        }

        let maybe_command = match cmd {
            ProjectCommand::Build => settings.commands.build,
            ProjectCommand::Format => settings.commands.format,
            ProjectCommand::Lint => settings.commands.lint,
            ProjectCommand::Test => settings.commands.test,
        };

        match maybe_command {
            Some(command) if !command.is_empty() => {
                let (cmd, args_slice) = command.split_first().expect("command is not empty");

                let mut command_runner = Command::new(cmd);
                command_runner.current_dir(canonical_dir);
                command_runner.args(args_slice);

                match command_runner.output().await {
                    Ok(result) => {
                        // Use lossy UTF-8 conversion to handle potentially invalid encodings
                        // in command output. This prevents server crashes while allowing
                        // output to be displayed with replacement characters (�).
                        let stderr = String::from_utf8_lossy(&result.stderr).to_string();
                        let stdout = String::from_utf8_lossy(&result.stdout).to_string();

                        Ok(CallToolResult {
                            content: vec![
                                Content::text(format!("stdout: \n\n {stdout}")),
                                Content::text(format!("stderr: \n\n {stderr}")),
                            ],
                            is_error: Some(!matches!(result.status.code(), Some(0))),
                            meta: None,
                            structured_content: None,
                        })
                    }
                    Err(error) => Err(McpError {
                        code: ErrorCode::INTERNAL_ERROR,
                        message: format!("failed to run {cmd} command due to {error:?}").into(),
                        data: None,
                    }),
                }
            }
            Some(_) | None => Err(McpError {
                code: ErrorCode::RESOURCE_NOT_FOUND,
                message: format!("The {} command was not configured for the project", cmd).into(),
                data: None,
            }),
        }
    }
}

#[tool_router(router = tool_router)]
impl ToolRunner {
    #[tool(description = "Runs the projects configured linter")]
    async fn run_lint(
        &self,
        Parameters(args): Parameters<RunArgs>,
    ) -> Result<CallToolResult, McpError> {
        Self::run_command(ProjectCommand::Lint, args).await
    }
    #[tool(description = "Runs the projects configured build")]
    async fn run_build(
        &self,
        Parameters(args): Parameters<RunArgs>,
    ) -> Result<CallToolResult, McpError> {
        Self::run_command(ProjectCommand::Build, args).await
    }
    #[tool(description = "Runs the projects configured formatter")]
    async fn run_formatter(
        &self,
        Parameters(args): Parameters<RunArgs>,
    ) -> Result<CallToolResult, McpError> {
        Self::run_command(ProjectCommand::Format, args).await
    }
    #[tool(description = "Runs the projects configured test tool")]
    async fn run_tests(
        &self,
        Parameters(args): Parameters<RunArgs>,
    ) -> Result<CallToolResult, McpError> {
        Self::run_command(ProjectCommand::Test, args).await
    }
}

#[tool_handler]
impl ServerHandler for ToolRunner {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation::from_build_env(),
            instructions: Some(
                "This server provides configured tooling from the project".to_string(),
            ),
            ..Default::default()
        }
    }
}

impl Default for ToolRunner {
    fn default() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

#[cfg(test)]
mod tests;
