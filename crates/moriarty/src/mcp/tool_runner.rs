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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub commands: Commands,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commands {
    pub lint: Option<Vec<String>>,
    pub test: Option<Vec<String>>,
    pub build: Option<Vec<String>>,
    pub format: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunArgs {
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

#[derive(Clone)]
pub struct ToolRunner {
    tool_router: ToolRouter<Self>,
}

impl ToolRunner {
    async fn load_project_settings(mut project_dir: PathBuf) -> miette::Result<ProjectConfig> {
        project_dir.push(".config");
        project_dir.push("tools.toml");

        let project_settings_contents = read_to_string(&project_dir)
            .await
            .into_diagnostic()
            .map_err(|error| {
                error.context(format!(
                    "failed to read project settings: {}",
                    project_dir.to_string_lossy()
                ))
            })?;

        let settings: ProjectConfig = toml::from_str(project_settings_contents.as_str())
            .into_diagnostic()
            .map_err(|error| error.context("failed to parse project settings"))?;

        Ok(settings)
    }

    async fn run_command(cmd: ProjectCommand, args: RunArgs) -> Result<CallToolResult, McpError> {
        let settings = match Self::load_project_settings(args.project_dir.clone()).await {
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
            Some(mut command) if !command.is_empty() => {
                // Vec pop from the back
                command.reverse();

                let mut command_runner = Command::new(command.pop().unwrap());
                command_runner.current_dir(args.project_dir);

                while let Some(arg) = command.pop() {
                    command_runner.arg(arg);
                }

                match command_runner.output().await {
                    Ok(result) => {
                        let stderr = match str::from_utf8(result.stderr.as_ref()) {
                            Ok(stderr) => stderr.to_string(),
                            Err(error) => {
                                return Err(McpError {
                                    code: ErrorCode::INTERNAL_ERROR,
                                    message: format!("stderr invalid: {error:?}").into(),
                                    data: None,
                                });
                            }
                        };

                        let stdout = match str::from_utf8(result.stdout.as_ref()) {
                            Ok(stdout) => stdout.to_string(),
                            Err(error) => {
                                return Err(McpError {
                                    code: ErrorCode::INTERNAL_ERROR,
                                    message: format!("stdout invalid: {error:?}").into(),
                                    data: None,
                                })
                            }
                        };

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
