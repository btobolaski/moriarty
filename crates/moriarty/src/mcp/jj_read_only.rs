//! MCP server for read-only jj (jujutsu) operations.
//!
//! This module provides an MCP (Model Context Protocol) server that exposes
//! read-only jj commands for project directories using an enum-based command pattern.
//!
//! # Architecture
//!
//! Unlike the git-read-only server which exposes separate MCP tools for each command,
//! this server uses a single tool that accepts a `JjCommand` enum parameter. This
//! reduces boilerplate while still providing type-safe command selection.
//!
//! # Security
//!
//! - **Shell injection protection**: Commands are executed directly without shell
//!   interpretation, preventing injection via shell metacharacters (`&&`, `||`, `|`, etc.)
//! - **Path validation**: Only operates on valid project directories (verified by
//!   checking for `.config/tools.toml` marker file)
//! - **Read-only operations**: All exposed jj commands are read-only; no modifications
//!   to the repository are possible

// standard library imports
use std::path::PathBuf;

// 3rd party crates
use rmcp::{
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::*,
    tool, tool_handler, tool_router, ErrorData as McpError, Json, ServerHandler,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

/// Supported jj commands that can be executed via the MCP server.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum JjCommand {
    /// Run `jj status` to show the working copy status
    Status,
    /// Run `jj diff` to show changes in the working copy
    Diff,
    /// Run `jj log` to show the commit history
    Log,
    /// Run `jj show` to show a specific revision
    Show,
    /// Run `jj op log` to show the operation history
    OpLog,
}

impl JjCommand {
    /// Returns the jj subcommand string for this command.
    fn as_args(&self) -> Vec<&str> {
        match self {
            Self::Status => vec!["status"],
            Self::Diff => vec!["diff"],
            Self::Log => vec!["log"],
            Self::Show => vec!["show"],
            Self::OpLog => vec!["op", "log"],
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Status => "status",
            Self::Diff => "diff",
            Self::Log => "log",
            Self::Show => "show",
            Self::OpLog => "op log",
        }
    }
}

/// Arguments for executing jj commands.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JjArgs {
    /// The directory of the project (must contain `.config/tools.toml`)
    pub project_dir: PathBuf,
    /// The jj command to execute
    pub command: JjCommand,
    /// Additional arguments to pass to the command (e.g., `["--summary"]` for diff)
    pub args: Vec<String>,
}

/// Result of executing a jj command.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommandResult {
    pub stdout: String,
    pub stderr: String,
    /// Exit code (0 indicates success)
    pub exit_code: i32,
}

/// MCP server providing read-only jj operations.
///
/// This server exposes a single tool that accepts a `JjCommand` enum to specify
/// which jj command to run. Supported commands:
/// - `Status`: Run `jj status`
/// - `Diff`: Run `jj diff`
/// - `Log`: Run `jj log`
/// - `Show`: Run `jj show`
/// - `OpLog`: Run `jj op log`
///
/// # Security
///
/// All operations validate that the project directory contains `.config/tools.toml`
/// to prevent execution in arbitrary system directories.
#[derive(Debug, Clone)]
pub struct JjReadOnly {
    pub tool_router: ToolRouter<Self>,
}

impl JjReadOnly {
    async fn run_jj_command(
        command: JjCommand,
        project_dir: PathBuf,
        args: Vec<String>,
    ) -> Result<Json<CommandResult>, McpError> {
        // Canonicalize path to prevent traversal attacks and resolve symlinks
        let canonical_dir = project_dir.canonicalize().map_err(|e| McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: format!(
                "Invalid project directory: {} ({})",
                project_dir.display(),
                e
            )
            .into(),
            data: None,
        })?;

        // Verify this is a valid project directory by checking for .config/tools.toml
        let config_path = canonical_dir.join(".config").join("tools.toml");
        if !config_path.exists() {
            return Err(McpError {
                code: ErrorCode::INVALID_PARAMS,
                message: format!(
                    "Invalid project directory: {} (missing .config/tools.toml)",
                    canonical_dir.display()
                )
                .into(),
                data: None,
            });
        }

        let mut cmd = Command::new("jj");
        cmd.args(command.as_args());
        cmd.current_dir(canonical_dir);
        cmd.args(args);

