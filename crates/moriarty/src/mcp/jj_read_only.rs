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
//! - **Path validation**: Rejects parent traversal and canonicalizes an existing
//!   directory before execution
//! - **Read-only operations**: Forces `--ignore-working-copy` and rejects
//!   external-tool, config-injection, and repository-override flags

// standard library imports
use std::path::PathBuf;

// 3rd party crates
use rmcp::{
    ErrorData as McpError, Json, ServerHandler, handler::server::wrapper::Parameters, model::*,
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::read_only::{CommandResult, run_read_only_command};

/// Supported jj commands that can be executed via the MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum JjCommand {
    Status,
    Diff,
    Log,
    Show,
    OpLog,
    FileShow,
    FileList,
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
            Self::FileShow => vec!["file", "show"],
            Self::FileList => vec!["file", "list"],
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Status => "status",
            Self::Diff => "diff",
            Self::Log => "log",
            Self::Show => "show",
            Self::OpLog => "op log",
            Self::FileShow => "file show",
            Self::FileList => "file list",
        }
    }
}

/// Arguments for executing jj commands.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JjArgs {
    pub project_dir: PathBuf,
    pub command: JjCommand,
    pub args: Vec<String>,
}

/// MCP server providing read-only jj operations.
///
/// This server exposes a single tool that accepts a `JjCommand` enum to specify
/// which jj command to run. Supported commands:
/// - `status`: Run `jj status`
/// - `diff`: Run `jj diff`
/// - `log`: Run `jj log`
/// - `show`: Run `jj show`
/// - `op-log`: Run `jj op log`
/// - `file-show`: Run `jj file show`
/// - `file-list`: Run `jj file list`
///
/// # Security
///
/// All operations reject parent traversal, canonicalize the target directory,
/// force `--ignore-working-copy`, and block jj flags that would expand the
/// server beyond repository-focused inspection.
#[derive(Debug, Clone)]
pub struct JjReadOnly;

impl JjReadOnly {
    fn validate_args(command: &JjCommand, args: &[String]) -> Result<(), McpError> {
        let rejected_arg = args.iter().find(|arg| {
            matches!(
                arg.as_str(),
                "--tool" | "-R" | "--repository" | "--config" | "--config-file" | "--config-toml"
            ) || arg.starts_with("--tool=")
                || arg.starts_with("-R")
                || arg.starts_with("--repository=")
                || arg.starts_with("--config=")
                || arg.starts_with("--config-file=")
                || arg.starts_with("--config-toml=")
        });

        if let Some(arg) = rejected_arg {
            return Err(McpError {
                code: ErrorCode::INVALID_PARAMS,
                message: format!(
                    "Invalid jj arguments for {}: {arg} is not allowed in read-only mode",
                    command.name()
                )
                .into(),
                data: None,
            });
        }

        Ok(())
    }

    async fn run_jj_command(
        command: JjCommand,
        project_dir: PathBuf,
        args: Vec<String>,
    ) -> Result<Json<CommandResult>, McpError> {
        Self::validate_args(&command, &args)?;

        let label = command.name().to_string();
        let mut subcommand_args = vec!["--ignore-working-copy"];
        subcommand_args.extend(command.as_args());
        run_read_only_command("jj", &label, project_dir, subcommand_args, args).await
    }
}

#[tool_router(router = tool_router)]
impl JjReadOnly {
    #[tool(
        description = "Runs read-only jj commands: status, diff, log, show, op-log, file-show, or file-list"
    )]
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
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                env!("CARGO_CRATE_NAME"),
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "This server provides one `run` tool for read-only jj commands. Select status, \
                 diff, log, show, op-log, file-show, or file-list with the command field."
                    .to_string(),
            )
    }
}

