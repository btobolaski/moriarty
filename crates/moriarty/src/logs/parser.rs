use std::collections::HashMap;
#[cfg(test)]
use std::path::Path;

use chrono::{DateTime, Utc};
#[cfg(test)]
use miette::IntoDiagnostic;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use tokio::fs::read_to_string;
use uuid::Uuid;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct QueueOperation {
    pub operation: String,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
    pub session_id: String,
}

/// Progress events from Claude Code 2.1+.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ProgressLogLine {
    pub parent_uuid: Uuid,
    pub is_sidechain: bool,
    pub user_type: String,
    pub cwd: String,
    pub session_id: Uuid,
    pub version: String,
    pub git_branch: String,
    /// Identifier for the agent/task that created this message. None for main conversation messages.
    pub agent_id: Option<String>,
    /// Session slug identifier (e.g., "noble-floating-lemon"). Added in Claude Code 2.0.51.
    pub slug: Option<String>,
    pub data: ProgressData,
    #[serde(rename = "toolUseID")]
    pub tool_use_id: String,
    #[serde(rename = "parentToolUseID")]
    pub parent_tool_use_id: String,
    pub uuid: Uuid,
    pub timestamp: DateTime<Utc>,
    /// Entry point that started the session (e.g., "cli"). Added in Claude Code 2.1.104+.
    pub entrypoint: Option<String>,
}

/// Progress event data types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ProgressData {
    HookProgress(HookProgressData),
    McpProgress(McpProgressData),
    BashProgress(BashProgressData),
    AgentProgress(Box<AgentProgressData>),
    WaitingForTask(WaitingForTaskData),
    QueryUpdate(QueryUpdateData),
    SearchResultsReceived(SearchResultsReceivedData),
}

/// Hook progress event data for tracking hook execution.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct HookProgressData {
    pub hook_event: String,
    pub hook_name: String,
    pub command: String,
}

/// MCP progress event data for tracking MCP tool execution.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct McpProgressData {
    pub status: String,
    pub server_name: String,
    pub tool_name: String,
    /// Elapsed time in milliseconds (present when status is "completed").
    pub elapsed_time_ms: Option<u64>,
}

/// Bash command progress event data for tracking long-running shell commands.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct BashProgressData {
    pub output: String,
    pub full_output: String,
    pub elapsed_time_seconds: u64,
    pub total_lines: usize,
}

/// Agent progress event data for tracking sub-agent execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct AgentProgressData {
    pub message: AgentProgressMessage,
    pub normalized_messages: Option<Vec<AgentProgressMessage>>,
    pub prompt: String,
    pub agent_id: String,
    /// Agent ID to resume from a previous execution.
    pub resume: Option<String>,
}

/// Message wrapper used in agent progress events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
pub enum AgentProgressMessage {
    #[serde(rename = "user")]
    #[serde(rename_all = "camelCase")]
    User {
        message: LogMessage,
        uuid: Uuid,
        timestamp: DateTime<Utc>,
        /// Tool use result data, present when this is a tool result message.
        tool_use_result: Option<serde_json::Value>,
    },
    #[serde(rename = "assistant")]
    #[serde(rename_all = "camelCase")]
    Assistant {
        message: Box<AssistantLogMessage>,
        request_id: String,
        uuid: Uuid,
        timestamp: DateTime<Utc>,
    },
    /// Progress message nested within agent progress.
    #[serde(rename = "progress")]
    #[serde(rename_all = "camelCase")]
    Progress {
        data: NestedProgressData,
        #[serde(rename = "toolUseID")]
        tool_use_id: String,
        #[serde(rename = "parentToolUseID")]
        parent_tool_use_id: String,
        uuid: Uuid,
        timestamp: DateTime<Utc>,
    },
    /// Attachment message containing hook execution results or failure details.
    /// Uses serde_json::Value because attachment schemas vary by hook type.
    #[serde(rename = "attachment")]
    #[serde(rename_all = "camelCase")]
    Attachment {
        attachment: serde_json::Value,
        uuid: Uuid,
        timestamp: DateTime<Utc>,
    },
}

/// Nested progress data within agent progress normalizedMessages.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum NestedProgressData {
    HookProgress(HookProgressData),
    McpProgress(McpProgressData),
    BashProgress(BashProgressData),
    QueryUpdate(QueryUpdateData),
    SearchResultsReceived(SearchResultsReceivedData),
}