        match cmd.output().await {
            Ok(result) => {
                // Use lossy UTF-8 conversion to handle potentially invalid encodings
                // in filenames (e.g., legacy encodings). This prevents server crashes
                // while allowing output to be displayed with replacement characters (�).
                // This is acceptable for a read-only tool where we never modify data.
                let stderr = String::from_utf8_lossy(&result.stderr).to_string();
                let stdout = String::from_utf8_lossy(&result.stdout).to_string();

                Ok(Json(CommandResult {
                    exit_code: result.status.code().unwrap_or(-1),
                    stderr,
                    stdout,
                }))
            }
            Err(error) => Err(McpError {
                code: ErrorCode::INTERNAL_ERROR,
                message: format!("{} failed: {error:?}", command.name()).into(),
                data: None,
            }),
        }
    }
}

#[tool_router(router = tool_router)]
impl JjReadOnly {
    #[tool(description = "Runs a jj (jujutsu) command with the provided command type and args")]
    async fn run(
        &self,
        Parameters(args): Parameters<JjArgs>,
    ) -> Result<Json<CommandResult>, McpError> {
        let JjArgs {
            project_dir,
            command,
            args,
        } = args;
        Self::run_jj_command(command, project_dir, args).await
    }
}

#[tool_handler]
impl ServerHandler for JjReadOnly {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation::from_build_env(),
            instructions: Some(
                "This server provides a tool for executing read-only jj (jujutsu) commands. \
                 Use the `run` tool with a JjCommand enum to specify which command to execute."
                    .to_string(),
            ),
            ..Default::default()
        }
    }
}

impl Default for JjReadOnly {
    fn default() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn setup_jj_repo() -> TempDir {
        let temp_dir = TempDir::new().unwrap();

        // Initialize jj repository
        std::process::Command::new("jj")
            .args(["git", "init", "--colocate"])
            .current_dir(temp_dir.path())
            .status()
            .unwrap();

        // Create .config/tools.toml to mark as valid project directory
        let config_dir = temp_dir.path().join(".config");
        std::fs::create_dir(&config_dir).unwrap();
        std::fs::write(
            config_dir.join("tools.toml"),
            "[commands]\nlint = [\"cargo\", \"clippy\"]\n",
        )
        .unwrap();

        temp_dir
    }

    #[test]
    fn test_jj_command_as_args() {
        assert_eq!(JjCommand::Status.as_args(), vec!["status"]);
        assert_eq!(JjCommand::Diff.as_args(), vec!["diff"]);
        assert_eq!(JjCommand::Log.as_args(), vec!["log"]);
        assert_eq!(JjCommand::Show.as_args(), vec!["show"]);
        assert_eq!(JjCommand::OpLog.as_args(), vec!["op", "log"]);
    }

    /// Helper function to test shell injection protection for any jj command and operator.
    async fn verify_no_shell_injection(command: JjCommand, operator: &str) {
        let temp_dir = setup_jj_repo().await;
        let args = vec![
            operator.to_string(),
            "echo".to_string(),
            "FAILED".to_string(),
        ];

        let result = JjReadOnly::run_jj_command(command, temp_dir.path().to_path_buf(), args).await;

        match result {
            Ok(cmd_result) => {
                // Check that jj output doesn't contain our injection attempt
                assert!(
                    !cmd_result.0.stdout.contains("FAILED"),
                    "Shell injection occurred with '{}': stdout contains 'FAILED'",
                    operator
                );
                assert!(
                    !cmd_result.0.stderr.contains("FAILED"),
                    "Shell injection occurred with '{}': stderr contains 'FAILED'",
                    operator
                );
            }
            Err(error) => {
                // If there's an error, check for shell-specific patterns that would
                // indicate shell execution rather than just jj rejecting bad arguments.
                let err_msg = error.message.to_string().to_lowercase();
                assert!(
                    !err_msg.contains("sh:")
                        && !err_msg.contains("bash:")
                        && !err_msg.contains("/bin/echo"),
                    "Shell execution detected in error with '{}': {}",
                    operator,
                    error.message
                );
            }
        }
    }