impl Default for JjReadOnly {
    fn default() -> Self {
        Self
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tempfile::TempDir;

    use super::*;

    fn run_jj(path: &Path, args: &[&str]) {
        let status = std::process::Command::new("jj")
            .args(args)
            .current_dir(path)
            .status()
            .unwrap();
        assert!(status.success(), "jj command {args:?} failed: {status:?}");
    }

    async fn setup_jj_repo() -> TempDir {
        let temp_dir = TempDir::new().unwrap();
        run_jj(temp_dir.path(), &["git", "init", "--colocate"]);
        temp_dir
    }

    #[test]
    fn test_jj_command_as_args() {
        assert_eq!(JjCommand::Status.as_args(), vec!["status"]);
        assert_eq!(JjCommand::Diff.as_args(), vec!["diff"]);
        assert_eq!(JjCommand::Log.as_args(), vec!["log"]);
        assert_eq!(JjCommand::Show.as_args(), vec!["show"]);
        assert_eq!(JjCommand::OpLog.as_args(), vec!["op", "log"]);
        assert_eq!(JjCommand::FileShow.as_args(), vec!["file", "show"]);
        assert_eq!(JjCommand::FileList.as_args(), vec!["file", "list"]);
    }

    #[test]
    fn test_command_wire_values() {
        for (command, wire) in [
            (JjCommand::Status, "\"status\""),
            (JjCommand::Diff, "\"diff\""),
            (JjCommand::Log, "\"log\""),
            (JjCommand::Show, "\"show\""),
            (JjCommand::OpLog, "\"op-log\""),
            (JjCommand::FileShow, "\"file-show\""),
            (JjCommand::FileList, "\"file-list\""),
        ] {
            assert_eq!(serde_json::to_string(&command).unwrap(), wire);
            assert_eq!(serde_json::from_str::<JjCommand>(wire).unwrap(), command);
        }
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
            JjCommand::FileShow,
            JjCommand::FileList,
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
        let server = JjReadOnly;

        let args = JjArgs {
            project_dir: temp_dir.path().to_path_buf(),
            command: JjCommand::Status,
            args: vec![],
        };

        let cmd_result = server.run(Parameters(args)).await.unwrap();
        assert_eq!(cmd_result.0.exit_code, 0);
    }

    // Shared path-safety tests: see git_read_only.rs for commentary.
    crate::mcp::read_only::test_support::path_safety_tests!(
        |dir: std::path::PathBuf, args: Vec<String>| async move {
            JjReadOnly::run_jj_command(JjCommand::Status, dir, args).await
        },
        setup_jj_repo(),
        |path: &std::path::Path| run_jj(path, &["git", "init", "--colocate"]),
    );

    #[tokio::test]
    async fn test_diff_command() {
        let temp_dir = setup_jj_repo().await;
        std::fs::write(temp_dir.path().join("test.txt"), "test content").unwrap();

        run_jj(temp_dir.path(), &["commit", "-m", "Test change"]);

        let server = JjReadOnly;
        let args = JjArgs {
            project_dir: temp_dir.path().to_path_buf(),
            command: JjCommand::Diff,
            args: vec!["-r".to_string(), "@-".to_string(), "--summary".to_string()],
        };

        let cmd_result = server.run(Parameters(args)).await.unwrap();
        assert_eq!(cmd_result.0.exit_code, 0);
        assert!(
            cmd_result.0.stdout.contains("test.txt") || cmd_result.0.stdout.contains("A test.txt"),
            "diff --summary should show the committed file"
        );
    }

    #[tokio::test]
    async fn test_log_command() {
        let temp_dir = setup_jj_repo().await;

        std::fs::write(temp_dir.path().join("test.txt"), "content").unwrap();
        run_jj(temp_dir.path(), &["describe", "-m", "Test commit"]);

        let server = JjReadOnly;
        let args = JjArgs {
            project_dir: temp_dir.path().to_path_buf(),
            command: JjCommand::Log,
            args: ["-r", "@"].iter().map(|s| s.to_string()).collect(),
        };

        let cmd_result = server.run(Parameters(args)).await.unwrap();
        assert_eq!(cmd_result.0.exit_code, 0);
        assert!(cmd_result.0.stdout.contains("Test commit"));
    }

    #[tokio::test]
    async fn test_show_command() {
        let temp_dir = setup_jj_repo().await;

        std::fs::write(temp_dir.path().join("test.txt"), "test content").unwrap();
        run_jj(temp_dir.path(), &["describe", "-m", "Test change"]);

        let server = JjReadOnly;
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
    async fn test_file_list_command() {
        let temp_dir = setup_jj_repo().await;
        std::fs::create_dir(temp_dir.path().join("nested")).unwrap();
        std::fs::write(temp_dir.path().join("nested/tracked.txt"), "nested").unwrap();
        std::fs::write(temp_dir.path().join("outside.txt"), "outside").unwrap();
        run_jj(temp_dir.path(), &["commit", "-m", "Record files"]);

        let args = JjArgs {
            project_dir: temp_dir.path().to_path_buf(),
            command: JjCommand::FileList,
            args: ["-r", "@-", "nested"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        };
        let cmd_result = JjReadOnly.run(Parameters(args)).await.unwrap();
        assert_eq!(cmd_result.0.exit_code, 0);
        assert!(cmd_result.0.stdout.contains("nested/tracked.txt"));
        assert!(!cmd_result.0.stdout.contains("outside.txt"));
    }

    #[tokio::test]
    async fn test_file_show_command() {
        let temp_dir = setup_jj_repo().await;
        let content = "distinctive file contents\n";
        std::fs::write(temp_dir.path().join("tracked.txt"), content).unwrap();
        run_jj(temp_dir.path(), &["commit", "-m", "Record file"]);

        let args = JjArgs {
            project_dir: temp_dir.path().to_path_buf(),
            command: JjCommand::FileShow,
            args: ["-r", "@-", "tracked.txt"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        };
        let cmd_result = JjReadOnly.run(Parameters(args)).await.unwrap();
        assert_eq!(cmd_result.0.exit_code, 0);
        assert_eq!(cmd_result.0.stdout, content);
    }

    #[tokio::test]
    async fn test_op_log_command() {
        let temp_dir = setup_jj_repo().await;
        let server = JjReadOnly;

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
        // Create a directory with no jj repo; read-only MCP should still run
        // the command and let jj report that the directory is not a repository.
        let temp_dir = TempDir::new().unwrap();

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
    async fn test_status_ignores_malformed_tools_config() {
        let temp_dir = setup_jj_repo().await;
        std::fs::create_dir_all(temp_dir.path().join(".config")).unwrap();
        std::fs::write(
            temp_dir.path().join(".config/tools.toml"),
            "this is not valid toml [[[[",
        )
        .unwrap();

        let cmd_result =
            JjReadOnly::run_jj_command(JjCommand::Status, temp_dir.path().to_path_buf(), vec![])
                .await
                .unwrap();
        assert_eq!(cmd_result.0.exit_code, 0);
    }

    #[tokio::test]
    async fn test_diff_rejects_read_only_escape_flags() {
        let temp_dir = setup_jj_repo().await;

        for args in [
            vec!["--tool".to_string(), ":git".to_string()],
            vec!["--tool=:git".to_string()],
            vec!["-R".to_string(), ".".to_string()],
            vec!["-R.".to_string()],
            vec!["--repository".to_string(), ".".to_string()],
            vec!["--repository=.".to_string()],
            vec!["--config".to_string(), "ui.diff.tool=':git'".to_string()],
            vec!["--config=ui.diff.tool=':git'".to_string()],
            vec!["--config-file".to_string(), "config.toml".to_string()],
            vec!["--config-file=config.toml".to_string()],
            vec![
                "--config-toml".to_string(),
                "ui.diff.tool=':git'".to_string(),
            ],
            vec!["--config-toml=ui.diff.tool=':git'".to_string()],
        ] {
            let rejected = args[0].clone();
            let Err(error) =
                JjReadOnly::run_jj_command(JjCommand::Diff, temp_dir.path().to_path_buf(), args)
                    .await
            else {
                panic!("Expected invalid params for {rejected}");
            };
            assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
            assert!(error.message.contains(&rejected));
        }
    }

    #[test]
    fn test_get_info_metadata() {
        let server = JjReadOnly;
        let info = server.get_info();

        assert!(
            info.capabilities.tools.is_some(),
            "JjReadOnly must expose tools capability"
        );
        assert!(
            info.capabilities.prompts.is_none(),
            "JjReadOnly should not expose prompts capability"
        );
        assert_eq!(info.server_info.name, "moriarty");
        assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
        assert!(
            info.instructions.as_deref().unwrap_or("").contains("jj"),
            "instructions should mention jj: {:?}",
            info.instructions
        );
    }
}
