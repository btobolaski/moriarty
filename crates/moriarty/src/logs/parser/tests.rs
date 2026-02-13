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

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse stop_hook_summary with empty arrays");

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
                assert_eq!(data.normalized_messages.len(), 4);
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
                assert_eq!(data.normalized_messages.len(), 1);
                match &data.normalized_messages[0] {
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
                assert_eq!(data.normalized_messages.len(), 1);
                match &data.normalized_messages[0] {
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
                assert_eq!(data.normalized_messages.len(), 1);
                match &data.normalized_messages[0] {
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
                assert_eq!(data.normalized_messages.len(), 1);
                match &data.normalized_messages[0] {
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
                assert_eq!(data.normalized_messages.len(), 1);
                match &data.normalized_messages[0] {
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
