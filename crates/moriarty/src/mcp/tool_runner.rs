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
                command,
                expected,
                actual,
            } => {
                return Err(McpError {
                    code: ErrorCode::INVALID_REQUEST,
                    message: format!(
                        "Binary for '{}' command has been modified since approval. \
                        Run: moriarty approve-project {} \
                        (expected: {}, actual: {})",
                        command,
                        canonical_dir.display(),
                        expected,
                        actual
                    )
                    .into(),
                    data: None,
                });
            }
            VerificationResult::CommandNotApproved { command } => {
                return Err(McpError {
                    code: ErrorCode::INVALID_REQUEST,
                    message: format!(
                        "Command '{}' not approved. Run: moriarty approve-project {}",
                        command,
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
mod tests {
    use super::*;
    use crate::project_config::{approvals, ProjectConfig};
    use std::path::Path;
    use tempfile::TempDir;

    // IMPORTANT: Tests use `_xdg_dir` variables to keep TempDir instances alive.
    // TempDir deletes its directory when dropped, so binding it to a variable (even
    // with underscore prefix) prevents premature cleanup. Without this binding, the
    // temporary XDG_CONFIG_HOME directory would be deleted before the test completes.

    fn setup_project_dir_with_config(config_content: &str) -> TempDir {
        let temp_dir = TempDir::new().unwrap();
        let config_dir = temp_dir.path().join(".config");
        std::fs::create_dir(&config_dir).unwrap();
        std::fs::write(config_dir.join("tools.toml"), config_content).unwrap();
        temp_dir
    }

    /// Safe to use std::env::set_var because cargo nextest isolates each test in a separate process.
    fn setup_isolated_xdg_config() -> tempfile::TempDir {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path());
        temp_dir
    }

    /// Pre-approves tools to bypass the approval TUI in integration tests.
    /// Helper to approve a project with the given config content.
    /// Returns the canonical project path for use in assertions.
    async fn approve_project_config(project_dir: &Path, config_content: &str) -> PathBuf {
        use std::collections::HashMap;

        let canonical_path = project_dir.canonicalize().unwrap();
        let config: ProjectConfig = toml::from_str(config_content).unwrap();
        let tools_config_hash = crate::hashing::hash_string(config_content);

        let mut commands = HashMap::new();
        for (name, cmd_array) in config.commands.all() {
            let binary_name = &cmd_array[0];
            let (original_path, resolved_path) =
                approvals::resolve_binary_path_with_original(binary_name, &canonical_path).unwrap();
            let binary_hash = crate::hashing::hash_file(&resolved_path).await.unwrap();

            commands.insert(
                name,
                approvals::CommandApproval {
                    original_path: original_path.to_string_lossy().to_string(),
                    canonical_path: resolved_path.to_string_lossy().to_string(),
                    binary_hash,
                },
            );
        }

        let canonical_path_clone = canonical_path.clone();
        approvals::ProjectApprovals::update(move |approvals| {
            approvals.approve_project(canonical_path_clone, tools_config_hash, commands);
        })
        .await
        .unwrap();

        canonical_path
    }

    async fn setup_project_dir_with_approvals(config_content: &str) -> (TempDir, TempDir) {
        let xdg_dir = setup_isolated_xdg_config();
        let temp_dir = setup_project_dir_with_config(config_content);
        approve_project_config(temp_dir.path(), config_content).await;
        (temp_dir, xdg_dir)
    }

    #[tokio::test]
    async fn test_load_project_settings_success() {
        let temp_dir = setup_project_dir_with_config(
            r#"
[commands]
lint = ["cargo", "clippy"]
test = ["cargo", "test"]
build = ["cargo", "build"]
format = ["cargo", "fmt"]
"#,
        );

        let config = load_project_settings(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        assert_eq!(
            config.commands.lint,
            Some(vec!["cargo".to_string(), "clippy".to_string()])
        );
        assert_eq!(
            config.commands.test,
            Some(vec!["cargo".to_string(), "test".to_string()])
        );
        assert_eq!(
            config.commands.build,
            Some(vec!["cargo".to_string(), "build".to_string()])
        );
        assert_eq!(
            config.commands.format,
            Some(vec!["cargo".to_string(), "fmt".to_string()])
        );
    }

    #[tokio::test]
    async fn test_load_project_settings_partial_config() {
        let temp_dir = setup_project_dir_with_config(
            r#"
[commands]
lint = ["cargo", "clippy"]
"#,
        );

        let config = load_project_settings(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        assert_eq!(
            config.commands.lint,
            Some(vec!["cargo".to_string(), "clippy".to_string()])
        );
        assert_eq!(config.commands.test, None);
        assert_eq!(config.commands.build, None);
        assert_eq!(config.commands.format, None);
    }

    #[tokio::test]
    async fn test_load_project_settings_missing_file() {
        let temp_dir = TempDir::new().unwrap();

        let result = load_project_settings(temp_dir.path().to_path_buf()).await;

        assert!(result.is_err());
        let error_msg = format!("{:?}", result.unwrap_err());
        assert!(error_msg.contains("failed to read project settings"));
    }

    #[tokio::test]
    async fn test_load_project_settings_malformed_toml() {
        let temp_dir = setup_project_dir_with_config("this is not valid toml [[[");

        let result = load_project_settings(temp_dir.path().to_path_buf()).await;

        assert!(result.is_err());
        let error_msg = format!("{:?}", result.unwrap_err());
        assert!(error_msg.contains("failed to parse project settings"));
    }

    #[tokio::test]
    async fn test_run_command_success() {
        let (temp_dir, _xdg_dir) = setup_project_dir_with_approvals(
            r#"
[commands]
test = ["echo", "test output"]
"#,
        )
        .await;

        let args = RunArgs {
            project_dir: temp_dir.path().to_path_buf(),
        };

        let result = ToolRunner::run_command(ProjectCommand::Test, args).await;

        let tool_result = result.unwrap();
        assert_eq!(tool_result.is_error, Some(false));
        assert_eq!(tool_result.content.len(), 2);
    }

    #[tokio::test]
    async fn test_run_command_not_configured() {
        // Verify that commands not in tools.toml are rejected even if approved
        let (temp_dir, _xdg_dir) = setup_project_dir_with_approvals(
            r#"
[commands]
lint = ["cargo", "clippy"]
"#,
        )
        .await;

        let args = RunArgs {
            project_dir: temp_dir.path().to_path_buf(),
        };

        let result = ToolRunner::run_command(ProjectCommand::Test, args).await;

        match result {
            Err(error) => {
                assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
                assert!(error.message.contains("not approved"));
            }
            Ok(_) => panic!("Expected error for unconfigured command"),
        }
    }

    #[tokio::test]
    async fn test_run_command_not_approved() {
        // Unlike other tests, deliberately skip approval setup to test rejection path
        let _xdg_dir = setup_isolated_xdg_config();
        let temp_dir = setup_project_dir_with_config(
            r#"
[commands]
test = ["echo", "hello"]
"#,
        );

        let args = RunArgs {
            project_dir: temp_dir.path().to_path_buf(),
        };

        let result = ToolRunner::run_command(ProjectCommand::Test, args).await;

        match result {
            Err(error) => {
                assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
                assert!(error.message.contains("not approved"));
            }
            Ok(_) => panic!("Expected error for unapproved project"),
        }
    }

    #[tokio::test]
    async fn test_run_command_nonzero_exit() {
        let (temp_dir, _xdg_dir) = setup_project_dir_with_approvals(
            r#"
[commands]
test = ["sh", "-c", "exit 1"]
"#,
        )
        .await;

        let args = RunArgs {
            project_dir: temp_dir.path().to_path_buf(),
        };

        let result = ToolRunner::run_command(ProjectCommand::Test, args).await;

        let tool_result = result.unwrap();
        assert_eq!(tool_result.is_error, Some(true));
    }

    #[tokio::test]
    async fn test_run_command_config_hash_mismatch() {
        // Simulate an attacker modifying tools.toml after legitimate approval
        let (temp_dir, _xdg_dir) = setup_project_dir_with_approvals(
            r#"
[commands]
test = ["echo", "original"]
"#,
        )
        .await;

        let config_path = temp_dir.path().join(".config/tools.toml");
        std::fs::write(
            config_path,
            r#"
[commands]
test = ["echo", "modified"]
"#,
        )
        .unwrap();

        let args = RunArgs {
            project_dir: temp_dir.path().to_path_buf(),
        };

        let result = ToolRunner::run_command(ProjectCommand::Test, args).await;

        match result {
            Err(error) => {
                assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
                assert!(error.message.contains("tools.toml has been modified"));
            }
            Ok(_) => panic!("Expected error for modified config"),
        }
    }

    #[tokio::test]
    async fn test_project_command_display() {
        assert_eq!(format!("{}", ProjectCommand::Lint), "lint");
        assert_eq!(format!("{}", ProjectCommand::Test), "test");
        assert_eq!(format!("{}", ProjectCommand::Build), "build");
        assert_eq!(format!("{}", ProjectCommand::Format), "format");
    }

    #[tokio::test]
    async fn test_run_lint_handler() {
        let (temp_dir, _xdg_dir) = setup_project_dir_with_approvals(
            r#"
[commands]
lint = ["echo", "Running lint"]
"#,
        )
        .await;

        let server = ToolRunner::default();
        let args = RunArgs {
            project_dir: temp_dir.path().to_path_buf(),
        };

        let result = server.run_lint(Parameters(args)).await;
        let tool_result = result.unwrap();
        assert_eq!(tool_result.is_error, Some(false));
        assert_eq!(tool_result.content.len(), 2);
    }

    #[tokio::test]
    async fn test_run_build_handler() {
        let (temp_dir, _xdg_dir) = setup_project_dir_with_approvals(
            r#"
[commands]
build = ["echo", "Building project"]
"#,
        )
        .await;

        let server = ToolRunner::default();
        let args = RunArgs {
            project_dir: temp_dir.path().to_path_buf(),
        };

        let result = server.run_build(Parameters(args)).await;
        let tool_result = result.unwrap();
        assert_eq!(tool_result.is_error, Some(false));
        assert_eq!(tool_result.content.len(), 2);
    }

    #[tokio::test]
    async fn test_run_formatter_handler() {
        let (temp_dir, _xdg_dir) = setup_project_dir_with_approvals(
            r#"
[commands]
format = ["echo", "Formatting code"]
"#,
        )
        .await;

        let server = ToolRunner::default();
        let args = RunArgs {
            project_dir: temp_dir.path().to_path_buf(),
        };

        let result = server.run_formatter(Parameters(args)).await;
        let tool_result = result.unwrap();
        assert_eq!(tool_result.is_error, Some(false));
        assert_eq!(tool_result.content.len(), 2);
    }

    #[tokio::test]
    async fn test_run_tests_handler() {
        let (temp_dir, _xdg_dir) = setup_project_dir_with_approvals(
            r#"
[commands]
test = ["echo", "Running tests"]
"#,
        )
        .await;

        let server = ToolRunner::default();
        let args = RunArgs {
            project_dir: temp_dir.path().to_path_buf(),
        };

        let result = server.run_tests(Parameters(args)).await;
        let tool_result = result.unwrap();
        assert_eq!(tool_result.is_error, Some(false));
        assert_eq!(tool_result.content.len(), 2);
    }

    #[tokio::test]
    async fn test_rejects_path_traversal_in_run_command() {
        let temp_dir = setup_project_dir_with_config(
            r#"
[commands]
test = ["echo", "hello"]
"#,
        );

        // Try to escape the project directory using parent directory references
        let malicious_path = temp_dir.path().join("..").join("..").join("tmp");

        let args = RunArgs {
            project_dir: malicious_path,
        };

        let result = ToolRunner::run_command(ProjectCommand::Test, args).await;

        match result {
            Err(error) => {
                assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
            }
            Ok(_) => panic!("Expected error for path traversal attempt"),
        }
    }

    #[tokio::test]
    async fn test_resolves_symlinks_in_tool_runner() {
        let (temp_dir, _xdg_dir) = setup_project_dir_with_approvals(
            r#"
[commands]
test = ["echo", "hello"]
"#,
        )
        .await;

        // Create a symlink to the project directory
        let link_dir = TempDir::new().unwrap();
        let link_path = link_dir.path().join("project_link");

        #[cfg(unix)]
        std::os::unix::fs::symlink(temp_dir.path(), &link_path).unwrap();

        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(temp_dir.path(), &link_path).unwrap();

        let args = RunArgs {
            project_dir: link_path,
        };

        let result = ToolRunner::run_command(ProjectCommand::Test, args).await;

        // Should succeed - canonicalize resolves the symlink
        let tool_result = result.unwrap();
        assert_eq!(tool_result.is_error, Some(false));
    }

    #[tokio::test]
    async fn test_detects_binary_swap_toctou_attack() {
        // TOCTOU attack: Approve legitimate binary, then swap with malicious one
        // This simulates an attacker replacing a binary after approval but before execution
        use std::io::Write;

        let _xdg_dir = setup_isolated_xdg_config();

        // Create a custom script that will be approved
        let temp_dir = TempDir::new().unwrap();
        let config_dir = temp_dir.path().join(".config");
        std::fs::create_dir(&config_dir).unwrap();

        let script_path = temp_dir.path().join("legitimate.sh");
        let mut script = std::fs::File::create(&script_path).unwrap();
        writeln!(script, "#!/usr/bin/env bash").unwrap();
        writeln!(script, "echo 'legitimate'").unwrap();
        drop(script);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).unwrap();
        }

        let config_content = format!(
            r#"
[commands]
test = ["{}"]
"#,
            script_path.display()
        );

        std::fs::write(config_dir.join("tools.toml"), &config_content).unwrap();

        // Approve the legitimate binary
        approve_project_config(temp_dir.path(), &config_content).await;

        // TOCTOU attack: Swap the binary with malicious content after approval
        let mut malicious_script = std::fs::File::create(&script_path).unwrap();
        writeln!(malicious_script, "#!/usr/bin/env bash").unwrap();
        writeln!(malicious_script, "echo 'malicious'").unwrap();
        writeln!(malicious_script, "rm -rf /").unwrap(); // Simulated malicious command
        drop(malicious_script);

        // Attempt to execute - should be rejected due to hash mismatch
        let args = RunArgs {
            project_dir: temp_dir.path().to_path_buf(),
        };

        let result = ToolRunner::run_command(ProjectCommand::Test, args).await;

        match result {
            Err(error) => {
                assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
                let msg_lower = error.message.to_lowercase();
                assert!(
                    (msg_lower.contains("binary") || msg_lower.contains("modified"))
                        && (msg_lower.contains("hash") || msg_lower.contains("sha256")),
                    "Error should indicate binary hash mismatch. Got: {}",
                    error.message
                );
            }
            Ok(_) => panic!("TOCTOU attack should be detected - binary was swapped after approval"),
        }
    }

    #[tokio::test]
    async fn test_detects_symlink_target_change_toctou() {
        // TOCTOU via symlink: Approve binary via symlink, then change symlink target
        // This tests that canonical path verification prevents symlink manipulation
        #[cfg(not(unix))]
        return; // Skip on non-Unix systems

        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::PermissionsExt;

            let _xdg_dir = setup_isolated_xdg_config();

            let temp_dir = TempDir::new().unwrap();
            let config_dir = temp_dir.path().join(".config");
            std::fs::create_dir(&config_dir).unwrap();

            // Create legitimate binary
            let legitimate_path = temp_dir.path().join("legitimate.sh");
            let mut legitimate = std::fs::File::create(&legitimate_path).unwrap();
            writeln!(legitimate, "#!/usr/bin/env bash").unwrap();
            writeln!(legitimate, "echo 'legitimate'").unwrap();
            drop(legitimate);
            let mut perms = std::fs::metadata(&legitimate_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&legitimate_path, perms).unwrap();

            // Create malicious binary
            let malicious_path = temp_dir.path().join("malicious.sh");
            let mut malicious = std::fs::File::create(&malicious_path).unwrap();
            writeln!(malicious, "#!/usr/bin/env bash").unwrap();
            writeln!(malicious, "echo 'malicious'").unwrap();
            drop(malicious);
            let mut perms = std::fs::metadata(&malicious_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&malicious_path, perms).unwrap();

            // Create symlink pointing to legitimate binary
            let symlink_path = temp_dir.path().join("script.sh");
            std::os::unix::fs::symlink(&legitimate_path, &symlink_path).unwrap();

            let config_content = format!(
                r#"
[commands]
test = ["{}"]
"#,
                symlink_path.display()
            );

            std::fs::write(config_dir.join("tools.toml"), &config_content).unwrap();

            // Approve via symlink (resolves to legitimate binary)
            approve_project_config(temp_dir.path(), &config_content).await;

            // TOCTOU attack: Change symlink to point to malicious binary
            std::fs::remove_file(&symlink_path).unwrap();
            std::os::unix::fs::symlink(&malicious_path, &symlink_path).unwrap();

            // Attempt execution - should be rejected (canonical path changed)
            let args = RunArgs {
                project_dir: temp_dir.path().to_path_buf(),
            };

            let result = ToolRunner::run_command(ProjectCommand::Test, args).await;

            match result {
                Err(error) => {
                    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
                    let msg_lower = error.message.to_lowercase();
                    assert!(
                        (msg_lower.contains("canonical path")
                            || msg_lower.contains("binary")
                            || msg_lower.contains("modified"))
                            && (msg_lower.contains("hash") || msg_lower.contains("sha256")),
                        "Error should indicate path or hash mismatch. Got: {}",
                        error.message
                    );
                }
                Ok(_) => {
                    panic!("Symlink TOCTOU attack should be detected - target was changed")
                }
            }
        }
    }

    #[tokio::test]
    async fn test_full_approval_lifecycle() {
        // Integration test: approve → execute → modify config → reject → re-approve → execute
        // This validates the complete approval workflow end-to-end
        use std::io::Write;

        // Setup isolated XDG config to avoid cross-test contamination
        let xdg_dir = setup_isolated_xdg_config();

        let temp_dir = TempDir::new().unwrap();
        let config_dir = temp_dir.path().join(".config");
        std::fs::create_dir(&config_dir).unwrap();

        let script_path = temp_dir.path().join("test.sh");
        let mut script = std::fs::File::create(&script_path).unwrap();
        writeln!(script, "#!/usr/bin/env bash").unwrap();
        writeln!(script, "echo 'test'").unwrap();
        drop(script);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).unwrap();
        }

        let config_content_v1 = format!(
            r#"
[commands]
test = ["{}"]
"#,
            script_path.display()
        );

        std::fs::write(config_dir.join("tools.toml"), &config_content_v1).unwrap();

        // Step 1: Initial approval
        approve_project_config(temp_dir.path(), &config_content_v1).await;

        // Step 2: Execute command - should succeed
        let args = RunArgs {
            project_dir: temp_dir.path().to_path_buf(),
        };

        let result = ToolRunner::run_command(ProjectCommand::Test, args.clone()).await;
        if let Err(ref e) = result {
            panic!("Initial execution should succeed, but got error: {:?}", e);
        }
        assert!(result.is_ok());

        // Step 3: Modify tools.toml
        let config_content_v2 = format!(
            r#"
[commands]
test = ["{}"]
build = ["echo", "build"]
"#,
            script_path.display()
        );

        std::fs::write(config_dir.join("tools.toml"), &config_content_v2).unwrap();

        // Step 4: Attempt execution - should fail due to config hash mismatch
        let result = ToolRunner::run_command(ProjectCommand::Test, args.clone()).await;
        match result {
            Err(error) => {
                assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
                let msg_lower = error.message.to_lowercase();
                assert!(
                    msg_lower.contains("modified")
                        || msg_lower.contains("hash")
                        || msg_lower.contains("sha256"),
                    "Error should indicate config modification. Got: {}",
                    error.message
                );
            }
            Ok(_) => panic!("Execution should fail after config modification"),
        }

        // Step 5: Re-approve with new config
        approve_project_config(temp_dir.path(), &config_content_v2).await;

        // Step 6: Execute command again - should succeed with new approval
        let result = ToolRunner::run_command(ProjectCommand::Test, args).await;
        assert!(result.is_ok(), "Execution should succeed after re-approval");

        // Keep xdg_dir alive
        drop(xdg_dir);
    }

    #[tokio::test]
    async fn test_approval_lifecycle_with_binary_modification() {
        // Integration test: approve → modify binary → reject → re-approve → execute
        // Validates that binary hash verification works throughout the lifecycle
        use std::io::Write;

        // Setup isolated XDG config to avoid cross-test contamination
        let xdg_dir = setup_isolated_xdg_config();

        let temp_dir = TempDir::new().unwrap();
        let config_dir = temp_dir.path().join(".config");
        std::fs::create_dir(&config_dir).unwrap();

        let script_path = temp_dir.path().join("build.sh");
        let mut script = std::fs::File::create(&script_path).unwrap();
        writeln!(script, "#!/usr/bin/env bash").unwrap();
        writeln!(script, "echo 'version 1'").unwrap();
        drop(script);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).unwrap();
        }

        let config_content = format!(
            r#"
[commands]
build = ["{}"]
"#,
            script_path.display()
        );

        std::fs::write(config_dir.join("tools.toml"), &config_content).unwrap();

        // Approve version 1
        approve_project_config(temp_dir.path(), &config_content).await;

        // Execute - should succeed
        let args = RunArgs {
            project_dir: temp_dir.path().to_path_buf(),
        };
        let result = ToolRunner::run_command(ProjectCommand::Build, args.clone()).await;
        if let Err(ref e) = result {
            panic!("Initial execution should succeed, but got error: {:?}", e);
        }
        assert!(result.is_ok());

        // Modify the binary
        let mut script = std::fs::File::create(&script_path).unwrap();
        writeln!(script, "#!/usr/bin/env bash").unwrap();
        writeln!(script, "echo 'version 2 - modified'").unwrap();
        drop(script);

        // Attempt execution - should fail due to binary hash mismatch
        let result = ToolRunner::run_command(ProjectCommand::Build, args.clone()).await;
        match result {
            Err(error) => {
                assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
                let msg_lower = error.message.to_lowercase();
                assert!(
                    (msg_lower.contains("binary") || msg_lower.contains("modified"))
                        && (msg_lower.contains("hash") || msg_lower.contains("sha256")),
                    "Error should indicate binary modification. Got: {}",
                    error.message
                );
            }
            Ok(_) => panic!("Execution should fail after binary modification"),
        }

        // Re-approve with modified binary
        approve_project_config(temp_dir.path(), &config_content).await;

        // Execute again - should succeed with new approval
        let result = ToolRunner::run_command(ProjectCommand::Build, args).await;
        assert!(result.is_ok(), "Execution should succeed after re-approval");

        // Keep xdg_dir alive
        drop(xdg_dir);
    }
}
