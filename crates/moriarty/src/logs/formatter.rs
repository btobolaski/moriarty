use super::parser::{
    AssistantLogLine, LogLine, LogMessageContent, LogMessageTaggedContent, ProgressData,
    ProgressLogLine, SystemLogLine, ToolResult, UserLogLine,
};
use chrono::{DateTime, Utc};

/// Format a timestamp in a human-readable way
fn format_timestamp(timestamp: &DateTime<Utc>) -> String {
    timestamp.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

/// Format message content (handles both String and Vec variants)
fn format_message_content(content: &LogMessageContent) -> String {
    match content {
        LogMessageContent::String(s) => s.clone(),
        LogMessageContent::Vec(items) => {
            let mut output = String::new();
            for item in items {
                match item {
                    LogMessageTaggedContent::Text { text } => {
                        output.push_str(text);
                        output.push('\n');
                    }
                    LogMessageTaggedContent::Thinking { thinking, .. } => {
                        output.push_str("💭 Thinking:\n");
                        output.push_str(thinking);
                        output.push_str("\n\n");
                    }
                    LogMessageTaggedContent::ToolUse { id, name, input } => {
                        output.push_str(&format!("🔧 Tool Use: {}\n", name));
                        output.push_str(&format!("   ID: {}\n", id));
                        if !input.is_empty() {
                            output.push_str("   Input:\n");
                            for (key, value) in input {
                                let value_str = serde_json::to_string_pretty(value)
                                    .unwrap_or_else(|_| format!("{:?}", value));
                                if let Some(line) = value_str.lines().next() {
                                    output.push_str(&format!("      {}: {}\n", key, line));
                                }
                            }
                        }
                        output.push('\n');
                    }
                    LogMessageTaggedContent::ToolResult(result) => {
                        output.push_str("📦 Tool Result:\n");
                        match result {
                            ToolResult::Current {
                                content,
                                is_error,
                                tool_use_id,
                            } => {
                                output.push_str(&format!("   Tool Use ID: {}\n", tool_use_id));
                                if let Some(true) = is_error {
                                    output.push_str("   Status: ❌ Error\n");
                                } else {
                                    output.push_str("   Status: ✅ Success\n");
                                }
                                let content_str = format_message_content(content);
                                // Show first few lines of content
                                for (i, line) in content_str.lines().take(3).enumerate() {
                                    if i == 0 {
                                        output.push_str("   Content:\n");
                                    }
                                    output.push_str(&format!("      {}\n", line));
                                }
                                if content_str.lines().count() > 3 {
                                    output.push_str("      ...\n");
                                }
                            }
                            ToolResult::V1 { tool_use_id } => {
                                output.push_str(&format!("   Tool Use ID: {}\n", tool_use_id));
                            }
                        }
                        output.push('\n');
                    }
                    LogMessageTaggedContent::Document { source } => {
                        output.push_str("📄 Document:\n");
                        output.push_str(&format!("   Type: {}\n", source.r#type));
                        output.push_str(&format!("   Media Type: {}\n", source.media_type));
                        output.push_str(&format!(
                            "   Data Length: {} characters\n",
                            source.data.len()
                        ));
                        output.push('\n');
                    }
                }
            }
            output
        }
    }
}

/// Format a user message
fn format_user_message(user: &UserLogLine) -> String {
    let mut output = String::new();

    output.push_str(&format!(
        "👤 User ({})\n",
        format_timestamp(&user.timestamp)
    ));

    if let Some(true) = user.is_meta {
        output.push_str("   [Meta Message]\n");
    }

    if let Some(true) = user.is_compact_summary {
        output.push_str("   [Compact Summary]\n");
    }

    if let Some(true) = user.is_visible_in_transcript_only {
        output.push_str("   [Transcript Only]\n");
    }

    output.push('\n');
    output.push_str(&format_message_content(&user.message.content));

    if let Some(ref thinking_metadata) = user.thinking_metadata {
        output.push_str(&format!(
            "\n💭 Thinking: level={}, disabled={}, triggers={}\n",
            thinking_metadata.level,
            thinking_metadata.disabled,
            thinking_metadata.triggers.len()
        ));
    }

    output
}

/// Format an assistant message
fn format_assistant_message(assistant: &AssistantLogLine) -> String {
    let mut output = String::new();

    output.push_str(&format!(
        "🤖 Assistant ({}) - {}\n",
        format_timestamp(&assistant.timestamp),
        assistant.message.model
    ));

    if let Some(true) = assistant.is_api_error_message {
        output.push_str("   ❌ API Error Message\n");
    }

    // Usage stats
    let usage = &assistant.message.usage;
    output.push_str(&format!(
        "   Tokens: in={} out={} cache_read={}\n",
        usage.input_tokens, usage.output_tokens, usage.cache_read_input_tokens
    ));

    if let Some(ref stop_reason) = assistant.message.stop_reason {
        output.push_str(&format!("   Stop: {}\n", stop_reason));
    }

    output.push('\n');
    output.push_str(&format_message_content(&assistant.message.content));

    output
}

/// Format a system log line
fn format_system_message(system: &SystemLogLine) -> String {
    match system {
        SystemLogLine::Error(error) => {
            let status = error
                .error
                .status
                .map(|s| s.to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            format!(
                "⚠️  System Error (retry {}/{})\n   Status: {}\n   Retry in: {:.0}ms\n",
                error.retry_attempt, error.max_retries, status, error.retry_in_ms
            )
        }
        SystemLogLine::CompactBoundary(boundary) => {
            format!(
                "📦 Compact Boundary\n   Trigger: {}\n   Pre-tokens: {}\n   {}\n",
                boundary.compact_metadata.trigger,
                boundary.compact_metadata.pre_tokens,
                boundary.content
            )
        }
        SystemLogLine::MicrocompactBoundary(boundary) => {
            format!(
                "📦 Microcompact Boundary\n   Trigger: {}\n   Pre-tokens: {}\n   Tokens saved: {}\n   {}\n",
                boundary.microcompact_metadata.trigger,
                boundary.microcompact_metadata.pre_tokens,
                boundary.microcompact_metadata.tokens_saved,
                boundary.content
            )
        }
        SystemLogLine::Informational(info) => {
            format!("ℹ️  System: {}\n", info.content)
        }
        SystemLogLine::ApiError(error) => {
            let status = error
                .error
                .status
                .map(|s| s.to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            format!(
                "❌ API Error (retry {}/{})\n   Status: {}\n   Retry in: {:.0}ms\n",
                error.retry_attempt, error.max_retries, status, error.retry_in_ms
            )
        }
        SystemLogLine::LocalCommand(cmd) => {
            format!("💻 Local Command [{}]\n   {}\n", cmd.level, cmd.content)
        }
        SystemLogLine::StopHookSummary(summary) => {
            let mut output = format!("🪝 Hook Summary ({} hook(s))\n", summary.hook_count);
            for hook_info in &summary.hook_infos {
                output.push_str(&format!("   Command: {}\n", hook_info.command));
            }
            if !summary.hook_errors.is_empty() {
                output.push_str(&format!("   Errors: {}\n", summary.hook_errors.len()));
            }
            if summary.prevented_continuation {
                let reason = if summary.stop_reason.is_empty() {
                    "no reason provided"
                } else {
                    &summary.stop_reason
                };
                output.push_str(&format!("   ⚠️  Prevented continuation: {}\n", reason));
            }
            output
        }
        SystemLogLine::TurnDuration(duration) => {
            format!("⏱️  Turn Duration: {}ms\n", duration.duration_ms)
        }
    }
}

/// Format a summary
fn format_summary(summary: &super::parser::Summary) -> String {
    format!("📝 Summary:\n{}\n", summary.summary)
}

/// Format a file history snapshot
fn format_file_history_snapshot(snapshot: &super::parser::FileHistorySnapshot) -> String {
    let update_type = if snapshot.is_snapshot_update {
        "Update"
    } else {
        "New"
    };
    format!(
        "📸 File History Snapshot ({})\n   Message ID: {}\n   Files tracked: {}\n",
        update_type,
        snapshot.message_id,
        snapshot.snapshot.tracked_file_backups.len()
    )
}

/// Format any LogLine into a human-readable string
pub fn format_log_line(log_line: &LogLine) -> String {
    match log_line {
        LogLine::User(user) => format_user_message(user),
        LogLine::Assistant(assistant) => format_assistant_message(assistant),
        LogLine::System(system) => format_system_message(system),
        LogLine::Summary(summary) => format_summary(summary),
        LogLine::FileHistorySnapshot(snapshot) => format_file_history_snapshot(snapshot),
        LogLine::QueueOperation(queue_op) => {
            let content = queue_op
                .content
                .as_ref()
                .map(|v| v.to_string())
                .unwrap_or_else(|| "None".to_string());
            format!(
                "📋 Queue Operation: {} ({})\n   Session: {}\n   Content: {}\n",
                queue_op.operation,
                format_timestamp(&queue_op.timestamp),
                queue_op.session_id,
                content
            )
        }
        LogLine::Progress(progress) => format_progress(progress),
    }
}

fn format_progress(progress: &ProgressLogLine) -> String {
    match &progress.data {
        ProgressData::HookProgress(data) => {
            format!(
                "⏳ Hook Progress: {} - {}\n   Command: {}\n",
                data.hook_event, data.hook_name, data.command
            )
        }
        ProgressData::McpProgress(data) => {
            let elapsed = data
                .elapsed_time_ms
                .map(|ms| format!(" ({}ms)", ms))
                .unwrap_or_default();
            format!(
                "⏳ MCP Progress: {} - {}/{}{}\n",
                data.status, data.server_name, data.tool_name, elapsed
            )
        }
        ProgressData::BashProgress(data) => {
            format!(
                "⏳ Bash Progress: {}s elapsed, {} lines\n   Output: {}\n",
                data.elapsed_time_seconds, data.total_lines, data.output
            )
        }
        ProgressData::AgentProgress(data) => {
            format!(
                "⏳ Agent Progress: {}\n   Prompt: {}\n",
                data.agent_id, data.prompt
            )
        }
        ProgressData::WaitingForTask(data) => {
            format!(
                "⏳ Waiting for Task: {} ({})\n",
                data.task_description, data.task_type
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logs::parser::{
        AssistantCacheCreation, AssistantLogLine, AssistantLogMessage, AssistantUsage,
        CompactBoundary, CompactMetadata, DocumentSource, FileHistorySnapshot,
        FileHistorySnapshotSnapshot, LocalCommandLog, LogMessage, LogMessageContent,
        LogMessageTaggedContent, Summary, SystemLogError, SystemLogErrorError,
        SystemLogInformational, SystemLogLine, ToolResult,
    };
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn create_test_user(content: LogMessageContent) -> UserLogLine {
        UserLogLine {
            parent_uuid: None,
            is_sidechain: false,
            agent_id: None,
            user_type: "test".to_string(),
            cwd: "/test".to_string(),
            session_id: Uuid::new_v4(),
            version: "1.0.0".to_string(),
            git_branch: "main".to_string(),
            slug: None,
            message: LogMessage {
                role: "user".to_string(),
                content,
            },
            is_meta: None,
            uuid: Uuid::new_v4(),
            timestamp: Utc::now(),
            tool_use_result: None,
            thinking_metadata: None,
            is_visible_in_transcript_only: None,
            is_compact_summary: None,
            todos: None,
            source_tool_assistant_uuid: None,
        }
    }

    fn create_test_assistant(content: LogMessageContent) -> AssistantLogLine {
        AssistantLogLine {
            parent_uuid: None,
            is_sidechain: false,
            agent_id: None,
            user_type: "test".to_string(),
            cwd: "/test".to_string(),
            session_id: "session".to_string(),
            version: "1.0.0".to_string(),
            git_branch: "main".to_string(),
            slug: None,
            message: AssistantLogMessage {
                id: "msg".to_string(),
                r#type: "message".to_string(),
                role: "assistant".to_string(),
                model: "claude".to_string(),
                container: None,
                content,
                stop_reason: None,
                stop_sequence: None,
                usage: AssistantUsage {
                    input_tokens: 10,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                    cache_creation: AssistantCacheCreation {
                        ephemeral_5m_input_tokens: 0,
                        ephemeral_1h_input_tokens: 0,
                    },
                    output_tokens: 20,
                    service_tier: None,
                    server_tool_use: None,
                },
                context_management: None,
            },
            request_id: None,
            uuid: Uuid::new_v4(),
            timestamp: Utc::now(),
            is_api_error_message: None,
            error: None,
        }
    }

    #[test]
    fn test_format_user_string_content() {
        let user = create_test_user(LogMessageContent::String("Hello".to_string()));
        let result = format_user_message(&user);
        assert!(!result.is_empty());
        assert!(result.contains("Hello"));
    }

    #[test]
    fn test_format_user_text_content() {
        let user = create_test_user(LogMessageContent::Vec(vec![
            LogMessageTaggedContent::Text {
                text: "Test text".to_string(),
            },
        ]));
        let result = format_user_message(&user);
        assert!(!result.is_empty());
        assert!(result.contains("Test text"));
    }

    #[test]
    fn test_format_user_thinking_content() {
        let user = create_test_user(LogMessageContent::Vec(vec![
            LogMessageTaggedContent::Thinking {
                thinking: "Thinking...".to_string(),
                signature: "sig".to_string(),
            },
        ]));
        let result = format_user_message(&user);
        assert!(!result.is_empty());
        assert!(result.contains("Thinking..."));
    }

    #[test]
    fn test_format_user_tool_use_content() {
        let mut input = HashMap::new();
        input.insert("key".to_string(), serde_json::json!("value"));
        let user = create_test_user(LogMessageContent::Vec(vec![
            LogMessageTaggedContent::ToolUse {
                id: "tool_1".to_string(),
                name: "TestTool".to_string(),
                input,
            },
        ]));
        let result = format_user_message(&user);
        assert!(!result.is_empty());
        assert!(result.contains("TestTool"));
    }

    #[test]
    fn test_format_user_tool_result_content() {
        let user = create_test_user(LogMessageContent::Vec(vec![
            LogMessageTaggedContent::ToolResult(ToolResult::Current {
                content: LogMessageContent::String("Result".to_string()),
                is_error: Some(false),
                tool_use_id: "tool_1".to_string(),
            }),
        ]));
        let result = format_user_message(&user);
        assert!(!result.is_empty());
        assert!(result.contains("Result"));
    }

    #[test]
    fn test_format_assistant_string_content() {
        let assistant = create_test_assistant(LogMessageContent::String("Response".to_string()));
        let result = format_assistant_message(&assistant);
        assert!(!result.is_empty());
        assert!(result.contains("Response"));
    }

    #[test]
    fn test_format_assistant_vec_content() {
        let assistant = create_test_assistant(LogMessageContent::Vec(vec![
            LogMessageTaggedContent::Text {
                text: "Assistant response".to_string(),
            },
        ]));
        let result = format_assistant_message(&assistant);
        assert!(!result.is_empty());
        assert!(result.contains("Assistant response"));
    }

    #[test]
    fn test_format_system_error() {
        let system = SystemLogLine::Error(SystemLogError {
            parent_uuid: Uuid::new_v4(),
            is_sidechain: false,
            user_type: "test".to_string(),
            cwd: "/test".to_string(),
            session_id: "session".to_string(),
            version: "1.0.0".to_string(),
            git_branch: "main".to_string(),
            slug: None,
            level: "error".to_string(),
            cause: None,
            error: SystemLogErrorError {
                status: Some(500),
                headers: Some(HashMap::new()),
                request_id: None,
                cause: None,
            },
            retry_in_ms: 1000.0,
            retry_attempt: 1,
            max_retries: 3,
            timestamp: Utc::now(),
            uuid: Uuid::new_v4(),
        });
        let result = format_system_message(&system);
        assert!(!result.is_empty());
        assert!(result.contains("500"));
    }

    #[test]
    fn test_format_system_api_error() {
        let system = SystemLogLine::ApiError(SystemLogError {
            parent_uuid: Uuid::new_v4(),
            is_sidechain: false,
            user_type: "test".to_string(),
            cwd: "/test".to_string(),
            session_id: "session".to_string(),
            version: "1.0.0".to_string(),
            git_branch: "main".to_string(),
            slug: None,
            level: "error".to_string(),
            cause: None,
            error: SystemLogErrorError {
                status: Some(429),
                headers: Some(HashMap::new()),
                request_id: None,
                cause: None,
            },
            retry_in_ms: 2000.0,
            retry_attempt: 2,
            max_retries: 5,
            timestamp: Utc::now(),
            uuid: Uuid::new_v4(),
        });
        let result = format_system_message(&system);
        assert!(!result.is_empty());
        assert!(result.contains("429"));
    }

    #[test]
    fn test_format_system_informational() {
        let system = SystemLogLine::Informational(SystemLogInformational {
            parent_uuid: Uuid::new_v4(),
            is_sidechain: false,
            git_branch: Some("main".to_string()),
            slug: None,
            user_type: "test".to_string(),
            cwd: "/test".to_string(),
            session_id: Uuid::new_v4(),
            version: "1.0.0".to_string(),
            content: "Info message".to_string(),
            is_meta: false,
            timestamp: Utc::now(),
            uuid: Uuid::new_v4(),
            level: "info".to_string(),
        });
        let result = format_system_message(&system);
        assert!(!result.is_empty());
        assert!(result.contains("Info message"));
    }

    #[test]
    fn test_format_system_local_command() {
        let system = SystemLogLine::LocalCommand(LocalCommandLog {
            parent_uuid: None,
            is_sidechain: false,
            user_type: "test".to_string(),
            cwd: "/test".to_string(),
            session_id: Uuid::new_v4(),
            version: "1.0.0".to_string(),
            git_branch: "main".to_string(),
            slug: None,
            content: "ls -la".to_string(),
            level: "debug".to_string(),
            timestamp: Utc::now(),
            uuid: Uuid::new_v4(),
            is_meta: false,
        });
        let result = format_system_message(&system);
        assert!(!result.is_empty());
        assert!(result.contains("ls -la"));
    }

    #[test]
    fn test_format_system_compact_boundary() {
        let system = SystemLogLine::CompactBoundary(CompactBoundary {
            parent_uuid: None,
            logical_parent_uuid: Uuid::new_v4(),
            is_sidechain: false,
            user_type: "test".to_string(),
            cwd: "/test".to_string(),
            session_id: Uuid::new_v4(),
            version: "1.0.0".to_string(),
            git_branch: "main".to_string(),
            slug: None,
            content: "Compacting".to_string(),
            is_meta: false,
            timestamp: Utc::now(),
            uuid: Uuid::new_v4(),
            level: "info".to_string(),
            compact_metadata: CompactMetadata {
                trigger: "auto".to_string(),
                pre_tokens: 1000,
            },
        });
        let result = format_system_message(&system);
        assert!(!result.is_empty());
        assert!(result.contains("Compacting"));
    }

    #[test]
    fn test_format_summary() {
        let summary = Summary {
            summary: "Test summary".to_string(),
            leaf_uuid: Uuid::new_v4(),
        };
        let result = format_summary(&summary);
        assert!(!result.is_empty());
        assert!(result.contains("Test summary"));
    }

    #[test]
    fn test_format_file_history_snapshot() {
        let snapshot = FileHistorySnapshot {
            message_id: Uuid::new_v4(),
            snapshot: FileHistorySnapshotSnapshot {
                message_id: Uuid::new_v4(),
                tracked_file_backups: HashMap::new(),
                timestamp: Utc::now(),
            },
            is_snapshot_update: false,
        };
        let result = format_file_history_snapshot(&snapshot);
        assert!(!result.is_empty());
    }

    #[test]
    fn test_format_log_line_user() {
        let user = create_test_user(LogMessageContent::String("Test".to_string()));
        let log_line = LogLine::User(user);
        let result = format_log_line(&log_line);
        assert!(!result.is_empty());
    }

    #[test]
    fn test_format_log_line_assistant() {
        let assistant = create_test_assistant(LogMessageContent::String("Test".to_string()));
        let log_line = LogLine::Assistant(assistant);
        let result = format_log_line(&log_line);
        assert!(!result.is_empty());
    }

    #[test]
    fn test_format_log_line_summary() {
        let summary = Summary {
            summary: "Test".to_string(),
            leaf_uuid: Uuid::new_v4(),
        };
        let log_line = LogLine::Summary(summary);
        let result = format_log_line(&log_line);
        assert!(!result.is_empty());
    }

    #[test]
    fn test_format_message_content_empty_string() {
        let content = LogMessageContent::String(String::new());
        let result = format_message_content(&content);
        assert_eq!(result, "");
    }

    #[test]
    fn test_format_message_content_empty_vec() {
        let content = LogMessageContent::Vec(vec![]);
        let result = format_message_content(&content);
        assert!(!result.is_empty() || result.is_empty()); // Just verify it doesn't panic
    }

    #[test]
    fn test_format_user_document_content() {
        let user = create_test_user(LogMessageContent::Vec(vec![
            LogMessageTaggedContent::Document {
                source: DocumentSource {
                    r#type: "base64".to_string(),
                    media_type: "image/png".to_string(),
                    data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJ".to_string(),
                },
            },
        ]));

        let result = format_user_message(&user);
        assert!(!result.is_empty());
        assert!(result.contains("📄 Document:"));
        assert!(result.contains("Type: base64"));
        assert!(result.contains("Media Type: image/png"));
        assert!(result.contains("Data Length: 44 characters"));
    }

    #[test]
    fn test_format_document_with_empty_data() {
        let user = create_test_user(LogMessageContent::Vec(vec![
            LogMessageTaggedContent::Document {
                source: DocumentSource {
                    r#type: "base64".to_string(),
                    media_type: "text/plain".to_string(),
                    data: String::new(),
                },
            },
        ]));

        let result = format_user_message(&user);
        assert!(result.contains("Data Length: 0 characters"));
    }

    #[test]
    fn test_format_message_with_mixed_content_including_document() {
        let user = create_test_user(LogMessageContent::Vec(vec![
            LogMessageTaggedContent::Text {
                text: "Here is the document:".to_string(),
            },
            LogMessageTaggedContent::Document {
                source: DocumentSource {
                    r#type: "base64".to_string(),
                    media_type: "application/pdf".to_string(),
                    data: "JVBERi0xLjQK".to_string(),
                },
            },
            LogMessageTaggedContent::Text {
                text: "Please review it.".to_string(),
            },
        ]));

        let result = format_user_message(&user);
        assert!(result.contains("Here is the document:"));
        assert!(result.contains("📄 Document:"));
        assert!(result.contains("application/pdf"));
        assert!(result.contains("Please review it."));
    }

    #[test]
    fn test_format_document_special_media_types() {
        let test_cases = vec![
            ("image/jpeg", "photo.jpg"),
            ("application/json", "{}"),
            ("text/html", "<html></html>"),
        ];

        for (media_type, data) in test_cases {
            let user = create_test_user(LogMessageContent::Vec(vec![
                LogMessageTaggedContent::Document {
                    source: DocumentSource {
                        r#type: "base64".to_string(),
                        media_type: media_type.to_string(),
                        data: data.to_string(),
                    },
                },
            ]));

            let result = format_user_message(&user);
            assert!(
                result.contains(&format!("Media Type: {}", media_type)),
                "Failed to format media_type: {}",
                media_type
            );
        }
    }

    #[test]
    fn test_format_log_line_user_with_document() {
        let user = create_test_user(LogMessageContent::Vec(vec![
            LogMessageTaggedContent::Document {
                source: DocumentSource {
                    r#type: "base64".to_string(),
                    media_type: "image/png".to_string(),
                    data: "abc123".to_string(),
                },
            },
        ]));

        let log_line = LogLine::User(user);
        let result = format_log_line(&log_line);

        assert!(!result.is_empty());
        assert!(result.contains("📄 Document:"));
        assert!(result.contains("Type: base64"));
    }

    #[test]
    fn test_format_queue_operation() {
        use crate::logs::parser::QueueOperation;
        use chrono::Utc;

        let queue_op = QueueOperation {
            operation: "enqueue".to_string(),
            timestamp: Utc::now(),
            content: Some(serde_json::Value::String(
                "Test operation content".to_string(),
            )),
            session_id: "75c1a8c9-5842-4fd4-a816-74109bf09cba".to_string(),
        };
        let log_line = LogLine::QueueOperation(queue_op.clone());
        let result = format_log_line(&log_line);

        assert!(!result.is_empty());
        assert!(result.contains("Queue Operation"));
        assert!(result.contains("enqueue"));
        assert!(result.contains("75c1a8c9-5842-4fd4-a816-74109bf09cba"));
        assert!(result.contains("Test operation content"));
    }

    #[test]
    fn test_format_log_line_queue_operation() {
        use crate::logs::parser::QueueOperation;
        use chrono::Utc;

        let queue_op = QueueOperation {
            operation: "dequeue".to_string(),
            timestamp: Utc::now(),
            content: Some(serde_json::Value::String("Another test".to_string())),
            session_id: "test-session-id".to_string(),
        };
        let log_line = LogLine::QueueOperation(queue_op);
        let result = format_log_line(&log_line);

        assert!(!result.is_empty());
        assert!(result.contains("dequeue"));
        assert!(result.contains("test-session-id"));
        assert!(result.contains("Another test"));
    }

    #[test]
    fn test_format_system_stop_hook_summary_basic() {
        use crate::logs::parser::{HookInfo, StopHookSummary};

        let system = SystemLogLine::StopHookSummary(StopHookSummary {
            parent_uuid: Uuid::new_v4(),
            is_sidechain: false,
            user_type: "test".to_string(),
            cwd: "/test".to_string(),
            session_id: Uuid::new_v4(),
            version: "1.0.0".to_string(),
            git_branch: "main".to_string(),
            slug: None,
            hook_count: 1,
            hook_infos: vec![HookInfo {
                command: "test-hook".to_string(),
            }],
            hook_errors: vec![],
            prevented_continuation: false,
            stop_reason: String::new(),
            has_output: false,
            level: "info".to_string(),
            timestamp: Utc::now(),
            uuid: Uuid::new_v4(),
            tool_use_id: "test-id".to_string(),
        });
        let result = format_system_message(&system);
        assert!(!result.is_empty());
        assert!(result.contains("Hook Summary"));
        assert!(result.contains("1 hook(s)"));
        assert!(result.contains("test-hook"));
        assert!(!result.contains("Errors:"));
        assert!(!result.contains("Prevented continuation"));
    }

    #[test]
    fn test_format_system_stop_hook_summary_multiple_hooks() {
        use crate::logs::parser::{HookInfo, StopHookSummary};

        let system = SystemLogLine::StopHookSummary(StopHookSummary {
            parent_uuid: Uuid::new_v4(),
            is_sidechain: false,
            user_type: "test".to_string(),
            cwd: "/test".to_string(),
            session_id: Uuid::new_v4(),
            version: "1.0.0".to_string(),
            git_branch: "main".to_string(),
            slug: None,
            hook_count: 3,
            hook_infos: vec![
                HookInfo {
                    command: "hook1".to_string(),
                },
                HookInfo {
                    command: "hook2".to_string(),
                },
                HookInfo {
                    command: "hook3".to_string(),
                },
            ],
            hook_errors: vec![],
            prevented_continuation: false,
            stop_reason: String::new(),
            has_output: false,
            level: "info".to_string(),
            timestamp: Utc::now(),
            uuid: Uuid::new_v4(),
            tool_use_id: "test-id".to_string(),
        });
        let result = format_system_message(&system);
        assert!(result.contains("3 hook(s)"));
        assert!(result.contains("hook1"));
        assert!(result.contains("hook2"));
        assert!(result.contains("hook3"));
    }

    #[test]
    fn test_format_system_stop_hook_summary_with_errors() {
        use crate::logs::parser::{HookError, HookErrorDetails, HookInfo, StopHookSummary};

        let system = SystemLogLine::StopHookSummary(StopHookSummary {
            parent_uuid: Uuid::new_v4(),
            is_sidechain: false,
            user_type: "test".to_string(),
            cwd: "/test".to_string(),
            session_id: Uuid::new_v4(),
            version: "1.0.0".to_string(),
            git_branch: "main".to_string(),
            slug: None,
            hook_count: 1,
            hook_infos: vec![HookInfo {
                command: "failing-hook".to_string(),
            }],
            hook_errors: vec![
                HookError::Structured(HookErrorDetails {
                    message: "Error 1".to_string(),
                    command: Some("failing-hook".to_string()),
                    exit_code: Some(1),
                }),
                HookError::Structured(HookErrorDetails {
                    message: "Error 2".to_string(),
                    command: None,
                    exit_code: None,
                }),
            ],
            prevented_continuation: false,
            stop_reason: String::new(),
            has_output: false,
            level: "error".to_string(),
            timestamp: Utc::now(),
            uuid: Uuid::new_v4(),
            tool_use_id: "test-id".to_string(),
        });
        let result = format_system_message(&system);
        assert!(result.contains("Errors: 2"));
    }

    #[test]
    fn test_format_system_stop_hook_summary_prevented_continuation() {
        use crate::logs::parser::{HookInfo, StopHookSummary};

        let system = SystemLogLine::StopHookSummary(StopHookSummary {
            parent_uuid: Uuid::new_v4(),
            is_sidechain: false,
            user_type: "test".to_string(),
            cwd: "/test".to_string(),
            session_id: Uuid::new_v4(),
            version: "1.0.0".to_string(),
            git_branch: "main".to_string(),
            slug: None,
            hook_count: 1,
            hook_infos: vec![HookInfo {
                command: "blocking-hook".to_string(),
            }],
            hook_errors: vec![],
            prevented_continuation: true,
            stop_reason: "Security policy violation".to_string(),
            has_output: true,
            level: "warning".to_string(),
            timestamp: Utc::now(),
            uuid: Uuid::new_v4(),
            tool_use_id: "test-id".to_string(),
        });
        let result = format_system_message(&system);
        assert!(result.contains("Prevented continuation"));
        assert!(result.contains("Security policy violation"));
    }

    #[test]
    fn test_format_system_stop_hook_summary_prevented_continuation_empty_reason() {
        use crate::logs::parser::{HookInfo, StopHookSummary};

        let system = SystemLogLine::StopHookSummary(StopHookSummary {
            parent_uuid: Uuid::new_v4(),
            is_sidechain: false,
            user_type: "test".to_string(),
            cwd: "/test".to_string(),
            session_id: Uuid::new_v4(),
            version: "1.0.0".to_string(),
            git_branch: "main".to_string(),
            slug: None,
            hook_count: 1,
            hook_infos: vec![HookInfo {
                command: "hook".to_string(),
            }],
            hook_errors: vec![],
            prevented_continuation: true,
            stop_reason: String::new(),
            has_output: false,
            level: "warning".to_string(),
            timestamp: Utc::now(),
            uuid: Uuid::new_v4(),
            tool_use_id: "test-id".to_string(),
        });
        let result = format_system_message(&system);
        assert!(result.contains("Prevented continuation"));
        assert!(result.contains("no reason provided"));
    }
}
