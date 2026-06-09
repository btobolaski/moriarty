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
fn test_parse_pr_link_log_line() {
    let json = serde_json::json!({
        "type": "pr-link",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "prNumber": 76,
        "prUrl": "https://github.com/owner/repo/pull/76",
        "prRepository": "owner/repo",
        "timestamp": "2026-06-03T00:14:43.059Z"
    });
    let LogLine::PrLink(pr_link) = serde_json::from_value(json).unwrap() else {
        panic!("expected a pr-link log line");
    };
    // The camelCase renames are the easy thing to get wrong, so assert each field round-trips.
    assert_eq!(
        pr_link.session_id,
        "550e8400-e29b-41d4-a716-446655440000"
            .parse::<Uuid>()
            .unwrap()
    );
    assert_eq!(pr_link.pr_number, 76);
    assert_eq!(pr_link.pr_url, "https://github.com/owner/repo/pull/76");
    assert_eq!(pr_link.pr_repository, "owner/repo");
    assert_eq!(
        pr_link.timestamp,
        "2026-06-03T00:14:43.059Z".parse::<DateTime<Utc>>().unwrap()
    );
}

#[test]
fn test_parse_pr_link_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "pr-link",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "prNumber": 76,
        "prUrl": "https://github.com/owner/repo/pull/76",
        "prRepository": "owner/repo",
        "timestamp": "2026-06-03T00:14:43.059Z",
        "extraField": "should fail"
    });
    let err = serde_json::from_value::<LogLine>(json).expect_err("Should reject unknown fields");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
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

    let line: LogLine = serde_json::from_value(json).expect("Failed to parse dequeue operation");

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
fn test_parse_file_history_snapshot() {
    let json = serde_json::json!({
        "type": "file-history-snapshot",
        "messageId": "550e8400-e29b-41d4-a716-446655440010",
        "snapshot": {
            "messageId": "550e8400-e29b-41d4-a716-446655440010",
            "trackedFileBackups": {
                "src/main.rs": {"hash": "abc123"}
            },
            "timestamp": "2025-01-01T00:00:00Z"
        },
        "isSnapshotUpdate": false
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse file-history-snapshot");

    match line {
        LogLine::FileHistorySnapshot(snapshot) => {
            assert_eq!(
                snapshot.message_id,
                Uuid::parse_str("550e8400-e29b-41d4-a716-446655440010").unwrap()
            );
            assert!(!snapshot.is_snapshot_update);
            assert!(snapshot
                .snapshot
                .tracked_file_backups
                .contains_key("src/main.rs"));
        }
        _ => panic!("Expected FileHistorySnapshot variant"),
    }
}

#[test]
fn test_parse_file_history_snapshot_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "file-history-snapshot",
        "messageId": "550e8400-e29b-41d4-a716-446655440010",
        "snapshot": {
            "messageId": "550e8400-e29b-41d4-a716-446655440010",
            "trackedFileBackups": {},
            "timestamp": "2025-01-01T00:00:00Z"
        },
        "isSnapshotUpdate": false,
        "unknownField": "should be rejected"
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in file-history-snapshot")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("unknownField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_file_history_snapshot_with_update() {
    let json = serde_json::json!({
        "type": "file-history-snapshot",
        "messageId": "550e8400-e29b-41d4-a716-446655440010",
        "snapshot": {
            "messageId": "550e8400-e29b-41d4-a716-446655440010",
            "trackedFileBackups": {
                "src/lib.rs": {"hash": "def456"}
            },
            "timestamp": "2025-01-01T00:00:00Z"
        },
        "isSnapshotUpdate": true
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse updated file-history-snapshot");

    match line {
        LogLine::FileHistorySnapshot(snapshot) => {
            assert!(snapshot.is_snapshot_update);
            assert!(snapshot
                .snapshot
                .tracked_file_backups
                .contains_key("src/lib.rs"));
        }
        _ => panic!("Expected FileHistorySnapshot variant"),
    }
}

#[test]
fn test_parse_file_history_snapshot_inner_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "file-history-snapshot",
        "messageId": "550e8400-e29b-41d4-a716-446655440010",
        "snapshot": {
            "messageId": "550e8400-e29b-41d4-a716-446655440010",
            "trackedFileBackups": {},
            "timestamp": "2025-01-01T00:00:00Z",
            "unknownField": "should be rejected"
        },
        "isSnapshotUpdate": false
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in file-history-snapshot snapshot")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("unknownField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_summary() {
    let json = serde_json::json!({
        "type": "summary",
        "summary": "Condensed conversation summary",
        "leafUuid": "550e8400-e29b-41d4-a716-446655440011"
    });

    let line: LogLine = serde_json::from_value(json).expect("Failed to parse summary");

    match line {
        LogLine::Summary(summary) => {
            assert_eq!(summary.summary, "Condensed conversation summary");
            assert_eq!(
                summary.leaf_uuid,
                Uuid::parse_str("550e8400-e29b-41d4-a716-446655440011").unwrap()
            );
        }
        _ => panic!("Expected Summary variant"),
    }
}

#[test]
fn test_parse_summary_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "summary",
        "summary": "Condensed conversation summary",
        "leafUuid": "550e8400-e29b-41d4-a716-446655440011",
        "unknownField": "should be rejected"
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in summary")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("unknownField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
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
        assert_eq!(assistant.message.model.raw(), "<synthetic>");
        assert_eq!(assistant.message.context_management, None);
        // This turn predates Claude Code 2.1.158, so the new API-error status must be absent.
        assert_eq!(assistant.api_error_status, None);
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

// Synthetic API-error assistant turn (Claude Code 2.1.158) carrying error type + HTTP status.
#[test]
fn test_parse_assistant_api_error_message_with_status() {
    let json = serde_json::json!({
        "parentUuid": "92511969-25ff-4e15-8b0e-705cb1a6df59",
        "isSidechain": false,
        "type": "assistant",
        "uuid": "2201f52c-7e6a-4415-8a94-1bbafcbd3747",
        "timestamp": "2026-06-05T15:39:29.956Z",
        "message": {
            "id": "b33613b3-4af5-4202-b471-0f290ba1a955",
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
                "server_tool_use": {"web_search_requests": 0, "web_fetch_requests": 0},
                "service_tier": null,
                "cache_creation": {"ephemeral_1h_input_tokens": 0, "ephemeral_5m_input_tokens": 0}
            },
            "content": [{"type": "text", "text": "API Error: 529 Overloaded."}],
            "context_management": null
        },
        "requestId": "req_011CbkMobZe6EibUaraVrDUU",
        "error": "server_error",
        "isApiErrorMessage": true,
        "apiErrorStatus": 529,
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "897f641d-35f9-4a70-8b47-f3c8f3d9e308",
        "version": "2.1.158",
        "gitBranch": "HEAD",
        "slug": "synchronous-sparking-scone"
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse api-error assistant message");

    match line {
        LogLine::Assistant(assistant) => {
            assert_eq!(assistant.is_api_error_message, Some(true));
            assert_eq!(assistant.error.as_deref(), Some("server_error"));
            assert_eq!(assistant.api_error_status, Some(529));
        }
        _ => panic!("Expected Assistant variant"),
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
        assert_eq!(assistant.message.model.raw(), "claude-3-5-sonnet");
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
fn test_parse_scheduled_task_fire() {
    let json = serde_json::json!({
        "parentUuid": "eee9f696-e699-4606-873c-3134cfe5a284",
        "isSidechain": false,
        "type": "system",
        "subtype": "scheduled_task_fire",
        "content": "Claude resuming /loop wakeup (Jun 1 10:45am)",
        "isMeta": false,
        "timestamp": "2026-06-01T15:45:52.142Z",
        "uuid": "ac7c4318-679d-45c7-8d86-3ca6934f8611",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/Users/brendan/src/switchboard-jj",
        "sessionId": "2883cea4-f496-44b6-a291-354d7e39bdc6",
        "version": "2.1.141",
        "gitBranch": "HEAD",
        "slug": "we-need-to-build-mutable-hamming"
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse scheduled_task_fire system message");

    match line {
        LogLine::System(SystemLogLine::ScheduledTaskFire(fire)) => {
            assert_eq!(fire.content, "Claude resuming /loop wakeup (Jun 1 10:45am)");
            assert_eq!(fire.entrypoint.as_deref(), Some("cli"));
            assert!(!fire.is_meta);
        }
        _ => panic!("Expected System(ScheduledTaskFire) variant"),
    }
}

#[test]
fn test_parse_scheduled_task_fire_rejects_unknown_fields() {
    let json = serde_json::json!({
        "parentUuid": "eee9f696-e699-4606-873c-3134cfe5a284",
        "isSidechain": false,
        "type": "system",
        "subtype": "scheduled_task_fire",
        "content": "Claude resuming /loop wakeup (Jun 1 10:45am)",
        "isMeta": false,
        "timestamp": "2026-06-01T15:45:52.142Z",
        "uuid": "ac7c4318-679d-45c7-8d86-3ca6934f8611",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/Users/brendan/src/switchboard-jj",
        "sessionId": "2883cea4-f496-44b6-a291-354d7e39bdc6",
        "version": "2.1.141",
        "gitBranch": "HEAD",
        "slug": "we-need-to-build-mutable-hamming",
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
            assert!(!summary.prevented_continuation);
            assert_eq!(summary.stop_reason, "");
            assert!(!summary.has_output);
            assert_eq!(summary.level, "suggestion");
            assert_eq!(summary.tool_use_id, "8f3746a9-caa9-4d2d-8e6e-e7a7b005d5d4");
        }
        _ => panic!("Expected System(StopHookSummary) variant"),
    }
}

// `hookAdditionalContext` (Claude Code 2.1.170+) has only ever been observed empty; the
// empty array parses, and a populated element must break parsing (the `()` element type
// only accepts JSON null) so an unmodeled real payload surfaces as a partial failure.
#[test]
fn test_parse_stop_hook_summary_with_hook_additional_context() {
    let base = serde_json::json!({
        "parentUuid": "5445927e-82b0-4164-91f3-782fafd2a49e",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "1a55057c-6af4-4c76-83a1-70b738990294",
        "version": "2.1.170",
        "gitBranch": "main",
        "type": "system",
        "subtype": "stop_hook_summary",
        "hookCount": 1,
        "hookInfos": [{"command": "moriarty hooks exec", "durationMs": 24}],
        "hookErrors": [],
        "hookAdditionalContext": [],
        "preventedContinuation": false,
        "stopReason": "",
        "hasOutput": true,
        "level": "suggestion",
        "timestamp": "2026-06-09T19:32:49.551Z",
        "uuid": "abc0350d-cc85-4624-ac9d-99dae25063a6",
        "toolUseID": "a311cdc8-d81f-42c0-b3e3-a481280f607a"
    });

    match serde_json::from_value::<LogLine>(base.clone())
        .expect("Failed to parse stop_hook_summary with empty hookAdditionalContext")
    {
        LogLine::System(SystemLogLine::StopHookSummary(summary)) => {
            assert_eq!(summary.hook_additional_context, Some(vec![]));
        }
        _ => panic!("Expected System(StopHookSummary) variant"),
    }

    let mut populated = base;
    populated["hookAdditionalContext"] = serde_json::json!([{"context": "surprise"}]);
    let err_msg = serde_json::from_value::<LogLine>(populated)
        .expect_err("populated hookAdditionalContext parsed but should have failed")
        .to_string();
    assert!(
        err_msg.contains("hookAdditionalContext") || err_msg.contains("unit"),
        "populated hookAdditionalContext should fail to parse, got: {err_msg}"
    );
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
    let error: HookError = serde_json::from_value(serde_json::json!("Error message")).unwrap();
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
fn test_parse_hook_info_with_duration_ms() {
    let json = serde_json::json!({
        "command": "test-hook",
        "durationMs": 1500
    });
    let info: HookInfo = serde_json::from_value(json).unwrap();
    assert_eq!(info.command, "test-hook");
    assert_eq!(info.duration_ms, Some(1500));
}

#[test]
fn test_parse_hook_info_without_duration_ms() {
    let json = serde_json::json!({
        "command": "test-hook"
    });
    let info: HookInfo = serde_json::from_value(json).unwrap();
    assert_eq!(info.command, "test-hook");
    assert_eq!(info.duration_ms, None);
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
            assert!(summary.prevented_continuation);
            assert_eq!(summary.stop_reason, "Multiple hooks failed");
            assert!(summary.has_output);
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

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse stop_hook_summary with empty arrays");

    match line {
        LogLine::System(SystemLogLine::StopHookSummary(summary)) => {
            assert_eq!(summary.hook_count, 0);
            assert_eq!(summary.hook_infos.len(), 0);
            assert_eq!(summary.hook_errors.len(), 0);
            assert!(!summary.prevented_continuation);
            assert!(!summary.has_output);
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

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse stop_hook_summary with string errors");

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

#[test]
fn test_parse_model_refusal_fallback() {
    let json = serde_json::json!({
        "type": "system",
        "subtype": "model_refusal_fallback",
        "parentUuid": "77502799-98d4-4548-b903-ed5d6f797e41",
        "isSidechain": false,
        "direction": "retry",
        "content": "Fable 5's safety measures flagged this message. Switched to Opus 4.8.",
        "level": "warning",
        "trigger": "refusal",
        "originalModel": "claude-fable-5",
        "fallbackModel": "claude-opus-4-8",
        "requestId": "req_011CbtEUxmnDLZxNhMjZT5dt",
        "apiRefusalCategory": null,
        "apiRefusalExplanation": null,
        "isMeta": false,
        "timestamp": "2026-06-09T19:24:49.832Z",
        "uuid": "6e45b19e-8f68-4144-9eac-c1577fe3737e",
        "retractedMessageUuids": ["6102750b-5a74-4578-bf67-d42e5b5f85ee"],
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "f671f20e-5ef4-41d5-bfe5-aa4b87a2bd54",
        "version": "2.1.170",
        "gitBranch": "HEAD",
        "slug": "i-need-to-run-hashed-stroustrup"
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse model_refusal_fallback");

    match line {
        LogLine::System(SystemLogLine::ModelRefusalFallback(fallback)) => {
            assert_eq!(fallback.direction, "retry");
            assert_eq!(fallback.trigger, "refusal");
            assert_eq!(fallback.original_model.raw(), "claude-fable-5");
            assert_eq!(fallback.fallback_model.raw(), "claude-opus-4-8");
            assert_eq!(fallback.api_refusal_category, None);
            assert_eq!(fallback.api_refusal_explanation, None);
            assert_eq!(fallback.retracted_message_uuids.len(), 1);
        }
        _ => panic!("Expected System(ModelRefusalFallback) variant"),
    }
}

#[test]
fn test_parse_model_refusal_fallback_with_populated_refusal_details() {
    let json = serde_json::json!({
        "type": "system",
        "subtype": "model_refusal_fallback",
        "parentUuid": "77502799-98d4-4548-b903-ed5d6f797e41",
        "isSidechain": false,
        "direction": "retry",
        "content": "Switched to Opus 4.8.",
        "level": "warning",
        "trigger": "refusal",
        "originalModel": "claude-fable-5",
        "fallbackModel": "claude-opus-4-8",
        "requestId": "req_011CbtEUxmnDLZxNhMjZT5dt",
        "apiRefusalCategory": "cyber",
        "apiRefusalExplanation": "Flagged for cybersecurity topics.",
        "isMeta": false,
        "timestamp": "2026-06-09T19:24:49.832Z",
        "uuid": "6e45b19e-8f68-4144-9eac-c1577fe3737e",
        "retractedMessageUuids": [],
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "f671f20e-5ef4-41d5-bfe5-aa4b87a2bd54",
        "version": "2.1.170",
        "gitBranch": "HEAD"
    });

    match serde_json::from_value::<LogLine>(json)
        .expect("Failed to parse model_refusal_fallback with populated refusal details")
    {
        LogLine::System(SystemLogLine::ModelRefusalFallback(fallback)) => {
            assert_eq!(fallback.api_refusal_category.as_deref(), Some("cyber"));
            assert_eq!(
                fallback.api_refusal_explanation.as_deref(),
                Some("Flagged for cybersecurity topics.")
            );
            assert!(fallback.retracted_message_uuids.is_empty());
        }
        _ => panic!("Expected System(ModelRefusalFallback) variant"),
    }
}

#[test]
fn test_parse_model_refusal_fallback_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "system",
        "subtype": "model_refusal_fallback",
        "parentUuid": "77502799-98d4-4548-b903-ed5d6f797e41",
        "isSidechain": false,
        "direction": "retry",
        "content": "Switched to Opus 4.8.",
        "level": "warning",
        "trigger": "refusal",
        "originalModel": "claude-fable-5",
        "fallbackModel": "claude-opus-4-8",
        "requestId": "req_011CbtEUxmnDLZxNhMjZT5dt",
        "apiRefusalCategory": null,
        "apiRefusalExplanation": null,
        "isMeta": false,
        "timestamp": "2026-06-09T19:24:49.832Z",
        "uuid": "6e45b19e-8f68-4144-9eac-c1577fe3737e",
        "retractedMessageUuids": [],
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "f671f20e-5ef4-41d5-bfe5-aa4b87a2bd54",
        "version": "2.1.170",
        "gitBranch": "HEAD",
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
fn test_parse_turn_duration() {
    let json = serde_json::json!({
        "type": "system",
        "subtype": "turn_duration",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.0.51",
        "gitBranch": "main",
        "slug": "noble-floating-lemon",
        "durationMs": 1234,
        "timestamp": "2025-01-16T00:00:00Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "isMeta": false
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse turn_duration system message");

    match line {
        LogLine::System(SystemLogLine::TurnDuration(duration)) => {
            assert_eq!(duration.duration_ms, 1234);
            assert_eq!(duration.slug, Some("noble-floating-lemon".to_string()));
            assert_eq!(duration.version, "2.0.51");
            assert!(!duration.is_meta);
        }
        _ => panic!("Expected System(TurnDuration) variant"),
    }
}

#[test]
fn test_parse_turn_duration_without_slug() {
    let json = serde_json::json!({
        "type": "system",
        "subtype": "turn_duration",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.0.50",
        "gitBranch": "main",
        "durationMs": 5678,
        "timestamp": "2025-01-16T00:00:00Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "isMeta": true
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Should parse turn_duration without slug field");

    match line {
        LogLine::System(SystemLogLine::TurnDuration(duration)) => {
            assert_eq!(duration.duration_ms, 5678);
            assert_eq!(duration.slug, None);
        }
        _ => panic!("Expected System(TurnDuration) variant"),
    }
}

#[test]
fn test_parse_turn_duration_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "system",
        "subtype": "turn_duration",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.0.51",
        "gitBranch": "main",
        "durationMs": 1234,
        "timestamp": "2025-01-16T00:00:00Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "isMeta": false,
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
fn test_parse_system_log_error() {
    let json = serde_json::json!({
        "type": "system",
        "subtype": "error",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "non-uuid-session-id",
        "version": "2.0.42",
        "gitBranch": "main",
        "level": "error",
        "cause": {"message": "upstream"},
        "error": {"requestID": "req_abc123"},
        "retryInMs": 1000.0,
        "retryAttempt": 1,
        "maxRetries": 3,
        "timestamp": "2025-01-01T00:00:00Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440001"
    });

    let line: LogLine = serde_json::from_value(json).expect("Failed to parse system error");

    match line {
        LogLine::System(SystemLogLine::Error(error)) => {
            assert_eq!(error.session_id, "non-uuid-session-id");
            assert_eq!(error.retry_in_ms, 1000.0);
            assert_eq!(error.retry_attempt, 1);
            assert_eq!(error.max_retries, 3);
            match &error.error {
                SystemErrorBody::Sdk(sdk) => {
                    assert_eq!(sdk.request_id.as_deref(), Some("req_abc123"));
                }
                other => panic!("Expected SDK error envelope, got {other:?}"),
            }
            assert!(error.cause.is_some());
        }
        _ => panic!("Expected System(Error) variant"),
    }
}

#[test]
fn test_parse_system_log_api_error() {
    let json = serde_json::json!({
        "type": "system",
        "subtype": "api_error",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "non-uuid-session-id",
        "version": "2.0.42",
        "gitBranch": "main",
        "level": "error",
        "error": {"requestID": "req_api_123", "status": 429, "headers": {"retry-after": "5"}},
        "retryInMs": 250.5,
        "retryAttempt": 2,
        "maxRetries": 5,
        "timestamp": "2025-01-01T00:00:00Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440003"
    });

    let line: LogLine = serde_json::from_value(json).expect("Failed to parse api_error");

    match line {
        LogLine::System(SystemLogLine::ApiError(error)) => {
            assert_eq!(error.session_id, "non-uuid-session-id");
            assert_eq!(error.retry_in_ms, 250.5);
            assert_eq!(error.retry_attempt, 2);
            assert_eq!(error.max_retries, 5);
            match &error.error {
                SystemErrorBody::Sdk(sdk) => {
                    assert_eq!(sdk.request_id.as_deref(), Some("req_api_123"));
                    assert_eq!(sdk.status, Some(429));
                    assert_eq!(
                        sdk.headers
                            .as_ref()
                            .and_then(|headers| headers.get("retry-after"))
                            .map(String::as_str),
                        Some("5")
                    );
                }
                other => panic!("Expected SDK error envelope, got {other:?}"),
            }
        }
        _ => panic!("Expected System(ApiError) variant"),
    }
}

// The networking-layer error envelope emitted by Claude Code 2.1.158 (real overloaded_error line).
#[test]
fn test_parse_system_log_api_error_client_envelope() {
    let json = serde_json::json!({
        "type": "system",
        "subtype": "api_error",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "non-uuid-session-id",
        "version": "2.1.158",
        "gitBranch": "HEAD",
        "level": "error",
        "error": {
            "message": "529 {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"},\"request_id\":\"req_011CbkMooev8tewDZ9JGCJ92\"}",
            "status": 529,
            "requestId": "req_011CbkMooev8tewDZ9JGCJ92",
            "formatted": "529 Overloaded",
            "connection": null,
            "isNetworkDown": false,
            "rateLimits": null
        },
        "retryInMs": 511.07263673020685,
        "retryAttempt": 1,
        "maxRetries": 10,
        "timestamp": "2026-06-05T15:36:05.139Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440009"
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse api_error client envelope");

    match line {
        LogLine::System(SystemLogLine::ApiError(error)) => {
            assert_eq!(error.retry_attempt, 1);
            assert_eq!(error.max_retries, 10);
            match &error.error {
                SystemErrorBody::Client(client) => {
                    assert_eq!(client.status, Some(529));
                    assert_eq!(
                        client.request_id.as_deref(),
                        Some("req_011CbkMooev8tewDZ9JGCJ92")
                    );
                    assert_eq!(client.formatted, "529 Overloaded");
                    assert!(client.message.starts_with("529 "));
                    assert!(!client.is_network_down);
                    assert_eq!(client.connection, None);
                    assert_eq!(client.rate_limits, None);
                }
                other => panic!("Expected client error envelope, got {other:?}"),
            }
        }
        _ => panic!("Expected System(ApiError) variant"),
    }
}

// Both `error` and `api_error` subtypes deserialize into `SystemLogError`, so the Client envelope
// must route correctly under subtype "error" too — not only under "api_error".
#[test]
fn test_parse_system_log_error_client_envelope() {
    let json = serde_json::json!({
        "type": "system",
        "subtype": "error",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "non-uuid-session-id",
        "version": "2.1.158",
        "gitBranch": "HEAD",
        "level": "error",
        "error": {
            "message": "529 Overloaded",
            "status": 529,
            "requestId": "req_err_123",
            "formatted": "529 Overloaded",
            "connection": null,
            "isNetworkDown": false,
            "rateLimits": null
        },
        "retryInMs": 511.0,
        "retryAttempt": 1,
        "maxRetries": 10,
        "timestamp": "2026-06-05T15:36:05.139Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440009"
    });

    match serde_json::from_value::<LogLine>(json).expect("Failed to parse error client envelope") {
        LogLine::System(SystemLogLine::Error(error)) => match &error.error {
            SystemErrorBody::Client(client) => {
                assert_eq!(client.status, Some(529));
                assert_eq!(client.request_id.as_deref(), Some("req_err_123"));
            }
            other => panic!("Expected client error envelope, got {other:?}"),
        },
        _ => panic!("Expected System(Error) variant"),
    }
}

// A request timeout fails before any HTTP response exists, so the Client envelope arrives
// without `status`/`requestId`; it must still resolve to the Client variant (not Sdk).
#[test]
fn test_parse_system_log_api_error_timeout_client_envelope() {
    let json = serde_json::json!({
        "type": "system",
        "subtype": "api_error",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "non-uuid-session-id",
        "version": "2.1.158",
        "gitBranch": "HEAD",
        "level": "error",
        "error": {
            "message": "Request timed out.",
            "formatted": "Request timed out.",
            "connection": null,
            "isNetworkDown": false,
            "rateLimits": null
        },
        "retryInMs": 542.2537521358778,
        "retryAttempt": 1,
        "maxRetries": 10,
        "timestamp": "2026-06-09T19:41:10.971Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440009"
    });

    match serde_json::from_value::<LogLine>(json).expect("Failed to parse timeout client envelope")
    {
        LogLine::System(SystemLogLine::ApiError(error)) => match &error.error {
            SystemErrorBody::Client(client) => {
                assert_eq!(client.message, "Request timed out.");
                assert_eq!(client.status, None);
                assert_eq!(client.request_id, None);
                assert!(!client.is_network_down);
            }
            other => panic!("Expected client error envelope, got {other:?}"),
        },
        _ => panic!("Expected System(ApiError) variant"),
    }
}

// The 1-occurrence SDK shape carrying only `cause` must still resolve to the SDK variant; this
// also guards that listing `Client` first does not swallow envelopes lacking its required fields.
#[test]
fn test_parse_system_log_api_error_cause_only_envelope() {
    let json = serde_json::json!({
        "type": "system",
        "subtype": "api_error",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "non-uuid-session-id",
        "version": "2.1.158",
        "gitBranch": "HEAD",
        "level": "error",
        "error": {"cause": {"code": "ECONNRESET"}},
        "retryInMs": 511.0,
        "retryAttempt": 1,
        "maxRetries": 10,
        "timestamp": "2026-06-05T15:36:05.139Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440009"
    });

    let line: LogLine = serde_json::from_value(json).expect("Failed to parse cause-only api_error");

    match line {
        LogLine::System(SystemLogLine::ApiError(error)) => match &error.error {
            SystemErrorBody::Sdk(sdk) => {
                assert!(sdk.cause.is_some());
                assert_eq!(sdk.status, None);
                assert_eq!(sdk.request_id, None);
                assert_eq!(sdk.headers, None);
            }
            other => panic!("Expected SDK error envelope, got {other:?}"),
        },
        _ => panic!("Expected System(ApiError) variant"),
    }
}

// `rateLimits` has only ever been observed as null, and `connection` only as null or the modeled
// socket-diagnostics shape; any other populated value is an unmodeled shape that must fail to
// parse (surfacing as a partial failure) rather than be dropped.
#[test]
fn test_parse_system_log_api_error_populated_diagnostics_break() {
    // Wrap a given `error` payload in an otherwise-valid system api_error line.
    let line_with = |error_obj: serde_json::Value| {
        serde_json::json!({
            "type": "system",
            "subtype": "api_error",
            "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
            "isSidechain": false,
            "userType": "external",
            "cwd": "/test",
            "sessionId": "non-uuid-session-id",
            "version": "2.1.158",
            "gitBranch": "HEAD",
            "level": "error",
            "error": error_obj,
            "retryInMs": 511.0,
            "retryAttempt": 1,
            "maxRetries": 10,
            "timestamp": "2026-06-05T15:36:05.139Z",
            "uuid": "550e8400-e29b-41d4-a716-446655440009"
        })
    };
    let base_error = serde_json::json!({
        "message": "503 Service Unavailable",
        "status": 503,
        "requestId": "req_x",
        "formatted": "503 Service Unavailable",
        "connection": null,
        "isNetworkDown": true,
        "rateLimits": null
    });

    // The all-null baseline parses (and exercises `isNetworkDown: true`); the only difference in the
    // failing cases below is the populated field, which proves that field is what breaks parsing.
    match serde_json::from_value::<LogLine>(line_with(base_error.clone()))
        .expect("baseline client envelope should parse")
    {
        LogLine::System(SystemLogLine::ApiError(error)) => match error.error {
            SystemErrorBody::Client(client) => assert!(client.is_network_down),
            other => panic!("Expected client error envelope, got {other:?}"),
        },
        _ => panic!("Expected System(ApiError) variant"),
    }

    // Any value other than null (or, for `connection`, the modeled socket-diagnostics shape)
    // must break parsing.
    for field in ["connection", "rateLimits"] {
        for value in [
            serde_json::json!({"unexpected": "shape"}),
            serde_json::json!("oops"),
            serde_json::json!(1),
        ] {
            let mut error = base_error.clone();
            error[field] = value;
            let err = serde_json::from_value::<LogLine>(line_with(error))
                .expect_err(&format!("populated {field} parsed but should have failed"))
                .to_string();
            // The populated value matches neither envelope, so disambiguation of the untagged
            // `SystemErrorBody` is what fails — not some unrelated field.
            assert!(
                err.contains("SystemErrorBody"),
                "populated {field} should fail SystemErrorBody disambiguation, got: {err}"
            );
        }
    }
}

// Connection-level failures (Claude Code 2.1.170+) populate `connection` with socket
// diagnostics instead of null; the envelope must still resolve to the Client variant.
#[test]
fn test_parse_system_log_api_error_populated_connection() {
    let json = serde_json::json!({
        "type": "system",
        "subtype": "api_error",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "non-uuid-session-id",
        "version": "2.1.170",
        "gitBranch": "HEAD",
        "level": "error",
        "error": {
            "message": "Connection error.",
            "formatted": "Unable to connect to API (ECONNRESET)",
            "connection": {
                "code": "ECONNRESET",
                "message": "The socket connection was closed unexpectedly. For more information, pass `verbose: true` in the second argument to fetch()",
                "isSSLError": false
            },
            "isNetworkDown": false,
            "rateLimits": null
        },
        "retryInMs": 588.0813039877115,
        "retryAttempt": 1,
        "maxRetries": 10,
        "timestamp": "2026-06-09T20:47:11.941Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440009"
    });

    let line = serde_json::from_value::<LogLine>(json)
        .expect("Failed to parse populated-connection client envelope");
    match &line {
        LogLine::System(SystemLogLine::ApiError(error)) => match &error.error {
            SystemErrorBody::Client(client) => {
                assert_eq!(client.message, "Connection error.");
                assert_eq!(client.status, None);
                assert_eq!(client.request_id, None);
                let connection = client
                    .connection
                    .as_ref()
                    .expect("connection diagnostics should be populated");
                assert_eq!(connection.code, "ECONNRESET");
                assert_eq!(
                    connection.message,
                    "The socket connection was closed unexpectedly. For more information, pass \
                     `verbose: true` in the second argument to fetch()"
                );
                assert!(!connection.is_ssl_error);
            }
            other => panic!("Expected client error envelope, got {other:?}"),
        },
        _ => panic!("Expected System(ApiError) variant"),
    }

    // The camelCase renames are the easy thing to get wrong, and `isSSLError` needs an explicit
    // `rename` because `rename_all = "camelCase"` would emit `isSslError`; pin the outbound key.
    let reserialized = serde_json::to_value(&line).expect("reserialize should succeed");
    let connection = &reserialized["error"]["connection"];
    assert_eq!(connection["code"], "ECONNRESET");
    assert_eq!(connection["isSSLError"], false);
    assert!(
        connection.get("isSslError").is_none(),
        "outbound key must be the pinned `isSSLError`, not the rename_all default"
    );
}

// SSL failures flip the only boolean in the diagnostics, so pin the `true` reading too, and pin
// that an unknown sibling key still breaks parsing (the struct is strict like its peers).
#[test]
fn test_parse_system_error_connection_ssl_and_unknown_fields() {
    let connection: SystemErrorConnection = serde_json::from_value(serde_json::json!({
        "code": "ERR_TLS_CERT_ALTNAME_INVALID",
        "message": "Hostname/IP does not match certificate's altnames",
        "isSSLError": true
    }))
    .expect("SSL connection diagnostics should parse");
    assert_eq!(connection.code, "ERR_TLS_CERT_ALTNAME_INVALID");
    assert!(connection.is_ssl_error);

    let err = serde_json::from_value::<SystemErrorConnection>(serde_json::json!({
        "code": "ECONNRESET",
        "message": "The socket connection was closed unexpectedly.",
        "isSSLError": false,
        "extraField": 1
    }))
    .expect_err("unknown connection field should fail to parse")
    .to_string();
    assert!(
        err.contains("extraField"),
        "error should name the unknown field, got: {err}"
    );
}

#[test]
fn test_parse_system_log_informational_without_git_branch() {
    let json = serde_json::json!({
        "type": "system",
        "subtype": "informational",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.0.0",
        "content": "Session started",
        "isMeta": false,
        "level": "info",
        "timestamp": "2025-01-01T00:00:00Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440002"
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse informational system message");

    match line {
        LogLine::System(SystemLogLine::Informational(info)) => {
            assert_eq!(info.git_branch, None);
            assert_eq!(info.content, "Session started");
            assert!(!info.is_meta);
        }
        _ => panic!("Expected System(Informational) variant"),
    }
}

#[test]
fn test_parse_system_log_error_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "system",
        "subtype": "error",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "non-uuid-session-id",
        "version": "2.0.42",
        "gitBranch": "main",
        "level": "error",
        "error": {"requestID": "req_abc123"},
        "retryInMs": 1000.0,
        "retryAttempt": 1,
        "maxRetries": 3,
        "timestamp": "2025-01-01T00:00:00Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "unknownField": "should be rejected"
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in SystemLogError")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("unknownField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_system_log_api_error_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "system",
        "subtype": "api_error",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "non-uuid-session-id",
        "version": "2.0.42",
        "gitBranch": "main",
        "level": "error",
        "error": {"requestID": "req_api_123", "status": 429},
        "retryInMs": 250.5,
        "retryAttempt": 2,
        "maxRetries": 5,
        "timestamp": "2025-01-01T00:00:00Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440003",
        "unknownField": "should be rejected"
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in SystemLogError api_error variant")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("unknownField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_system_log_informational_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "system",
        "subtype": "informational",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.0.0",
        "content": "Session started",
        "isMeta": false,
        "level": "info",
        "timestamp": "2025-01-01T00:00:00Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "unknownField": "should be rejected"
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in SystemLogInformational")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("unknownField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_user_log_line_with_source_tool_assistant_uuid() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.0.51",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "sourceToolAssistantUUID": "550e8400-e29b-41d4-a716-446655440099"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(
        line.source_tool_assistant_uuid,
        Some(Uuid::parse_str("550e8400-e29b-41d4-a716-446655440099").unwrap())
    );
}

#[test]
fn test_parse_user_log_line_with_null_source_tool_assistant_uuid() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.0.51",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "sourceToolAssistantUUID": null
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.source_tool_assistant_uuid, None);
}

#[test]
fn test_parse_user_log_line_without_source_tool_assistant_uuid() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.0.50",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.source_tool_assistant_uuid, None);
}

#[test]
fn test_parse_progress_hook_progress() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "slug": "test-slug",
        "type": "progress",
        "data": {
            "type": "hook_progress",
            "hookEvent": "PreToolUse",
            "hookName": "PreToolUse:Bash",
            "command": "moriarty hooks exec"
        },
        "toolUseID": "toolu_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T21:54:19.450Z"
    });

    let line: LogLine = serde_json::from_value(json).expect("Failed to parse hook_progress");

    match line {
        LogLine::Progress(progress) => {
            assert_eq!(progress.tool_use_id, "toolu_test");
            match progress.data {
                ProgressData::HookProgress(data) => {
                    assert_eq!(data.hook_event, "PreToolUse");
                    assert_eq!(data.hook_name, "PreToolUse:Bash");
                }
                _ => panic!("Expected HookProgress variant"),
            }
        }
        _ => panic!("Expected Progress variant"),
    }
}

#[test]
fn test_parse_progress_mcp_progress() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "type": "progress",
        "data": {
            "type": "mcp_progress",
            "status": "completed",
            "serverName": "git-read-only",
            "toolName": "show",
            "elapsedTimeMs": 9
        },
        "toolUseID": "toolu_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T21:55:09.748Z"
    });

    let line: LogLine = serde_json::from_value(json).expect("Failed to parse mcp_progress");

    match line {
        LogLine::Progress(progress) => match progress.data {
            ProgressData::McpProgress(data) => {
                assert_eq!(data.status, "completed");
                assert_eq!(data.server_name, "git-read-only");
                assert_eq!(data.elapsed_time_ms, Some(9));
            }
            _ => panic!("Expected McpProgress variant"),
        },
        _ => panic!("Expected Progress variant"),
    }
}

#[test]
fn test_parse_progress_agent_progress_with_assistant_message() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "slug": "test-slug",
        "type": "progress",
        "data": {
            "type": "agent_progress",
            "message": {
                "type": "user",
                "timestamp": "2026-01-18T21:43:02.787Z",
                "message": {"role": "user", "content": "test"},
                "uuid": "550e8400-e29b-41d4-a716-446655440004"
            },
            "normalizedMessages": [
                {
                    "type": "assistant",
                    "timestamp": "2026-01-18T21:54:47.639Z",
                    "message": {
                        "model": "claude-opus-4-5-20251101",
                        "id": "msg_test",
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "text", "text": "test"}],
                        "stop_reason": null,
                        "stop_sequence": null,
                        "usage": {
                            "input_tokens": 3,
                            "cache_creation_input_tokens": 100,
                            "cache_read_input_tokens": 0,
                            "cache_creation": {
                                "ephemeral_5m_input_tokens": 100,
                                "ephemeral_1h_input_tokens": 0
                            },
                            "output_tokens": 1,
                            "service_tier": "standard"
                        },
                        "context_management": null
                    },
                    "requestId": "req_test",
                    "uuid": "550e8400-e29b-41d4-a716-446655440003"
                },
                {
                    "type": "progress",
                    "data": {
                        "type": "hook_progress",
                        "hookEvent": "PreToolUse",
                        "hookName": "PreToolUse:Bash",
                        "command": "moriarty hooks exec"
                    },
                    "toolUseID": "toolu_test",
                    "parentToolUseID": "toolu_parent",
                    "uuid": "550e8400-e29b-41d4-a716-446655440005",
                    "timestamp": "2026-01-18T21:43:02.698Z"
                },
                {
                    "type": "attachment",
                    "attachment": {"type": "hook_success", "hookName": "test"},
                    "uuid": "550e8400-e29b-41d4-a716-446655440006",
                    "timestamp": "2026-01-18T21:43:02.724Z"
                },
                {
                    "type": "user",
                    "message": {"role": "user", "content": [{"tool_use_id": "test", "type": "tool_result", "content": "No files found"}]},
                    "uuid": "550e8400-e29b-41d4-a716-446655440007",
                    "timestamp": "2026-01-18T21:43:02.787Z",
                    "toolUseResult": {"filenames": [], "durationMs": 38}
                }
            ],
            "prompt": "test prompt",
            "agentId": "abc123"
        },
        "toolUseID": "agent_msg_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T21:54:47.655Z"
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse agent_progress with assistant");

    match line {
        LogLine::Progress(progress) => match progress.data {
            ProgressData::AgentProgress(data) => {
                assert_eq!(data.agent_id, "abc123");
                assert_eq!(data.prompt, "test prompt");
                assert_eq!(data.normalized_messages.as_ref().unwrap().len(), 4);
            }
            _ => panic!("Expected AgentProgress variant"),
        },
        _ => panic!("Expected Progress variant"),
    }
}

#[test]
fn test_parse_progress_bash_progress() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "type": "progress",
        "data": {
            "type": "bash_progress",
            "output": "Running command...",
            "fullOutput": "Running command...\nProcessing...",
            "elapsedTimeSeconds": 5,
            "totalLines": 2
        },
        "toolUseID": "bash-progress-0",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T21:55:09.748Z"
    });

    let line: LogLine = serde_json::from_value(json).expect("Failed to parse bash_progress");

    match line {
        LogLine::Progress(progress) => match progress.data {
            ProgressData::BashProgress(data) => {
                assert_eq!(data.output, "Running command...");
                assert_eq!(data.full_output, "Running command...\nProcessing...");
                assert_eq!(data.elapsed_time_seconds, 5);
                assert_eq!(data.total_lines, 2);
            }
            _ => panic!("Expected BashProgress variant"),
        },
        _ => panic!("Expected Progress variant"),
    }
}

#[test]
fn test_parse_progress_waiting_for_task() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "type": "progress",
        "data": {
            "type": "waiting_for_task",
            "taskDescription": "Check if all files parse correctly now",
            "taskType": "local_bash"
        },
        "toolUseID": "task-output-waiting",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T22:17:23.813Z"
    });

    let line: LogLine = serde_json::from_value(json).expect("Failed to parse waiting_for_task");

    match line {
        LogLine::Progress(progress) => match progress.data {
            ProgressData::WaitingForTask(data) => {
                assert_eq!(
                    data.task_description,
                    "Check if all files parse correctly now"
                );
                assert_eq!(data.task_type, "local_bash");
            }
            _ => panic!("Expected WaitingForTask variant"),
        },
        _ => panic!("Expected Progress variant"),
    }
}

#[test]
fn test_parse_progress_query_update() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "type": "progress",
        "data": {
            "type": "query_update",
            "query": "rust fs-err crate lock unlock file documentation 2026"
        },
        "toolUseID": "query-update-id",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T22:17:23.813Z"
    });

    let line: LogLine = serde_json::from_value(json).expect("Failed to parse query_update");

    match line {
        LogLine::Progress(progress) => match progress.data {
            ProgressData::QueryUpdate(data) => {
                assert_eq!(
                    data.query,
                    "rust fs-err crate lock unlock file documentation 2026"
                );
            }
            _ => panic!("Expected QueryUpdate variant"),
        },
        _ => panic!("Expected Progress variant"),
    }
}

