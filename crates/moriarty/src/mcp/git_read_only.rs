//! MCP server for read-only git operations.
//!
//! This module provides an MCP (Model Context Protocol) server that exposes
//! read-only git commands (status, diff, log, show) for project directories.
//!
//! # Security
//!
//! - **Shell injection protection**: Commands are executed directly without shell
//!   interpretation, preventing injection via shell metacharacters (`&&`, `||`, `|`, etc.)
//! - **Path validation**: Rejects parent traversal and canonicalizes an existing
//!   directory before execution
//! - **Read-only operations**: Forces git's no-lock/no-ext-diff/no-textconv
//!   modes and rejects flags that widen the command beyond repository-focused
//!   inspection

use std::path::{Path, PathBuf};

use rmcp::{
    handler::server::{router::prompt::PromptRouter, tool::ToolRouter, wrapper::Parameters},
    model::*,
    prompt, prompt_handler, prompt_router,
    service::RequestContext,
    tool, tool_handler, tool_router, ErrorData as McpError, Json, RoleServer, ServerHandler,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::read_only::{run_read_only_command, CommandResult};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StatusArgs {
    pub project_dir: PathBuf,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiffArgs {
    pub project_dir: PathBuf,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LogArgs {
    pub project_dir: PathBuf,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ShowArgs {
    pub project_dir: PathBuf,
    pub args: Vec<String>,
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
/// All operations reject parent traversal, canonicalize the target directory,
/// and pin git to the non-locking, internal-diff code paths so the server stays
/// within repository-focused inspection.
#[derive(Debug, Clone)]
pub struct GitReadOnly {
    pub tool_router: ToolRouter<Self>,
    pub prompt_router: PromptRouter<Self>,
}

impl GitReadOnly {
    fn validate_args(subcommand: &str, args: &[String]) -> Result<(), McpError> {
        let rejected_arg = args.iter().find(|arg| {
            arg.as_str() == "--output"
                || arg.starts_with("--output=")
                || arg.as_str() == "--ext-diff"
                || arg.as_str() == "--textconv"
                || (subcommand == "diff" && arg.as_str() == "--no-index")
        });

        if let Some(arg) = rejected_arg {
            return Err(McpError {
                code: ErrorCode::INVALID_PARAMS,
                message: format!(
                    "Invalid git arguments for {subcommand}: {arg} is not allowed in read-only mode"
                )
                .into(),
                data: None,
            });
        }

        Ok(())
    }

    async fn run_git_command(
        subcommand: &str,
        project_dir: PathBuf,
        args: Vec<String>,
    ) -> Result<Json<CommandResult>, McpError> {
        Self::validate_args(subcommand, &args)?;

        let mut subcommand_args = vec!["--no-optional-locks", subcommand];
        if matches!(subcommand, "diff" | "log" | "show") {
            subcommand_args.extend(["--no-ext-diff", "--no-textconv"]);
        }

        run_read_only_command("git", subcommand, project_dir, subcommand_args, args).await
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

/// Build the two-message `GetPromptResult` used by every git-prompt handler.
///
/// `tool_label` is the git subcommand (e.g. `"status"`) interpolated into the
/// user message (`"run the {tool_label} tool…"`); `description_phrase` is the
/// phrase following `"get git "` in the description string; `role_msg` is the
/// assistant preamble for this prompt.
fn build_prompt(
    tool_label: &str,
    description_phrase: &str,
    role_msg: &str,
    project_dir: &Path,
) -> GetPromptResult {
    let project = project_dir.to_string_lossy();
    GetPromptResult::new(vec![
        PromptMessage::new_text(PromptMessageRole::Assistant, role_msg.to_string()),
        PromptMessage::new_text(
            PromptMessageRole::User,
            format!("run the {tool_label} tool with project \"{project}\""),
        ),
    ])
    .with_description(format!("get git {description_phrase} for {project}"))
}

#[prompt_router]
impl GitReadOnly {
    #[prompt(name = "status", description = "Gets the git status")]
    async fn status_prompt(
        &self,
        Parameters(args): Parameters<StatusArgs>,
    ) -> Result<GetPromptResult, McpError> {
        Ok(build_prompt(
            "status",
            "status",
            "You report the git status",
            &args.project_dir,
        ))
    }

    #[prompt(name = "diff", description = "Gets the git diff")]
    async fn diff_prompt(
        &self,
        Parameters(args): Parameters<DiffArgs>,
    ) -> Result<GetPromptResult, McpError> {
        Ok(build_prompt(
            "diff",
            "diff status",
            "You report the results of git diff",
            &args.project_dir,
        ))
    }

    #[prompt(name = "log", description = "Gets the git log")]
    async fn log_prompt(
        &self,
        Parameters(args): Parameters<LogArgs>,
    ) -> Result<GetPromptResult, McpError> {
        Ok(build_prompt(
            "log",
            "log",
            "You report the results of git log",
            &args.project_dir,
        ))
    }

    #[prompt(name = "show", description = "Gets the git show output")]
    async fn show_prompt(
        &self,
        Parameters(args): Parameters<ShowArgs>,
    ) -> Result<GetPromptResult, McpError> {
        Ok(build_prompt(
            "show",
            "show output",
            "You report the results of git show",
            &args.project_dir,
        ))
    }
}

#[tool_handler]
#[prompt_handler]
impl ServerHandler for GitReadOnly {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_prompts()
                .enable_tools()
                .build(),
        )
        .with_server_info(Implementation::new(
            env!("CARGO_CRATE_NAME"),
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(
            "This server provides prompt templates for read only git actions. All prompts are designed to provide structured, context-aware assistance".to_string(),
        )
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

        let status = std::process::Command::new("git")
            .args(["init"])
            .current_dir(temp_dir.path())
            .status()
            .unwrap();
        assert!(status.success(), "git init failed: {status:?}");

        let status = std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(temp_dir.path())
            .status()
            .unwrap();
        assert!(status.success(), "git config user.email failed: {status:?}");

        let status = std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(temp_dir.path())
            .status()
            .unwrap();
        assert!(status.success(), "git config user.name failed: {status:?}");

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

    // Shared path-safety tests (traversal, symlink resolution, directories
    // without project config, nonexistent directory) live in
    // `crate::mcp::read_only::test_support` because `git_read_only` and
    // `jj_read_only` validate the project dir via the same helper.
    crate::mcp::read_only::test_support::path_safety_tests!(
        |dir: std::path::PathBuf, args: Vec<String>| async move {
            GitReadOnly::run_git_command("status", dir, args).await
        },
        setup_git_repo(),
        |path: &std::path::Path| {
            let status = std::process::Command::new("git")
                .args(["init"])
                .current_dir(path)
                .status()
                .unwrap();
            assert!(status.success(), "git init failed: {status:?}");
        },
    );

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

        std::fs::write(temp_dir.path().join("test.txt"), "content").unwrap();
        let status = std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(temp_dir.path())
            .status()
            .unwrap();
        assert!(status.success(), "git add failed: {status:?}");

        let status = std::process::Command::new("git")
            .args(["commit", "-m", "Test commit"])
            .current_dir(temp_dir.path())
            .status()
            .unwrap();
        assert!(status.success(), "git commit failed: {status:?}");

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
    async fn test_prompt_handlers() {
        let temp_dir = setup_git_repo().await;
        let server = GitReadOnly::default();
        let project_dir = temp_dir.path().to_path_buf();
        let project_str = project_dir.to_string_lossy().to_string();

        // Pair each handler result with a name so assertion failures in the
        // shared loop name the specific prompt that misbehaved.
        let prompts = [
            (
                "status",
                server
                    .status_prompt(Parameters(StatusArgs {
                        project_dir: project_dir.clone(),
                        args: vec![],
                    }))
                    .await
                    .unwrap(),
            ),
            (
                "diff",
                server
                    .diff_prompt(Parameters(DiffArgs {
                        project_dir: project_dir.clone(),
                        args: vec![],
                    }))
                    .await
                    .unwrap(),
            ),
            (
                "log",
                server
                    .log_prompt(Parameters(LogArgs {
                        project_dir: project_dir.clone(),
                        args: vec![],
                    }))
                    .await
                    .unwrap(),
            ),
            (
                "show",
                server
                    .show_prompt(Parameters(ShowArgs {
                        project_dir: project_dir.clone(),
                        args: vec![],
                    }))
                    .await
                    .unwrap(),
            ),
        ];

        for (name, prompt) in prompts {
            let description = prompt
                .description
                .unwrap_or_else(|| panic!("{name}: description missing"));
            assert!(
                description.contains(&project_str),
                "{name}: description {description:?} missing project path"
            );
            assert_eq!(prompt.messages.len(), 2, "{name}: message count");
            assert_eq!(
                prompt.messages[0].role,
                PromptMessageRole::Assistant,
                "{name}: messages[0] role"
            );
            assert_eq!(
                prompt.messages[1].role,
                PromptMessageRole::User,
                "{name}: messages[1] role"
            );
        }
    }

    #[tokio::test]
    async fn test_handles_not_a_git_repository() {
        // Create a directory with no git repo; read-only MCP should still run
        // the command and let git report that the directory is not a repository.
        let temp_dir = TempDir::new().unwrap();

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
    async fn test_status_ignores_malformed_tools_config() {
        let temp_dir = setup_git_repo().await;
        std::fs::create_dir_all(temp_dir.path().join(".config")).unwrap();
        std::fs::write(
            temp_dir.path().join(".config/tools.toml"),
            "this is not valid toml [[[[",
        )
        .unwrap();

        let cmd_result = GitReadOnly::run_git_command(
            "status",
            temp_dir.path().to_path_buf(),
            vec!["--short".to_string()],
        )
        .await
        .unwrap();
        assert_eq!(cmd_result.0.exit_code, 0);
    }

    #[tokio::test]
    async fn test_diff_rejects_read_only_escape_flags() {
        let temp_dir = setup_git_repo().await;
        let output_path = temp_dir.path().join("leak.patch");
        let output_path_string = output_path.to_string_lossy().into_owned();

        for args in [
            vec!["--output".to_string(), output_path_string.clone()],
            vec![format!("--output={output_path_string}")],
            vec!["--no-index".to_string()],
            vec!["--ext-diff".to_string()],
            vec!["--textconv".to_string()],
        ] {
            let rejected = args[0].clone();
            let Err(error) =
                GitReadOnly::run_git_command("diff", temp_dir.path().to_path_buf(), args).await
            else {
                panic!("Expected invalid params for {rejected}");
            };
            assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
            assert!(error.message.contains(&rejected));
            assert!(
                !output_path.exists(),
                "rejected git args created output file"
            );
        }
    }

    #[test]
    fn test_get_info_metadata() {
        let server = GitReadOnly::default();
        let info = server.get_info();

        assert!(
            info.capabilities.tools.is_some(),
            "GitReadOnly must expose tools capability"
        );
        assert!(
            info.capabilities.prompts.is_some(),
            "GitReadOnly must expose prompts capability"
        );
        assert_eq!(info.server_info.name, "moriarty");
        assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
        assert!(
            info.instructions
                .as_deref()
                .unwrap_or("")
                .contains("git"),
            "instructions should mention git: {:?}",
            info.instructions
        );
    }
}
