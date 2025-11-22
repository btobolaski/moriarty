use std::{collections::HashMap, path::Path};

use chrono::{DateTime, Utc};
use miette::IntoDiagnostic;
use serde::{Deserialize, Serialize};
use tokio::fs::read_to_string;
use uuid::Uuid;

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "subtype")]
#[serde(rename_all = "snake_case")]
pub enum SystemLogLine {
    Error(SystemLogError),
    CompactBoundary(CompactBoundary),
    Informational(SystemLogInformational),
    ApiError(SystemLogError),
    LocalCommand(LocalCommandLog),
    StopHookSummary(StopHookSummary),
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
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThinkingMetadata {
    pub level: String,
    pub disabled: bool,
    pub triggers: Vec<ThinkingTrigger>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
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
    },
    ToolResult(ToolResult),
    Document {
        source: DocumentSource,
    },
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
    pub message: AssistantLogMessage,
    pub request_id: Option<String>,
    pub uuid: Uuid,
    pub timestamp: DateTime<Utc>,
    pub is_api_error_message: Option<bool>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_user_log_line_with_agent_id() {
        let json = serde_json::json!({
            "agentId": "agent-123",
            "parentUuid": null,
            "isSidechain": false,
            "userType": "test",
            "cwd": "/test",
            "sessionId": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0",
            "gitBranch": "main",
            "message": {"role": "user", "content": "test"},
            "uuid": "550e8400-e29b-41d4-a716-446655440001",
            "timestamp": "2025-01-01T00:00:00Z"
        });
        let line: UserLogLine = serde_json::from_value(json).unwrap();
        assert_eq!(line.agent_id, Some("agent-123".to_string()));
    }

    #[test]
    fn test_parse_user_log_line_with_null_agent_id() {
        let json = serde_json::json!({
            "agentId": null,
            "parentUuid": null,
            "isSidechain": false,
            "userType": "test",
            "cwd": "/test",
            "sessionId": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0",
            "gitBranch": "main",
            "message": {"role": "user", "content": "test"},
            "uuid": "550e8400-e29b-41d4-a716-446655440001",
            "timestamp": "2025-01-01T00:00:00Z"
        });
        let line: UserLogLine = serde_json::from_value(json).unwrap();
        assert_eq!(line.agent_id, None);
    }

    #[test]
    fn test_parse_user_log_line_without_agent_id() {
        let json = serde_json::json!({
            "parentUuid": null,
            "isSidechain": false,
            "userType": "test",
            "cwd": "/test",
            "sessionId": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0",
            "gitBranch": "main",
            "message": {"role": "user", "content": "test"},
            "uuid": "550e8400-e29b-41d4-a716-446655440001",
            "timestamp": "2025-01-01T00:00:00Z"
        });
        let line: UserLogLine = serde_json::from_value(json).unwrap();
        assert_eq!(line.agent_id, None);
    }

    #[test]
    fn test_parse_user_log_line_with_todos() {
        let json = serde_json::json!({
            "parentUuid": null,
            "isSidechain": false,
            "userType": "test",
            "cwd": "/test",
            "sessionId": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0",
            "gitBranch": "main",
            "message": {"role": "user", "content": "test"},
            "uuid": "550e8400-e29b-41d4-a716-446655440001",
            "timestamp": "2025-01-01T00:00:00Z",
            "todos": [
                {"content": "Task 1", "status": "pending", "activeForm": "Working on Task 1"},
                {"content": "Task 2", "status": "completed", "activeForm": "Working on Task 2"}
            ]
        });
        let line: UserLogLine = serde_json::from_value(json).unwrap();
        assert!(line.todos.is_some());
        let todos = line.todos.unwrap();
        assert_eq!(todos.len(), 2);
        assert_eq!(todos[0].content, "Task 1");
        assert_eq!(todos[0].status, TodoStatus::Pending);
        assert_eq!(todos[0].active_form, "Working on Task 1");
        assert_eq!(todos[1].content, "Task 2");
        assert_eq!(todos[1].status, TodoStatus::Completed);
        assert_eq!(todos[1].active_form, "Working on Task 2");
    }

