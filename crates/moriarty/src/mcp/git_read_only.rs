//! MCP server for read-only git operations.
//!
//! This module provides an MCP (Model Context Protocol) server that exposes
//! read-only git commands (status, diff, log, show) for project directories.
//!
//! # Security
//!
//! - **Shell injection protection**: Commands are executed directly without shell
//!   interpretation, preventing injection via shell metacharacters (`&&`, `||`, `|`, etc.)
//! - **Path validation**: Only operates on valid project directories (verified by
//!   checking for `.config/tools.toml` marker file)
//! - **Read-only operations**: All exposed git commands are read-only; no modifications
//!   to the repository are possible

use std::{path::PathBuf, str};

use rmcp::{
    handler::server::{router::prompt::PromptRouter, tool::ToolRouter, wrapper::Parameters},
    model::*,
    prompt, prompt_handler, prompt_router,
    service::RequestContext,
    tool, tool_handler, tool_router, ErrorData as McpError, Json, RoleServer, ServerHandler,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StatusArgs {
    /// The directory of the project (must contain `.config/tools.toml`)
    pub project_dir: PathBuf,
    /// Additional arguments (e.g., `["--short"]`)
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiffArgs {
    /// The directory of the project (must contain `.config/tools.toml`)
    pub project_dir: PathBuf,
    /// Additional arguments (e.g., `["--cached"]`)
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LogArgs {
    /// The directory of the project (must contain `.config/tools.toml`)
    pub project_dir: PathBuf,
    /// Additional arguments (e.g., `["--oneline", "-10"]`)
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ShowArgs {
    /// The directory of the project (must contain `.config/tools.toml`)
    pub project_dir: PathBuf,
    /// Additional arguments (e.g., `["HEAD"]`)
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommandResult {
    pub stdout: String,
    pub stderr: String,
    /// Exit code (0 indicates success)
    pub exit_code: i32,
}

/// MCP server providing read-only git operations.
///
/// This server exposes four git commands via MCP tools:
/// - `status`: Run `git status` with optional arguments
/// - `diff`: Run `git diff` with optional arguments
/// - `log`: Run `git log` with optional arguments
/// - `show`: Run `git show` with optional arguments
///
/// # Security
///
/// All operations validate that the project directory contains `.config/tools.toml`
/// to prevent execution in arbitrary system directories.
#[derive(Debug, Clone)]
pub struct GitReadOnly {
    pub tool_router: ToolRouter<Self>,
    pub prompt_router: PromptRouter<Self>,
}

impl GitReadOnly {
    async fn run_git_command(
        subcommand: &str,
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

        let mut cmd = Command::new("git");
        cmd.arg(subcommand);
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
                message: format!("{} failed: {error:?}", subcommand).into(),
                data: None,
            }),
        }
    }
}

#[tool_router(router = tool_router)]
impl GitReadOnly {
    #[tool(description = "Runs `git status` with the provided args")]
    async fn status(
        &self,
        Parameters(args): Parameters<StatusArgs>,
    ) -> Result<Json<CommandResult>, McpError> {
        let StatusArgs { project_dir, args } = args;
        Self::run_git_command("status", project_dir, args).await
    }

    #[tool(description = "Runs `git diff` with the provided args")]
    async fn diff(
        &self,
        Parameters(args): Parameters<DiffArgs>,
    ) -> Result<Json<CommandResult>, McpError> {
        let DiffArgs { project_dir, args } = args;
        Self::run_git_command("diff", project_dir, args).await
    }

    #[tool(description = "Runs `git show` with the provided args")]
    async fn show(
        &self,
        Parameters(args): Parameters<ShowArgs>,
    ) -> Result<Json<CommandResult>, McpError> {
        let ShowArgs { project_dir, args } = args;
        Self::run_git_command("show", project_dir, args).await
    }

    #[tool(description = "Runs `git log` with the provided args")]
    async fn log(
        &self,
        Parameters(args): Parameters<LogArgs>,
    ) -> Result<Json<CommandResult>, McpError> {
        let LogArgs { project_dir, args } = args;
        Self::run_git_command("log", project_dir, args).await
    }
}

#[prompt_router]
impl GitReadOnly {
    #[prompt(name = "status", description = "Gets the git status")]
    async fn status_prompt(
        &self,
        Parameters(args): Parameters<StatusArgs>,
    ) -> Result<GetPromptResult, McpError> {
        let messages = vec![
            PromptMessage::new_text(PromptMessageRole::Assistant, "You report the git status"),
            PromptMessage::new_text(
                PromptMessageRole::User,
                format!(
                    "run the status tool with project \"{}\"",
                    args.project_dir.to_string_lossy()
                ),
            ),
        ];

        Ok(GetPromptResult {
            description: Some(format!(
                "get git status for {}",
                args.project_dir.to_string_lossy()
            )),
            messages,
        })
    }

    #[prompt(name = "diff", description = "Gets the git diff")]
    async fn diff_prompt(
        &self,
        Parameters(args): Parameters<DiffArgs>,
    ) -> Result<GetPromptResult, McpError> {
        let messages = vec![
            PromptMessage::new_text(
                PromptMessageRole::Assistant,
                "You report the results of git diff",
            ),
            PromptMessage::new_text(
                PromptMessageRole::User,
                format!(
                    "run the diff tool with project \"{}\"",
                    args.project_dir.to_string_lossy()
                ),
            ),
        ];

        Ok(GetPromptResult {
            description: Some(format!(
                "get git diff status for {}",
                args.project_dir.to_string_lossy()
            )),
            messages,
        })
    }

    #[prompt(name = "log", description = "Gets the git log")]
    async fn log_prompt(
        &self,
        Parameters(args): Parameters<LogArgs>,
    ) -> Result<GetPromptResult, McpError> {
        let messages = vec![
            PromptMessage::new_text(
                PromptMessageRole::Assistant,
                "You report the results of git log",
            ),
            PromptMessage::new_text(
                PromptMessageRole::User,
                format!(
                    "run the log tool with project \"{}\"",
                    args.project_dir.to_string_lossy()
                ),
            ),
        ];

        Ok(GetPromptResult {
            description: Some(format!(
                "get git log for {}",
                args.project_dir.to_string_lossy()
            )),
            messages,
        })
    }

    #[prompt(name = "show", description = "Gets the git show output")]
    async fn show_prompt(
        &self,
        Parameters(args): Parameters<ShowArgs>,
    ) -> Result<GetPromptResult, McpError> {
        let messages = vec![
            PromptMessage::new_text(
                PromptMessageRole::Assistant,
                "You report the results of git show",
            ),
            PromptMessage::new_text(
                PromptMessageRole::User,
                format!(
                    "run the show tool with project \"{}\"",
                    args.project_dir.to_string_lossy()
                ),
            ),
        ];

        Ok(GetPromptResult {
            description: Some(format!(
                "get git show output for {}",
                args.project_dir.to_string_lossy()
            )),
            messages,
        })
    }
}

#[tool_handler]
#[prompt_handler]
impl ServerHandler for GitReadOnly {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_prompts().enable_tools().build(),
            server_info: Implementation::from_build_env(),
            instructions: Some(
                "This server provides prompt templates for read only git actions. All prompts are designed to provide structured, context-aware assistance".to_string()
            ),
            ..Default::default()
        }
    }
}

