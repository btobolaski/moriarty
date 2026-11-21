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
//! execute arbitrary **approved** commands from `.config/tools.toml`. `verify_and_load_project`
//! verifies the configured commands and checks against their approved argv, resolved paths, and
//! binary hashes before anything runs; `run_checks` applies the same identity checks to each
//! `[[check]]` through [`crate::checks`]. Approval gates *what* runs, but does not inspect command
//! semantics or sandbox execution. The security model assumes:
//!
//! - **Trusted configuration files**: Only use with project directories where you
//!   trust the contents of `.config/tools.toml`
//! - **No content validation or sandboxing**: Approved commands run as configured, without
//!   checking for dangerous patterns or operations
//! - **Full filesystem access**: Commands run with the same permissions as the MCP
//!   server process
//!
//! ## Approval binding (commands and checks alike)
//!
//! Project tools are not approved by hashing `tools.toml` wholesale (that fired on comments,
//! whitespace, reordering, and unrelated commands). Instead each approval binds **exactly what
//! executes**: the command's argv from `tools.toml`, the resolved binary paths, and the binary's
//! SHA-256, stored as a **match-any set of approved versions per command name**. Only three things
//! ever trigger re-approval — a new command/check name, an argument change, or a binary change —
//! so cosmetic config edits and deleted commands never do. Approvals are keyed by **repository
//! root** (shared across jj workspaces and git worktrees), while the configuration that runs is
//! loaded from the caller's **workspace root** (`detect_workspace_root`), so each worktree runs
//! its own tooling while identical content across worktrees needs no re-approval. Stored binary
//! paths are normalized relative to the workspace root when inside it, so a byte-identical
//! `./script.sh` in a second worktree matches the first worktree's approval.
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
//! 3. **Avoid shell execution**: The trusted process supervisor launches approved
//!    programs with literal arguments and no implicit shell, but the configuration
//!    itself can specify shell invocation:
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

use std::{borrow::Cow, future::Future, path::PathBuf, sync::Arc, time::Instant};