    #[test]
    fn test_parse_user_log_line_with_in_progress_todo() {
        let json = serde_json::json!({
            "parentUuid": null,
            "isSidechain": false,
            "userType": "test",
            "cwd": "/test",
            "sessionId": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0",
            "gitBranch": "main",
            "message": {"role": "user", "content": "test"},
            "uuid": "550e8400-e29b-41d4-a716-446655440001",
            "timestamp": "2025-01-01T00:00:00Z",
            "todos": [
                {"content": "Task 1", "status": "in_progress", "activeForm": "Working on Task 1"}
            ]
        });
        let line: UserLogLine = serde_json::from_value(json).unwrap();
        let todos = line.todos.unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].content, "Task 1");
        assert_eq!(todos[0].status, TodoStatus::InProgress);
        assert_eq!(todos[0].active_form, "Working on Task 1");
    }

    #[test]
    fn test_parse_user_log_line_with_null_todos() {
        let json = serde_json::json!({
            "parentUuid": null,
            "isSidechain": false,
            "userType": "test",
            "cwd": "/test",
            "sessionId": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0",
            "gitBranch": "main",
            "message": {"role": "user", "content": "test"},
            "uuid": "550e8400-e29b-41d4-a716-446655440001",
            "timestamp": "2025-01-01T00:00:00Z",
            "todos": null
        });
        let line: UserLogLine = serde_json::from_value(json).unwrap();
        assert_eq!(line.todos, None);
    }

    #[test]
    fn test_parse_user_log_line_without_todos() {
        let json = serde_json::json!({
            "parentUuid": null,
            "isSidechain": false,
            "userType": "test",
            "cwd": "/test",
            "sessionId": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0",
            "gitBranch": "main",
            "message": {"role": "user", "content": "test"},
            "uuid": "550e8400-e29b-41d4-a716-446655440001",
            "timestamp": "2025-01-01T00:00:00Z"
        });
        let line: UserLogLine = serde_json::from_value(json).unwrap();
        assert_eq!(line.todos, None);
    }

    #[test]
    fn test_parse_user_log_line_with_empty_todos() {
        let json = serde_json::json!({
            "parentUuid": null,
            "isSidechain": false,
            "userType": "test",
            "cwd": "/test",
            "sessionId": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0",
            "gitBranch": "main",
            "message": {"role": "user", "content": "test"},
            "uuid": "550e8400-e29b-41d4-a716-446655440001",
            "timestamp": "2025-01-01T00:00:00Z",
            "todos": []
        });
        let line: UserLogLine = serde_json::from_value(json).unwrap();
        assert_eq!(line.todos, Some(vec![]));
    }

    #[test]
    fn test_parse_user_log_line_rejects_unknown_fields() {
        let json = serde_json::json!({
            "parentUuid": null,
            "isSidechain": false,
            "userType": "test",
            "cwd": "/test",
            "sessionId": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0",
            "gitBranch": "main",
            "message": {"role": "user", "content": "test"},
            "uuid": "550e8400-e29b-41d4-a716-446655440001",
            "timestamp": "2025-01-01T00:00:00Z",
            "unknownField": "should be rejected"
        });

        let err_msg = serde_json::from_value::<UserLogLine>(json)
            .expect_err("Should reject unknown fields due to deny_unknown_fields")
            .to_string();
        assert!(
            err_msg.contains("unknown field") || err_msg.contains("unknownField"),
            "Error should mention unknown field, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_parse_todo_rejects_unknown_fields() {
        let json = serde_json::json!({
            "parentUuid": null,
            "isSidechain": false,
            "userType": "test",
            "cwd": "/test",
            "sessionId": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0",
            "gitBranch": "main",
            "message": {"role": "user", "content": "test"},
            "uuid": "550e8400-e29b-41d4-a716-446655440001",
            "timestamp": "2025-01-01T00:00:00Z",
            "todos": [
                {
                    "content": "Task 1",
                    "status": "pending",
                    "activeForm": "Working on Task 1",
                    "unknownField": "should be rejected"
                }
            ]
        });

        let err_msg = serde_json::from_value::<UserLogLine>(json)
            .expect_err("Should reject unknown fields in Todo struct")
            .to_string();
        assert!(
            err_msg.contains("unknown field") || err_msg.contains("unknownField"),
            "Error should mention unknown field, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_parse_assistant_log_line_with_agent_id() {
        let json = serde_json::json!({
            "agentId": "task-456",
            "parentUuid": null,
            "isSidechain": false,
            "userType": "test",
            "cwd": "/test",
            "sessionId": "test-session",
            "version": "1.0",
            "gitBranch": "main",
            "message": {
                "id": "msg-1",
                "type": "message",
                "role": "assistant",
                "content": "response",
                "model": "claude-3-5-sonnet",
                "stop_reason": "end_turn",
                "usage": {
                    "input_tokens": 100,
                    "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": 0,
                    "cache_creation": {
                        "ephemeral_5m_input_tokens": 0,
                        "ephemeral_1h_input_tokens": 0
                    },
                    "output_tokens": 50
                }
            },
            "uuid": "550e8400-e29b-41d4-a716-446655440002",
            "timestamp": "2025-01-01T00:00:00Z"
        });
        let line: AssistantLogLine = serde_json::from_value(json).unwrap();
        assert_eq!(line.agent_id, Some("task-456".to_string()));
    }

    #[test]
    fn test_parse_assistant_log_line_with_null_agent_id() {
        let json = serde_json::json!({
            "agentId": null,
            "parentUuid": null,
            "isSidechain": false,
            "userType": "test",
            "cwd": "/test",
            "sessionId": "test-session",
            "version": "1.0",
            "gitBranch": "main",
            "message": {
                "id": "msg-1",
                "type": "message",
                "role": "assistant",
                "content": "response",
                "model": "claude-3-5-sonnet",
                "stop_reason": "end_turn",
                "usage": {
                    "input_tokens": 100,
                    "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": 0,
                    "cache_creation": {
                        "ephemeral_5m_input_tokens": 0,
                        "ephemeral_1h_input_tokens": 0
                    },
                    "output_tokens": 50
                }
            },
            "uuid": "550e8400-e29b-41d4-a716-446655440002",
            "timestamp": "2025-01-01T00:00:00Z"
        });
        let line: AssistantLogLine = serde_json::from_value(json).unwrap();
        assert_eq!(line.agent_id, None);
    }

    #[test]
    fn test_parse_assistant_log_line_without_agent_id() {
        let json = serde_json::json!({
            "parentUuid": null,
            "isSidechain": false,
            "userType": "test",
            "cwd": "/test",
            "sessionId": "test-session",
            "version": "1.0",
            "gitBranch": "main",
            "message": {
                "id": "msg-1",
                "type": "message",
                "role": "assistant",
                "content": "response",
                "model": "claude-3-5-sonnet",
                "stop_reason": "end_turn",
                "usage": {
                    "input_tokens": 100,
                    "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": 0,
                    "cache_creation": {
                        "ephemeral_5m_input_tokens": 0,
                        "ephemeral_1h_input_tokens": 0
                    },
                    "output_tokens": 50
                }
            },
            "uuid": "550e8400-e29b-41d4-a716-446655440002",
            "timestamp": "2025-01-01T00:00:00Z"
        });
        let line: AssistantLogLine = serde_json::from_value(json).unwrap();
        assert_eq!(line.agent_id, None);
    }

    #[test]
    fn test_parse_document_content() {
        let json = serde_json::json!({
            "type": "document",
            "source": {
                "type": "base64",
                "media_type": "image/png",
                "data": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
            }
        });
        let content: LogMessageTaggedContent = serde_json::from_value(json).unwrap();

        match content {
            LogMessageTaggedContent::Document { source } => {
                assert_eq!(source.r#type, "base64");
                assert_eq!(source.media_type, "image/png");
                assert!(!source.data.is_empty());
            }
            _ => panic!("Expected Document variant"),
        }
    }

    #[test]
    fn test_parse_user_message_with_document() {
        let json = serde_json::json!({
            "parentUuid": null,
            "isSidechain": false,
            "userType": "test",
            "cwd": "/test",
            "sessionId": "550e8400-e29b-41d4-a716-446655440000",
            "version": "1.0",
            "gitBranch": "main",
            "message": {
                "role": "user",
                "content": [{
                    "type": "document",
                    "source": {
                        "type": "base64",
                        "media_type": "application/pdf",
                        "data": "JVBERi0xLjQK"
                    }
                }]
            },
            "uuid": "550e8400-e29b-41d4-a716-446655440001",
            "timestamp": "2025-01-01T00:00:00Z"
        });

        let line: UserLogLine = serde_json::from_value(json).unwrap();

        if let LogMessageContent::Vec(items) = &line.message.content {
            assert_eq!(items.len(), 1);
            if let LogMessageTaggedContent::Document { source } = &items[0] {
                assert_eq!(source.r#type, "base64");
                assert_eq!(source.media_type, "application/pdf");
                assert_eq!(source.data, "JVBERi0xLjQK");
            } else {
                panic!("Expected Document variant");
            }
        } else {
            panic!("Expected Vec content");
        }
    }

    #[test]
    fn test_parse_document_rejects_unknown_fields() {
        let json = serde_json::json!({
            "type": "document",
            "source": {
                "type": "base64",
                "media_type": "image/png",
                "data": "abc123",
                "unknown_field": "should fail"
            }
        });

        let err_msg = serde_json::from_value::<LogMessageTaggedContent>(json)
            .expect_err("Should reject unknown fields due to deny_unknown_fields")
            .to_string();
        assert!(
            err_msg.contains("unknown field") || err_msg.contains("unknown_field"),
            "Error should mention unknown field, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_parse_document_with_empty_data() {
        let json = serde_json::json!({
            "type": "document",
            "source": {
                "type": "base64",
                "media_type": "text/plain",
                "data": ""
            }
        });

        let content: LogMessageTaggedContent = serde_json::from_value(json).unwrap();
        match content {
            LogMessageTaggedContent::Document { source } => {
                assert_eq!(source.data, "");
            }
            _ => panic!("Expected Document variant"),
        }
    }

    #[test]
    fn test_parse_document_variant_rejects_unknown_fields() {
        let json = serde_json::json!({
            "type": "document",
            "source": {
                "type": "base64",
                "media_type": "image/png",
                "data": "abc123"
            },
            "extra_field": "should be rejected"
        });

        let err_msg = serde_json::from_value::<LogMessageTaggedContent>(json)
            .expect_err("Should reject unknown fields at Document variant level")
            .to_string();
        assert!(
            err_msg.contains("unknown field") || err_msg.contains("extra_field"),
            "Error should mention unknown field, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_parse_queue_operation() {
        let json = serde_json::json!({
            "type": "queue-operation",
            "operation": "enqueue",
            "timestamp": "2025-11-04T21:54:38.826Z",
            "content": "Use the rustdoc agent, as you've been instructed to do in order to find the definition for AudioFrame.",
            "sessionId": "75c1a8c9-5842-4fd4-a816-74109bf09cba"
        });

        let line: LogLine =
            serde_json::from_value(json).expect("Failed to parse valid queue-operation JSON");
        match line {
            LogLine::QueueOperation(op) => {
                assert_eq!(op.operation, "enqueue");
                assert_eq!(op.session_id, "75c1a8c9-5842-4fd4-a816-74109bf09cba");
                assert_eq!(
                    op.content,
                    Some(serde_json::Value::String("Use the rustdoc agent, as you've been instructed to do in order to find the definition for AudioFrame.".to_string()))
                );
                assert_eq!(op.timestamp.to_rfc3339(), "2025-11-04T21:54:38.826+00:00");
            }
            _ => panic!("Expected QueueOperation variant"),
        }
    }

    #[test]
    fn test_parse_queue_operation_rejects_unknown_fields() {
        let json = serde_json::json!({
            "type": "queue-operation",
            "operation": "enqueue",
            "timestamp": "2025-11-04T21:54:38.826Z",
            "content": "Test",
            "sessionId": "test-session",
            "extraField": "should be rejected"
        });

        let err_msg = serde_json::from_value::<LogLine>(json)
            .expect_err("Should reject unknown fields due to deny_unknown_fields")
            .to_string();
        assert!(
            err_msg.contains("unknown field") || err_msg.contains("extraField"),
            "Error should mention unknown field, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_parse_queue_operation_missing_field() {
        let json = serde_json::json!({
            "type": "queue-operation",
            "operation": "enqueue",
            "timestamp": "2025-11-04T21:54:38.826Z",
            "content": "Test content"
            // Missing sessionId
        });

        let _err = serde_json::from_value::<LogLine>(json)
            .expect_err("Should fail when required field is missing");
    }

    #[test]
    fn test_parse_queue_operation_with_empty_fields() {
        let json = serde_json::json!({
            "type": "queue-operation",
            "operation": "",
            "timestamp": "2025-11-04T21:54:38.826Z",
            "content": "",
            "sessionId": ""
        });

        let line: LogLine = serde_json::from_value(json).expect("Should parse with empty strings");

        if let LogLine::QueueOperation(op) = line {
            assert_eq!(op.operation, "");
            assert_eq!(op.content, Some(serde_json::Value::String("".to_string())));
            assert_eq!(op.session_id, "");
        } else {
            panic!("Expected QueueOperation variant");
        }
    }

    #[test]
    fn test_parse_queue_operation_dequeue() {
        let json = serde_json::json!({
            "type": "queue-operation",
            "operation": "dequeue",
            "timestamp": "2025-11-04T20:14:25.650Z",
            "content": "Maybe you should fetch the page that is linked?",
            "sessionId": "6282703f-30e7-4990-b1dd-3482afa261a5"
        });

        let line: LogLine =
            serde_json::from_value(json).expect("Failed to parse dequeue operation");

        if let LogLine::QueueOperation(op) = line {
            assert_eq!(op.operation, "dequeue");
            assert_eq!(
                op.content,
                Some(serde_json::Value::String(
                    "Maybe you should fetch the page that is linked?".to_string()
                ))
            );
            assert_eq!(op.session_id, "6282703f-30e7-4990-b1dd-3482afa261a5");
        } else {
            panic!("Expected QueueOperation variant");
        }
    }

    #[test]
    fn test_parse_assistant_with_web_fetch_and_context_management() {
        // Test new format with web_fetch_requests and context_management
        let json = serde_json::json!({
            "parentUuid": "47f0c699-1f24-49a0-889a-39fd30eabfdf",
            "isSidechain": false,
            "userType": "external",
            "cwd": "/test",
            "sessionId": "test-session",
            "version": "2.0.32",
            "gitBranch": "main",
            "type": "assistant",
            "uuid": "61cbef9e-8788-420f-acce-c2c0e921ddbc",
            "timestamp": "2025-11-06T16:44:40.009Z",
            "message": {
                "id": "001c3926-2728-4847-a14c-baf326b78196",
                "container": null,
                "model": "<synthetic>",
                "role": "assistant",
                "stop_reason": "stop_sequence",
                "stop_sequence": "",
                "type": "message",
                "usage": {
                    "input_tokens": 0,
                    "output_tokens": 0,
                    "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": 0,
                    "server_tool_use": {
                        "web_search_requests": 0,
                        "web_fetch_requests": 0
                    },
                    "service_tier": null,
                    "cache_creation": {
                        "ephemeral_1h_input_tokens": 0,
                        "ephemeral_5m_input_tokens": 0
                    }
                },
                "content": [{"type": "text", "text": "No response requested."}],
                "context_management": null
            },
            "isApiErrorMessage": false
        });

        let line: LogLine = serde_json::from_value(json).expect("Should parse new format");
        if let LogLine::Assistant(assistant) = line {
            assert_eq!(assistant.message.model, "<synthetic>");
            assert_eq!(assistant.message.context_management, None);
            assert_eq!(
                assistant
                    .message
                    .usage
                    .server_tool_use
                    .as_ref()
                    .unwrap()
                    .web_fetch_requests,
                Some(0)
            );
        } else {
            panic!("Expected Assistant variant");
        }
    }

    #[test]
    fn test_parse_assistant_without_web_fetch_requests() {
        // Test backward compatibility with old format (no web_fetch_requests)
        let json = serde_json::json!({
            "parentUuid": null,
            "isSidechain": false,
            "userType": "test",
            "cwd": "/test",
            "sessionId": "test-session",
            "version": "1.0",
            "gitBranch": "main",
            "type": "assistant",
            "uuid": "550e8400-e29b-41d4-a716-446655440002",
            "timestamp": "2025-01-01T00:00:00Z",
            "message": {
                "id": "msg-1",
                "type": "message",
                "role": "assistant",
                "content": "response",
                "model": "claude-3-5-sonnet",
                "stop_reason": "end_turn",
                "stop_sequence": null,
                "usage": {
                    "input_tokens": 100,
                    "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": 0,
                    "cache_creation": {
                        "ephemeral_5m_input_tokens": 0,
                        "ephemeral_1h_input_tokens": 0
                    },
                    "output_tokens": 50,
                    "server_tool_use": {
                        "web_search_requests": 5
                    }
                }
            }
        });

        let line: LogLine = serde_json::from_value(json).expect("Should parse old format");
        if let LogLine::Assistant(assistant) = line {
            assert_eq!(assistant.message.model, "claude-3-5-sonnet");
            assert_eq!(
                assistant
                    .message
                    .usage
                    .server_tool_use
                    .as_ref()
                    .unwrap()
                    .web_search_requests,
                5
            );
            assert_eq!(
                assistant
                    .message
                    .usage
                    .server_tool_use
                    .as_ref()
                    .unwrap()
                    .web_fetch_requests,
                None
            );
        } else {
            panic!("Expected Assistant variant");
        }
    }

    #[test]
    fn test_parse_stop_hook_summary() {
        let json = serde_json::json!({
            "parentUuid": "5445927e-82b0-4164-91f3-782fafd2a49e",
            "isSidechain": false,
            "userType": "external",
            "cwd": "/home/brendan/src/moriarty",
            "sessionId": "1a55057c-6af4-4c76-83a1-70b738990294",
            "version": "2.0.42",
            "gitBranch": "main",
            "type": "system",
            "subtype": "stop_hook_summary",
            "hookCount": 1,
            "hookInfos": [{"command": "moriarty hooks exec"}],
            "hookErrors": [],
            "preventedContinuation": false,
            "stopReason": "",
            "hasOutput": false,
            "level": "suggestion",
            "timestamp": "2025-11-18T05:27:44.883Z",
            "uuid": "35c84fed-bf99-42dc-a7bb-eae460cd23ab",
            "toolUseID": "8f3746a9-caa9-4d2d-8e6e-e7a7b005d5d4"
        });

        let line: LogLine =
            serde_json::from_value(json).expect("Failed to parse stop_hook_summary system message");

        match line {
            LogLine::System(SystemLogLine::StopHookSummary(summary)) => {
                assert_eq!(summary.hook_count, 1);
                assert_eq!(summary.hook_infos.len(), 1);
                assert_eq!(summary.hook_infos[0].command, "moriarty hooks exec");
                assert_eq!(summary.hook_errors.len(), 0);
                assert_eq!(summary.prevented_continuation, false);
                assert_eq!(summary.stop_reason, "");
                assert_eq!(summary.has_output, false);
                assert_eq!(summary.level, "suggestion");
                assert_eq!(summary.tool_use_id, "8f3746a9-caa9-4d2d-8e6e-e7a7b005d5d4");
            }
            _ => panic!("Expected System(StopHookSummary) variant"),
        }
    }

    #[test]
    fn test_parse_stop_hook_summary_rejects_unknown_fields() {
        let json = serde_json::json!({
            "parentUuid": "5445927e-82b0-4164-91f3-782fafd2a49e",
            "isSidechain": false,
            "userType": "external",
            "cwd": "/home/brendan/src/moriarty",
            "sessionId": "1a55057c-6af4-4c76-83a1-70b738990294",
            "version": "2.0.42",
            "gitBranch": "main",
            "type": "system",
            "subtype": "stop_hook_summary",
            "hookCount": 1,
            "hookInfos": [{"command": "moriarty hooks exec"}],
            "hookErrors": [],
            "preventedContinuation": false,
            "stopReason": "",
            "hasOutput": false,
            "level": "suggestion",
            "timestamp": "2025-11-18T05:27:44.883Z",
            "uuid": "35c84fed-bf99-42dc-a7bb-eae460cd23ab",
            "toolUseID": "8f3746a9-caa9-4d2d-8e6e-e7a7b005d5d4",
            "unknownField": "should be rejected"
        });

        let err_msg = serde_json::from_value::<LogLine>(json)
            .expect_err("Should reject unknown fields due to deny_unknown_fields")
            .to_string();
        assert!(
            err_msg.contains("unknown field") || err_msg.contains("unknownField"),
            "Error should mention unknown field, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_parse_hook_error_with_all_fields() {
        let json = serde_json::json!({
            "message": "Command failed",
            "command": "test-hook",
            "exitCode": 1
        });

        let error: HookError = serde_json::from_value(json).expect("Failed to parse HookError");
        assert_eq!(error.message(), "Command failed");
        assert_eq!(error.command(), Some("test-hook"));
        assert_eq!(error.exit_code(), Some(1));
    }

    #[test]
    fn test_parse_hook_error_minimal() {
        let json = serde_json::json!({
            "message": "Error occurred"
        });

        let error: HookError = serde_json::from_value(json).expect("Failed to parse HookError");
        assert_eq!(error.message(), "Error occurred");
        assert_eq!(error.command(), None);
        assert_eq!(error.exit_code(), None);
    }

    #[test]
    fn test_parse_hook_error_from_string() {
        let error: HookError =
            serde_json::from_value(serde_json::json!("Error message")).unwrap();
        assert_eq!(error.message(), "Error message");
        assert_eq!(error.command(), None);
        assert_eq!(error.exit_code(), None);
    }

    #[test]
    fn test_parse_hook_error_rejects_unknown_fields() {
        let json = serde_json::json!({
            "message": "Error",
            "unknownField": "value"
        });

        let err_msg = serde_json::from_value::<HookError>(json)
            .expect_err("Should reject unknown fields due to deny_unknown_fields")
            .to_string();
        assert!(
            err_msg.contains("unknown field")
                || err_msg.contains("unknownField")
                || err_msg.contains("did not match any variant"),
            "Error should mention unknown field or variant mismatch, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_parse_hook_info_rejects_unknown_fields() {
        let json = serde_json::json!({
            "command": "test-command",
            "extraField": "bad"
        });

        let err_msg = serde_json::from_value::<HookInfo>(json)
            .expect_err("Should reject unknown fields due to deny_unknown_fields")
            .to_string();
        assert!(
            err_msg.contains("unknown field") || err_msg.contains("extraField"),
            "Error should mention unknown field, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_parse_stop_hook_summary_with_multiple_hooks_and_errors() {
        let json = serde_json::json!({
            "parentUuid": "5445927e-82b0-4164-91f3-782fafd2a49e",
            "isSidechain": false,
            "userType": "external",
            "cwd": "/home/brendan/src/moriarty",
            "sessionId": "1a55057c-6af4-4c76-83a1-70b738990294",
            "version": "2.0.42",
            "gitBranch": "main",
            "type": "system",
            "subtype": "stop_hook_summary",
            "hookCount": 3,
            "hookInfos": [
                {"command": "hook1"},
                {"command": "hook2"},
                {"command": "hook3"}
            ],
            "hookErrors": [
                {"message": "Error 1", "command": "hook1", "exitCode": 1},
                {"message": "Error 2"}
            ],
            "preventedContinuation": true,
            "stopReason": "Multiple hooks failed",
            "hasOutput": true,
            "level": "error",
            "timestamp": "2025-11-18T05:27:44.883Z",
            "uuid": "35c84fed-bf99-42dc-a7bb-eae460cd23ab",
            "toolUseID": "8f3746a9-caa9-4d2d-8e6e-e7a7b005d5d4"
        });

        let line: LogLine = serde_json::from_value(json)
            .expect("Failed to parse stop_hook_summary with multiple hooks");

        match line {
            LogLine::System(SystemLogLine::StopHookSummary(summary)) => {
                assert_eq!(summary.hook_count, 3);
                assert_eq!(summary.hook_infos.len(), 3);
                assert_eq!(summary.hook_infos[0].command, "hook1");
                assert_eq!(summary.hook_infos[1].command, "hook2");
                assert_eq!(summary.hook_infos[2].command, "hook3");
                assert_eq!(summary.hook_errors.len(), 2);
                assert_eq!(summary.hook_errors[0].message(), "Error 1");
                assert_eq!(summary.hook_errors[0].command(), Some("hook1"));
                assert_eq!(summary.hook_errors[0].exit_code(), Some(1));
                assert_eq!(summary.hook_errors[1].message(), "Error 2");
                assert_eq!(summary.hook_errors[1].command(), None);
                assert_eq!(summary.prevented_continuation, true);
                assert_eq!(summary.stop_reason, "Multiple hooks failed");
                assert_eq!(summary.has_output, true);
                assert_eq!(summary.level, "error");
            }
            _ => panic!("Expected System(StopHookSummary) variant"),
        }
    }

    #[test]
    fn test_parse_stop_hook_summary_with_empty_arrays() {
        let json = serde_json::json!({
            "parentUuid": "5445927e-82b0-4164-91f3-782fafd2a49e",
            "isSidechain": false,
            "userType": "external",
            "cwd": "/home/brendan/src/moriarty",
            "sessionId": "1a55057c-6af4-4c76-83a1-70b738990294",
            "version": "2.0.42",
            "gitBranch": "main",
            "type": "system",
            "subtype": "stop_hook_summary",
            "hookCount": 0,
            "hookInfos": [],
            "hookErrors": [],
            "preventedContinuation": false,
            "stopReason": "",
            "hasOutput": false,
            "level": "info",
            "timestamp": "2025-11-18T05:27:44.883Z",
            "uuid": "35c84fed-bf99-42dc-a7bb-eae460cd23ab",
            "toolUseID": "test-id"
        });

        let line: LogLine = serde_json::from_value(json)
            .expect("Failed to parse stop_hook_summary with empty arrays");

        match line {
            LogLine::System(SystemLogLine::StopHookSummary(summary)) => {
                assert_eq!(summary.hook_count, 0);
                assert_eq!(summary.hook_infos.len(), 0);
                assert_eq!(summary.hook_errors.len(), 0);
                assert_eq!(summary.prevented_continuation, false);
                assert_eq!(summary.has_output, false);
            }
            _ => panic!("Expected System(StopHookSummary) variant"),
        }
    }

    #[test]
    fn test_parse_stop_hook_summary_with_string_errors() {
        let json = serde_json::json!({
            "parentUuid": "a2c16202-b7fb-446c-86e4-7dc55db7f24f",
            "isSidechain": false,
            "userType": "external",
            "cwd": "/test",
            "sessionId": "550e8400-e29b-41d4-a716-446655440000",
            "version": "2.0.47",
            "gitBranch": "main",
            "type": "system",
            "subtype": "stop_hook_summary",
            "hookCount": 1,
            "hookInfos": [{"command": "test-hook"}],
            "hookErrors": ["Error 1", "Error 2"],
            "preventedContinuation": false,
            "stopReason": "",
            "hasOutput": true,
            "level": "suggestion",
            "timestamp": "2025-11-22T19:55:01.863Z",
            "uuid": "49bbbff9-1b81-4c32-bc20-4ae8c41a40d6",
            "toolUseID": "65d059ca-f330-4ffc-8c15-a606cb13bc56"
        });

        let line: LogLine = serde_json::from_value(json)
            .expect("Failed to parse stop_hook_summary with string errors");

        match line {
            LogLine::System(SystemLogLine::StopHookSummary(summary)) => {
                assert_eq!(summary.hook_errors.len(), 2);
                assert_eq!(summary.hook_errors[0].message(), "Error 1");
                assert_eq!(summary.hook_errors[0].command(), None);
                assert_eq!(summary.hook_errors[0].exit_code(), None);
                assert_eq!(summary.hook_errors[1].message(), "Error 2");
                assert_eq!(summary.hook_errors[1].command(), None);
                assert_eq!(summary.hook_errors[1].exit_code(), None);
            }
            _ => panic!("Expected System(StopHookSummary) variant"),
        }
    }

    #[test]
    fn test_parse_stop_hook_summary_with_mixed_error_formats() {
        let json = serde_json::json!({
            "parentUuid": "a2c16202-b7fb-446c-86e4-7dc55db7f24f",
            "isSidechain": false,
            "userType": "external",
            "cwd": "/test",
            "sessionId": "550e8400-e29b-41d4-a716-446655440000",
            "version": "2.0.47",
            "gitBranch": "main",
            "type": "system",
            "subtype": "stop_hook_summary",
            "hookCount": 2,
            "hookInfos": [{"command": "hook1"}, {"command": "hook2"}],
            "hookErrors": [
                "Simple error message",
                {"message": "Detailed error", "command": "hook1", "exitCode": 1},
                "Another simple error"
            ],
            "preventedContinuation": true,
            "stopReason": "Multiple hooks failed",
            "hasOutput": true,
            "level": "error",
            "timestamp": "2025-11-22T19:55:01.863Z",
            "uuid": "49bbbff9-1b81-4c32-bc20-4ae8c41a40d6",
            "toolUseID": "65d059ca-f330-4ffc-8c15-a606cb13bc56"
        });

        let line: LogLine = serde_json::from_value(json)
            .expect("Failed to parse stop_hook_summary with mixed error formats");

        match line {
            LogLine::System(SystemLogLine::StopHookSummary(summary)) => {
                assert_eq!(summary.hook_errors.len(), 3);
                // First error: string format
                assert_eq!(summary.hook_errors[0].message(), "Simple error message");
                assert_eq!(summary.hook_errors[0].command(), None);
                assert_eq!(summary.hook_errors[0].exit_code(), None);
                // Second error: structured format
                assert_eq!(summary.hook_errors[1].message(), "Detailed error");
                assert_eq!(summary.hook_errors[1].command(), Some("hook1"));
                assert_eq!(summary.hook_errors[1].exit_code(), Some(1));
                // Third error: string format
                assert_eq!(summary.hook_errors[2].message(), "Another simple error");
                assert_eq!(summary.hook_errors[2].command(), None);
                assert_eq!(summary.hook_errors[2].exit_code(), None);
            }
            _ => panic!("Expected System(StopHookSummary) variant"),
        }
    }
}
