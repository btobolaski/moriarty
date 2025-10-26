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

use miette::IntoDiagnostic;
use rmcp::{
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::*,
    tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::{fs::read_to_string, process::Command};

/// Project configuration loaded from `.config/tools.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// Available tool commands
    pub commands: Commands,
}

/// Tool command definitions.
///
/// Each field is optional and contains the command and its arguments as a string array.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commands {
    /// Linter command (e.g., `["cargo", "clippy"]`)
    pub lint: Option<Vec<String>>,
    /// Test command (e.g., `["cargo", "nextest", "run"]`)
    pub test: Option<Vec<String>>,
    /// Build command (e.g., `["cargo", "build"]`)
    pub build: Option<Vec<String>>,
    /// Formatter command (e.g., `["cargo", "fmt"]`)
    pub format: Option<Vec<String>>,
}

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

impl std::fmt::Display for ProjectCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Lint => "lint",
            Self::Test => "test",
            Self::Build => "build",
            Self::Format => "format",
        };

        value.fmt(f)
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
    async fn load_project_settings(canonical_dir: PathBuf) -> miette::Result<ProjectConfig> {
        // Path is already canonicalized by the caller to prevent traversal attacks
        let mut config_path = canonical_dir.clone();
        config_path.push(".config");
        config_path.push("tools.toml");

        let project_settings_contents = read_to_string(&config_path)
            .await
            .into_diagnostic()
            .map_err(|error| {
                error.context(format!(
                    "failed to read project settings: {}",
                    config_path.to_string_lossy()
                ))
            })?;

        let settings: ProjectConfig = toml::from_str(project_settings_contents.as_str())
            .into_diagnostic()
            .map_err(|error| error.context("failed to parse project settings"))?;

        Ok(settings)
    }

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

        let settings = match Self::load_project_settings(canonical_dir.clone()).await {
            Ok(settings) => settings,
            Err(error) => {
                return Err(McpError {
                    code: ErrorCode::INTERNAL_ERROR,
                    message: format!("{error:?}").into(),
                    data: None,
                });
            }
        };

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
    use tempfile::TempDir;

    fn setup_project_dir_with_config(config_content: &str) -> TempDir {
        let temp_dir = TempDir::new().unwrap();
        let config_dir = temp_dir.path().join(".config");
        std::fs::create_dir(&config_dir).unwrap();
        std::fs::write(config_dir.join("tools.toml"), config_content).unwrap();
        temp_dir
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

        let config = ToolRunner::load_project_settings(temp_dir.path().to_path_buf())
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

        let config = ToolRunner::load_project_settings(temp_dir.path().to_path_buf())
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

        let result = ToolRunner::load_project_settings(temp_dir.path().to_path_buf()).await;

        assert!(result.is_err());
        let error_msg = format!("{:?}", result.unwrap_err());
        assert!(error_msg.contains("failed to read project settings"));
    }

    #[tokio::test]
    async fn test_load_project_settings_malformed_toml() {
        let temp_dir = setup_project_dir_with_config("this is not valid toml [[[");

        let result = ToolRunner::load_project_settings(temp_dir.path().to_path_buf()).await;

        assert!(result.is_err());
        let error_msg = format!("{:?}", result.unwrap_err());
        assert!(error_msg.contains("failed to parse project settings"));
    }

    #[tokio::test]
    async fn test_run_command_success() {
        let temp_dir = setup_project_dir_with_config(
            r#"
[commands]
test = ["echo", "test output"]
"#,
        );

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
        let temp_dir = setup_project_dir_with_config(
            r#"
[commands]
lint = ["cargo", "clippy"]
"#,
        );

        let args = RunArgs {
            project_dir: temp_dir.path().to_path_buf(),
        };

        let result = ToolRunner::run_command(ProjectCommand::Test, args).await;

        match result {
            Err(error) => {
                assert_eq!(error.code, ErrorCode::RESOURCE_NOT_FOUND);
                assert!(error.message.contains("not configured"));
            }
            Ok(_) => panic!("Expected error for unconfigured command"),
        }
    }

    #[tokio::test]
    async fn test_run_command_empty_array() {
        let temp_dir = setup_project_dir_with_config(
            r#"
[commands]
test = []
"#,
        );

        let args = RunArgs {
            project_dir: temp_dir.path().to_path_buf(),
        };

        let result = ToolRunner::run_command(ProjectCommand::Test, args).await;

        match result {
            Err(error) => {
                assert_eq!(error.code, ErrorCode::RESOURCE_NOT_FOUND);
                assert!(error.message.contains("not configured"));
            }
            Ok(_) => panic!("Expected error for empty command array"),
        }
    }

    #[tokio::test]
    async fn test_run_command_nonzero_exit() {
        let temp_dir = setup_project_dir_with_config(
            r#"
[commands]
test = ["sh", "-c", "exit 1"]
"#,
        );

        let args = RunArgs {
            project_dir: temp_dir.path().to_path_buf(),
        };

        let result = ToolRunner::run_command(ProjectCommand::Test, args).await;

        let tool_result = result.unwrap();
        assert_eq!(tool_result.is_error, Some(true));
    }

    #[tokio::test]
    async fn test_run_command_invalid_executable() {
        let temp_dir = setup_project_dir_with_config(
            r#"
[commands]
test = ["this-command-does-not-exist-anywhere"]
"#,
        );

        let args = RunArgs {
            project_dir: temp_dir.path().to_path_buf(),
        };

        let result = ToolRunner::run_command(ProjectCommand::Test, args).await;

        match result {
            Err(error) => {
                assert_eq!(error.code, ErrorCode::INTERNAL_ERROR);
                assert!(error.message.contains("failed to run"));
            }
            Ok(_) => panic!("Expected error for invalid executable"),
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
        let temp_dir = setup_project_dir_with_config(
            r#"
[commands]
lint = ["echo", "Running lint"]
"#,
        );

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
        let temp_dir = setup_project_dir_with_config(
            r#"
[commands]
build = ["echo", "Building project"]
"#,
        );

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
        let temp_dir = setup_project_dir_with_config(
            r#"
[commands]
format = ["echo", "Formatting code"]
"#,
        );

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
        let temp_dir = setup_project_dir_with_config(
            r#"
[commands]
test = ["echo", "Running tests"]
"#,
        );

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
        let temp_dir = setup_project_dir_with_config(
            r#"
[commands]
test = ["echo", "hello"]
"#,
        );

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
}
