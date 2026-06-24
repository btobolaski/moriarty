//! MCP server for running project-configured tools.
//!
//! This module provides an MCP server that runs project-configured tooling from
//! `.config/tools.toml`: the `[commands]` (lint, test, build, format) and the `[[checks]]`.
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
//! **IMPORTANT**: The four command tools (`run_lint`/`run_build`/`run_formatter`/`run_tests`)
//! execute arbitrary commands from `.config/tools.toml` without validation or sandboxing.
//! (`run_checks` differs: it requires each `[[check]]` to be approved and verifies the config and
//! binary hashes before running anything — see [`crate::checks`].) The security model assumes:
//!
//! - **Trusted configuration files**: Only use with project directories where you
//!   trust the contents of `.config/tools.toml`
//! - **No runtime validation of commands**: the four command tools execute their configured
//!   command as-is, without checking for dangerous patterns or operations (checks are verified)
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
//! - **Resource exhaustion (command tools)**: the four command tools can consume unlimited CPU,
//!   memory, or disk; `run_checks` bounds checks with a 5-minute timeout and output-size caps
//!   (see [`crate::checks`])

use std::path::PathBuf;

use rmcp::{
    ErrorData as McpError, ServerHandler, handler::server::wrapper::Parameters, model::*, tool,
    tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{checks::CheckRunOutcome, project_config::runner::verify_and_load_project};

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
/// This server provides five MCP tools:
/// - `run_lint`: Execute the configured linter
/// - `run_build`: Execute the configured build command
/// - `run_formatter`: Execute the configured formatter
/// - `run_tests`: Execute the configured test runner
/// - `run_checks`: Run all configured `[[checks]]` the way the Stop hook does
///
/// The four command tools read their command from `[commands]` in `.config/tools.toml`;
/// `run_checks` runs the project's `[[checks]]` under the Stop hook's resource limits and
/// checks-only verification (see [`crate::checks`]).
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
pub struct ToolRunner;

impl ToolRunner {
    async fn run_command(cmd: ProjectCommand, args: RunArgs) -> Result<CallToolResult, McpError> {
        let project = verify_and_load_project(args.project_dir)
            .await
            .map_err(|e| McpError {
                code: ErrorCode::INVALID_REQUEST,
                message: format!("{:?}", e).into(),
                data: None,
            })?;

        // Run the specific command
        let command_name = cmd.as_str();
        let output = project.run_command(command_name).await.map_err(|e| {
            let error_msg = format!("{:?}", e);
            if error_msg.contains("not configured") {
                McpError {
                    code: ErrorCode::RESOURCE_NOT_FOUND,
                    message: format!("The {} command was not configured for the project", cmd)
                        .into(),
                    data: None,
                }
            } else {
                McpError {
                    code: ErrorCode::INTERNAL_ERROR,
                    message: error_msg.into(),
                    data: None,
                }
            }
        })?;

        let content = vec![
            Content::text(format!("stdout: \n\n {}", output.stdout)),
            Content::text(format!("stderr: \n\n {}", output.stderr)),
        ];
        if matches!(output.exit_code, Some(0)) {
            Ok(CallToolResult::success(content))
        } else {
            Ok(CallToolResult::error(content))
        }
    }

    async fn run_checks_impl(args: RunArgs) -> Result<CallToolResult, McpError> {
        let outcome = crate::checks::run_configured_checks(&args.project_dir)
            .await
            .map_err(|e| McpError {
                code: ErrorCode::INTERNAL_ERROR,
                message: format!("{:?}", e).into(),
                data: None,
            })?;

        // Gate and limit failures (Blocked) are surfaced as tool-result errors rather than
        // McpError so the agent sees the Stop hook's actionable reason text (e.g.
        // "Run: moriarty approve-project ..."); only an unexpected internal failure is an McpError.
        match outcome {
            CheckRunOutcome::NoChecks(note) => Ok(CallToolResult::success(vec![Content::text(
                format!("No checks were run: {note}"),
            )])),
            CheckRunOutcome::Blocked(reason) => {
                Ok(CallToolResult::error(vec![Content::text(reason)]))
            }
            CheckRunOutcome::Ran { outputs, failures } => {
                let mut content: Vec<Content> = outputs.into_iter().map(Content::text).collect();
                if failures.is_empty() {
                    Ok(CallToolResult::success(content))
                } else {
                    content.push(Content::text(format!(
                        "Checks failed:\n\n{}",
                        failures.join("\n\n")
                    )));
                    Ok(CallToolResult::error(content))
                }
            }
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
    #[tool(description = "Runs the project's configured checks ([[checks]] in .config/tools.toml)")]
    async fn run_checks(
        &self,
        Parameters(args): Parameters<RunArgs>,
    ) -> Result<CallToolResult, McpError> {
        Self::run_checks_impl(args).await
    }
}

#[tool_handler]
impl ServerHandler for ToolRunner {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                env!("CARGO_CRATE_NAME"),
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "This server provides configured tooling from the project".to_string(),
            )
    }
}

impl Default for ToolRunner {
    fn default() -> Self {
        Self
    }
}

#[cfg(test)]
mod tests;
