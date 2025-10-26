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
    /// The directory of the project
    pub project_dir: PathBuf,
    /// Any arguments to add to `git status`
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiffArgs {
    /// The directory of the project
    pub project_dir: PathBuf,
    /// Any arguments to add to `git diff`
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommandResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[derive(Debug, Clone)]
pub struct GitReadOnly {
    pub tool_router: ToolRouter<Self>,
    pub prompt_router: PromptRouter<Self>,
}

#[tool_router(router = tool_router)]
impl GitReadOnly {
    #[tool(description = "Runs `git status` with the provided args")]
    async fn status(
        &self,
        Parameters(args): Parameters<StatusArgs>,
    ) -> Result<Json<CommandResult>, McpError> {
        let StatusArgs { project_dir, args } = args;

        let mut cmd = Command::new("git");
        cmd.arg("status");
        cmd.current_dir(project_dir);

        for arg in args {
            cmd.arg(arg);
        }

        match cmd.output().await {
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

                Ok(Json(CommandResult {
                    exit_code: result.status.code().unwrap_or(-1),
                    stderr,
                    stdout,
                }))
            }
            Err(error) => Err(McpError {
                code: ErrorCode::INTERNAL_ERROR,
                message: format!("status failed: {error:?}").into(),
                data: None,
            }),
        }
    }

    #[tool(description = "Runs `git diff` with the provided args")]
    async fn diff(
        &self,
        Parameters(args): Parameters<DiffArgs>,
    ) -> Result<Json<CommandResult>, McpError> {
        let DiffArgs { project_dir, args } = args;

        let mut cmd = Command::new("git");
        cmd.arg("diff");
        cmd.current_dir(project_dir);

        for arg in args {
            cmd.arg(arg);
        }

        match cmd.output().await {
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

                Ok(Json(CommandResult {
                    exit_code: result.status.code().unwrap_or(-1),
                    stderr,
                    stdout,
                }))
            }
            Err(error) => Err(McpError {
                code: ErrorCode::INTERNAL_ERROR,
                message: format!("diff failed: {error:?}").into(),
                data: None,
            }),
        }
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
        Parameters(args): Parameters<StatusArgs>,
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
