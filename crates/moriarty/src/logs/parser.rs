use std::{collections::HashMap, path::Path};

use chrono::{DateTime, Utc};
use miette::IntoDiagnostic;
use serde::{Deserialize, Serialize};
use tokio::fs::read_to_string;
use uuid::Uuid;

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
    pub status: u16,
    pub headers: HashMap<String, String>,
    #[serde(rename = "requestID")]
    pub request_id: Option<String>,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantCacheCreation {
    pub ephemeral_5m_input_tokens: usize,
    pub ephemeral_1h_input_tokens: usize,
}

pub async fn read_file(file: impl AsRef<Path>) -> miette::Result<Vec<LogLine>> {
    let string_contents = read_to_string(file).await.into_diagnostic()?;

    let mut log_lines = Vec::new();

    for line in string_contents.split('\n') {
        if !line.is_empty() {
            let log_line: LogLine =
                serde_json::from_str(line)
                    .into_diagnostic()
                    .inspect_err(|_| {
                        println!("{line}");
                    })?;
            log_lines.push(log_line);
        }
    }

    Ok(log_lines)
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
}
