use std::{collections::HashMap, path::Path};

use chrono::{DateTime, Utc};
use miette::IntoDiagnostic;
use serde::{Deserialize, Serialize};
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
    pub normalized_messages: Vec<AgentProgressMessage>,
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
    Assistant(AssistantLogLine),
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
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct HookInfo {
    pub command: String,
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
    pub usage: AssistantUsage,
    pub context_management: Option<serde_json::Value>,
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