impl Default for GitReadOnly {
    fn default() -> Self {
        Self {
            tool_router: Self::tool_router(),
            prompt_router: Self::prompt_router(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn setup_git_repo() -> TempDir {
        let temp_dir = TempDir::new().unwrap();

        // Initialize git repository
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(temp_dir.path())
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(temp_dir.path())
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
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

    /// Helper function to test shell injection protection for any git command and operator.
    async fn verify_no_shell_injection(command: &str, operator: &str) {
        let temp_dir = setup_git_repo().await;
        let args = vec![
            operator.to_string(),
            "echo".to_string(),
            "FAILED".to_string(),
        ];

        let result =
            GitReadOnly::run_git_command(command, temp_dir.path().to_path_buf(), args).await;

        match result {
            Ok(cmd_result) => {
                // Check that git output doesn't contain our injection attempt
                assert!(
                    !cmd_result.0.stdout.contains("FAILED"),
                    "Shell injection occurred in '{}' with '{}': stdout contains 'FAILED'",
                    command,
                    operator
                );
                assert!(
                    !cmd_result.0.stderr.contains("FAILED"),
                    "Shell injection occurred in '{}' with '{}': stderr contains 'FAILED'",
                    command,
                    operator
                );
            }
            Err(error) => {
                // If there's an error, check for shell-specific patterns that would
                // indicate shell execution rather than just git rejecting bad arguments.
                // We avoid checking for "FAILED" since git might include our test args
                // in error messages like "unrecognized argument: FAILED".
                let err_msg = error.message.to_string().to_lowercase();
                assert!(
                    !err_msg.contains("sh:")
                        && !err_msg.contains("bash:")
                        && !err_msg.contains("/bin/echo"),
                    "Shell execution detected in error for '{}' with '{}': {}",
                    command,
                    operator,
                    error.message
                );
            }
        }
    }

    #[tokio::test]
    async fn test_shell_injection_protection() {
        let commands = ["status", "diff", "log", "show"];
        let operators = ["&&", "||", "|"];

        for command in &commands {
            for operator in &operators {
                verify_no_shell_injection(command, operator).await;
            }
        }
    }

    #[tokio::test]
    async fn test_status_succeeds_with_valid_args() {
        let temp_dir = setup_git_repo().await;

        let args = StatusArgs {
            project_dir: temp_dir.path().to_path_buf(),
            args: vec!["--short".to_string()],
        };

        let cmd_result = GitReadOnly::run_git_command("status", args.project_dir, args.args)
            .await
            .unwrap();
        assert_eq!(cmd_result.0.exit_code, 0);
    }

    #[tokio::test]
    async fn test_rejects_directory_without_config_file() {
        let temp_dir = TempDir::new().unwrap();

        // Initialize git but don't create .config/tools.toml
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(temp_dir.path())
            .status()
            .unwrap();

        let args = StatusArgs {
            project_dir: temp_dir.path().to_path_buf(),
            args: vec![],
        };

        let result = GitReadOnly::run_git_command("status", args.project_dir, args.args).await;

        match result {
            Err(error) => {
                assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
                assert!(error.message.contains("Invalid project directory"));
                assert!(error.message.contains("missing .config/tools.toml"));
            }
            Ok(_) => panic!("Expected error for directory without .config/tools.toml"),
        }
    }

    #[tokio::test]
    async fn test_rejects_nonexistent_directory() {
        let args = StatusArgs {
            project_dir: PathBuf::from("/nonexistent/directory"),
            args: vec![],
        };

        let result = GitReadOnly::run_git_command("status", args.project_dir, args.args).await;

        match result {
            Err(error) => {
                assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
            }
            Ok(_) => panic!("Expected error for nonexistent directory"),
        }
    }

    #[tokio::test]
    async fn test_diff_tool_handler() {
        let temp_dir = setup_git_repo().await;
        let server = GitReadOnly::default();

        let args = DiffArgs {
            project_dir: temp_dir.path().to_path_buf(),
            args: vec!["--cached".to_string()],
        };

        let cmd_result = server.diff(Parameters(args)).await.unwrap();
        assert_eq!(cmd_result.0.exit_code, 0);
    }

    #[tokio::test]
    async fn test_log_tool_handler() {
        let temp_dir = setup_git_repo().await;

        // Create a commit so log has output
        std::fs::write(temp_dir.path().join("test.txt"), "content").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(temp_dir.path())
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "Test commit"])
            .current_dir(temp_dir.path())
            .status()
            .unwrap();

        let server = GitReadOnly::default();
        let args = LogArgs {
            project_dir: temp_dir.path().to_path_buf(),
            args: vec!["--oneline".to_string(), "-1".to_string()],
        };

        let cmd_result = server.log(Parameters(args)).await.unwrap();
        assert_eq!(cmd_result.0.exit_code, 0);
        assert!(cmd_result.0.stdout.contains("Test commit"));
    }

    #[tokio::test]
    async fn test_show_tool_handler() {
        let temp_dir = setup_git_repo().await;
        let server = GitReadOnly::default();

        let args = ShowArgs {
            project_dir: temp_dir.path().to_path_buf(),
            args: vec!["HEAD".to_string()],
        };

        server.show(Parameters(args)).await.unwrap();
    }

    #[tokio::test]
    async fn test_status_prompt() {
        let temp_dir = setup_git_repo().await;
        let server = GitReadOnly::default();

        let args = StatusArgs {
            project_dir: temp_dir.path().to_path_buf(),
            args: vec![],
        };

        let prompt = server.status_prompt(Parameters(args)).await.unwrap();
        assert!(prompt.description.is_some());
        assert!(prompt
            .description
            .unwrap()
            .contains(&temp_dir.path().to_string_lossy().to_string()));
        assert_eq!(prompt.messages.len(), 2);
        assert_eq!(prompt.messages[0].role, PromptMessageRole::Assistant);
        assert_eq!(prompt.messages[1].role, PromptMessageRole::User);
    }

    #[tokio::test]
    async fn test_diff_prompt() {
        let temp_dir = setup_git_repo().await;
        let server = GitReadOnly::default();

        let args = DiffArgs {
            project_dir: temp_dir.path().to_path_buf(),
            args: vec![],
        };

        let prompt = server.diff_prompt(Parameters(args)).await.unwrap();
        assert!(prompt.description.is_some());
        assert_eq!(prompt.messages.len(), 2);
        assert_eq!(prompt.messages[0].role, PromptMessageRole::Assistant);
        assert_eq!(prompt.messages[1].role, PromptMessageRole::User);
    }

    #[tokio::test]
    async fn test_log_prompt() {
        let temp_dir = setup_git_repo().await;
        let server = GitReadOnly::default();

        let args = LogArgs {
            project_dir: temp_dir.path().to_path_buf(),
            args: vec![],
        };

        let prompt = server.log_prompt(Parameters(args)).await.unwrap();
        assert!(prompt.description.is_some());
        assert!(prompt
            .description
            .unwrap()
            .contains(&temp_dir.path().to_string_lossy().to_string()));
        assert_eq!(prompt.messages.len(), 2);
        assert_eq!(prompt.messages[0].role, PromptMessageRole::Assistant);
        assert_eq!(prompt.messages[1].role, PromptMessageRole::User);
    }

    #[tokio::test]
    async fn test_show_prompt() {
        let temp_dir = setup_git_repo().await;
        let server = GitReadOnly::default();

        let args = ShowArgs {
            project_dir: temp_dir.path().to_path_buf(),
            args: vec![],
        };

        let prompt = server.show_prompt(Parameters(args)).await.unwrap();
        assert!(prompt.description.is_some());
        assert!(prompt
            .description
            .unwrap()
            .contains(&temp_dir.path().to_string_lossy().to_string()));
        assert_eq!(prompt.messages.len(), 2);
        assert_eq!(prompt.messages[0].role, PromptMessageRole::Assistant);
        assert_eq!(prompt.messages[1].role, PromptMessageRole::User);
    }

    #[tokio::test]
    async fn test_handles_not_a_git_repository() {
        // Create a directory with .config/tools.toml but no git repo
        let temp_dir = TempDir::new().unwrap();
        std::fs::create_dir_all(temp_dir.path().join(".config")).unwrap();
        std::fs::write(
            temp_dir.path().join(".config").join("tools.toml"),
            "[commands]\n",
        )
        .unwrap();

        let args = StatusArgs {
            project_dir: temp_dir.path().to_path_buf(),
            args: vec![],
        };

        // Should return a result (not crash), but the command will fail with non-zero exit
        let cmd_result = GitReadOnly::run_git_command("status", args.project_dir, args.args)
            .await
            .unwrap();
        assert_ne!(cmd_result.0.exit_code, 0);
        assert!(
            cmd_result.0.stderr.contains("not a git repository")
                || cmd_result.0.stderr.contains("not a git repo")
        );
    }

    #[tokio::test]
    async fn test_rejects_path_with_parent_traversal() {
        let temp_dir = setup_git_repo().await;

        // Try to escape the project directory using parent directory references
        // This will traverse to a directory that doesn't have .config/tools.toml
        let malicious_path = temp_dir.path().join("..").join("..").join("tmp");

        let args = StatusArgs {
            project_dir: malicious_path,
            args: vec![],
        };

        let result = GitReadOnly::run_git_command("status", args.project_dir, args.args).await;

        match result {
            Err(error) => {
                // Should reject with INVALID_PARAMS - either from canonicalize failing
                // or from missing .config/tools.toml
                assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
                assert!(error.message.contains("Invalid project directory"));
            }
            Ok(_) => panic!("Expected error for path traversal attempt"),
        }
    }

    #[tokio::test]
    async fn test_resolves_symlinks_safely() {
        let temp_dir = setup_git_repo().await;

        // Create a symlink to the project directory
        let link_dir = TempDir::new().unwrap();
        let link_path = link_dir.path().join("project_link");

        #[cfg(unix)]
        std::os::unix::fs::symlink(temp_dir.path(), &link_path).unwrap();

        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(temp_dir.path(), &link_path).unwrap();

        let args = StatusArgs {
            project_dir: link_path,
            args: vec!["--short".to_string()],
        };

        // Should succeed - canonicalize resolves the symlink
        let cmd_result = GitReadOnly::run_git_command("status", args.project_dir, args.args)
            .await
            .unwrap();
        assert_eq!(cmd_result.0.exit_code, 0);
    }
}