#[test]
fn test_parse_progress_rejects_unknown_fields() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "type": "progress",
        "data": {
            "type": "hook_progress",
            "hookEvent": "PreToolUse",
            "hookName": "PreToolUse:Bash",
            "command": "moriarty hooks exec"
        },
        "toolUseID": "toolu_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T21:54:19.450Z",
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
fn test_parse_hook_progress_data_rejects_unknown_fields() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "type": "progress",
        "data": {
            "type": "hook_progress",
            "hookEvent": "PreToolUse",
            "hookName": "PreToolUse:Bash",
            "command": "moriarty hooks exec",
            "extraField": "should be rejected"
        },
        "toolUseID": "toolu_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T21:54:19.450Z"
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in HookProgressData")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("extraField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_mcp_progress_data_rejects_unknown_fields() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "type": "progress",
        "data": {
            "type": "mcp_progress",
            "status": "completed",
            "serverName": "test-server",
            "toolName": "test-tool",
            "elapsedTimeMs": 10,
            "extraField": "should be rejected"
        },
        "toolUseID": "toolu_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T21:55:09.748Z"
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in McpProgressData")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("extraField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_bash_progress_data_rejects_unknown_fields() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "type": "progress",
        "data": {
            "type": "bash_progress",
            "output": "test output",
            "fullOutput": "test full output",
            "elapsedTimeSeconds": 5,
            "totalLines": 1,
            "extraField": "should be rejected"
        },
        "toolUseID": "toolu_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T21:55:09.748Z"
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in BashProgressData")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("extraField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_waiting_for_task_data_rejects_unknown_fields() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "type": "progress",
        "data": {
            "type": "waiting_for_task",
            "taskDescription": "test task",
            "taskType": "local_bash",
            "extraField": "should be rejected"
        },
        "toolUseID": "toolu_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T22:17:23.813Z"
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in WaitingForTaskData")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("extraField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_query_update_data_rejects_unknown_fields() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "type": "progress",
        "data": {
            "type": "query_update",
            "query": "test query",
            "extraField": "should be rejected"
        },
        "toolUseID": "toolu_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T22:17:23.813Z"
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in QueryUpdateData")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("extraField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_agent_progress_data_rejects_unknown_fields() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "type": "progress",
        "data": {
            "type": "agent_progress",
            "message": {
                "type": "user",
                "timestamp": "2026-01-18T21:43:02.787Z",
                "message": {"role": "user", "content": "test"},
                "uuid": "550e8400-e29b-41d4-a716-446655440004"
            },
            "normalizedMessages": [],
            "prompt": "test prompt",
            "agentId": "abc123",
            "extraField": "should be rejected"
        },
        "toolUseID": "toolu_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T21:54:47.655Z"
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in AgentProgressData")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("extraField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_agent_progress_message_user_rejects_unknown_fields() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "type": "progress",
        "data": {
            "type": "agent_progress",
            "message": {
                "type": "user",
                "timestamp": "2026-01-18T21:43:02.787Z",
                "message": {"role": "user", "content": "test"},
                "uuid": "550e8400-e29b-41d4-a716-446655440004",
                "extraField": "should be rejected"
            },
            "normalizedMessages": [],
            "prompt": "test prompt",
            "agentId": "abc123"
        },
        "toolUseID": "toolu_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T21:54:47.655Z"
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in AgentProgressMessage::User")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("extraField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_agent_progress_message_assistant_rejects_unknown_fields() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "type": "progress",
        "data": {
            "type": "agent_progress",
            "message": {
                "type": "assistant",
                "timestamp": "2026-01-18T21:54:47.639Z",
                "message": {
                    "model": "claude-opus-4-5-20251101",
                    "id": "msg_test",
                    "type": "message",
                    "role": "assistant",
                    "content": "test",
                    "stop_reason": null,
                    "stop_sequence": null,
                    "usage": {
                        "input_tokens": 3,
                        "cache_creation_input_tokens": 0,
                        "cache_read_input_tokens": 0,
                        "cache_creation": {
                            "ephemeral_5m_input_tokens": 0,
                            "ephemeral_1h_input_tokens": 0
                        },
                        "output_tokens": 1
                    }
                },
                "requestId": "req_test",
                "uuid": "550e8400-e29b-41d4-a716-446655440003",
                "extraField": "should be rejected"
            },
            "normalizedMessages": [],
            "prompt": "test prompt",
            "agentId": "abc123"
        },
        "toolUseID": "toolu_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T21:54:47.655Z"
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in AgentProgressMessage::Assistant")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("extraField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_agent_progress_message_progress_rejects_unknown_fields() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "type": "progress",
        "data": {
            "type": "agent_progress",
            "message": {
                "type": "user",
                "timestamp": "2026-01-18T21:43:02.787Z",
                "message": {"role": "user", "content": "test"},
                "uuid": "550e8400-e29b-41d4-a716-446655440004"
            },
            "normalizedMessages": [
                {
                    "type": "progress",
                    "data": {
                        "type": "hook_progress",
                        "hookEvent": "PreToolUse",
                        "hookName": "PreToolUse:Bash",
                        "command": "test"
                    },
                    "toolUseID": "toolu_test",
                    "parentToolUseID": "toolu_parent",
                    "uuid": "550e8400-e29b-41d4-a716-446655440005",
                    "timestamp": "2026-01-18T21:43:02.698Z",
                    "extraField": "should be rejected"
                }
            ],
            "prompt": "test prompt",
            "agentId": "abc123"
        },
        "toolUseID": "toolu_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T21:54:47.655Z"
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in AgentProgressMessage::Progress")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("extraField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_agent_progress_message_attachment_rejects_unknown_fields() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "type": "progress",
        "data": {
            "type": "agent_progress",
            "message": {
                "type": "user",
                "timestamp": "2026-01-18T21:43:02.787Z",
                "message": {"role": "user", "content": "test"},
                "uuid": "550e8400-e29b-41d4-a716-446655440004"
            },
            "normalizedMessages": [
                {
                    "type": "attachment",
                    "attachment": {"type": "hook_success"},
                    "uuid": "550e8400-e29b-41d4-a716-446655440006",
                    "timestamp": "2026-01-18T21:43:02.724Z",
                    "extraField": "should be rejected"
                }
            ],
            "prompt": "test prompt",
            "agentId": "abc123"
        },
        "toolUseID": "toolu_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T21:54:47.655Z"
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in AgentProgressMessage::Attachment")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("extraField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_nested_progress_data_rejects_unknown_fields() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "type": "progress",
        "data": {
            "type": "agent_progress",
            "message": {
                "type": "user",
                "timestamp": "2026-01-18T21:43:02.787Z",
                "message": {"role": "user", "content": "test"},
                "uuid": "550e8400-e29b-41d4-a716-446655440004"
            },
            "normalizedMessages": [
                {
                    "type": "progress",
                    "data": {
                        "type": "hook_progress",
                        "hookEvent": "PreToolUse",
                        "hookName": "PreToolUse:Bash",
                        "command": "test",
                        "extraField": "should be rejected"
                    },
                    "toolUseID": "toolu_test",
                    "parentToolUseID": "toolu_parent",
                    "uuid": "550e8400-e29b-41d4-a716-446655440005",
                    "timestamp": "2026-01-18T21:43:02.698Z"
                }
            ],
            "prompt": "test prompt",
            "agentId": "abc123"
        },
        "toolUseID": "toolu_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T21:54:47.655Z"
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in NestedProgressData")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("extraField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_mcp_progress_without_elapsed_time() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "type": "progress",
        "data": {
            "type": "mcp_progress",
            "status": "started",
            "serverName": "git-read-only",
            "toolName": "show"
        },
        "toolUseID": "toolu_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T21:55:09.748Z"
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Should parse mcp_progress without elapsed_time");

    match line {
        LogLine::Progress(progress) => match progress.data {
            ProgressData::McpProgress(data) => {
                assert_eq!(data.status, "started");
                assert_eq!(data.elapsed_time_ms, None);
            }
            _ => panic!("Expected McpProgress variant"),
        },
        _ => panic!("Expected Progress variant"),
    }
}