use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, Service,
    handler::server::{tool::ToolCallContext, wrapper::Parameters},
    model::*,
    service::{NotificationContext, RequestContext},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, to_value};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{
    MCP_LEGACY_PROTOCOL_VERSION, MCP_MODERN_PROTOCOL_VERSION, MCP_SUPPORTED_PROTOCOL_VERSIONS,
    default_project_dir, tasks::ConnectionTasks,
};
use crate::{
    checks::CheckRunOutcome,
    execution::{Scope, ScopedOutcome, TaskObservation},
    project_config::runner::verify_and_load_project,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunArgs {
    /// The project directory containing `.config/tools.toml`; defaults to the server's cwd.
    #[serde(default = "default_project_dir")]
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
#[derive(Default)]
struct ToolRunner {
    modern: bool,
}

// rmcp already provides a blanket Service impl for ServerHandler. Keep a separate
// entry point for selecting the capability view before delegating to that impl.
#[derive(Clone, Copy, Default)]
pub(super) struct ProjectTools;

impl Service<RoleServer> for ProjectTools {
    async fn handle_request(
        &self,
        request: ClientRequest,
        context: RequestContext<RoleServer>,
    ) -> Result<ServerResult, McpError> {
        // Use only request metadata; rmcp still validates it before consulting get_info().
        // Initialize always negotiates a legacy revision, even if its payload names the draft.
        let handler = ToolRunner {
            modern: !matches!(request, ClientRequest::InitializeRequest(_))
                && context.meta.protocol_version().as_ref() == Some(&MCP_MODERN_PROTOCOL_VERSION),
        };
        handler.handle_request(request, context).await
    }

    async fn handle_notification(
        &self,
        notification: ClientNotification,
        context: NotificationContext<RoleServer>,
    ) -> Result<(), McpError> {
        ToolRunner::default()
            .handle_notification(notification, context)
            .await
    }

    fn get_info(&self) -> ServerConfig {
        ServerHandler::get_info(&ToolRunner::default())
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(MCP_SUPPORTED_PROTOCOL_VERSIONS)
    }
}

#[derive(Clone)]
struct TaskAdmission {
    host: Arc<Scope>,
    request: CancellationToken,
    tasks: ConnectionTasks,
}

fn connection_tasks(context: &RequestContext<RoleServer>) -> Result<&ConnectionTasks, McpError> {
    context
        .extensions
        .get::<ConnectionTasks>()
        .ok_or_else(|| McpError::internal_error("Missing connection task store", None))
}

fn failed(error: McpError) -> TaskPayload {
    let error = wire_object(error).unwrap_or_else(|fallback| {
        // Keep the required JSON-RPC fields even if the SDK serializer fails.
        JsonObject::from_iter([
            ("code".into(), Value::from(fallback.code.0)),
            (
                "message".into(),
                Value::String(fallback.message.into_owned()),
            ),
        ])
    });
    TaskPayload::Failed { error }
}

fn wire_object(value: impl Serialize) -> Result<JsonObject, McpError> {
    match to_value(value) {
        Ok(Value::Object(object)) => Ok(object),
        _ => Err(McpError::internal_error("Cannot encode task outcome", None)),
    }
}

fn task_payload(
    outcome: ScopedOutcome<TaskObservation<Result<CallToolResult, McpError>>>,
) -> TaskPayload {
    if let Err(cleanup) = outcome.cleanup {
        let (diagnostic, data) = match outcome.value {
            TaskObservation::Returned(Ok(result)) => {
                // Checks can carry the initiating diagnostic in an error tool result. Keep
                // the bounded evidence as data rather than interpolating captures into a message.
                let data = if result.is_error == Some(true) {
                    wire_object(result).ok().map(|result| {
                        Value::Object(JsonObject::from_iter([(
                            "toolResult".into(),
                            Value::Object(result),
                        )]))
                    })
                } else {
                    None
                };
                (format!("Settlement: {cleanup:#}"), data)
            }
            TaskObservation::Returned(Err(primary)) => {
                (format!("{primary}\nSettlement: {cleanup:#}"), primary.data)
            }
            TaskObservation::Interrupted(primary) => {
                (format!("{primary:#}\nSettlement: {cleanup:#}"), None)
            }
        };
        return failed(McpError::internal_error(diagnostic, data));
    }
    match outcome.value {
        TaskObservation::Returned(Ok(result)) => match wire_object(result) {
            Ok(result) => TaskPayload::Completed { result },
            Err(error) => failed(error),
        },
        TaskObservation::Returned(Err(error)) => failed(error),
        TaskObservation::Interrupted(_) => TaskPayload::Cancelled,
    }
}

impl ToolRunner {
    async fn execute_tool(
        &self,
        extensions: Extensions,
        work: impl Future<Output = Result<CallToolResult, McpError>> + Send + 'static,
    ) -> Result<CallToolResponse, McpError> {
        let Some(admission) = extensions.get::<TaskAdmission>() else {
            return work.await.map(Into::into);
        };
        // Generating the ID can call into the OS; do it before taking the admission lock.
        let id = Uuid::new_v4().to_string();
        let cancellation = CancellationToken::new();
        let tasks = admission.tasks.clone();
        admission
            .host
            .start_task(&admission.request, cancellation.clone(), work, move || {
                let task = tasks
                    .lock()
                    .unwrap()
                    .register(id.clone(), cancellation, Instant::now());
                (CreateTaskResult::new(task).into(), move |outcome| {
                    let payload = task_payload(outcome);
                    tasks.lock().unwrap().finish(&id, payload, Instant::now());
                })
            })
            .map_err(|error| McpError::internal_error(error.to_string(), None))
    }

    async fn run_command(cmd: ProjectCommand, args: RunArgs) -> Result<CallToolResult, McpError> {
        let project = verify_and_load_project(args.project_dir)
            .await
            .map_err(|e| McpError {
                code: ErrorCode::INVALID_REQUEST,
                message: format!("{e:#}").into(),
                data: None,
            })?;

        // Run the specific command
        let command_name = cmd.as_str();
        let output = project.run_command(command_name).await.map_err(|e| {
            if e.to_string().contains("not configured") {
                McpError {
                    code: ErrorCode::RESOURCE_NOT_FOUND,
                    message: format!("The {} command was not configured for the project", cmd)
                        .into(),
                    data: None,
                }
            } else {
                McpError {
                    code: ErrorCode::INTERNAL_ERROR,
                    message: format!("{e:#}").into(),
                    data: None,
                }
            }
        })?;

        let content = vec![
            ContentBlock::text(format!("stdout: \n\n {}", output.stdout)),
            ContentBlock::text(format!("stderr: \n\n {}", output.stderr)),
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
                message: format!("{e:#}").into(),
                data: None,
            })?;

        // Gate and limit failures (Blocked) are surfaced as tool-result errors rather than
        // McpError so the agent sees the Stop hook's actionable reason text (e.g.
        // "Run: moriarty approve-project ..."); only an unexpected internal failure is an McpError.
        match outcome {
            CheckRunOutcome::NoChecks(note) => {
                Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                    "No checks were run: {note}"
                ))]))
            }
            CheckRunOutcome::Blocked(reason) => {
                Ok(CallToolResult::error(vec![ContentBlock::text(reason)]))
            }
            CheckRunOutcome::Ran { outputs, failures } => {
                let mut content: Vec<ContentBlock> =
                    outputs.into_iter().map(ContentBlock::text).collect();
                if failures.is_empty() {
                    Ok(CallToolResult::success(content))
                } else {
                    content.push(ContentBlock::text(format!(
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
        extensions: Extensions,
    ) -> Result<CallToolResponse, McpError> {
        self.execute_tool(extensions, Self::run_command(ProjectCommand::Lint, args))
            .await
    }
    #[tool(description = "Runs the projects configured build")]
    async fn run_build(
        &self,
        Parameters(args): Parameters<RunArgs>,
        extensions: Extensions,
    ) -> Result<CallToolResponse, McpError> {
        self.execute_tool(extensions, Self::run_command(ProjectCommand::Build, args))
            .await
    }
    #[tool(description = "Runs the projects configured formatter")]
    async fn run_formatter(
        &self,
        Parameters(args): Parameters<RunArgs>,
        extensions: Extensions,
    ) -> Result<CallToolResponse, McpError> {
        self.execute_tool(extensions, Self::run_command(ProjectCommand::Format, args))
            .await
    }
    #[tool(description = "Runs the projects configured test tool")]
    async fn run_tests(
        &self,
        Parameters(args): Parameters<RunArgs>,
        extensions: Extensions,
    ) -> Result<CallToolResponse, McpError> {
        self.execute_tool(extensions, Self::run_command(ProjectCommand::Test, args))
            .await
    }
    #[tool(description = "Runs the project's configured checks ([[checks]] in .config/tools.toml)")]
    async fn run_checks(
        &self,
        Parameters(args): Parameters<RunArgs>,
        extensions: Extensions,
    ) -> Result<CallToolResponse, McpError> {
        self.execute_tool(extensions, Self::run_checks_impl(args))
            .await
    }
}

#[tool_handler]
impl ServerHandler for ToolRunner {
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        mut context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        if self.modern
            && context
                .meta
                .client_capabilities()
                .is_some_and(|caps| caps.supports_tasks())
        {
            let host = context
                .extensions
                .get::<Arc<Scope>>()
                .cloned()
                .ok_or_else(|| {
                    McpError::internal_error("Task call has no host execution scope", None)
                })?;
            let tasks = connection_tasks(&context)?.clone();
            context.extensions.insert(TaskAdmission {
                host,
                request: context.ct.clone(),
                tasks,
            });
        }
        Self::tool_router()
            .call(ToolCallContext::new(self, request, context))
            .await
    }

    async fn get_task(
        &self,
        request: GetTaskParams,
        context: RequestContext<RoleServer>,
    ) -> Result<GetTaskResult, McpError> {
        let snapshot = connection_tasks(&context)?
            .lock()
            .unwrap()
            .get(&request.task_id, Instant::now())?;
        // rmcp needs an owned result; do not copy a terminal payload under the store lock.
        Ok(GetTaskResult::new(Arc::unwrap_or_clone(snapshot)))
    }

    async fn update_task(
        &self,
        request: UpdateTaskParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        connection_tasks(&context)?
            .lock()
            .unwrap()
            .acknowledge_update(&request.task_id, Instant::now())
    }

    async fn cancel_task(
        &self,
        request: CancelTaskParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        connection_tasks(&context)?
            .lock()
            .unwrap()
            .request_cancel(&request.task_id, Instant::now())
    }

    fn get_info(&self) -> ServerConfig {
        let capabilities = ServerCapabilities::builder().enable_tools();
        let capabilities = if self.modern {
            capabilities.enable_tasks().build()
        } else {
            capabilities.build()
        };
        ServerConfig::new(capabilities)
            .with_protocol_version(MCP_LEGACY_PROTOCOL_VERSION)
            .with_server_info(Implementation::new(
                env!("CARGO_CRATE_NAME"),
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "This server provides configured tooling from the project".to_string(),
            )
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(MCP_SUPPORTED_PROTOCOL_VERSIONS)
    }
}

#[cfg(test)]
mod tests;
