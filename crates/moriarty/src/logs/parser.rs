use std::{collections::HashMap, path::Path};

use chrono::{DateTime, Utc};
use miette::IntoDiagnostic;
use serde::{Deserialize, Serialize};
use tokio::fs::read_to_string;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct QueueOperation {
    pub operation: String,
    pub timestamp: DateTime<Utc>,
    pub content: String,
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

        let result = serde_json::from_value::<LogMessageTaggedContent>(json);
        let err = result.expect_err("Should reject unknown fields due to deny_unknown_fields");
        let err_msg = err.to_string();
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

        let result = serde_json::from_value::<LogMessageTaggedContent>(json);
        let err = result.expect_err("Should reject unknown fields at Document variant level");
        let err_msg = err.to_string();
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
                assert_eq!(op.content, "Use the rustdoc agent, as you've been instructed to do in order to find the definition for AudioFrame.");
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

        let result = serde_json::from_value::<LogLine>(json);
        let err = result.expect_err("Should reject unknown fields due to deny_unknown_fields");
        let err_msg = err.to_string();
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

        let result = serde_json::from_value::<LogLine>(json);
        assert!(
            result.is_err(),
            "Should fail when required field is missing"
        );
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
            assert_eq!(op.content, "");
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
                "Maybe you should fetch the page that is linked?"
            );
            assert_eq!(op.session_id, "6282703f-30e7-4990-b1dd-3482afa261a5");
        } else {
            panic!("Expected QueueOperation variant");
        }
    }
}