#[test]
fn test_parse_progress_with_agent_id() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "agentId": "agent-123",
        "slug": "test-slug",
        "type": "progress",
        "data": {
            "type": "hook_progress",
            "hookEvent": "PreToolUse",
            "hookName": "PreToolUse:Bash",
            "command": "test"
        },
        "toolUseID": "toolu_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T21:54:19.450Z"
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Should parse progress with agent_id and slug");

    match line {
        LogLine::Progress(progress) => {
            assert_eq!(progress.agent_id, Some("agent-123".to_string()));
            assert_eq!(progress.slug, Some("test-slug".to_string()));
        }
        _ => panic!("Expected Progress variant"),
    }
}

#[test]
fn test_parse_progress_without_agent_id() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "type": "progress",
        "data": {
            "type": "hook_progress",
            "hookEvent": "PreToolUse",
            "hookName": "PreToolUse:Bash",
            "command": "test"
        },
        "toolUseID": "toolu_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T21:54:19.450Z"
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Should parse progress without agent_id and slug");

    match line {
        LogLine::Progress(progress) => {
            assert_eq!(progress.agent_id, None);
            assert_eq!(progress.slug, None);
        }
        _ => panic!("Expected Progress variant"),
    }
}

#[test]
fn test_parse_nested_mcp_progress_in_agent() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "type": "progress",
        "data": {
            "type": "agent_progress",
            "message": {
                "type": "user",
                "timestamp": "2026-01-18T21:43:02.787Z",
                "message": {"role": "user", "content": "test"},
                "uuid": "550e8400-e29b-41d4-a716-446655440004"
            },
            "normalizedMessages": [
                {
                    "type": "progress",
                    "data": {
                        "type": "mcp_progress",
                        "status": "completed",
                        "serverName": "git-read-only",
                        "toolName": "show",
                        "elapsedTimeMs": 15
                    },
                    "toolUseID": "toolu_mcp",
                    "parentToolUseID": "toolu_parent",
                    "uuid": "550e8400-e29b-41d4-a716-446655440005",
                    "timestamp": "2026-01-18T21:43:02.698Z"
                }
            ],
            "prompt": "test prompt",
            "agentId": "abc123"
        },
        "toolUseID": "toolu_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T21:54:47.655Z"
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Should parse nested mcp_progress in agent");

    match line {
        LogLine::Progress(progress) => match progress.data {
            ProgressData::AgentProgress(data) => {
                let msgs = data.normalized_messages.as_ref().unwrap();
                assert_eq!(msgs.len(), 1);
                match &msgs[0] {
                    AgentProgressMessage::Progress { data, .. } => match data {
                        NestedProgressData::McpProgress(mcp) => {
                            assert_eq!(mcp.server_name, "git-read-only");
                            assert_eq!(mcp.tool_name, "show");
                            assert_eq!(mcp.elapsed_time_ms, Some(15));
                        }
                        _ => panic!("Expected McpProgress variant in NestedProgressData"),
                    },
                    _ => panic!("Expected Progress variant in AgentProgressMessage"),
                }
            }
            _ => panic!("Expected AgentProgress variant"),
        },
        _ => panic!("Expected Progress variant"),
    }
}