    #[tokio::test]
    async fn test_shell_injection_protection() {
        let commands = [
            JjCommand::Status,
            JjCommand::Diff,
            JjCommand::Log,
            JjCommand::Show,
            JjCommand::OpLog,
        ];
        let operators = ["&&", "||", "|"];

        for command in &commands {
            for operator in &operators {
                verify_no_shell_injection(command.clone(), operator).await;
            }
        }
    }

    #[tokio::test]
    async fn test_status_succeeds_with_valid_args() {
        let temp_dir = setup_jj_repo().await;

        let args = JjArgs {
            project_dir: temp_dir.path().to_path_buf(),
            command: JjCommand::Status,
            args: vec![],
        };

        let cmd_result = JjReadOnly::run_jj_command(args.command, args.project_dir, args.args)
            .await
            .unwrap();
        assert_eq!(cmd_result.0.exit_code, 0);
    }

    #[tokio::test]
    async fn test_status_tool_handler() {
        let temp_dir = setup_jj_repo().await;
        let server = JjReadOnly::default();

        let args = JjArgs {
            project_dir: temp_dir.path().to_path_buf(),
            command: JjCommand::Status,
            args: vec![],
        };

        let cmd_result = server.run(Parameters(args)).await.unwrap();
        assert_eq!(cmd_result.0.exit_code, 0);
    }

    #[tokio::test]
    async fn test_rejects_directory_without_config_file() {
        let temp_dir = TempDir::new().unwrap();

        // Initialize jj but don't create .config/tools.toml
        std::process::Command::new("jj")
            .args(["git", "init", "--colocate"])
            .current_dir(temp_dir.path())
            .status()
            .unwrap();

        let args = JjArgs {
            project_dir: temp_dir.path().to_path_buf(),
            command: JjCommand::Status,
            args: vec![],
        };

        let Err(error) =
            JjReadOnly::run_jj_command(args.command, args.project_dir, args.args).await
        else {
            panic!("Expected error for directory without .config/tools.toml");
        };

        assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
        assert!(error.message.contains("Invalid project directory"));
        assert!(error.message.contains("missing .config/tools.toml"));
    }

