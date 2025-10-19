use std::{collections::HashMap, path::Path};

use chrono::{DateTime, Utc};
use miette::IntoDiagnostic;
use serde::{Deserialize, Serialize};
use tokio::fs::read_to_string;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    ToolResult {
        content: LogMessageContent,
        is_error: Option<bool>,
        tool_use_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct AssistantLogLine {
    pub parent_uuid: Option<Uuid>,
    pub is_sidechain: bool,
    pub user_type: String,
    pub cwd: String,
    pub session_id: String,
    pub version: String,
    pub git_branch: String,
    pub message: AssistantLogMessage,
    pub request_id: String,
    pub uuid: Uuid,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantLogMessage {
    pub id: String,
    pub r#type: String,
    pub role: String,
    pub model: String,
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
    pub service_tier: String,
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
            let log_line: LogLine = serde_json::from_str(line)
                .into_diagnostic()
                .inspect_err(|_| {
                    println!("{line}");
                })?;
            log_lines.push(log_line);
        }
    }

    Ok(log_lines)
}