#[test]
fn test_parse_compact_boundary() {
    let json = serde_json::json!({
        "parentUuid": null,
        "logicalParentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.0.0",
        "gitBranch": "main",
        "slug": "noble-floating-lemon",
        "type": "system",
        "subtype": "compact_boundary",
        "content": "Compacted",
        "isMeta": false,
        "timestamp": "2025-01-01T00:00:00Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "level": "info",
        "compactMetadata": {
            "trigger": "manual",
            "preTokens": 100000
        }
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse compact_boundary system message");

    match line {
        LogLine::System(SystemLogLine::CompactBoundary(boundary)) => {
            assert!(boundary.parent_uuid.is_none());
            assert_eq!(
                boundary.logical_parent_uuid,
                Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap()
            );
            assert_eq!(boundary.content, "Compacted");
            assert!(!boundary.is_meta);
            assert_eq!(boundary.compact_metadata.trigger, "manual");
            assert_eq!(boundary.compact_metadata.pre_tokens, 100000);
            // Pre-2.1.158 logs omit the preserved-segment metadata entirely.
            assert_eq!(boundary.compact_metadata.post_tokens, None);
            assert_eq!(boundary.compact_metadata.duration_ms, None);
            assert_eq!(boundary.compact_metadata.pre_compact_discovered_tools, None);
            assert_eq!(boundary.compact_metadata.preserved_segment, None);
            assert_eq!(boundary.compact_metadata.preserved_messages, None);
            assert_eq!(boundary.slug.as_deref(), Some("noble-floating-lemon"));
        }
        _ => panic!("Expected System(CompactBoundary) variant"),
    }
}

#[test]
fn test_parse_compact_boundary_with_preserved_metadata() {
    let json = serde_json::json!({
        "parentUuid": null,
        "logicalParentUuid": "6315c98b-3b35-4963-b061-a33490298c1e",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "897f641d-35f9-4a70-8b47-f3c8f3d9e308",
        "version": "2.1.158",
        "gitBranch": "HEAD",
        "slug": "synchronous-sparking-scone",
        "type": "system",
        "subtype": "compact_boundary",
        "content": "Conversation compacted",
        "isMeta": false,
        "timestamp": "2026-06-05T07:38:03.902Z",
        "uuid": "dbec8794-4cb6-421b-8952-7dd0ac346d4f",
        "level": "info",
        "compactMetadata": {
            "trigger": "manual",
            "preTokens": 808766,
            "durationMs": 96146,
            "preCompactDiscoveredTools": ["TaskCreate", "TaskList", "TaskUpdate", "WebFetch"],
            "preservedSegment": {
                "headUuid": "f6a42fbc-3b1e-4588-8ad8-97b38b2db1b0",
                "anchorUuid": "a48a8d61-431a-4c7a-9aa7-c986f0683bfc",
                "tailUuid": "6315c98b-3b35-4963-b061-a33490298c1e"
            },
            "preservedMessages": {
                "anchorUuid": "a48a8d61-431a-4c7a-9aa7-c986f0683bfc",
                "uuids": ["f6a42fbc-3b1e-4588-8ad8-97b38b2db1b0", "6315c98b-3b35-4963-b061-a33490298c1e"],
                "allUuids": ["f6a42fbc-3b1e-4588-8ad8-97b38b2db1b0", "4333f766-0f94-4bac-8e2a-f04908a7cb23", "6315c98b-3b35-4963-b061-a33490298c1e"]
            },
            "postTokens": 8676
        }
    });

    let line: LogLine = serde_json::from_value(json)
        .expect("Failed to parse compact_boundary with preserved metadata");

    match line {
        LogLine::System(SystemLogLine::CompactBoundary(boundary)) => {
            let meta = boundary.compact_metadata;
            assert_eq!(meta.trigger, "manual");
            assert_eq!(meta.pre_tokens, 808766);
            assert_eq!(meta.post_tokens, Some(8676));
            assert_eq!(meta.duration_ms, Some(96146));
            assert_eq!(
                meta.pre_compact_discovered_tools,
                Some(vec![
                    "TaskCreate".to_string(),
                    "TaskList".to_string(),
                    "TaskUpdate".to_string(),
                    "WebFetch".to_string(),
                ])
            );

            let segment = meta.preserved_segment.expect("preserved_segment present");
            assert_eq!(
                segment.head_uuid,
                Uuid::parse_str("f6a42fbc-3b1e-4588-8ad8-97b38b2db1b0").unwrap()
            );
            assert_eq!(
                segment.anchor_uuid,
                Uuid::parse_str("a48a8d61-431a-4c7a-9aa7-c986f0683bfc").unwrap()
            );
            assert_eq!(
                segment.tail_uuid,
                Uuid::parse_str("6315c98b-3b35-4963-b061-a33490298c1e").unwrap()
            );

            let messages = meta.preserved_messages.expect("preserved_messages present");
            assert_eq!(
                messages.anchor_uuid,
                Uuid::parse_str("a48a8d61-431a-4c7a-9aa7-c986f0683bfc").unwrap()
            );
            assert_eq!(
                messages.uuids,
                vec![
                    Uuid::parse_str("f6a42fbc-3b1e-4588-8ad8-97b38b2db1b0").unwrap(),
                    Uuid::parse_str("6315c98b-3b35-4963-b061-a33490298c1e").unwrap(),
                ]
            );
            // allUuids is a distinct superset of uuids, so this catches a uuids/allUuids swap.
            assert_eq!(
                messages.all_uuids,
                vec![
                    Uuid::parse_str("f6a42fbc-3b1e-4588-8ad8-97b38b2db1b0").unwrap(),
                    Uuid::parse_str("4333f766-0f94-4bac-8e2a-f04908a7cb23").unwrap(),
                    Uuid::parse_str("6315c98b-3b35-4963-b061-a33490298c1e").unwrap(),
                ]
            );
        }
        _ => panic!("Expected System(CompactBoundary) variant"),
    }
}

#[test]
fn test_parse_compact_boundary_with_partial_preserved_metadata() {
    // An intermediate log shape: some 2.1.158 fields present, others absent. Each new field is
    // independently optional, so a partial mix must parse with the absent fields as None.
    let json = serde_json::json!({
        "parentUuid": null,
        "logicalParentUuid": "6315c98b-3b35-4963-b061-a33490298c1e",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "897f641d-35f9-4a70-8b47-f3c8f3d9e308",
        "version": "2.1.158",
        "gitBranch": "HEAD",
        "slug": null,
        "type": "system",
        "subtype": "compact_boundary",
        "content": "Conversation compacted",
        "isMeta": false,
        "timestamp": "2026-06-05T07:38:03.902Z",
        "uuid": "dbec8794-4cb6-421b-8952-7dd0ac346d4f",
        "level": "info",
        "compactMetadata": {
            "trigger": "auto",
            "preTokens": 808766,
            "postTokens": 8676,
            "durationMs": 96146
        }
    });

    let line: LogLine = serde_json::from_value(json)
        .expect("Failed to parse compact_boundary with partial preserved metadata");

    match line {
        LogLine::System(SystemLogLine::CompactBoundary(boundary)) => {
            let meta = boundary.compact_metadata;
            assert_eq!(meta.trigger, "auto");
            assert_eq!(meta.pre_tokens, 808766);
            assert_eq!(meta.post_tokens, Some(8676));
            assert_eq!(meta.duration_ms, Some(96146));
            assert_eq!(meta.pre_compact_discovered_tools, None);
            assert_eq!(meta.preserved_segment, None);
            assert_eq!(meta.preserved_messages, None);
        }
        _ => panic!("Expected System(CompactBoundary) variant"),
    }
}

#[test]
fn test_parse_preserved_segment_rejects_unknown_fields() {
    let json = serde_json::json!({
        "headUuid": "f6a42fbc-3b1e-4588-8ad8-97b38b2db1b0",
        "anchorUuid": "a48a8d61-431a-4c7a-9aa7-c986f0683bfc",
        "tailUuid": "6315c98b-3b35-4963-b061-a33490298c1e",
        "extraField": "should be rejected"
    });

    let err_msg = serde_json::from_value::<PreservedSegment>(json)
        .expect_err("Should reject unknown fields in PreservedSegment")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("extraField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_preserved_messages_rejects_unknown_fields() {
    let json = serde_json::json!({
        "anchorUuid": "a48a8d61-431a-4c7a-9aa7-c986f0683bfc",
        "uuids": [],
        "allUuids": [],
        "extraField": "should be rejected"
    });

    let err_msg = serde_json::from_value::<PreservedMessages>(json)
        .expect_err("Should reject unknown fields in PreservedMessages")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("extraField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_compact_boundary_rejects_unknown_fields() {
    let json = serde_json::json!({
        "parentUuid": null,
        "logicalParentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.0.0",
        "gitBranch": "main",
        "type": "system",
        "subtype": "compact_boundary",
        "content": "Compacted",
        "isMeta": false,
        "timestamp": "2025-01-01T00:00:00Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "level": "info",
        "compactMetadata": {
            "trigger": "manual",
            "preTokens": 100000
        },
        "extraField": "should be rejected"
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in CompactBoundary")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("extraField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_compact_metadata_rejects_unknown_fields() {
    let json = serde_json::json!({
        "parentUuid": null,
        "logicalParentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.0.0",
        "gitBranch": "main",
        "type": "system",
        "subtype": "compact_boundary",
        "content": "Compacted",
        "isMeta": false,
        "timestamp": "2025-01-01T00:00:00Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "level": "info",
        "compactMetadata": {
            "trigger": "manual",
            "preTokens": 100000,
            "extraField": "should be rejected"
        }
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in CompactMetadata")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("extraField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_local_command() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.0.51",
        "gitBranch": "main",
        "slug": "bold-flying-eagle",
        "type": "system",
        "subtype": "local_command",
        "content": "ls -la",
        "level": "info",
        "timestamp": "2025-01-01T00:00:00Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "isMeta": false
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse local_command system message");

    match line {
        LogLine::System(SystemLogLine::LocalCommand(command)) => {
            assert!(command.parent_uuid.is_none());
            assert_eq!(command.content, "ls -la");
            assert_eq!(command.git_branch, "main");
            assert_eq!(command.slug.as_deref(), Some("bold-flying-eagle"));
            assert_eq!(command.entrypoint, None);
        }
        _ => panic!("Expected System(LocalCommand) variant"),
    }
}

#[test]
fn test_parse_microcompact_boundary() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "HEAD",
        "slug": "test-slug",
        "type": "system",
        "subtype": "microcompact_boundary",
        "content": "Context microcompacted",
        "isMeta": false,
        "timestamp": "2026-01-18T23:44:09.153Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "level": "info",
        "microcompactMetadata": {
            "trigger": "auto",
            "preTokens": 58482,
            "tokensSaved": 20010,
            "compactedToolIds": ["toolu_01", "toolu_02"],
            "clearedAttachmentUUIDs": []
        }
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse microcompact_boundary system message");

    match line {
        LogLine::System(SystemLogLine::MicrocompactBoundary(boundary)) => {
            assert_eq!(boundary.content, "Context microcompacted");
            assert_eq!(boundary.level, "info");
            assert_eq!(boundary.microcompact_metadata.trigger, "auto");
            assert_eq!(boundary.microcompact_metadata.pre_tokens, 58482);
            assert_eq!(boundary.microcompact_metadata.tokens_saved, 20010);
            assert_eq!(boundary.microcompact_metadata.compacted_tool_ids.len(), 2);
            assert!(boundary
                .microcompact_metadata
                .cleared_attachment_uuids
                .is_empty());
        }
        _ => panic!("Expected System(MicrocompactBoundary) variant"),
    }
}

#[test]
fn test_parse_microcompact_boundary_with_entrypoint() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.104",
        "gitBranch": "HEAD",
        "type": "system",
        "subtype": "microcompact_boundary",
        "content": "Context microcompacted",
        "isMeta": false,
        "timestamp": "2026-01-18T23:44:09.153Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "level": "info",
        "entrypoint": "cli",
        "microcompactMetadata": {
            "trigger": "auto",
            "preTokens": 58482,
            "tokensSaved": 20010,
            "compactedToolIds": [],
            "clearedAttachmentUUIDs": []
        }
    });

    let line: LogLine = serde_json::from_value(json)
        .expect("Failed to parse microcompact_boundary with entrypoint");

    match line {
        LogLine::System(SystemLogLine::MicrocompactBoundary(boundary)) => {
            assert_eq!(boundary.entrypoint.as_deref(), Some("cli"));
        }
        _ => panic!("Expected System(MicrocompactBoundary) variant"),
    }
}

#[test]
fn test_parse_microcompact_boundary_rejects_unknown_fields() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "HEAD",
        "type": "system",
        "subtype": "microcompact_boundary",
        "content": "Context microcompacted",
        "isMeta": false,
        "timestamp": "2026-01-18T23:44:09.153Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "level": "info",
        "microcompactMetadata": {
            "trigger": "auto",
            "preTokens": 58482,
            "tokensSaved": 20010,
            "compactedToolIds": [],
            "clearedAttachmentUUIDs": []
        },
        "extraField": "should be rejected"
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in MicrocompactBoundary")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("extraField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_microcompact_metadata_rejects_unknown_fields() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "HEAD",
        "type": "system",
        "subtype": "microcompact_boundary",
        "content": "Context microcompacted",
        "isMeta": false,
        "timestamp": "2026-01-18T23:44:09.153Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "level": "info",
        "microcompactMetadata": {
            "trigger": "auto",
            "preTokens": 58482,
            "tokensSaved": 20010,
            "compactedToolIds": [],
            "clearedAttachmentUUIDs": [],
            "extraField": "should be rejected"
        }
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in MicrocompactMetadata")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("extraField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_nested_hook_progress_in_agent() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "type": "progress",
        "data": {
            "type": "agent_progress",
            "message": {
                "type": "user",
                "message": {"role": "user", "content": "test"},
                "uuid": "550e8400-e29b-41d4-a716-446655440003",
                "timestamp": "2026-01-18T21:43:02.787Z"
            },
            "normalizedMessages": [{
                "type": "progress",
                "data": {
                    "type": "hook_progress",
                    "hookEvent": "PreToolUse",
                    "hookName": "PreToolUse:Bash",
                    "command": "moriarty hooks exec"
                },
                "toolUseID": "toolu_test",
                "parentToolUseID": "toolu_parent",
                "uuid": "550e8400-e29b-41d4-a716-446655440005",
                "timestamp": "2026-01-18T21:43:02.698Z"
            }],
            "prompt": "test",
            "agentId": "test"
        },
        "toolUseID": "agent_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T21:54:47.655Z"
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Should parse nested hook_progress in agent");

    match line {
        LogLine::Progress(progress) => match progress.data {
            ProgressData::AgentProgress(data) => {
                let msgs = data.normalized_messages.as_ref().unwrap();
                assert_eq!(msgs.len(), 1);
                match &msgs[0] {
                    AgentProgressMessage::Progress { data, .. } => match data {
                        NestedProgressData::HookProgress(hook) => {
                            assert_eq!(hook.hook_event, "PreToolUse");
                            assert_eq!(hook.hook_name, "PreToolUse:Bash");
                            assert_eq!(hook.command, "moriarty hooks exec");
                        }
                        _ => panic!("Expected HookProgress variant in NestedProgressData"),
                    },
                    _ => panic!("Expected Progress variant in AgentProgressMessage"),
                }
            }
            _ => panic!("Expected AgentProgress variant"),
        },
        _ => panic!("Expected Progress variant"),
    }
}

#[test]
fn test_parse_nested_bash_progress_in_agent() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "agentId": "agent-123",
        "slug": "test-slug",
        "type": "progress",
        "data": {
            "type": "agent_progress",
            "message": {
                "type": "user",
                "message": {"role": "user", "content": "test"},
                "uuid": "550e8400-e29b-41d4-a716-446655440001",
                "timestamp": "2026-01-18T21:43:02.787Z"
            },
            "normalizedMessages": [{
                "type": "progress",
                "data": {
                    "type": "bash_progress",
                    "output": "Running command...",
                    "fullOutput": "Running command...\nDone!",
                    "elapsedTimeSeconds": 5,
                    "totalLines": 2
                },
                "toolUseID": "toolu_test",
                "parentToolUseID": "toolu_parent",
                "uuid": "550e8400-e29b-41d4-a716-446655440003",
                "timestamp": "2026-01-18T21:43:10.123Z"
            }],
            "prompt": "test prompt",
            "agentId": "agent-123"
        },
        "toolUseID": "agent_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T21:54:47.655Z"
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Should parse nested bash_progress in agent");

    match line {
        LogLine::Progress(progress) => match progress.data {
            ProgressData::AgentProgress(data) => {
                let msgs = data.normalized_messages.as_ref().unwrap();
                assert_eq!(msgs.len(), 1);
                match &msgs[0] {
                    AgentProgressMessage::Progress { data, .. } => match data {
                        NestedProgressData::BashProgress(bash) => {
                            assert_eq!(bash.output, "Running command...");
                            assert_eq!(bash.full_output, "Running command...\nDone!");
                            assert_eq!(bash.elapsed_time_seconds, 5);
                            assert_eq!(bash.total_lines, 2);
                        }
                        _ => panic!("Expected BashProgress variant in NestedProgressData"),
                    },
                    _ => panic!("Expected Progress variant in AgentProgressMessage"),
                }
            }
            _ => panic!("Expected AgentProgress variant"),
        },
        _ => panic!("Expected Progress variant"),
    }
}

#[test]
fn test_parse_log_line_rejects_unknown_type() {
    let json = serde_json::json!({
        "type": "unknown_type",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "1.0",
        "gitBranch": "main",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2025-01-01T00:00:00Z"
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown log line type")
        .to_string();
    assert!(
        err_msg.contains("unknown variant")
            || err_msg.contains("unknown_type")
            || err_msg.contains("did not match any variant"),
        "Error should mention unknown variant, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_system_log_line_rejects_unknown_subtype() {
    let json = serde_json::json!({
        "type": "system",
        "subtype": "unknown_subtype",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "1.0",
        "gitBranch": "main",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2025-01-01T00:00:00Z"
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown system log subtype")
        .to_string();
    assert!(
        err_msg.contains("unknown variant")
            || err_msg.contains("unknown_subtype")
            || err_msg.contains("did not match any variant"),
        "Error should mention unknown variant, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_progress_search_results_received() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "type": "progress",
        "data": {
            "type": "search_results_received",
            "resultCount": 5,
            "query": "rust testing best practices"
        },
        "toolUseID": "search-results-id",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T22:17:23.813Z"
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse search_results_received");

    match line {
        LogLine::Progress(progress) => match progress.data {
            ProgressData::SearchResultsReceived(data) => {
                assert_eq!(data.result_count, 5);
                assert_eq!(data.query, "rust testing best practices");
            }
            _ => panic!("Expected SearchResultsReceived variant"),
        },
        _ => panic!("Expected Progress variant"),
    }
}

#[test]
fn test_parse_search_results_received_data_rejects_unknown_fields() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "type": "progress",
        "data": {
            "type": "search_results_received",
            "resultCount": 3,
            "query": "test query",
            "extraField": "should be rejected"
        },
        "toolUseID": "toolu_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T22:17:23.813Z"
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in SearchResultsReceivedData")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("extraField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_search_results_received_zero_results() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "type": "progress",
        "data": {
            "type": "search_results_received",
            "resultCount": 0,
            "query": "nonexistent topic xyz123"
        },
        "toolUseID": "search-id",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T22:17:23.813Z"
    });

    let line: LogLine = serde_json::from_value(json).unwrap();
    match line {
        LogLine::Progress(progress) => match progress.data {
            ProgressData::SearchResultsReceived(data) => {
                assert_eq!(data.result_count, 0);
            }
            _ => panic!("Expected SearchResultsReceived variant"),
        },
        _ => panic!("Expected Progress variant"),
    }
}

#[test]
fn test_parse_nested_query_update_in_agent() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "agentId": "agent-123",
        "slug": "test-slug",
        "type": "progress",
        "data": {
            "type": "agent_progress",
            "message": {
                "type": "user",
                "message": {"role": "user", "content": "test"},
                "uuid": "550e8400-e29b-41d4-a716-446655440001",
                "timestamp": "2026-01-18T21:43:02.787Z"
            },
            "normalizedMessages": [{
                "type": "progress",
                "data": {
                    "type": "query_update",
                    "query": "rust async patterns 2026"
                },
                "toolUseID": "toolu_query",
                "parentToolUseID": "toolu_parent",
                "uuid": "550e8400-e29b-41d4-a716-446655440003",
                "timestamp": "2026-01-18T21:43:10.123Z"
            }],
            "prompt": "test prompt",
            "agentId": "agent-123"
        },
        "toolUseID": "agent_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T21:54:47.655Z"
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Should parse nested query_update in agent");

    match line {
        LogLine::Progress(progress) => match progress.data {
            ProgressData::AgentProgress(data) => {
                let msgs = data.normalized_messages.as_ref().unwrap();
                assert_eq!(msgs.len(), 1);
                match &msgs[0] {
                    AgentProgressMessage::Progress { data, .. } => match data {
                        NestedProgressData::QueryUpdate(query) => {
                            assert_eq!(query.query, "rust async patterns 2026");
                        }
                        _ => panic!("Expected QueryUpdate variant in NestedProgressData"),
                    },
                    _ => panic!("Expected Progress variant in AgentProgressMessage"),
                }
            }
            _ => panic!("Expected AgentProgress variant"),
        },
        _ => panic!("Expected Progress variant"),
    }
}

#[test]
fn test_parse_nested_search_results_received_in_agent() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.12",
        "gitBranch": "main",
        "agentId": "agent-123",
        "slug": "test-slug",
        "type": "progress",
        "data": {
            "type": "agent_progress",
            "message": {
                "type": "user",
                "message": {"role": "user", "content": "test"},
                "uuid": "550e8400-e29b-41d4-a716-446655440001",
                "timestamp": "2026-01-18T21:43:02.787Z"
            },
            "normalizedMessages": [{
                "type": "progress",
                "data": {
                    "type": "search_results_received",
                    "resultCount": 8,
                    "query": "rust testing frameworks"
                },
                "toolUseID": "toolu_search",
                "parentToolUseID": "toolu_parent",
                "uuid": "550e8400-e29b-41d4-a716-446655440003",
                "timestamp": "2026-01-18T21:43:15.456Z"
            }],
            "prompt": "test prompt",
            "agentId": "agent-123"
        },
        "toolUseID": "agent_test",
        "parentToolUseID": "toolu_parent",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-01-18T21:54:47.655Z"
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Should parse nested search_results_received in agent");

    match line {
        LogLine::Progress(progress) => match progress.data {
            ProgressData::AgentProgress(data) => {
                let msgs = data.normalized_messages.as_ref().unwrap();
                assert_eq!(msgs.len(), 1);
                match &msgs[0] {
                    AgentProgressMessage::Progress { data, .. } => match data {
                        NestedProgressData::SearchResultsReceived(search) => {
                            assert_eq!(search.result_count, 8);
                            assert_eq!(search.query, "rust testing frameworks");
                        }
                        _ => panic!("Expected SearchResultsReceived variant in NestedProgressData"),
                    },
                    _ => panic!("Expected Progress variant in AgentProgressMessage"),
                }
            }
            _ => panic!("Expected AgentProgress variant"),
        },
        _ => panic!("Expected Progress variant"),
    }
}

#[test]
fn test_parse_assistant_usage_with_inference_geo() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "test-session",
        "version": "2.1.12",
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
                "output_tokens": 50,
                "inference_geo": "us-east-1"
            }
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2025-01-01T00:00:00Z"
    });
    let line: AssistantLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(
        line.message.usage.inference_geo,
        Some("us-east-1".to_string())
    );
}