/// Waiting for task progress data for tracking background task status.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct WaitingForTaskData {
    pub task_description: String,
    pub task_type: String,
}

/// Emitted by Claude Code during web searches when the search query is refined or updated.
/// Allows tracking query evolution during agent research phases.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct QueryUpdateData {
    pub query: String,
}

/// Emitted by Claude Code when web search results are received from the search backend.
/// Includes result count to track search effectiveness and query quality.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SearchResultsReceivedData {
    pub result_count: u32,
    pub query: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
pub enum LogLine {
    #[serde(rename = "user")]
    User(UserLogLine),
    #[serde(rename = "assistant")]
    Assistant(Box<AssistantLogLine>),
    #[serde(rename = "file-history-snapshot")]
    FileHistorySnapshot(FileHistorySnapshot),
    #[serde(rename = "summary")]
    Summary(Summary),
    #[serde(rename = "system")]
    System(SystemLogLine),
    #[serde(rename = "queue-operation")]
    QueueOperation(QueueOperation),
    #[serde(rename = "progress")]
    Progress(ProgressLogLine),
    #[serde(rename = "custom-title")]
    CustomTitle(CustomTitle),
    #[serde(rename = "agent-name")]
    AgentName(AgentName),
    #[serde(rename = "last-prompt")]
    LastPrompt(LastPrompt),
    #[serde(rename = "permission-mode")]
    PermissionModeChange(PermissionModeChange),
    #[serde(rename = "attachment")]
    Attachment(Box<AttachmentLogLine>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CustomTitle {
    pub custom_title: String,
    pub session_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct AgentName {
    pub agent_name: String,
    pub session_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct LastPrompt {
    pub last_prompt: String,
    pub session_id: Uuid,
}

/// Permission mode change event. Added in Claude Code 2.1.104+.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PermissionModeChange {
    pub permission_mode: PermissionMode,
    pub session_id: Uuid,
}

/// Attachment log line for deferred tools, hooks, and other metadata. Added in Claude Code 2.1.104+.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct AttachmentLogLine {
    pub parent_uuid: Option<Uuid>,
    pub is_sidechain: bool,
    pub attachment: AttachmentData,
    pub uuid: Uuid,
    pub timestamp: DateTime<Utc>,
    pub user_type: String,
    pub entrypoint: Option<String>,
    pub cwd: String,
    pub session_id: Uuid,
    pub version: String,
    pub git_branch: String,
    pub slug: Option<String>,
}

/// Attachment payload types. Added in Claude Code 2.1.104+.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum AttachmentData {
    AutoMode(AutoMode),
    AutoModeExit(AutoModeExit),
    CommandPermissions(CommandPermissions),
    DeferredToolsDelta(DeferredToolsDelta),
    EditedTextFile(EditedTextFile),
    HookBlockingError(HookBlockingError),
    HookCancelled(HookCancelled),
    HookNonBlockingError(HookNonBlockingError),
    HookSuccess(HookSuccess),
    HookSystemMessage(HookSystemMessage),
    McpInstructionsDelta(McpInstructionsDelta),
    PlanMode(PlanModeAttachment),
    PlanModeExit(PlanModeExitAttachment),
    PlanModeReentry(PlanModeReentryAttachment),
    QueuedCommand(QueuedCommand),
    SkillListing(SkillListing),
    TaskReminder(TaskReminder),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct AutoMode {
    pub reminder_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutoModeExit {}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CommandPermissions {
    pub allowed_tools: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct DeferredToolsDelta {
    pub added_names: Vec<String>,
    pub added_lines: Vec<String>,
    pub removed_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct EditedTextFile {
    pub filename: String,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct HookBlockingError {
    pub hook_name: String,
    #[serde(rename = "toolUseID")]
    pub tool_use_id: String,
    pub hook_event: String,
    pub blocking_error: BlockingErrorDetails,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct BlockingErrorDetails {
    pub blocking_error: String,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct HookCancelled {
    pub hook_name: String,
    #[serde(rename = "toolUseID")]
    pub tool_use_id: String,
    pub hook_event: String,
    pub command: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct HookNonBlockingError {
    pub hook_name: String,
    #[serde(rename = "toolUseID")]
    pub tool_use_id: String,
    pub hook_event: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub command: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct HookSuccess {
    pub hook_name: String,
    #[serde(rename = "toolUseID")]
    pub tool_use_id: String,
    pub hook_event: String,
    pub content: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub command: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct HookSystemMessage {
    pub content: String,
    pub hook_name: String,
    #[serde(rename = "toolUseID")]
    pub tool_use_id: String,
    pub hook_event: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct McpInstructionsDelta {
    pub added_names: Vec<String>,
    pub added_blocks: Vec<String>,
    pub removed_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PlanModeAttachment {
    pub reminder_type: String,
    pub is_sub_agent: bool,
    pub plan_file_path: String,
    pub plan_exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PlanModeExitAttachment {
    pub plan_file_path: String,
    pub plan_exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PlanModeReentryAttachment {
    pub plan_file_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct QueuedCommand {
    pub prompt: String,
    pub command_mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SkillListing {
    pub content: String,
    pub skill_count: u32,
    pub is_initial: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct TaskReminder {
    pub content: Vec<TaskReminderItem>,
    pub item_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct TaskReminderItem {
    pub id: String,
    pub subject: String,
    pub description: String,
    pub active_form: Option<String>,
    pub status: String,
    pub blocks: Vec<String>,
    pub blocked_by: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "subtype")]
#[serde(rename_all = "snake_case")]
pub enum SystemLogLine {
    Error(SystemLogError),
    CompactBoundary(CompactBoundary),
    MicrocompactBoundary(MicrocompactBoundary),
    Informational(SystemLogInformational),
    ApiError(SystemLogError),
    LocalCommand(LocalCommandLog),
    StopHookSummary(StopHookSummary),
    TurnDuration(TurnDuration),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct LocalCommandLog {
    pub parent_uuid: Option<Uuid>,
    pub is_sidechain: bool,
    pub user_type: String,
    pub cwd: String,
    pub session_id: Uuid,
    pub version: String,
    pub git_branch: String,
    /// Session slug identifier (e.g., "noble-floating-lemon"). Added in Claude Code 2.0.51.
    pub slug: Option<String>,
    pub content: String,
    pub level: String,
    pub timestamp: DateTime<Utc>,
    pub uuid: Uuid,
    pub is_meta: bool,
    /// Entry point that started the session (e.g., "cli"). Added in Claude Code 2.1.104+.
    pub entrypoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct StopHookSummary {
    pub parent_uuid: Uuid,
    pub is_sidechain: bool,
    pub user_type: String,
    pub cwd: String,
    pub session_id: Uuid,
    pub version: String,
    pub git_branch: String,
    /// Session slug identifier (e.g., "noble-floating-lemon"). Added in Claude Code 2.0.51.
    pub slug: Option<String>,
    pub hook_count: usize,
    pub hook_infos: Vec<HookInfo>,
    pub hook_errors: Vec<HookError>,
    pub prevented_continuation: bool,
    pub stop_reason: String,
    pub has_output: bool,
    pub level: String,
    pub timestamp: DateTime<Utc>,
    pub uuid: Uuid,
    #[serde(rename = "toolUseID")]
    pub tool_use_id: String,
    /// Entry point that started the session (e.g., "cli"). Added in Claude Code 2.1.104+.
    pub entrypoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct HookInfo {
    pub command: String,
    pub duration_ms: Option<u64>,
}

/// Hook errors from Claude Code, supporting both legacy and current formats to maintain
/// backward compatibility when parsing logs from different Claude Code versions. Uses untagged
/// serde to automatically deserialize either format without requiring version detection logic.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HookError {
    /// String-only format introduced in Claude Code 2.0.47 to simplify error reporting
    /// when command and exit_code details aren't available or relevant
    String(String),
    /// Structured format used in earlier versions to provide additional debugging context
    /// about which hook failed and how
    Structured(HookErrorDetails),
}

// The accessors below are only exercised by `logs::parser::tests`; production
// code matches the enum variants directly. Gate on `cfg(test)` to avoid the
// `dead_code` warning in release builds rather than pretending a binary crate
// has downstream consumers.
#[cfg(test)]
impl HookError {
    pub fn message(&self) -> &str {
        match self {
            HookError::String(s) => s,
            HookError::Structured(details) => &details.message,
        }
    }

    pub fn command(&self) -> Option<&str> {
        match self {
            HookError::String(_) => None,
            HookError::Structured(details) => details.command.as_deref(),
        }
    }

    pub fn exit_code(&self) -> Option<i32> {
        match self {
            HookError::String(_) => None,
            HookError::Structured(details) => details.exit_code,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct HookErrorDetails {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SystemLogError {
    pub parent_uuid: Uuid,
    pub is_sidechain: bool,
    pub user_type: String,
    pub cwd: String,
    pub session_id: String,
    pub version: String,
    pub git_branch: String,
    /// Session slug identifier (e.g., "noble-floating-lemon"). Added in Claude Code 2.0.51.
    pub slug: Option<String>,
    pub level: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<serde_json::Value>,
    pub error: SystemLogErrorError,
    pub retry_in_ms: f64,
    pub retry_attempt: usize,
    pub max_retries: usize,
    pub timestamp: DateTime<Utc>,
    pub uuid: Uuid,
    /// Entry point that started the session (e.g., "cli"). Added in Claude Code 2.1.104+.
    pub entrypoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SystemLogErrorError {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
    #[serde(rename = "requestID")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CompactBoundary {
    pub parent_uuid: Option<Uuid>,
    pub logical_parent_uuid: Uuid,
    pub is_sidechain: bool,
    pub user_type: String,
    pub cwd: String,
    pub session_id: Uuid,
    pub version: String,
    pub git_branch: String,
    /// Session slug identifier (e.g., "noble-floating-lemon"). Added in Claude Code 2.0.51.
    pub slug: Option<String>,
    pub content: String,
    pub is_meta: bool,
    pub timestamp: DateTime<Utc>,
    pub uuid: Uuid,
    pub level: String,
    pub compact_metadata: CompactMetadata,
    /// Entry point that started the session (e.g., "cli"). Added in Claude Code 2.1.104+.
    pub entrypoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SystemLogInformational {
    pub parent_uuid: Uuid,
    pub is_sidechain: bool,
    pub git_branch: Option<String>,
    /// Session slug identifier (e.g., "noble-floating-lemon"). Added in Claude Code 2.0.51.
    pub slug: Option<String>,
    pub user_type: String,
    pub cwd: String,
    pub session_id: Uuid,
    pub version: String,
    pub content: String,
    pub is_meta: bool,
    pub timestamp: DateTime<Utc>,
    pub uuid: Uuid,
    pub level: String,
    /// Entry point that started the session (e.g., "cli"). Added in Claude Code 2.1.104+.
    pub entrypoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CompactMetadata {
    pub trigger: String,
    pub pre_tokens: usize,
}

/// Microcompact boundary event from Claude Code 2.1.12+. Unlike full compaction, microcompaction
/// selectively removes tool use content to reduce context size while preserving conversation flow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct MicrocompactBoundary {
    pub parent_uuid: Uuid,
    pub is_sidechain: bool,
    pub user_type: String,
    pub cwd: String,
    pub session_id: Uuid,
    pub version: String,
    pub git_branch: String,
    /// Session slug identifier (e.g., "noble-floating-lemon"). Added in Claude Code 2.0.51.
    pub slug: Option<String>,
    pub content: String,
    pub is_meta: bool,
    pub timestamp: DateTime<Utc>,
    pub uuid: Uuid,
    pub level: String,
    pub microcompact_metadata: MicrocompactMetadata,
    /// Entry point that started the session (e.g., "cli"). Added in Claude Code 2.1.104+.
    pub entrypoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct MicrocompactMetadata {
    pub trigger: String,
    pub pre_tokens: usize,
    pub tokens_saved: usize,
    pub compacted_tool_ids: Vec<String>,
    #[serde(rename = "clearedAttachmentUUIDs")]
    pub cleared_attachment_uuids: Vec<Uuid>,
}

/// Duration of a single turn (user message → assistant response cycle).
/// Added in Claude Code 2.0.51+ for performance tracking.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct TurnDuration {
    pub parent_uuid: Uuid,
    pub is_sidechain: bool,
    pub user_type: String,
    pub cwd: String,
    pub session_id: Uuid,
    pub version: String,
    pub git_branch: String,
    /// Session slug identifier (e.g., "noble-floating-lemon"). Added in Claude Code 2.0.51.
    pub slug: Option<String>,
    pub duration_ms: u64,
    pub timestamp: DateTime<Utc>,
    pub uuid: Uuid,
    pub is_meta: bool,
    /// Entry point that started the session (e.g., "cli"). Added in Claude Code 2.1.104+.
    pub entrypoint: Option<String>,
    /// Number of messages in the turn. Added in Claude Code 2.1.104+.
    pub message_count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct FileHistorySnapshot {
    pub message_id: Uuid,
    pub snapshot: FileHistorySnapshotSnapshot,
    pub is_snapshot_update: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub summary: String,
    pub leaf_uuid: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct FileHistorySnapshotSnapshot {
    pub message_id: Uuid,
    pub tracked_file_backups: HashMap<String, serde_json::Value>,
    pub timestamp: DateTime<Utc>,
}

/// Task execution state tracked in Claude Code's todo system. These states enable Claude Code
/// to persist task progress across session restarts and provide status visibility in the UI.
/// The linear progression (Pending → InProgress → Completed) ensures only one task is active
/// at a time, preventing execution chaos from parallel task attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

/// Task item from Claude Code's todo tracking system. Todos are persisted in log files to enable
/// session recovery after crashes or disconnections - Claude Code can resume incomplete work by
/// reading the last todo state from logs. Storing both imperative and continuous forms avoids
/// complex string transformations and ensures consistent UI messaging across different task states.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct Todo {
    /// Task description in imperative form (e.g., "Run tests")
    pub content: String,
    /// Task status
    pub status: TodoStatus,
    /// Present continuous form for display during execution (e.g., "Running tests")
    pub active_form: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct UserLogLine {
    pub parent_uuid: Option<Uuid>,
    pub is_sidechain: bool,
    /// Identifier for the agent/task that created this message. None for main conversation messages.
    pub agent_id: Option<String>,
    pub user_type: String,
    pub cwd: String,
    pub session_id: Uuid,
    pub version: String,
    pub git_branch: String,
    /// Session slug identifier (e.g., "noble-floating-lemon"). Added in Claude Code 2.0.51.
    pub slug: Option<String>,
    pub message: LogMessage,
    pub is_meta: Option<bool>,
    pub uuid: Uuid,
    pub timestamp: DateTime<Utc>,
    pub tool_use_result: Option<ToolUseResult>,
    pub thinking_metadata: Option<ThinkingMetadata>,
    pub is_visible_in_transcript_only: Option<bool>,
    pub is_compact_summary: Option<bool>,
    /// Todo list from Claude Code 2.0.47+. Contains tasks being tracked in the conversation.
    pub todos: Option<Vec<Todo>>,
    /// UUID of the assistant message that triggered the tool use. Added in Claude Code 2.0.51+.
    #[serde(rename = "sourceToolAssistantUUID")]
    pub source_tool_assistant_uuid: Option<Uuid>,
    /// Prompt identifier for tracking prompt lineage. Added in Claude Code 2.1.77+.
    pub prompt_id: Option<Uuid>,
    /// Current permission mode. Added in Claude Code 2.1.77+.
    pub permission_mode: Option<PermissionMode>,
    /// Plan content when in plan mode. Added in Claude Code 2.1.77+.
    pub plan_content: Option<String>,
    /// Entry point that started the session (e.g., "cli"). Added in Claude Code 2.1.104+.
    pub entrypoint: Option<String>,
    /// Origin of the message (e.g., task-notification). Added in Claude Code 2.1.104+.
    pub origin: Option<MessageOrigin>,
}

/// Origin metadata for a message. Added in Claude Code 2.1.104+.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct MessageOrigin {
    pub kind: String,
}

/// Permission mode for the conversation. Added in Claude Code 2.1.77+.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    Default,
    Plan,
    AcceptEdits,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThinkingMetadata {
    pub level: String,
    pub disabled: bool,
    pub triggers: Vec<ThinkingTrigger>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThinkingTrigger {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolUseResult {
    String(String),
    Map(HashMap<String, serde_json::Value>),
    Vec(Vec<ToolUseResult>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct LogMessage {
    pub role: String,
    pub content: LogMessageContent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
pub enum LogMessageContent {
    String(String),
    Vec(Vec<LogMessageTaggedContent>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum LogMessageTaggedContent {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
        signature: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: HashMap<String, serde_json::Value>,
        /// Caller information for tool invocation tracking. Added in Claude Code 2.1.12+.
        caller: Option<ToolUseCaller>,
    },
    ToolResult(ToolResult),
    Document {
        source: DocumentSource,
    },
    ToolReference {
        tool_name: String,
    },
}

/// Caller information for tool use tracking. Added in Claude Code 2.1.12+.
/// Indicates how the tool was invoked (directly by the agent or through another mechanism).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolUseCaller {
    pub r#type: String,
}

/// Represents a document attached to a message (e.g., PDF, image, text file)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentSource {
    /// Type of data encoding (currently only "base64" is used by Claude Code)
    pub r#type: String,
    /// MIME type of the document (e.g., "application/pdf", "image/png", "text/plain")
    pub media_type: String,
    /// The document data as a string. When `type` is "base64", this contains base64-encoded
    /// binary data. The parser accepts any string without validation.
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
pub enum ToolResult {
    Current {
        content: LogMessageContent,
        is_error: Option<bool>,
        tool_use_id: String,
    },
    V1 {
        tool_use_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct AssistantLogLine {
    pub parent_uuid: Option<Uuid>,
    pub is_sidechain: bool,
    /// Identifier for the agent/task that created this message. None for main conversation messages.
    pub agent_id: Option<String>,
    pub user_type: String,
    pub cwd: String,
    pub session_id: String,
    pub version: String,
    pub git_branch: String,
    /// Session slug identifier (e.g., "noble-floating-lemon"). Added in Claude Code 2.0.51.
    pub slug: Option<String>,
    pub message: AssistantLogMessage,
    pub request_id: Option<String>,
    pub uuid: Uuid,
    pub timestamp: DateTime<Utc>,
    pub is_api_error_message: Option<bool>,
    /// Error type when this is an API error message (e.g., "invalid_request").
    pub error: Option<String>,
    /// Entry point that started the session (e.g., "cli"). Added in Claude Code 2.1.104+.
    pub entrypoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantLogMessage {
    pub id: String,
    pub r#type: String,
    pub role: String,
    pub model: String,
    pub container: Option<String>,
    pub content: LogMessageContent,
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
    pub stop_details: Option<StopDetails>,
    pub usage: AssistantUsage,
    pub context_management: Option<serde_json::Value>,
}

/// Stop details from the Anthropic Messages API. Added in Claude Code 2.1.77+.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StopDetails {
    pub r#type: StopType,
    pub stop_sequence: Option<String>,
}

/// The reason the model stopped generating tokens.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub enum StopType {
    EndTurn,
    StopSequence,
    MaxTokens,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantUsage {
    pub input_tokens: usize,
    pub cache_creation_input_tokens: usize,
    pub cache_read_input_tokens: usize,
    pub cache_creation: AssistantCacheCreation,
    pub output_tokens: usize,
    pub service_tier: Option<String>,
    pub server_tool_use: Option<ServerToolUse>,
    /// Geographic region where inference was performed. Added in Claude Code 2.1.12+.
    pub inference_geo: Option<String>,
    /// Iteration data for responses. Added in Claude Code 2.1.77+.
    pub iterations: Option<Vec<Iteration>>,
    /// Speed setting for the response. Added in Claude Code 2.1.77+.
    pub speed: Option<Speed>,
}

/// Iteration data from Claude Code's response. Added in Claude Code 2.1.77+ as empty objects,
/// populated with token counts in 2.1.104+.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Iteration {
    pub input_tokens: Option<usize>,
    pub output_tokens: Option<usize>,
    pub cache_read_input_tokens: Option<usize>,
    pub cache_creation_input_tokens: Option<usize>,
    pub cache_creation: Option<AssistantCacheCreation>,
    pub r#type: Option<String>,
}

/// Speed setting for inference. Added in Claude Code 2.1.77+.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub enum Speed {
    Standard,
    Fast,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerToolUse {
    pub web_search_requests: usize,
    pub web_fetch_requests: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantCacheCreation {
    pub ephemeral_5m_input_tokens: usize,
    pub ephemeral_1h_input_tokens: usize,
}

#[cfg(test)]
pub async fn read_file(file: impl AsRef<Path>) -> miette::Result<Vec<LogLine>> {
    let string_contents = read_to_string(file).await.into_diagnostic()?;

    // Parse lines sequentially (each file is processed in parallel via rayon at a higher level)
    let log_lines: Result<Vec<LogLine>, _> = string_contents
        .split('\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            serde_json::from_str::<LogLine>(line).inspect_err(|_| {
                println!("{line}");
            })
        })
        .collect();

    log_lines.into_diagnostic()
}