    #[tokio::test]
    async fn test_rejects_nonexistent_directory() {
        let args = JjArgs {
            project_dir: PathBuf::from("/nonexistent/directory"),
            command: JjCommand::Status,
            args: vec![],
        };

        let Err(error) =
            JjReadOnly::run_jj_command(args.command, args.project_dir, args.args).await
        else {
            panic!("Expected error for nonexistent directory");
        };

        assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn test_diff_command() {
        let temp_dir = setup_jj_repo().await;

        // Create a file to have something in the diff
        std::fs::write(temp_dir.path().join("test.txt"), "test content").unwrap();

        let server = JjReadOnly::default();
        let args = JjArgs {
            project_dir: temp_dir.path().to_path_buf(),
            command: JjCommand::Diff,
            args: vec!["--summary".to_string()],
        };

        let cmd_result = server.run(Parameters(args)).await.unwrap();
        assert_eq!(cmd_result.0.exit_code, 0);
        // Should show the new file in the diff summary
        assert!(
            cmd_result.0.stdout.contains("test.txt") || cmd_result.0.stdout.contains("A test.txt"),
            "diff --summary should show the new file"
        );
    }

    #[tokio::test]
    async fn test_log_command() {
        let temp_dir = setup_jj_repo().await;

        // Create a file and describe the change to have something in the log
        std::fs::write(temp_dir.path().join("test.txt"), "content").unwrap();
        std::process::Command::new("jj")
            .args(["describe", "-m", "Test commit"])
            .current_dir(temp_dir.path())
            .status()
            .unwrap();

        let server = JjReadOnly::default();
        let args = JjArgs {
            project_dir: temp_dir.path().to_path_buf(),
            command: JjCommand::Log,
            args: vec!["-r", "@"].iter().map(|s| s.to_string()).collect(),
        };

        let cmd_result = server.run(Parameters(args)).await.unwrap();
        assert_eq!(cmd_result.0.exit_code, 0);
        assert!(cmd_result.0.stdout.contains("Test commit"));
    }

    #[tokio::test]
    async fn test_show_command() {
        let temp_dir = setup_jj_repo().await;

        // Create a file and describe the change to have something to show
        std::fs::write(temp_dir.path().join("test.txt"), "test content").unwrap();
        std::process::Command::new("jj")
            .args(["describe", "-m", "Test change"])
            .current_dir(temp_dir.path())
            .status()
            .unwrap();

        let server = JjReadOnly::default();
        let args = JjArgs {
            project_dir: temp_dir.path().to_path_buf(),
            command: JjCommand::Show,
            args: vec!["@".to_string()],
        };

        let cmd_result = server.run(Parameters(args)).await.unwrap();
        assert_eq!(cmd_result.0.exit_code, 0);
        // Should show the change description
        assert!(
            cmd_result.0.stdout.contains("Test change"),
            "show output should contain change description"
        );
    }

    #[tokio::test]
    async fn test_op_log_command() {
        let temp_dir = setup_jj_repo().await;
        let server = JjReadOnly::default();

        let args = JjArgs {
            project_dir: temp_dir.path().to_path_buf(),
            command: JjCommand::OpLog,
            args: vec!["--limit".to_string(), "5".to_string()],
        };

        let cmd_result = server.run(Parameters(args)).await.unwrap();
        assert_eq!(cmd_result.0.exit_code, 0);
        // Should have some output from op log (operations exist from setup)
        assert!(
            !cmd_result.0.stdout.is_empty(),
            "op log should produce output"
        );
    }

    #[tokio::test]
    async fn test_handles_not_a_jj_repository() {
        // Create a directory with .config/tools.toml but no jj repo
        let temp_dir = TempDir::new().unwrap();
        std::fs::create_dir_all(temp_dir.path().join(".config")).unwrap();
        std::fs::write(
            temp_dir.path().join(".config").join("tools.toml"),
            "[commands]\n",
        )
        .unwrap();

        let args = JjArgs {
            project_dir: temp_dir.path().to_path_buf(),
            command: JjCommand::Status,
            args: vec![],
        };

        // Should return a result (not crash), but the command will fail with non-zero exit
        let cmd_result = JjReadOnly::run_jj_command(args.command, args.project_dir, args.args)
            .await
            .unwrap();
        assert_ne!(cmd_result.0.exit_code, 0);
        // jj will complain about not being in a repository
        assert!(
            cmd_result.0.stderr.contains("not a jj repo")
                || cmd_result.0.stderr.contains("no workspace")
                || cmd_result.0.stderr.contains("There is no jj repo")
        );
    }

    #[tokio::test]
    async fn test_rejects_path_with_parent_traversal() {
        let temp_dir = setup_jj_repo().await;

        // Try to escape the project directory using parent directory references
        // This will traverse to a directory that doesn't have .config/tools.toml
        let malicious_path = temp_dir.path().join("..").join("..").join("tmp");

        let args = JjArgs {
            project_dir: malicious_path,
            command: JjCommand::Status,
            args: vec![],
        };

        // Should reject with INVALID_PARAMS - either from canonicalize failing
        // or from missing .config/tools.toml
        let Err(error) =
            JjReadOnly::run_jj_command(args.command, args.project_dir, args.args).await
        else {
            panic!("Expected error for path traversal attempt");
        };

        assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
        assert!(error.message.contains("Invalid project directory"));
    }

    #[tokio::test]
    async fn test_resolves_symlinks_safely() {
        let temp_dir = setup_jj_repo().await;

        // Create a symlink to the project directory
        let link_dir = TempDir::new().unwrap();
        let link_path = link_dir.path().join("project_link");

        #[cfg(unix)]
        std::os::unix::fs::symlink(temp_dir.path(), &link_path).unwrap();

        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(temp_dir.path(), &link_path).unwrap();

        let args = JjArgs {
            project_dir: link_path,
            command: JjCommand::Status,
            args: vec![],
        };

        // Should succeed - canonicalize resolves the symlink
        let cmd_result = JjReadOnly::run_jj_command(args.command, args.project_dir, args.args)
            .await
            .unwrap();
        assert_eq!(cmd_result.0.exit_code, 0);
    }
}