#[test]
fn test_parse_assistant_usage_with_null_inference_geo() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "test-session",
        "version": "2.1.12",
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
                "output_tokens": 50,
                "inference_geo": null
            }
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2025-01-01T00:00:00Z"
    });
    let line: AssistantLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.message.usage.inference_geo, None);
}

#[test]
fn test_parse_assistant_usage_without_inference_geo() {
    // Documents backward compatibility - older logs won't have this field
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
    assert_eq!(line.message.usage.inference_geo, None);
}

#[test]
fn test_parse_assistant_usage_rejects_unknown_fields() {
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
                "output_tokens": 50,
                "unknown_field": "should fail"
            }
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2025-01-01T00:00:00Z"
    });

    let err_msg = serde_json::from_value::<AssistantLogLine>(json)
        .expect_err("Should reject unknown fields due to deny_unknown_fields")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("unknown_field"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_tool_use_with_caller() {
    let json = serde_json::json!({
        "type": "tool_use",
        "id": "toolu_123",
        "name": "Bash",
        "input": {"command": "ls -la"},
        "caller": {"type": "direct"}
    });
    let content: LogMessageTaggedContent = serde_json::from_value(json).unwrap();

    match content {
        LogMessageTaggedContent::ToolUse {
            id,
            name,
            input,
            caller,
        } => {
            assert_eq!(id, "toolu_123");
            assert_eq!(name, "Bash");
            assert_eq!(input.get("command").unwrap(), "ls -la");
            let caller = caller.expect("caller should be present");
            assert_eq!(caller.r#type, "direct");
        }
        _ => panic!("Expected ToolUse variant"),
    }
}

#[test]
fn test_parse_tool_use_without_caller() {
    // Documents backward compatibility - older logs won't have this field
    let json = serde_json::json!({
        "type": "tool_use",
        "id": "toolu_456",
        "name": "Read",
        "input": {"file_path": "/tmp/test.txt"}
    });
    let content: LogMessageTaggedContent = serde_json::from_value(json).unwrap();

    match content {
        LogMessageTaggedContent::ToolUse {
            id,
            name,
            input,
            caller,
        } => {
            assert_eq!(id, "toolu_456");
            assert_eq!(name, "Read");
            assert_eq!(input.get("file_path").unwrap(), "/tmp/test.txt");
            assert!(caller.is_none(), "caller should be None for older logs");
        }
        _ => panic!("Expected ToolUse variant"),
    }
}

#[test]
fn test_parse_tool_use_caller_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "tool_use",
        "id": "toolu_789",
        "name": "Bash",
        "input": {},
        "caller": {"type": "direct", "unknown_field": "should fail"}
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
fn test_parse_fallback_content_block() {
    let json = serde_json::json!({
        "type": "fallback",
        "from": {"model": "claude-fable-5"},
        "to": {"model": "claude-opus-4-8"}
    });
    match serde_json::from_value::<LogMessageTaggedContent>(json)
        .expect("Failed to parse fallback block")
    {
        LogMessageTaggedContent::Fallback { from, to } => {
            assert_eq!(from.model.raw(), "claude-fable-5");
            assert_eq!(to.model.raw(), "claude-opus-4-8");
        }
        other => panic!("Expected Fallback variant, got {other:?}"),
    }
}

#[test]
fn test_parse_fallback_content_block_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "fallback",
        "from": {"model": "claude-fable-5", "reason": "overloaded"},
        "to": {"model": "claude-opus-4-8"}
    });
    let err_msg = serde_json::from_value::<LogMessageTaggedContent>(json)
        .expect_err("Should reject unknown fields due to deny_unknown_fields")
        .to_string();
    assert!(
        err_msg.contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_user_log_line_with_prompt_id() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.77",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "promptId": "550e8400-e29b-41d4-a716-446655440088"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(
        line.prompt_id,
        Some(Uuid::parse_str("550e8400-e29b-41d4-a716-446655440088").unwrap())
    );
}

#[test]
fn test_parse_user_log_line_with_null_prompt_id() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.77",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "promptId": null
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.prompt_id, None);
}

#[test]
fn test_parse_user_log_line_without_prompt_id() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.0.50",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.prompt_id, None);
}

#[test]
fn test_parse_user_log_line_with_prompt_source() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.170",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "promptSource": "typed"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.prompt_source.as_deref(), Some("typed"));
}

#[test]
fn test_parse_user_log_line_with_source_tool_use_id() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.170",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "sourceToolUseID": "toolu_01TnFtjG2oYQQKKKUULR9y6V"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(
        line.source_tool_use_id.as_deref(),
        Some("toolu_01TnFtjG2oYQQKKKUULR9y6V")
    );
}

#[test]
fn test_parse_user_log_line_with_permission_mode_plan() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.77",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "permissionMode": "plan"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.permission_mode, Some(PermissionMode::Plan));
}

#[test]
fn test_parse_user_log_line_with_permission_mode_accept_edits() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.77",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "permissionMode": "acceptEdits"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.permission_mode, Some(PermissionMode::AcceptEdits));
}

#[test]
fn test_parse_user_log_line_with_permission_mode_default() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.77",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "permissionMode": "default"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.permission_mode, Some(PermissionMode::Default));
}

#[test]
fn test_parse_user_log_line_without_permission_mode() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.0.50",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.permission_mode, None);
}

#[test]
fn test_parse_user_log_line_with_plan_content() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.77",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "planContent": "# My Plan\n\n## Steps\n1. Do the thing"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(
        line.plan_content,
        Some("# My Plan\n\n## Steps\n1. Do the thing".to_string())
    );
}

#[test]
fn test_parse_user_log_line_with_null_plan_content() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.77",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "planContent": null
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.plan_content, None);
}

#[test]
fn test_parse_user_log_line_without_plan_content() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.0.50",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.plan_content, None);
}

#[test]
fn test_parse_assistant_usage_with_iterations_and_speed() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "test-session",
        "version": "2.1.77",
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
                "output_tokens": 50,
                "iterations": [],
                "speed": "standard"
            }
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2025-01-01T00:00:00Z"
    });
    let line: AssistantLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.message.usage.iterations, Some(vec![]));
    assert_eq!(line.message.usage.speed, Some(Speed::Standard));
}

#[test]
fn test_parse_assistant_usage_with_speed_fast() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "test-session",
        "version": "2.1.77",
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
                "output_tokens": 50,
                "speed": "fast"
            }
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2025-01-01T00:00:00Z"
    });
    let line: AssistantLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.message.usage.iterations, None);
    assert_eq!(line.message.usage.speed, Some(Speed::Fast));
}

#[test]
fn test_parse_assistant_usage_with_null_iterations_and_speed() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "test-session",
        "version": "2.1.77",
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
                "output_tokens": 50,
                "iterations": null,
                "speed": null
            }
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2025-01-01T00:00:00Z"
    });
    let line: AssistantLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.message.usage.iterations, None);
    assert_eq!(line.message.usage.speed, None);
}

#[test]
fn test_parse_assistant_usage_without_iterations_and_speed() {
    // Backward compatibility - older logs won't have these fields
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
    assert_eq!(line.message.usage.iterations, None);
    assert_eq!(line.message.usage.speed, None);
}

#[test]
fn test_parse_user_log_line_rejects_unknown_permission_mode() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.77",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "permissionMode": "bypassPermissions"
    });
    let err_msg = serde_json::from_value::<UserLogLine>(json)
        .expect_err("Should reject unknown permissionMode variant")
        .to_string();
    assert!(
        err_msg.contains("unknown variant") || err_msg.contains("bypassPermissions"),
        "Error should mention unknown variant, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_assistant_usage_rejects_unknown_speed() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "test-session",
        "version": "2.1.77",
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
                "output_tokens": 50,
                "speed": "turbo"
            }
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2025-01-01T00:00:00Z"
    });
    let err_msg = serde_json::from_value::<AssistantLogLine>(json)
        .expect_err("Should reject unknown speed variant")
        .to_string();
    assert!(
        err_msg.contains("unknown variant") || err_msg.contains("turbo"),
        "Error should mention unknown variant, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_assistant_message_with_stop_details_end_turn() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "test-session",
        "version": "2.1.77",
        "gitBranch": "main",
        "message": {
            "id": "msg-1",
            "type": "message",
            "role": "assistant",
            "content": "response",
            "model": "claude-opus-4-5",
            "stop_reason": "end_turn",
            "stop_details": {"type": "end_turn", "stop_sequence": null},
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
    let stop_details = line.message.stop_details.unwrap();
    assert_eq!(stop_details.r#type, StopType::EndTurn);
    assert_eq!(stop_details.stop_sequence, None);
}

#[test]
fn test_parse_assistant_message_with_stop_details_stop_sequence() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "test-session",
        "version": "2.1.77",
        "gitBranch": "main",
        "message": {
            "id": "msg-1",
            "type": "message",
            "role": "assistant",
            "content": "response",
            "model": "claude-opus-4-5",
            "stop_reason": "stop_sequence",
            "stop_details": {"type": "stop_sequence", "stop_sequence": "</result>"},
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
    let stop_details = line.message.stop_details.unwrap();
    assert_eq!(stop_details.r#type, StopType::StopSequence);
    assert_eq!(stop_details.stop_sequence, Some("</result>".to_string()));
}

#[test]
fn test_parse_assistant_message_without_stop_details() {
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
    assert_eq!(line.message.stop_details, None);
}

#[test]
fn test_parse_stop_details_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "end_turn",
        "stop_sequence": null,
        "extra_field": "should fail"
    });
    let err = serde_json::from_value::<StopDetails>(json)
        .expect_err("Should reject unknown fields in StopDetails");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
}

#[test]
fn test_parse_stop_details_rejects_unknown_stop_type() {
    let json = serde_json::json!({
        "type": "tool_use",
        "stop_sequence": null
    });
    let err_msg = serde_json::from_value::<StopDetails>(json)
        .expect_err("Should reject unknown stop type variant")
        .to_string();
    assert!(
        err_msg.contains("unknown variant") || err_msg.contains("tool_use"),
        "Error should mention unknown variant, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_custom_title_log_line() {
    let json = serde_json::json!({
        "type": "custom-title",
        "customTitle": "My Custom Session Title",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000"
    });
    let line: LogLine = serde_json::from_value(json).expect("Should parse custom-title");
    match line {
        LogLine::CustomTitle(ct) => {
            assert_eq!(ct.custom_title, "My Custom Session Title");
            assert_eq!(
                ct.session_id,
                "550e8400-e29b-41d4-a716-446655440000"
                    .parse::<Uuid>()
                    .unwrap()
            );
        }
        _ => panic!("Expected CustomTitle variant"),
    }
}

#[test]
fn test_parse_custom_title_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "custom-title",
        "customTitle": "Title",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "extraField": "should fail"
    });
    let err = serde_json::from_value::<LogLine>(json).expect_err("Should reject unknown fields");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
}

#[test]
fn test_parse_agent_progress_without_normalized_messages() {
    let json = serde_json::json!({
        "type": "progress",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "human",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.0",
        "gitBranch": "main",
        "data": {
            "type": "agent_progress",
            "message": {
                "type": "user",
                "message": {
                    "role": "user",
                    "content": "test"
                },
                "uuid": "550e8400-e29b-41d4-a716-446655440000",
                "timestamp": "2025-01-01T00:00:00Z",
                "toolUseResult": null
            },
            "prompt": "do something",
            "agentId": "agent-1"
        },
        "toolUseID": "tool-1",
        "parentToolUseID": "parent-1",
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2025-01-01T00:00:00Z"
    });
    let line: LogLine = serde_json::from_value(json)
        .expect("Should parse agent_progress without normalizedMessages");
    match line {
        LogLine::Progress(progress) => match progress.data {
            ProgressData::AgentProgress(data) => {
                assert!(data.normalized_messages.is_none());
                assert_eq!(data.agent_id, "agent-1");
                assert_eq!(data.prompt, "do something");
            }
            _ => panic!("Expected AgentProgress variant"),
        },
        _ => panic!("Expected Progress variant"),
    }
}

#[test]
fn test_parse_agent_progress_with_null_normalized_messages() {
    let json = serde_json::json!({
        "type": "progress",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "human",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.0",
        "gitBranch": "main",
        "data": {
            "type": "agent_progress",
            "message": {
                "type": "user",
                "message": {
                    "role": "user",
                    "content": "test"
                },
                "uuid": "550e8400-e29b-41d4-a716-446655440000",
                "timestamp": "2025-01-01T00:00:00Z",
                "toolUseResult": null
            },
            "normalizedMessages": null,
            "prompt": "do something",
            "agentId": "agent-1"
        },
        "toolUseID": "tool-1",
        "parentToolUseID": "parent-1",
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2025-01-01T00:00:00Z"
    });
    let line: LogLine = serde_json::from_value(json)
        .expect("Should parse agent_progress with null normalizedMessages");
    match line {
        LogLine::Progress(progress) => match progress.data {
            ProgressData::AgentProgress(data) => {
                assert!(data.normalized_messages.is_none());
            }
            _ => panic!("Expected AgentProgress variant"),
        },
        _ => panic!("Expected Progress variant"),
    }
}

#[test]
fn test_parse_tool_reference_content() {
    let json = serde_json::json!({
        "type": "tool_reference",
        "tool_name": "WebFetch"
    });
    let content: LogMessageTaggedContent =
        serde_json::from_value(json).expect("Should parse tool_reference");
    match content {
        LogMessageTaggedContent::ToolReference { tool_name } => {
            assert_eq!(tool_name, "WebFetch");
        }
        _ => panic!("Expected ToolReference variant"),
    }
}

#[test]
fn test_parse_tool_reference_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "tool_reference",
        "tool_name": "WebFetch",
        "extra": "should fail"
    });
    let err = serde_json::from_value::<LogMessageTaggedContent>(json)
        .expect_err("Should reject unknown fields");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
}

#[test]
fn test_parse_tool_result_with_tool_reference_content() {
    let json = serde_json::json!([
        {"type": "text", "text": "Result text"},
        {"type": "tool_reference", "tool_name": "WebFetch"}
    ]);
    let content: Vec<LogMessageTaggedContent> =
        serde_json::from_value(json).expect("Should parse content vec with tool_reference");
    assert_eq!(content.len(), 2);
    assert!(matches!(&content[0], LogMessageTaggedContent::Text { text } if text == "Result text"));
    assert!(
        matches!(&content[1], LogMessageTaggedContent::ToolReference { tool_name } if tool_name == "WebFetch")
    );
}

#[test]
fn test_parse_agent_name_log_line() {
    let json = r#"{"type":"agent-name","agentName":"task-agent","sessionId":"550e8400-e29b-41d4-a716-446655440000"}"#;
    let log_line: LogLine = serde_json::from_str(json).unwrap();
    match log_line {
        LogLine::AgentName(an) => {
            assert_eq!(an.agent_name, "task-agent");
            assert_eq!(
                an.session_id,
                "550e8400-e29b-41d4-a716-446655440000"
                    .parse::<Uuid>()
                    .unwrap()
            );
        }
        other => panic!("Expected AgentName, got {:?}", other),
    }
}

#[test]
fn test_parse_agent_name_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "agent-name",
        "agentName": "task-agent",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "extraField": "should fail"
    });
    let err = serde_json::from_value::<LogLine>(json).expect_err("Should reject unknown fields");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
}

#[test]
fn test_parse_last_prompt_log_line() {
    let json = r#"{"type":"last-prompt","lastPrompt":"Fix the bug","sessionId":"550e8400-e29b-41d4-a716-446655440000"}"#;
    let log_line: LogLine = serde_json::from_str(json).unwrap();
    match log_line {
        LogLine::LastPrompt(lp) => {
            assert_eq!(lp.last_prompt.as_deref(), Some("Fix the bug"));
            assert_eq!(lp.leaf_uuid, None);
            assert_eq!(
                lp.session_id,
                "550e8400-e29b-41d4-a716-446655440000"
                    .parse::<Uuid>()
                    .unwrap()
            );
        }
        other => panic!("Expected LastPrompt, got {:?}", other),
    }
}

#[test]
fn test_parse_last_prompt_log_line_with_leaf_uuid() {
    let json = r#"{"type":"last-prompt","leafUuid":"4629e822-f089-4f87-aa1f-7d93ebe10d81","sessionId":"d1226c8d-4fe8-441b-95a0-bbfa8aae1a59"}"#;
    let log_line: LogLine = serde_json::from_str(json).unwrap();
    match log_line {
        LogLine::LastPrompt(lp) => {
            assert_eq!(lp.last_prompt, None);
            assert_eq!(
                lp.leaf_uuid,
                Some(
                    "4629e822-f089-4f87-aa1f-7d93ebe10d81"
                        .parse::<Uuid>()
                        .unwrap()
                )
            );
            assert_eq!(
                lp.session_id,
                "d1226c8d-4fe8-441b-95a0-bbfa8aae1a59"
                    .parse::<Uuid>()
                    .unwrap()
            );
        }
        other => panic!("Expected LastPrompt, got {:?}", other),
    }
}

#[test]
fn test_parse_last_prompt_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "last-prompt",
        "lastPrompt": "Fix the bug",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "extraField": "should fail"
    });
    let err = serde_json::from_value::<LogLine>(json).expect_err("Should reject unknown fields");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
}

#[test]
fn test_parse_permission_mode_change_default() {
    let json = r#"{"type":"permission-mode","permissionMode":"default","sessionId":"550e8400-e29b-41d4-a716-446655440000"}"#;
    let log_line: LogLine = serde_json::from_str(json).unwrap();
    match log_line {
        LogLine::PermissionModeChange(pm) => {
            assert_eq!(pm.permission_mode, PermissionMode::Default);
            assert_eq!(
                pm.session_id,
                "550e8400-e29b-41d4-a716-446655440000"
                    .parse::<Uuid>()
                    .unwrap()
            );
        }
        other => panic!("Expected PermissionModeChange, got {:?}", other),
    }
}

#[test]
fn test_parse_permission_mode_change_plan() {
    let json = r#"{"type":"permission-mode","permissionMode":"plan","sessionId":"550e8400-e29b-41d4-a716-446655440000"}"#;
    let log_line: LogLine = serde_json::from_str(json).unwrap();
    match log_line {
        LogLine::PermissionModeChange(pm) => {
            assert_eq!(pm.permission_mode, PermissionMode::Plan);
        }
        other => panic!("Expected PermissionModeChange, got {:?}", other),
    }
}

#[test]
fn test_parse_permission_mode_change_accept_edits() {
    let json = r#"{"type":"permission-mode","permissionMode":"acceptEdits","sessionId":"550e8400-e29b-41d4-a716-446655440000"}"#;
    let log_line: LogLine = serde_json::from_str(json).unwrap();
    match log_line {
        LogLine::PermissionModeChange(pm) => {
            assert_eq!(pm.permission_mode, PermissionMode::AcceptEdits);
        }
        other => panic!("Expected PermissionModeChange, got {:?}", other),
    }
}

#[test]
fn test_parse_permission_mode_change_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "permission-mode",
        "permissionMode": "default",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "extraField": "should fail"
    });
    let err = serde_json::from_value::<LogLine>(json).expect_err("Should reject unknown fields");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
}

#[test]
fn test_parse_mode_normal() {
    let json =
        r#"{"type":"mode","mode":"normal","sessionId":"550e8400-e29b-41d4-a716-446655440000"}"#;
    let log_line: LogLine = serde_json::from_str(json).unwrap();
    match log_line {
        LogLine::Mode(line) => {
            assert_eq!(line.mode, SessionMode::Normal);
            assert_eq!(
                line.session_id,
                "550e8400-e29b-41d4-a716-446655440000"
                    .parse::<Uuid>()
                    .unwrap()
            );
        }
        other => panic!("Expected Mode, got {:?}", other),
    }
}

#[test]
fn test_parse_mode_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "mode",
        "mode": "normal",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "extraField": "should fail"
    });
    let err = serde_json::from_value::<LogLine>(json).expect_err("Should reject unknown fields");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
}

// Locks in the closed-enum design: an unmodeled mode must fail loud rather than parse silently, so
// `cost_analyzer` surfaces the new mode (and the maintainer adds the variant) instead of ignoring it.
#[test]
fn test_parse_mode_rejects_unknown_mode() {
    let json = r#"{"type":"mode","mode":"vim","sessionId":"550e8400-e29b-41d4-a716-446655440000"}"#;
    let err = serde_json::from_str::<LogLine>(json).expect_err("Should reject unknown mode value");
    assert!(
        err.to_string().contains("unknown variant"),
        "Error should mention unknown variant, got: {}",
        err
    );
}

#[test]
fn test_parse_user_log_line_with_entrypoint() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.104",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "entrypoint": "cli"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.entrypoint, Some("cli".to_string()));
}

#[test]
fn test_parse_user_log_line_with_null_entrypoint() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.104",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "entrypoint": null
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.entrypoint, None);
}

#[test]
fn test_parse_user_log_line_without_entrypoint() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.0.50",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.entrypoint, None);
}

#[test]
fn test_parse_attachment_deferred_tools_delta() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "deferred_tools_delta",
            "addedNames": ["WebFetch", "WebSearch"],
            "addedLines": ["WebFetch", "WebSearch"],
            "removedNames": []
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2025-01-01T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.104",
        "gitBranch": "main",
        "slug": "test-slug"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => match att.attachment {
            AttachmentData::DeferredToolsDelta(delta) => {
                assert_eq!(delta.added_names, vec!["WebFetch", "WebSearch"]);
                assert!(delta.readded_names.is_empty());
                assert!(delta.pending_mcp_servers.is_empty());
                assert_eq!(att.entrypoint, Some("cli".to_string()));
            }
            other => panic!("Expected DeferredToolsDelta, got {:?}", other),
        },
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_deferred_tools_delta_with_readded_and_pending() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "deferred_tools_delta",
            "addedNames": ["WebFetch"],
            "addedLines": ["WebFetch"],
            "removedNames": ["OldTool"],
            "readdedNames": ["PreviouslyRemoved"],
            "pendingMcpServers": ["server-a", "server-b"]
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2026-05-28T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.141",
        "gitBranch": "main",
        "slug": null
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => match att.attachment {
            AttachmentData::DeferredToolsDelta(delta) => {
                assert_eq!(delta.removed_names, vec!["OldTool"]);
                assert_eq!(delta.readded_names, vec!["PreviouslyRemoved"]);
                assert_eq!(delta.pending_mcp_servers, vec!["server-a", "server-b"]);
            }
            other => panic!("Expected DeferredToolsDelta, got {:?}", other),
        },
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_file() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": true,
        "agentId": "agent-1",
        "attachment": {
            "type": "file",
            "filename": "/abs/path/to/file.md",
            "content": {
                "type": "text",
                "file": {
                    "filePath": "/abs/path/to/file.md",
                    "content": "hello",
                    "numLines": 1,
                    "startLine": 1,
                    "totalLines": 1
                }
            },
            "displayPath": "to/file.md"
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2026-05-28T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.141",
        "gitBranch": "main",
        "slug": null
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            assert_eq!(att.agent_id, Some("agent-1".to_string()));
            match att.attachment {
                AttachmentData::File(file) => {
                    assert_eq!(file.filename, "/abs/path/to/file.md");
                    assert_eq!(file.display_path, "to/file.md");
                    let FileAttachmentContent::Text { file: body } = file.content;
                    assert_eq!(body.file_path, "/abs/path/to/file.md");
                    assert_eq!(body.content, "hello");
                    assert_eq!(body.num_lines, 1);
                    assert_eq!(body.start_line, 1);
                    assert_eq!(body.total_lines, 1);
                }
                other => panic!("Expected File attachment, got {:?}", other),
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_nested_memory() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "nested_memory",
            "path": "/abs/CLAUDE.md",
            "content": {
                "path": "/abs/CLAUDE.md",
                "type": "Project",
                "content": "# Hello",
                "contentDiffersFromDisk": false
            },
            "displayPath": "CLAUDE.md"
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2026-05-28T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.141",
        "gitBranch": "main",
        "slug": null
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            assert_eq!(att.agent_id, None);
            match att.attachment {
                AttachmentData::NestedMemory(memory) => {
                    assert_eq!(memory.path, "/abs/CLAUDE.md");
                    assert_eq!(memory.display_path, "CLAUDE.md");
                    assert_eq!(memory.content.r#type, "Project");
                    assert_eq!(memory.content.content, "# Hello");
                    assert!(!memory.content.content_differs_from_disk);
                    assert_eq!(memory.content.raw_content, None);
                }
                other => panic!("Expected NestedMemory attachment, got {:?}", other),
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_directory() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "directory",
            "path": "/Users/brendan/src/project",
            "content": "src\nCargo.toml\nREADME.md",
            "displayPath": "project"
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2026-05-28T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.141",
        "gitBranch": "main",
        "slug": null
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => match att.attachment {
            AttachmentData::Directory(dir) => {
                assert_eq!(dir.path, "/Users/brendan/src/project");
                assert_eq!(dir.content, "src\nCargo.toml\nREADME.md");
                assert_eq!(dir.display_path, "project");
            }
            other => panic!("Expected Directory attachment, got {:?}", other),
        },
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_compact_file_reference() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": "e394396c-27d0-4c3b-aabc-4e914bc70b1f",
        "isSidechain": false,
        "attachment": {
            "type": "compact_file_reference",
            "filename": "/Users/brendan/src/moriarty/crates/moriarty/src/hooks/tests.rs",
            "displayPath": "crates/moriarty/src/hooks/tests.rs"
        },
        "uuid": "f441b451-1f8a-43cf-a0bc-69a5cc70b228",
        "timestamp": "2026-06-05T07:38:04.385Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/Users/brendan/src/moriarty",
        "sessionId": "897f641d-35f9-4a70-8b47-f3c8f3d9e308",
        "version": "2.1.158",
        "gitBranch": "HEAD",
        "slug": "synchronous-sparking-scone"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => match att.attachment {
            AttachmentData::CompactFileReference(file_ref) => {
                assert_eq!(
                    file_ref.filename,
                    "/Users/brendan/src/moriarty/crates/moriarty/src/hooks/tests.rs"
                );
                assert_eq!(file_ref.display_path, "crates/moriarty/src/hooks/tests.rs");
            }
            other => panic!("Expected CompactFileReference attachment, got {:?}", other),
        },
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_compact_file_reference_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "compact_file_reference",
        "filename": "/Users/brendan/src/moriarty/src/main.rs",
        "displayPath": "src/main.rs",
        "extraField": "should be rejected"
    });
    let err_msg = serde_json::from_value::<AttachmentData>(json)
        .expect_err("Should reject unknown fields in CompactFileReference")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("extraField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_attachment_plan_file_reference() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": "7db8acac-5341-4868-9486-59ec84f98fca",
        "isSidechain": false,
        "attachment": {
            "type": "plan_file_reference",
            "planFilePath": "/Users/test/.claude/plans/example-plan.md",
            "planContent": "# Plan: Example feature\n\n## Context\n\nDo the thing.\n"
        },
        "uuid": "00225ffb-fbfa-4a32-82c9-4a9bfed9e6f3",
        "timestamp": "2026-06-05T00:30:01.611Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/Users/test/src/example",
        "sessionId": "45cb4ea8-fbef-4605-9e26-fcbfa729a305",
        "version": "2.1.158",
        "gitBranch": "HEAD",
        "slug": "example-plan"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => match att.attachment {
            AttachmentData::PlanFileReference(plan_ref) => {
                assert_eq!(
                    plan_ref.plan_file_path,
                    "/Users/test/.claude/plans/example-plan.md"
                );
                assert_eq!(
                    plan_ref.plan_content,
                    "# Plan: Example feature\n\n## Context\n\nDo the thing.\n"
                );
            }
            other => panic!("Expected PlanFileReference attachment, got {:?}", other),
        },
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_plan_file_reference_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "plan_file_reference",
        "planFilePath": "/Users/test/.claude/plans/example.md",
        "planContent": "# Plan\n",
        "extraField": "should be rejected"
    });
    let err_msg = serde_json::from_value::<AttachmentData>(json)
        .expect_err("Should reject unknown fields in PlanFileReference")
        .to_string();
    assert!(
        err_msg.contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

// Both fields are required (no `Option`/`#[serde(default)]`); this guards against either silently
// becoming optional, which would let underspecified attachments parse and drop their content.
#[test]
fn test_parse_attachment_plan_file_reference_rejects_missing_required_fields() {
    let missing_content = serde_json::json!({
        "type": "plan_file_reference",
        "planFilePath": "/Users/test/.claude/plans/example.md"
    });
    let err_msg = serde_json::from_value::<AttachmentData>(missing_content)
        .expect_err("Should reject PlanFileReference missing planContent")
        .to_string();
    assert!(
        err_msg.contains("missing field") && err_msg.contains("planContent"),
        "Error should name the missing planContent field, got: {}",
        err_msg
    );

    let missing_path = serde_json::json!({
        "type": "plan_file_reference",
        "planContent": "# Plan\n"
    });
    let err_msg = serde_json::from_value::<AttachmentData>(missing_path)
        .expect_err("Should reject PlanFileReference missing planFilePath")
        .to_string();
    assert!(
        err_msg.contains("missing field") && err_msg.contains("planFilePath"),
        "Error should name the missing planFilePath field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_attachment_skill_listing_with_names() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "skill_listing",
            "content": "- a: does a\n- b: does b",
            "skillCount": 2,
            "isInitial": true,
            "names": ["a", "b"]
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2026-05-28T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.141",
        "gitBranch": "main",
        "slug": null
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => match att.attachment {
            AttachmentData::SkillListing(listing) => {
                assert_eq!(listing.skill_count, 2);
                assert!(listing.is_initial);
                assert_eq!(listing.names, Some(vec!["a".to_string(), "b".to_string()]));
            }
            other => panic!("Expected SkillListing attachment, got {:?}", other),
        },
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

// Older Claude Code logs emit `skill_listing` without `names`; the field must stay optional so those
// transcripts still parse.
#[test]
fn test_parse_attachment_skill_listing_without_names() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "skill_listing",
            "content": "- a: does a",
            "skillCount": 1,
            "isInitial": false
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2026-05-28T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.141",
        "gitBranch": "main",
        "slug": null
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => match att.attachment {
            AttachmentData::SkillListing(listing) => {
                assert_eq!(listing.skill_count, 1);
                assert_eq!(listing.names, None);
            }
            other => panic!("Expected SkillListing attachment, got {:?}", other),
        },
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_file_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "file",
            "filename": "/abs/file.md",
            "content": {
                "type": "text",
                "file": {
                    "filePath": "/abs/file.md",
                    "content": "hi",
                    "numLines": 1,
                    "startLine": 1,
                    "totalLines": 1
                }
            },
            "displayPath": "file.md",
            "extraField": "should fail"
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2026-05-28T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.141",
        "gitBranch": "main",
        "slug": null
    });
    let err = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields on FileAttachment");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
}

#[test]
fn test_parse_attachment_file_body_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "file",
            "filename": "/abs/file.md",
            "content": {
                "type": "text",
                "file": {
                    "filePath": "/abs/file.md",
                    "content": "hi",
                    "numLines": 1,
                    "startLine": 1,
                    "totalLines": 1,
                    "extraField": "should fail"
                }
            },
            "displayPath": "file.md"
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2026-05-28T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.141",
        "gitBranch": "main",
        "slug": null
    });
    let err = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields on FileAttachmentTextBody");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
}

#[test]
fn test_parse_attachment_nested_memory_with_raw_content() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "nested_memory",
            "path": "/abs/CLAUDE.md",
            "content": {
                "path": "/abs/CLAUDE.md",
                "type": "Project",
                "content": "# Processed",
                "contentDiffersFromDisk": true,
                "rawContent": "<!-- template -->\n# Processed"
            },
            "displayPath": "CLAUDE.md"
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2026-05-28T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.158",
        "gitBranch": "main",
        "slug": null
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => match att.attachment {
            AttachmentData::NestedMemory(memory) => {
                assert!(memory.content.content_differs_from_disk);
                assert_eq!(
                    memory.content.raw_content.as_deref(),
                    Some("<!-- template -->\n# Processed")
                );
            }
            other => panic!("Expected NestedMemory attachment, got {:?}", other),
        },
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_nested_memory_content_differs_without_raw_content() {
    // rawContent is documented as present only when contentDiffersFromDisk is
    // true, but the field is optional so Claude Code may omit it even then.
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "nested_memory",
            "path": "/abs/CLAUDE.md",
            "content": {
                "path": "/abs/CLAUDE.md",
                "type": "Project",
                "content": "# Processed",
                "contentDiffersFromDisk": true
            },
            "displayPath": "CLAUDE.md"
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2026-05-28T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.158",
        "gitBranch": "main",
        "slug": null
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => match att.attachment {
            AttachmentData::NestedMemory(memory) => {
                assert!(memory.content.content_differs_from_disk);
                assert_eq!(memory.content.raw_content, None);
            }
            other => panic!("Expected NestedMemory attachment, got {:?}", other),
        },
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_nested_memory_content_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "nested_memory",
            "path": "/abs/CLAUDE.md",
            "content": {
                "path": "/abs/CLAUDE.md",
                "type": "Project",
                "content": "# Hello",
                "contentDiffersFromDisk": false,
                "extraField": "should fail"
            },
            "displayPath": "CLAUDE.md"
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2026-05-28T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.141",
        "gitBranch": "main",
        "slug": null
    });
    let err = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields on NestedMemoryContent");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
}

#[test]
fn test_parse_ai_title_log_line() {
    let json = serde_json::json!({
        "type": "ai-title",
        "aiTitle": "Port Pi extension functionality to Claude",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000"
    });
    let line: LogLine = serde_json::from_value(json).expect("Should parse ai-title");
    match line {
        LogLine::AiTitle(at) => {
            assert_eq!(at.ai_title, "Port Pi extension functionality to Claude");
            assert_eq!(
                at.session_id,
                "550e8400-e29b-41d4-a716-446655440000"
                    .parse::<Uuid>()
                    .unwrap()
            );
        }
        _ => panic!("Expected AiTitle variant"),
    }
}

#[test]
fn test_parse_ai_title_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "ai-title",
        "aiTitle": "Title",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "extraField": "should fail"
    });
    let err = serde_json::from_value::<LogLine>(json).expect_err("Should reject unknown fields");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
}

#[test]
fn test_parse_assistant_log_line_with_attribution_agent() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": true,
        "agentId": "agent-1",
        "userType": "external",
        "cwd": "/test",
        "sessionId": "test-session",
        "version": "2.1.141",
        "gitBranch": "main",
        "message": {
            "id": "msg-1",
            "type": "message",
            "role": "assistant",
            "content": "response",
            "model": "claude-haiku-4-5-20251001",
            "stop_reason": null,
            "usage": {
                "input_tokens": 3,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0,
                "cache_creation": {
                    "ephemeral_5m_input_tokens": 0,
                    "ephemeral_1h_input_tokens": 0
                },
                "output_tokens": 1
            }
        },
        "requestId": "req-1",
        "attributionAgent": "code-quality-reviewer",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-05-28T00:00:00Z"
    });
    let line: AssistantLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(
        line.attribution_agent,
        Some("code-quality-reviewer".to_string())
    );
}

#[test]
fn test_parse_assistant_log_line_with_attribution_skill() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "test-session",
        "version": "2.1.141",
        "gitBranch": "main",
        "message": {
            "id": "msg-1",
            "type": "message",
            "role": "assistant",
            "content": "response",
            "model": "claude-opus-4-7",
            "stop_reason": null,
            "usage": {
                "input_tokens": 3,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0,
                "cache_creation": {
                    "ephemeral_5m_input_tokens": 0,
                    "ephemeral_1h_input_tokens": 0
                },
                "output_tokens": 1
            }
        },
        "requestId": "req-1",
        "attributionSkill": "plannotator-review",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-05-28T00:00:00Z"
    });
    let line: AssistantLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(
        line.attribution_skill,
        Some("plannotator-review".to_string())
    );
    assert_eq!(line.attribution_agent, None);
}

#[test]
fn test_parse_assistant_log_line_without_attribution_agent() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "test-session",
        "version": "2.1.141",
        "gitBranch": "main",
        "message": {
            "id": "msg-1",
            "type": "message",
            "role": "assistant",
            "content": "response",
            "model": "claude-haiku-4-5-20251001",
            "stop_reason": null,
            "usage": {
                "input_tokens": 3,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0,
                "cache_creation": {
                    "ephemeral_5m_input_tokens": 0,
                    "ephemeral_1h_input_tokens": 0
                },
                "output_tokens": 1
            }
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-05-28T00:00:00Z"
    });
    let line: AssistantLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.attribution_agent, None);
    assert_eq!(line.attribution_skill, None);
    assert_eq!(line.attribution_mcp_server, None);
    assert_eq!(line.attribution_mcp_tool, None);
}

#[test]
fn test_parse_assistant_log_line_with_attribution_mcp() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "test-session",
        "version": "2.1.141",
        "gitBranch": "main",
        "message": {
            "id": "msg-1",
            "type": "message",
            "role": "assistant",
            "content": "response",
            "model": "claude-opus-4-7",
            "stop_reason": null,
            "usage": {
                "input_tokens": 3,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0,
                "cache_creation": {
                    "ephemeral_5m_input_tokens": 0,
                    "ephemeral_1h_input_tokens": 0
                },
                "output_tokens": 1
            }
        },
        "requestId": "req-1",
        "attributionMcpServer": "project-tools",
        "attributionMcpTool": "run_tests",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-05-28T00:00:00Z"
    });
    let line: AssistantLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(
        line.attribution_mcp_server,
        Some("project-tools".to_string())
    );
    assert_eq!(line.attribution_mcp_tool, Some("run_tests".to_string()));
    assert_eq!(line.attribution_agent, None);
    assert_eq!(line.attribution_skill, None);
}

#[test]
fn test_parse_assistant_message_with_messages_changed_diagnostics() {
    let json = serde_json::json!({
        "id": "msg-1",
        "type": "message",
        "role": "assistant",
        "content": "response",
        "model": "claude-opus-4-7",
        "stop_reason": "tool_use",
        "usage": {
            "input_tokens": 6,
            "cache_creation_input_tokens": 300010,
            "cache_read_input_tokens": 17819,
            "cache_creation": {
                "ephemeral_5m_input_tokens": 0,
                "ephemeral_1h_input_tokens": 300010
            },
            "output_tokens": 224
        },
        "diagnostics": {
            "cache_miss_reason": {
                "type": "messages_changed",
                "cache_missed_input_tokens": 239706
            }
        }
    });
    let message: AssistantLogMessage = serde_json::from_value(json).unwrap();
    match message.diagnostics {
        Some(Diagnostics {
            cache_miss_reason:
                Some(CacheMissReason::MessagesChanged {
                    cache_missed_input_tokens,
                }),
        }) => assert_eq!(cache_missed_input_tokens, 239706),
        other => panic!("Expected MessagesChanged diagnostics, got {:?}", other),
    }
}

#[test]
fn test_parse_assistant_message_with_system_changed_diagnostics() {
    let json = serde_json::json!({
        "id": "msg-1",
        "type": "message",
        "role": "assistant",
        "content": "response",
        "model": "claude-opus-4-7",
        "stop_reason": "tool_use",
        "usage": {
            "input_tokens": 6,
            "cache_creation_input_tokens": 277136,
            "cache_read_input_tokens": 0,
            "cache_creation": {
                "ephemeral_5m_input_tokens": 0,
                "ephemeral_1h_input_tokens": 277136
            },
            "output_tokens": 200
        },
        "diagnostics": {
            "cache_miss_reason": {
                "type": "system_changed",
                "cache_missed_input_tokens": 277136
            }
        }
    });
    let message: AssistantLogMessage = serde_json::from_value(json).unwrap();
    match message.diagnostics {
        Some(Diagnostics {
            cache_miss_reason:
                Some(CacheMissReason::SystemChanged {
                    cache_missed_input_tokens,
                }),
        }) => assert_eq!(cache_missed_input_tokens, 277136),
        other => panic!("Expected SystemChanged diagnostics, got {:?}", other),
    }
}

// A model fallback (Fable 5 → Opus 4.8 under load) produces a `model_changed` cache-miss
// reason plus per-iteration `model` fields whose values differ across iterations.
#[test]
fn test_parse_assistant_message_with_model_changed_diagnostics_and_iteration_models() {
    let json = serde_json::json!({
        "id": "msg-1",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "fallback", "from": {"model": "claude-fable-5"}, "to": {"model": "claude-opus-4-8"}}],
        "model": "claude-opus-4-8",
        "stop_reason": "tool_use",
        "usage": {
            "input_tokens": 2,
            "cache_creation_input_tokens": 155209,
            "cache_read_input_tokens": 0,
            "cache_creation": {
                "ephemeral_5m_input_tokens": 0,
                "ephemeral_1h_input_tokens": 155209
            },
            "output_tokens": 301,
            "iterations": [
                {
                    "input_tokens": 2,
                    "output_tokens": 160,
                    "cache_read_input_tokens": 0,
                    "cache_creation_input_tokens": 155209,
                    "cache_creation": {"ephemeral_5m_input_tokens": 0, "ephemeral_1h_input_tokens": 155209},
                    "type": "message",
                    "model": "claude-fable-5"
                },
                {
                    "input_tokens": 2,
                    "output_tokens": 301,
                    "cache_read_input_tokens": 0,
                    "cache_creation_input_tokens": 155209,
                    "cache_creation": {"ephemeral_5m_input_tokens": 0, "ephemeral_1h_input_tokens": 155209},
                    "type": "fallback_message",
                    "model": "claude-opus-4-8"
                }
            ]
        },
        "diagnostics": {
            "cache_miss_reason": {
                "type": "model_changed",
                "cache_missed_input_tokens": 133891
            }
        }
    });
    let message: AssistantLogMessage = serde_json::from_value(json).unwrap();
    match message.diagnostics {
        Some(Diagnostics {
            cache_miss_reason:
                Some(CacheMissReason::ModelChanged {
                    cache_missed_input_tokens,
                }),
        }) => assert_eq!(cache_missed_input_tokens, 133891),
        other => panic!("Expected ModelChanged diagnostics, got {:?}", other),
    }
    let iterations = message.usage.iterations.expect("iterations present");
    assert_eq!(iterations.len(), 2);
    assert_eq!(
        iterations[0].model.as_ref().map(|m| m.raw()),
        Some("claude-fable-5")
    );
    assert_eq!(iterations[0].r#type.as_deref(), Some("message"));
    assert_eq!(
        iterations[1].model.as_ref().map(|m| m.raw()),
        Some("claude-opus-4-8")
    );
    assert_eq!(iterations[1].r#type.as_deref(), Some("fallback_message"));
}

#[test]
fn test_parse_assistant_message_with_tools_changed_diagnostics() {
    let json = serde_json::json!({
        "id": "msg-1",
        "type": "message",
        "role": "assistant",
        "content": "response",
        "model": "claude-opus-4-7",
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 6,
            "cache_creation_input_tokens": 45701,
            "cache_read_input_tokens": 0,
            "cache_creation": {
                "ephemeral_5m_input_tokens": 0,
                "ephemeral_1h_input_tokens": 45701
            },
            "output_tokens": 285
        },
        "diagnostics": {
            "cache_miss_reason": {
                "type": "tools_changed",
                "cache_missed_input_tokens": 39797
            }
        }
    });
    let message: AssistantLogMessage = serde_json::from_value(json).unwrap();
    match message.diagnostics {
        Some(Diagnostics {
            cache_miss_reason:
                Some(CacheMissReason::ToolsChanged {
                    cache_missed_input_tokens,
                }),
        }) => assert_eq!(cache_missed_input_tokens, 39797),
        other => panic!("Expected ToolsChanged diagnostics, got {:?}", other),
    }
}

#[test]
fn test_parse_assistant_message_with_previous_message_not_found_diagnostics() {
    let json = serde_json::json!({
        "id": "msg-1",
        "type": "message",
        "role": "assistant",
        "content": "response",
        "model": "claude-opus-4-7",
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 1,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 0,
            "cache_creation": {
                "ephemeral_5m_input_tokens": 0,
                "ephemeral_1h_input_tokens": 0
            },
            "output_tokens": 1
        },
        "diagnostics": {
            "cache_miss_reason": {
                "type": "previous_message_not_found"
            }
        }
    });
    let message: AssistantLogMessage = serde_json::from_value(json).unwrap();
    assert_eq!(
        message.diagnostics,
        Some(Diagnostics {
            cache_miss_reason: Some(CacheMissReason::PreviousMessageNotFound),
        })
    );
}

#[test]
fn test_parse_assistant_message_with_unavailable_cache_miss_reason() {
    let json = serde_json::json!({
        "id": "msg-1",
        "type": "message",
        "role": "assistant",
        "content": "response",
        "model": "claude-opus-4-7",
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 1,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 0,
            "cache_creation": {
                "ephemeral_5m_input_tokens": 0,
                "ephemeral_1h_input_tokens": 0
            },
            "output_tokens": 1
        },
        "diagnostics": {
            "cache_miss_reason": {
                "type": "unavailable"
            }
        }
    });
    let message: AssistantLogMessage = serde_json::from_value(json).unwrap();
    assert_eq!(
        message.diagnostics,
        Some(Diagnostics {
            cache_miss_reason: Some(CacheMissReason::Unavailable),
        })
    );
}

#[test]
fn test_parse_assistant_message_with_null_diagnostics() {
    let json = serde_json::json!({
        "id": "msg-1",
        "type": "message",
        "role": "assistant",
        "content": "response",
        "model": "claude-opus-4-7",
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 1,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 0,
            "cache_creation": {
                "ephemeral_5m_input_tokens": 0,
                "ephemeral_1h_input_tokens": 0
            },
            "output_tokens": 1
        },
        "diagnostics": null
    });
    let message: AssistantLogMessage = serde_json::from_value(json).unwrap();
    assert_eq!(message.diagnostics, None);
}

#[test]
fn test_parse_assistant_message_with_null_cache_miss_reason() {
    let json = serde_json::json!({
        "id": "msg-1",
        "type": "message",
        "role": "assistant",
        "content": "response",
        "model": "claude-opus-4-7",
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 1,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 0,
            "cache_creation": {
                "ephemeral_5m_input_tokens": 0,
                "ephemeral_1h_input_tokens": 0
            },
            "output_tokens": 1
        },
        "diagnostics": {
            "cache_miss_reason": null
        }
    });
    let message: AssistantLogMessage = serde_json::from_value(json).unwrap();
    assert_eq!(
        message.diagnostics,
        Some(Diagnostics {
            cache_miss_reason: None,
        })
    );
}

#[test]
fn test_parse_cache_miss_reason_rejects_unknown_fields_in_variant() {
    let json = serde_json::json!({
        "id": "msg-1",
        "type": "message",
        "role": "assistant",
        "content": "response",
        "model": "claude-opus-4-7",
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 1,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 0,
            "cache_creation": {
                "ephemeral_5m_input_tokens": 0,
                "ephemeral_1h_input_tokens": 0
            },
            "output_tokens": 1
        },
        "diagnostics": {
            "cache_miss_reason": {
                "type": "messages_changed",
                "cache_missed_input_tokens": 100,
                "extraField": "should fail"
            }
        }
    });
    let err = serde_json::from_value::<AssistantLogMessage>(json)
        .expect_err("Should reject unknown fields in CacheMissReason variant");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
}

#[test]
fn test_parse_diagnostics_rejects_unknown_fields() {
    let json = serde_json::json!({
        "id": "msg-1",
        "type": "message",
        "role": "assistant",
        "content": "response",
        "model": "claude-opus-4-7",
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 1,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 0,
            "cache_creation": {
                "ephemeral_5m_input_tokens": 0,
                "ephemeral_1h_input_tokens": 0
            },
            "output_tokens": 1
        },
        "diagnostics": {
            "cache_miss_reason": null,
            "extraField": "should fail"
        }
    });
    let err = serde_json::from_value::<AssistantLogMessage>(json)
        .expect_err("Should reject unknown fields in diagnostics");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
}

#[test]
fn test_parse_attachment_deferred_tools_delta_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "deferred_tools_delta",
            "addedNames": [],
            "addedLines": [],
            "removedNames": [],
            "extraField": "should fail"
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2026-05-28T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.141",
        "gitBranch": "main",
        "slug": null
    });
    let err = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in deferred_tools_delta");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
}

#[test]
fn test_parse_last_prompt_log_line_with_both_fields() {
    let json = r#"{"type":"last-prompt","lastPrompt":"Fix the bug","leafUuid":"4629e822-f089-4f87-aa1f-7d93ebe10d81","sessionId":"550e8400-e29b-41d4-a716-446655440000"}"#;
    let log_line: LogLine = serde_json::from_str(json).unwrap();
    match log_line {
        LogLine::LastPrompt(lp) => {
            assert_eq!(lp.last_prompt.as_deref(), Some("Fix the bug"));
            assert_eq!(
                lp.leaf_uuid,
                Some(
                    "4629e822-f089-4f87-aa1f-7d93ebe10d81"
                        .parse::<Uuid>()
                        .unwrap()
                )
            );
            assert_eq!(
                lp.session_id,
                "550e8400-e29b-41d4-a716-446655440000"
                    .parse::<Uuid>()
                    .unwrap()
            );
        }
        other => panic!("Expected LastPrompt, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_hook_success() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "hook_success",
            "hookName": "PreToolUse:Bash",
            "toolUseID": "toolu_123",
            "hookEvent": "PreToolUse",
            "content": "",
            "stdout": "{}\n",
            "stderr": "",
            "exitCode": 0,
            "command": "moriarty hooks exec",
            "durationMs": 30
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2025-01-01T00:00:00Z",
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.104",
        "gitBranch": "main"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::HookSuccess(hook) = &att.attachment {
                assert_eq!(hook.hook_name, "PreToolUse:Bash");
                assert_eq!(hook.exit_code, 0);
                assert_eq!(hook.duration_ms, 30);
            } else {
                panic!("Expected HookSuccess, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_hook_permission_decision() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "hook_permission_decision",
            "decision": "allow",
            "toolUseID": "toolu_01CF2aDiUqw4Q9vvgSncRUz6",
            "hookEvent": "PermissionRequest"
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2026-05-28T22:02:12.611Z",
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.141",
        "gitBranch": "HEAD"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::HookPermissionDecision(hook) = &att.attachment {
                assert_eq!(hook.decision, PermissionDecisionKind::Allow);
                assert_eq!(hook.tool_use_id, "toolu_01CF2aDiUqw4Q9vvgSncRUz6");
                assert_eq!(hook.hook_event, "PermissionRequest");
            } else {
                panic!("Expected HookPermissionDecision, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

fn hook_permission_decision_envelope(attachment: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": attachment,
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2026-05-28T22:02:12.611Z",
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.141",
        "gitBranch": "HEAD"
    })
}

#[test]
fn test_parse_attachment_hook_permission_decision_deny() {
    let json = hook_permission_decision_envelope(serde_json::json!({
        "type": "hook_permission_decision",
        "decision": "deny",
        "toolUseID": "toolu_deny",
        "hookEvent": "PermissionRequest"
    }));
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => match &att.attachment {
            AttachmentData::HookPermissionDecision(hook) => {
                assert_eq!(hook.decision, PermissionDecisionKind::Deny);
            }
            other => panic!("Expected HookPermissionDecision, got {:?}", other),
        },
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_hook_permission_decision_ask() {
    let json = hook_permission_decision_envelope(serde_json::json!({
        "type": "hook_permission_decision",
        "decision": "ask",
        "toolUseID": "toolu_ask",
        "hookEvent": "PermissionRequest"
    }));
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => match &att.attachment {
            AttachmentData::HookPermissionDecision(hook) => {
                assert_eq!(hook.decision, PermissionDecisionKind::Ask);
            }
            other => panic!("Expected HookPermissionDecision, got {:?}", other),
        },
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_hook_permission_decision_rejects_unknown_decision() {
    let json = hook_permission_decision_envelope(serde_json::json!({
        "type": "hook_permission_decision",
        "decision": "block",
        "toolUseID": "toolu_block",
        "hookEvent": "PermissionRequest"
    }));
    let err = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown PermissionDecisionKind values");
    assert!(
        err.to_string().contains("unknown variant"),
        "Error should mention unknown variant, got: {}",
        err
    );
}

#[test]
fn test_parse_attachment_hook_permission_decision_rejects_unknown_fields() {
    let json = hook_permission_decision_envelope(serde_json::json!({
        "type": "hook_permission_decision",
        "decision": "allow",
        "toolUseID": "toolu_extra",
        "hookEvent": "PermissionRequest",
        "extraField": "should fail"
    }));
    let err = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in HookPermissionDecision");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
}

#[test]
fn test_parse_attachment_plan_mode() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "plan_mode",
            "reminderType": "full",
            "isSubAgent": false,
            "planFilePath": "/tmp/plan.md",
            "planExists": true
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2025-01-01T00:00:00Z",
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.104",
        "gitBranch": "main"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            assert!(matches!(att.attachment, AttachmentData::PlanMode(_)));
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_task_reminder() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "task_reminder",
            "content": [{
                "id": "1",
                "subject": "Fix bug",
                "description": "Fix the parsing bug",
                "activeForm": "Fixing bug",
                "status": "in_progress",
                "blocks": [],
                "blockedBy": []
            }],
            "itemCount": 1
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2025-01-01T00:00:00Z",
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.104",
        "gitBranch": "main"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::TaskReminder(reminder) = &att.attachment {
                assert_eq!(reminder.item_count, 1);
                assert_eq!(reminder.content[0].subject, "Fix bug");
            } else {
                panic!("Expected TaskReminder, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_task_reminder_without_active_form() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "task_reminder",
            "content": [{
                "id": "1",
                "subject": "Fix bug",
                "description": "Fix the parsing bug",
                "status": "in_progress",
                "blocks": [],
                "blockedBy": []
            }],
            "itemCount": 1
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2025-01-01T00:00:00Z",
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.104",
        "gitBranch": "main"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::TaskReminder(reminder) = &att.attachment {
                assert_eq!(reminder.content[0].active_form, None);
            } else {
                panic!("Expected TaskReminder, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "deferred_tools_delta",
            "addedNames": [],
            "addedLines": [],
            "removedNames": []
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2025-01-01T00:00:00Z",
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.104",
        "gitBranch": "main",
        "extraField": "should fail"
    });
    let err = serde_json::from_value::<LogLine>(json).expect_err("Should reject unknown fields");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
}

#[test]
fn test_parse_assistant_log_line_with_entrypoint() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "test-session",
        "version": "2.1.104",
        "gitBranch": "main",
        "message": {
            "id": "msg-1",
            "type": "message",
            "role": "assistant",
            "content": "response",
            "model": "claude-sonnet-4-6",
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
        "timestamp": "2025-01-01T00:00:00Z",
        "entrypoint": "cli"
    });
    let line: AssistantLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.entrypoint, Some("cli".to_string()));
}

#[test]
fn test_parse_assistant_log_line_without_entrypoint() {
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
    assert_eq!(line.entrypoint, None);
}

#[test]
fn test_parse_iteration_with_fields() {
    let json = serde_json::json!({
        "input_tokens": 3,
        "output_tokens": 131,
        "cache_read_input_tokens": 7407,
        "cache_creation_input_tokens": 1841,
        "cache_creation": {
            "ephemeral_5m_input_tokens": 1841,
            "ephemeral_1h_input_tokens": 0
        },
        "type": "message"
    });
    let iteration: Iteration = serde_json::from_value(json).unwrap();
    assert_eq!(iteration.input_tokens, Some(3));
    assert_eq!(iteration.output_tokens, Some(131));
    assert_eq!(iteration.cache_read_input_tokens, Some(7407));
    assert_eq!(iteration.cache_creation_input_tokens, Some(1841));
    assert_eq!(iteration.r#type, Some("message".to_string()));
    assert!(iteration.cache_creation.is_some());
}

#[test]
fn test_parse_iteration_empty() {
    let json = serde_json::json!({});
    let iteration: Iteration = serde_json::from_value(json).unwrap();
    assert_eq!(iteration.input_tokens, None);
    assert_eq!(iteration.output_tokens, None);
    assert_eq!(iteration.r#type, None);
}

#[test]
fn test_parse_attachment_mcp_instructions_delta() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "mcp_instructions_delta",
            "addedNames": ["git-read-only"],
            "addedBlocks": ["## git-read-only\nServer instructions"],
            "removedNames": []
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2025-01-01T00:00:00Z",
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.104",
        "gitBranch": "main"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::McpInstructionsDelta(delta) = &att.attachment {
                assert_eq!(delta.added_names, vec!["git-read-only"]);
                assert_eq!(delta.removed_names.len(), 0);
            } else {
                panic!("Expected McpInstructionsDelta, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_plan_mode_exit() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "plan_mode_exit",
            "planFilePath": "/tmp/plan.md",
            "planExists": true
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2025-01-01T00:00:00Z",
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.104",
        "gitBranch": "main"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::PlanModeExit(exit) = &att.attachment {
                assert_eq!(exit.plan_file_path, "/tmp/plan.md");
                assert!(exit.plan_exists);
            } else {
                panic!("Expected PlanModeExit, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_queued_command() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "queued_command",
            "prompt": "Run the tests",
            "commandMode": "prompt"
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2025-01-01T00:00:00Z",
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.104",
        "gitBranch": "main"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::QueuedCommand(cmd) = &att.attachment {
                assert_eq!(cmd.prompt, "Run the tests");
                assert_eq!(cmd.command_mode, "prompt");
            } else {
                panic!("Expected QueuedCommand, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_skill_listing() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "skill_listing",
            "content": "- commit: Create commits\n- review: Review PRs",
            "skillCount": 2,
            "isInitial": true
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2025-01-01T00:00:00Z",
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.104",
        "gitBranch": "main"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::SkillListing(listing) = &att.attachment {
                assert_eq!(listing.skill_count, 2);
                assert!(listing.is_initial);
            } else {
                panic!("Expected SkillListing, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_auto_mode() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "auto_mode",
            "reminderType": "full"
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2025-01-01T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.104",
        "gitBranch": "main",
        "slug": "test-slug"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::AutoMode(auto) = &att.attachment {
                assert_eq!(auto.reminder_type, "full");
            } else {
                panic!("Expected AutoMode, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_auto_mode_exit() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "auto_mode_exit"
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2025-01-01T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.104",
        "gitBranch": "main",
        "slug": "test-slug"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            assert!(matches!(att.attachment, AttachmentData::AutoModeExit(_)));
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_command_permissions() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "command_permissions",
            "allowedTools": ["Bash", "Read"]
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2025-01-01T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.104",
        "gitBranch": "main",
        "slug": "test-slug"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::CommandPermissions(perms) = &att.attachment {
                assert_eq!(perms.allowed_tools, vec!["Bash", "Read"]);
            } else {
                panic!("Expected CommandPermissions, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_date_change() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "date_change",
            "newDate": "2026-06-01"
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2025-01-01T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.141",
        "gitBranch": "main",
        "slug": "test-slug"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::DateChange(change) = &att.attachment {
                assert_eq!(
                    change.new_date,
                    chrono::NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
                );
            } else {
                panic!("Expected DateChange, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_date_change_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "date_change",
            "newDate": "2026-06-01",
            "extraField": "should fail"
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2025-01-01T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.141",
        "gitBranch": "main",
        "slug": "test-slug"
    });
    let err = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in date_change");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
}

#[test]
fn test_parse_attachment_edited_text_file() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "edited_text_file",
            "filename": "/src/main.rs",
            "snippet": "fn main() {\n    println!(\"hello\");\n}"
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2025-01-01T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.104",
        "gitBranch": "main",
        "slug": "test-slug"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::EditedTextFile(edited) = &att.attachment {
                assert_eq!(edited.filename, "/src/main.rs");
                assert_eq!(edited.snippet, "fn main() {\n    println!(\"hello\");\n}");
            } else {
                panic!("Expected EditedTextFile, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_plan_mode_reentry() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "plan_mode_reentry",
            "planFilePath": "/Users/test/.claude/plans/my-plan.md"
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2025-01-01T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.104",
        "gitBranch": "main",
        "slug": "test-slug"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::PlanModeReentry(reentry) = &att.attachment {
                assert_eq!(
                    reentry.plan_file_path,
                    "/Users/test/.claude/plans/my-plan.md"
                );
            } else {
                panic!("Expected PlanModeReentry, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_hook_non_blocking_error() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "hook_non_blocking_error",
            "hookName": "PostToolUse:ExitPlanMode",
            "toolUseID": "toolu_01MpjtQCRgkG3zhy3rWBNGfx",
            "hookEvent": "PostToolUse",
            "stderr": "hook failed",
            "stdout": "",
            "exitCode": 1,
            "command": "moriarty hooks exec",
            "durationMs": 107
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2025-01-01T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.104",
        "gitBranch": "main",
        "slug": "test-slug"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::HookNonBlockingError(err) = &att.attachment {
                assert_eq!(err.hook_name, "PostToolUse:ExitPlanMode");
                assert_eq!(err.tool_use_id, "toolu_01MpjtQCRgkG3zhy3rWBNGfx");
                assert_eq!(err.exit_code, 1);
                assert_eq!(err.duration_ms, 107);
            } else {
                panic!("Expected HookNonBlockingError, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_hook_blocking_error() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "attachment": {
            "type": "hook_blocking_error",
            "hookName": "Stop",
            "toolUseID": "25ac3468-1b14-498d-b231-f6a80674f20d",
            "hookEvent": "Stop",
            "blockingError": {
                "blockingError": "Checks failed:\n\nCheck 'semgrep' failed with exit code 2",
                "command": "moriarty hooks exec"
            }
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2025-01-01T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.104",
        "gitBranch": "main",
        "slug": "test-slug"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::HookBlockingError(err) = &att.attachment {
                assert_eq!(err.hook_name, "Stop");
                assert_eq!(err.tool_use_id, "25ac3468-1b14-498d-b231-f6a80674f20d");
                assert_eq!(err.hook_event, "Stop");
                assert_eq!(err.blocking_error.command, "moriarty hooks exec");
                assert_eq!(
                    err.blocking_error.blocking_error,
                    "Checks failed:\n\nCheck 'semgrep' failed with exit code 2"
                );
            } else {
                panic!("Expected HookBlockingError, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_hook_blocking_error_rejects_unknown_nested_fields() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "hook_blocking_error",
            "hookName": "Stop",
            "toolUseID": "25ac3468-1b14-498d-b231-f6a80674f20d",
            "hookEvent": "Stop",
            "blockingError": {
                "blockingError": "some error",
                "command": "moriarty hooks exec",
                "unexpectedField": true
            }
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2025-01-01T00:00:00Z",
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.104",
        "gitBranch": "main"
    });
    let err = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in BlockingErrorDetails");
    assert!(err.to_string().contains("unknown field"), "{err}");
}

#[test]
fn test_parse_attachment_hook_cancelled() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "attachment": {
            "type": "hook_cancelled",
            "hookName": "Stop",
            "toolUseID": "21ef6391-1417-40ab-b9ba-e55f5684c31a",
            "hookEvent": "Stop",
            "command": "moriarty hooks exec",
            "durationMs": 3184
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2025-01-01T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.104",
        "gitBranch": "main",
        "slug": "test-slug"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::HookCancelled(cancelled) = &att.attachment {
                assert_eq!(cancelled.hook_name, "Stop");
                assert_eq!(
                    cancelled.tool_use_id,
                    "21ef6391-1417-40ab-b9ba-e55f5684c31a"
                );
                assert_eq!(cancelled.hook_event, "Stop");
                assert_eq!(cancelled.command, "moriarty hooks exec");
                assert_eq!(cancelled.duration_ms, 3184);
            } else {
                panic!("Expected HookCancelled, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_hook_system_message() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "attachment": {
            "type": "hook_system_message",
            "content": "Checks failed:\n\nCheck 'semgrep' failed with exit code 2",
            "hookName": "Stop",
            "toolUseID": "25ac3468-1b14-498d-b231-f6a80674f20d",
            "hookEvent": "Stop"
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2025-01-01T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.104",
        "gitBranch": "main",
        "slug": "test-slug"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::HookSystemMessage(msg) = &att.attachment {
                assert_eq!(msg.hook_name, "Stop");
                assert_eq!(msg.tool_use_id, "25ac3468-1b14-498d-b231-f6a80674f20d");
                assert_eq!(msg.hook_event, "Stop");
                assert_eq!(
                    msg.content,
                    "Checks failed:\n\nCheck 'semgrep' failed with exit code 2"
                );
            } else {
                panic!("Expected HookSystemMessage, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_turn_duration_with_message_count() {
    let json = serde_json::json!({
        "type": "system",
        "subtype": "turn_duration",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.104",
        "gitBranch": "main",
        "slug": "test-slug",
        "durationMs": 5678,
        "timestamp": "2025-01-16T00:00:00Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "isMeta": false,
        "entrypoint": "cli",
        "messageCount": 4
    });
    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse turn_duration with messageCount");
    match line {
        LogLine::System(SystemLogLine::TurnDuration(duration)) => {
            assert_eq!(duration.duration_ms, 5678);
            assert_eq!(duration.message_count, Some(4));
            assert_eq!(duration.entrypoint, Some("cli".to_string()));
        }
        _ => panic!("Expected System(TurnDuration) variant"),
    }
}

#[test]
fn test_parse_user_log_line_with_origin() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.104",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "origin": {"kind": "task-notification"}
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    let origin = line.origin.unwrap();
    assert_eq!(origin.kind, "task-notification");
}

#[test]
fn test_parse_user_log_line_with_null_origin() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.104",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "origin": null
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.origin, None);
}

#[test]
fn test_parse_user_log_line_without_origin() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.0.50",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.origin, None);
}

#[test]
fn test_parse_user_log_line_with_interrupted_message_id() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.104",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "interruptedMessageId": "msg_01Hs25nR7X58UvPnVBqreDRB"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(
        line.interrupted_message_id,
        Some("msg_01Hs25nR7X58UvPnVBqreDRB".to_string())
    );
}

#[test]
fn test_parse_user_log_line_with_null_interrupted_message_id() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.104",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "interruptedMessageId": null
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.interrupted_message_id, None);
}

#[test]
fn test_parse_message_origin_rejects_unknown_fields() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.104",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "origin": {"kind": "task-notification", "extraField": "should fail"}
    });
    let err =
        serde_json::from_value::<UserLogLine>(json).expect_err("Should reject unknown fields");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
}

#[test]
fn test_parse_user_log_line_with_mcp_meta() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.158",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "mcpMeta": {
            "structuredContent": {
                "exit_code": 0,
                "stderr": "",
                "stdout": "diff output"
            }
        }
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    let mcp_meta = line.mcp_meta.expect("mcpMeta should be present");
    let Some(ToolUseResult::Map(content)) = mcp_meta.structured_content else {
        panic!("structuredContent from an MCP server is a JSON object");
    };
    assert_eq!(content["exit_code"], serde_json::json!(0));
    assert_eq!(content["stderr"], serde_json::json!(""));
    assert_eq!(content["stdout"], serde_json::json!("diff output"));
}

#[test]
fn test_parse_user_log_line_without_mcp_meta() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.158",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.mcp_meta, None);
}

#[test]
fn test_parse_mcp_meta_rejects_unknown_fields() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.158",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "mcpMeta": {
            "structuredContent": {"exit_code": 0},
            "extraField": "should fail"
        }
    });
    let err =
        serde_json::from_value::<UserLogLine>(json).expect_err("Should reject unknown fields");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
}

#[test]
fn test_parse_user_log_line_with_mcp_meta_string_content() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.158",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "mcpMeta": {"structuredContent": "plain text result"}
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    let mcp_meta = line.mcp_meta.expect("mcpMeta should be present");
    assert_eq!(
        mcp_meta.structured_content,
        Some(ToolUseResult::String("plain text result".to_string()))
    );
}

#[test]
fn test_parse_user_log_line_with_null_structured_content() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.158",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "mcpMeta": {"structuredContent": null}
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    let mcp_meta = line.mcp_meta.expect("mcpMeta should be present");
    assert_eq!(mcp_meta.structured_content, None);
}

#[test]
fn test_parse_user_log_line_with_null_mcp_meta() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.158",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "mcpMeta": null
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.mcp_meta, None);
}

#[test]
fn test_parse_user_log_line_with_mcp_meta_and_tool_use_result() {
    // The same MCP tool-result turn carries both the rendered string form (`toolUseResult`) and
    // the structured object form (`mcpMeta.structuredContent`); both must decode independently.
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.158",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "toolUseResult": "rendered string result",
        "mcpMeta": {"structuredContent": {"exit_code": 0}}
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(
        line.tool_use_result,
        Some(ToolUseResult::String("rendered string result".to_string()))
    );
    let mcp_meta = line.mcp_meta.expect("mcpMeta should be present");
    let Some(ToolUseResult::Map(content)) = mcp_meta.structured_content else {
        panic!("structuredContent from an MCP server is a JSON object");
    };
    assert_eq!(content["exit_code"], serde_json::json!(0));
}

#[test]
fn test_parse_user_log_line_with_empty_mcp_meta() {
    // An empty `mcpMeta` (no `structuredContent` key) must parse: serde defaults the absent
    // `Option` field to `None` even under `deny_unknown_fields`, so an MCP result without
    // structured content does not drop the whole log line.
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.158",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "mcpMeta": {}
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    let mcp_meta = line.mcp_meta.expect("mcpMeta should be present");
    assert_eq!(mcp_meta.structured_content, None);
}

#[test]
fn test_parse_assistant_log_line_with_null_entrypoint() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "test-session",
        "version": "2.1.104",
        "gitBranch": "main",
        "message": {
            "id": "msg-1",
            "type": "message",
            "role": "assistant",
            "content": "response",
            "model": "claude-sonnet-4-6",
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
        "timestamp": "2025-01-01T00:00:00Z",
        "entrypoint": null
    });
    let line: AssistantLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.entrypoint, None);
}
