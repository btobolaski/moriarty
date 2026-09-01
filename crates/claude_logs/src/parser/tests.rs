use super::*;

/// `target` selects the object the extra field is inserted into, so a caller can exercise a nested
/// payload's strictness (an artifact entry) as well as the line's own.
fn assert_log_line_rejects_extra_field(
    mut json: serde_json::Value,
    target: fn(&mut serde_json::Value) -> &mut serde_json::Value,
) {
    target(&mut json)
        .as_object_mut()
        .expect("mutation target is a JSON object")
        .insert("extraField".to_string(), serde_json::json!("should fail"));

    let err = serde_json::from_value::<LogLine>(json).expect_err("Should reject unknown fields");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
}

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

fn atis_latch_json() -> serde_json::Value {
    serde_json::json!({
        "type": "atis-latch",
        "atis": "",
        "sessionId": "027ad547-7c91-4ffa-9b42-03d81d73828c"
    })
}

#[test]
fn test_parse_atis_latch_log_line() {
    let LogLine::AtisLatch(latch) = serde_json::from_value(atis_latch_json()).unwrap() else {
        panic!("expected an atis-latch log line");
    };
    assert_eq!(latch.atis, "");
    assert_eq!(
        latch.session_id,
        "027ad547-7c91-4ffa-9b42-03d81d73828c"
            .parse::<Uuid>()
            .unwrap()
    );
}

#[test]
fn test_parse_atis_latch_rejects_unknown_fields() {
    let mut json = atis_latch_json();
    json.as_object_mut()
        .expect("fixture is a JSON object")
        .insert("extraField".to_string(), serde_json::json!("should fail"));
    let err = serde_json::from_value::<LogLine>(json).expect_err("Should reject unknown fields");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
}

fn frame_link_json() -> serde_json::Value {
    serde_json::json!({
        "type": "frame-link",
        "sessionId": "c0d69030-a005-4543-8798-38425c069dc7",
        "path": "/tmp/scratchpad/h2-service-dependency-graph.md",
        "frameUrl": "https://claude.ai/code/artifact/453110f4-9db6-4428-ad3c-9df991340402",
        "title": "h2-service-dependency-graph.md",
        "artifactCount": 1,
        "timestamp": "2026-08-31T15:27:02.823Z"
    })
}

#[test]
fn test_parse_frame_link_log_line() {
    let LogLine::FrameLink(FrameLink::Published(link)) =
        serde_json::from_value(frame_link_json()).unwrap()
    else {
        panic!("expected a published frame-link log line");
    };
    assert_eq!(link.path, "/tmp/scratchpad/h2-service-dependency-graph.md");
    assert_eq!(
        link.frame_url,
        "https://claude.ai/code/artifact/453110f4-9db6-4428-ad3c-9df991340402"
    );
    assert_eq!(link.title, "h2-service-dependency-graph.md");
    assert_eq!(link.artifact_count, 1);
}

#[test]
fn test_parse_frame_link_log_line_without_artifact_details() {
    let json = serde_json::json!({
        "type": "frame-link",
        "sessionId": "c0d69030-a005-4543-8798-38425c069dc7",
        "artifactCount": 1,
        "timestamp": "2026-08-31T16:03:24.519Z"
    });
    let LogLine::FrameLink(FrameLink::Bare(link)) = serde_json::from_value(json).unwrap() else {
        panic!("expected a bare frame-link log line");
    };
    assert_eq!(link.artifact_count, 1);
}

#[test]
fn test_parse_frame_link_rejects_partial_artifact_details() {
    let mut json = frame_link_json();
    json.as_object_mut()
        .expect("fixture is a JSON object")
        .remove("title");
    let err = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject a half-present artifact triple");
    assert!(
        err.to_string().contains("did not match any variant"),
        "Error should report that no frame-link shape matched, got: {}",
        err
    );
}

fn artifact_comment_monitor_json() -> serde_json::Value {
    serde_json::json!({
        "type": "artifact-comment-monitor",
        "v": 1,
        "sessionId": "c0d69030-a005-4543-8798-38425c069dc7",
        "artifacts": {
            "453110f4-9db6-4428-ad3c-9df991340402": {
                "state": "armed",
                "writtenAtMs": 1788190023083u64,
                "title": "h2-service-dependency-graph.md"
            }
        }
    })
}

#[test]
fn test_parse_artifact_comment_monitor_log_line() {
    let LogLine::ArtifactCommentMonitor(monitor) =
        serde_json::from_value(artifact_comment_monitor_json()).unwrap()
    else {
        panic!("expected an artifact-comment-monitor log line");
    };
    assert_eq!(monitor.v, 1);
    let artifact = monitor
        .artifacts
        .get(
            &"453110f4-9db6-4428-ad3c-9df991340402"
                .parse::<Uuid>()
                .unwrap(),
        )
        .expect("fixture has one monitored artifact");
    assert_eq!(artifact.state, "armed");
    assert_eq!(artifact.written_at_ms, 1788190023083);
    assert_eq!(artifact.title, "h2-service-dependency-graph.md");
}

#[test]
fn test_parse_artifact_comment_monitor_rejects_unknown_artifact_fields() {
    assert_log_line_rejects_extra_field(artifact_comment_monitor_json(), |json| {
        &mut json["artifacts"]["453110f4-9db6-4428-ad3c-9df991340402"]
    });
}

fn artifact_autoreact_ledger_json() -> serde_json::Value {
    serde_json::json!({
        "type": "artifact-autoreact-ledger",
        "v": 1,
        "sessionId": "c0d69030-a005-4543-8798-38425c069dc7",
        "accountUuid": "ef6692be-04e0-43f8-a5a2-3212d362e10b",
        "artifacts": {
            "453110f4-9db6-4428-ad3c-9df991340402": {
                "savedAt": 1788190028700u64,
                "stampHighWater": null,
                "everBaselined": true,
                "everHadThreads": false,
                "turnTimestamps": [],
                "threads": []
            }
        }
    })
}

#[test]
fn test_parse_artifact_autoreact_ledger_log_line() {
    let LogLine::ArtifactAutoreactLedger(ledger) =
        serde_json::from_value(artifact_autoreact_ledger_json()).unwrap()
    else {
        panic!("expected an artifact-autoreact-ledger log line");
    };
    assert_eq!(
        ledger.account_uuid,
        "ef6692be-04e0-43f8-a5a2-3212d362e10b"
            .parse::<Uuid>()
            .unwrap()
    );
    let artifact = ledger
        .artifacts
        .get(
            &"453110f4-9db6-4428-ad3c-9df991340402"
                .parse::<Uuid>()
                .unwrap(),
        )
        .expect("fixture has one ledger artifact");
    assert_eq!(artifact.saved_at, 1788190028700);
    assert_eq!(artifact.stamp_high_water, None);
    assert!(artifact.ever_baselined);
    assert!(!artifact.ever_had_threads);
    assert!(artifact.turn_timestamps.is_empty());
    assert!(artifact.threads.is_empty());
}

#[test]
fn test_parse_artifact_autoreact_ledger_rejects_unknown_artifact_fields() {
    assert_log_line_rejects_extra_field(artifact_autoreact_ledger_json(), |json| {
        &mut json["artifacts"]["453110f4-9db6-4428-ad3c-9df991340402"]
    });
}

#[test]
fn test_parse_fork_context_ref_log_line() {
    let json = serde_json::json!({
        "type": "fork-context-ref",
        "agentId": "awhere-does-this-c5f743e50c0b4c81",
        "parentSessionId": "8b6b2715-243e-4ae3-b857-739a9ed419ac",
        "parentLastUuid": "e9897053-95a9-4964-968d-f4426ec737f3",
        "contextLength": 143
    });
    let LogLine::ForkContextRef(fork) = serde_json::from_value(json).unwrap() else {
        panic!("expected a fork-context-ref log line");
    };
    assert_eq!(fork.agent_id, "awhere-does-this-c5f743e50c0b4c81");
    assert_eq!(
        fork.parent_session_id,
        "8b6b2715-243e-4ae3-b857-739a9ed419ac"
            .parse::<Uuid>()
            .unwrap()
    );
    assert_eq!(
        fork.parent_last_uuid,
        "e9897053-95a9-4964-968d-f4426ec737f3"
            .parse::<Uuid>()
            .unwrap()
    );
    assert_eq!(fork.context_length, 143);
}

#[test]
fn test_parse_fork_context_ref_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "fork-context-ref",
        "agentId": "awhere-does-this-c5f743e50c0b4c81",
        "parentSessionId": "8b6b2715-243e-4ae3-b857-739a9ed419ac",
        "parentLastUuid": "e9897053-95a9-4964-968d-f4426ec737f3",
        "contextLength": 143,
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
fn test_parse_fork_context_ref_rejects_missing_field() {
    let json = serde_json::json!({
        "type": "fork-context-ref",
        "agentId": "awhere-does-this-c5f743e50c0b4c81",
        "parentSessionId": "8b6b2715-243e-4ae3-b857-739a9ed419ac",
        "parentLastUuid": "e9897053-95a9-4964-968d-f4426ec737f3"
    });
    let err = serde_json::from_value::<LogLine>(json).expect_err("Should reject missing field");
    assert!(
        err.to_string().contains("contextLength"),
        "Error should mention the missing field, got: {}",
        err
    );
}

// `rename_all = "camelCase"` and `deny_unknown_fields` interact: a serialize-side rename bug would
// make a serialized record fail to parse back, so assert the wire keys explicitly via round-trip.
#[test]
fn test_fork_context_ref_round_trips() {
    let json = serde_json::json!({
        "type": "fork-context-ref",
        "agentId": "awhere-does-this-c5f743e50c0b4c81",
        "parentSessionId": "8b6b2715-243e-4ae3-b857-739a9ed419ac",
        "parentLastUuid": "e9897053-95a9-4964-968d-f4426ec737f3",
        "contextLength": 143
    });
    let parsed: LogLine = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(serde_json::to_value(&parsed).unwrap(), json);
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
fn test_parse_image_content() {
    let json = serde_json::json!({
        "type": "image",
        "source": {
            "type": "base64",
            "media_type": "image/png",
            "data": "iVBORw0KGgo="
        }
    });

    let content: LogMessageTaggedContent = serde_json::from_value(json).unwrap();
    let LogMessageTaggedContent::Image { source } = content else {
        panic!("Expected Image variant");
    };
    assert_eq!(source.r#type, "base64");
    assert_eq!(source.media_type, "image/png");
    assert_eq!(source.data, "iVBORw0KGgo=");
}

#[test]
fn test_parse_image_content_in_tool_result() {
    let json = serde_json::json!({
        "tool_use_id": "toolu_019v9avQKZUB4HVmeqbHZtcX",
        "type": "tool_result",
        "content": [{
            "type": "image",
            "source": {
                "type": "base64",
                "data": "iVBORw0KGgo=",
                "media_type": "image/png"
            }
        }]
    });

    let content: LogMessageTaggedContent = serde_json::from_value(json).unwrap();
    match content {
        LogMessageTaggedContent::ToolResult(ToolResult::Current { content, .. }) => {
            let LogMessageContent::Vec(items) = content else {
                panic!("Expected Vec content");
            };
            match &items[0] {
                LogMessageTaggedContent::Image { source } => {
                    assert_eq!(source.r#type, "base64");
                    assert_eq!(source.media_type, "image/png");
                    assert_eq!(source.data, "iVBORw0KGgo=");
                }
                other => panic!("Expected Image variant, got {other:?}"),
            }
        }
        _ => panic!("Expected ToolResult variant"),
    }
}

#[test]
fn test_parse_image_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "image",
        "source": {
            "type": "base64",
            "media_type": "image/png",
            "data": "abc123"
        },
        "extra_field": "should be rejected"
    });

    let err_msg = serde_json::from_value::<LogMessageTaggedContent>(json)
        .expect_err("Should reject unknown fields at Image variant level")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("extra_field"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
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
            assert!(
                snapshot
                    .snapshot
                    .tracked_file_backups
                    .contains_key("src/main.rs")
            );
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
            assert!(
                snapshot
                    .snapshot
                    .tracked_file_backups
                    .contains_key("src/lib.rs")
            );
        }
        _ => panic!("Expected FileHistorySnapshot variant"),
    }
}

// Checkpoint snapshots (Claude Code 2.1.219+) carry `preCheckpoint`; ordinary ones omit it.
#[test]
fn test_parse_file_history_snapshot_with_pre_checkpoint() {
    let json = serde_json::json!({
        "type": "file-history-snapshot",
        "messageId": "550e8400-e29b-41d4-a716-446655440010",
        "snapshot": {
            "messageId": "550e8400-e29b-41d4-a716-446655440010",
            "trackedFileBackups": {},
            "timestamp": "2025-01-01T00:00:00Z",
            "preCheckpoint": true
        },
        "isSnapshotUpdate": false
    });

    let line: LogLine = serde_json::from_value(json)
        .expect("Failed to parse file-history-snapshot with preCheckpoint");

    match line {
        LogLine::FileHistorySnapshot(snapshot) => {
            assert_eq!(snapshot.snapshot.pre_checkpoint, Some(true));
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

// Incremental file-history record (Claude Code 2.1.214+) noting a single tracked file's backup.
#[test]
fn test_parse_file_history_delta() {
    let json = serde_json::json!({
        "type": "file-history-delta",
        "messageId": "7c772208-16a4-4c6e-9a85-32b5887e04d8",
        "snapshotMessageId": "336ca9bd-c0e7-46ca-a9fc-fda53b740ee2",
        "trackingPath": "scripts/populate-dev-key-vault-evidence.mjs",
        "backup": {
            "backupFileName": "d959a531a8cd1839@v1",
            "version": 1,
            "backupTime": "2026-07-21T00:19:11.576Z",
            "realParentDir": "/private/tmp/claude-501/session/scratchpad"
        },
        "timestamp": "2026-07-21T00:19:11.576Z"
    });

    let line: LogLine =
        serde_json::from_value(json.clone()).expect("Failed to parse file-history-delta");
    assert_eq!(serde_json::to_value(&line).unwrap(), json);
    match line {
        LogLine::FileHistoryDelta(delta) => {
            assert_eq!(
                delta.message_id,
                Uuid::parse_str("7c772208-16a4-4c6e-9a85-32b5887e04d8").unwrap()
            );
            assert_eq!(
                delta.snapshot_message_id,
                Uuid::parse_str("336ca9bd-c0e7-46ca-a9fc-fda53b740ee2").unwrap()
            );
            assert_eq!(
                delta.tracking_path,
                "scripts/populate-dev-key-vault-evidence.mjs"
            );
            assert_eq!(
                delta.backup.backup_file_name.as_deref(),
                Some("d959a531a8cd1839@v1")
            );
            assert_eq!(delta.backup.version, 1);
            assert_eq!(
                delta.backup.real_parent_dir.as_deref(),
                Some("/private/tmp/claude-501/session/scratchpad")
            );
        }
        _ => panic!("Expected FileHistoryDelta variant"),
    }
}

// A delta for a newly tracked file records the backup before the backup file exists, so
// `backupFileName` is null.
#[test]
fn test_parse_file_history_delta_null_backup_file_name() {
    let json = serde_json::json!({
        "type": "file-history-delta",
        "messageId": "68b04e69-6ac5-4168-91e2-468df7f42f1f",
        "snapshotMessageId": "336ca9bd-c0e7-46ca-a9fc-fda53b740ee2",
        "trackingPath": "memory/runbook-style-derive-values.md",
        "backup": {
            "backupFileName": null,
            "version": 1,
            "backupTime": "2026-07-21T15:26:54.933Z"
        },
        "timestamp": "2026-07-21T15:26:54.933Z"
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse file-history-delta with null backup");
    match line {
        LogLine::FileHistoryDelta(delta) => {
            assert_eq!(delta.backup.backup_file_name, None);
            assert_eq!(delta.backup.real_parent_dir, None);
        }
        _ => panic!("Expected FileHistoryDelta variant"),
    }
}

#[test]
fn test_parse_file_history_delta_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "file-history-delta",
        "messageId": "7c772208-16a4-4c6e-9a85-32b5887e04d8",
        "snapshotMessageId": "336ca9bd-c0e7-46ca-a9fc-fda53b740ee2",
        "trackingPath": "src/main.rs",
        "backup": {
            "backupFileName": "d959a531a8cd1839@v1",
            "version": 1,
            "backupTime": "2026-07-21T00:19:11.576Z"
        },
        "timestamp": "2026-07-21T00:19:11.576Z",
        "unknownField": "should be rejected"
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in file-history-delta")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("unknownField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_file_history_delta_inner_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "file-history-delta",
        "messageId": "7c772208-16a4-4c6e-9a85-32b5887e04d8",
        "snapshotMessageId": "336ca9bd-c0e7-46ca-a9fc-fda53b740ee2",
        "trackingPath": "src/main.rs",
        "backup": {
            "backupFileName": "d959a531a8cd1839@v1",
            "version": 1,
            "backupTime": "2026-07-21T00:19:11.576Z",
            "unknownField": "should be rejected"
        },
        "timestamp": "2026-07-21T00:19:11.576Z"
    });

    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in file-history-delta backup")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("unknownField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

// `rename_all = "camelCase"` plus `deny_unknown_fields` interact: a serialize-side rename bug would
// make a serialized record fail to parse back, so assert the wire keys via round-trip.
#[test]
fn test_file_history_delta_round_trips() {
    let json = serde_json::json!({
        "type": "file-history-delta",
        "messageId": "7c772208-16a4-4c6e-9a85-32b5887e04d8",
        "snapshotMessageId": "336ca9bd-c0e7-46ca-a9fc-fda53b740ee2",
        "trackingPath": "src/main.rs",
        "backup": {
            "backupFileName": "d959a531a8cd1839@v1",
            "version": 1,
            "backupTime": "2026-07-21T00:19:11.576Z"
        },
        "timestamp": "2026-07-21T00:19:11.576Z"
    });
    let parsed: LogLine = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(serde_json::to_value(&parsed).unwrap(), json);
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
            assert_eq!(assistant.error_details, None);
        }
        _ => panic!("Expected Assistant variant"),
    }
}

// Synthetic API-error assistant turn (Claude Code 2.1.201) carrying the raw upstream error body in
// `errorDetails` alongside `error`/`apiErrorStatus`.
#[test]
fn test_parse_assistant_api_error_message_with_error_details() {
    let json = serde_json::json!({
        "parentUuid": "eede2871-e78d-4c6e-864a-76cac855f446",
        "isSidechain": false,
        "type": "assistant",
        "uuid": "97d081ac-aa71-436d-bda0-0a53b196e6fe",
        "timestamp": "2026-07-07T22:47:12.395Z",
        "message": {
            "id": "5754569c-0201-4018-a884-9d76cbf94c47",
            "container": null,
            "model": "<synthetic>",
            "role": "assistant",
            "stop_details": null,
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
            "content": [{"type": "text", "text": "You've hit your monthly spend limit. /model to switch models."}],
            "context_management": null
        },
        "requestId": "req_011CcoWaJyfmubPDDPN2iUDU",
        "error": "rate_limit",
        "errorDetails": "429 {\"type\":\"error\",\"error\":{\"type\":\"rate_limit_error\",\"message\":\"This request would exceed your account's monthly spend limit. Please try again later.\"},\"request_id\":\"req_011CcoWaJyfmubPDDPN2iUDU\"}",
        "isApiErrorMessage": true,
        "apiErrorStatus": 429,
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "e51c5fa7-4122-484e-8183-8c531ff7b98c",
        "version": "2.1.201",
        "gitBranch": "HEAD"
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse api-error assistant message");

    match line {
        LogLine::Assistant(assistant) => {
            assert_eq!(assistant.is_api_error_message, Some(true));
            assert_eq!(assistant.error.as_deref(), Some("rate_limit"));
            assert_eq!(assistant.api_error_status, Some(429));
            assert_eq!(
                assistant.error_details.as_deref(),
                Some(
                    "429 {\"type\":\"error\",\"error\":{\"type\":\"rate_limit_error\",\"message\":\"This request would exceed your account's monthly spend limit. Please try again later.\"},\"request_id\":\"req_011CcoWaJyfmubPDDPN2iUDU\"}"
                )
            );
        }
        _ => panic!("Expected Assistant variant"),
    }
}

// Claude Code 2.1.206 began repeating the session id under the snake_case key `session_id`
// alongside the camelCase `sessionId` on the full conversation records (user, assistant,
// attachment, and the `stop_hook_summary` system record). The parser must accept the duplicate
// rather than reject the line; it lands in `session_id_snake` and always matches `sessionId`.
#[test]
fn test_parse_assistant_with_snake_case_session_id() {
    let json = serde_json::json!({
        "parentUuid": "eede2871-e78d-4c6e-864a-76cac855f446",
        "isSidechain": false,
        "type": "assistant",
        "uuid": "97d081ac-aa71-436d-bda0-0a53b196e6fe",
        "timestamp": "2026-07-09T22:47:12.395Z",
        "message": {
            "id": "msg_011Cctf6yyjGaNhwgqmUY6KP",
            "container": null,
            "model": "claude-opus-4-8",
            "role": "assistant",
            "stop_details": null,
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "type": "message",
            "usage": {
                "input_tokens": 4,
                "output_tokens": 8,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0,
                "server_tool_use": {"web_search_requests": 0, "web_fetch_requests": 0},
                "service_tier": null,
                "cache_creation": {"ephemeral_1h_input_tokens": 0, "ephemeral_5m_input_tokens": 0}
            },
            "content": [{"type": "text", "text": "ok"}],
            "context_management": null
        },
        "requestId": "req_011Cctf6yyjGaNhwgqmUY6KP",
        // Distinct from `sessionId` below so the assertions prove the snake_case key maps to
        // `session_id_snake` specifically; real logs always carry the same value in both.
        "session_id": "11111111-1111-1111-1111-111111111111",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "22222222-2222-2222-2222-222222222222",
        "version": "2.1.206",
        "gitBranch": "HEAD"
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse assistant with snake_case session_id");
    match line {
        LogLine::Assistant(assistant) => {
            assert_eq!(
                assistant.session_id_snake.as_deref(),
                Some("11111111-1111-1111-1111-111111111111")
            );
            assert_eq!(assistant.session_id, "22222222-2222-2222-2222-222222222222");
        }
        _ => panic!("Expected Assistant variant"),
    }
}

// The `effort` field on assistant turns (Claude Code 2.1.214+) records the reasoning-effort level.
#[test]
fn test_parse_assistant_with_effort() {
    let json = serde_json::json!({
        "parentUuid": "eede2871-e78d-4c6e-864a-76cac855f446",
        "isSidechain": false,
        "type": "assistant",
        "uuid": "97d081ac-aa71-436d-bda0-0a53b196e6fe",
        "timestamp": "2026-07-21T00:14:47.742Z",
        "message": {
            "id": "msg_011Cctf6yyjGaNhwgqmUY6KP",
            "container": null,
            "model": "claude-fable-5",
            "role": "assistant",
            "stop_details": null,
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "type": "message",
            "usage": {
                "input_tokens": 4,
                "output_tokens": 8,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0,
                "server_tool_use": {"web_search_requests": 0, "web_fetch_requests": 0},
                "service_tier": null,
                "cache_creation": {"ephemeral_1h_input_tokens": 0, "ephemeral_5m_input_tokens": 0}
            },
            "content": [{"type": "text", "text": "ok"}],
            "context_management": null
        },
        "requestId": "req_011Cctf6yyjGaNhwgqmUY6KP",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "22222222-2222-2222-2222-222222222222",
        "version": "2.1.214",
        "gitBranch": "HEAD",
        "effort": "xhigh"
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse assistant with effort");
    match line {
        LogLine::Assistant(assistant) => {
            assert_eq!(assistant.effort, Some(ReasoningEffort::Xhigh));
        }
        _ => panic!("Expected Assistant variant"),
    }
}

#[test]
fn test_parse_assistant_with_is_aborted_mid_stream() {
    let json = serde_json::json!({
        "parentUuid": "eede2871-e78d-4c6e-864a-76cac855f446",
        "isSidechain": false,
        "type": "assistant",
        "uuid": "97d081ac-aa71-436d-bda0-0a53b196e6fe",
        "timestamp": "2026-07-27T21:55:37.279Z",
        "message": {
            "id": "msg_011Cctf6yyjGaNhwgqmUY6KP",
            "container": null,
            "model": "claude-opus-5",
            "role": "assistant",
            "stop_details": null,
            "stop_reason": null,
            "stop_sequence": null,
            "type": "message",
            "usage": {
                "input_tokens": 2,
                "output_tokens": 1,
                "cache_creation_input_tokens": 261,
                "cache_read_input_tokens": 61828,
                "service_tier": "standard",
                "cache_creation": {"ephemeral_1h_input_tokens": 261, "ephemeral_5m_input_tokens": 0}
            },
            "content": [{"type": "text", "text": "interrupted"}]
        },
        "requestId": "req_011Cctf6yyjGaNhwgqmUY6KP",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "22222222-2222-2222-2222-222222222222",
        "session_id": "22222222-2222-2222-2222-222222222222",
        "version": "2.1.219",
        "gitBranch": "HEAD",
        "isAbortedMidStream": true,
        "effort": "xhigh"
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse assistant with isAbortedMidStream");
    match line {
        LogLine::Assistant(assistant) => assert_eq!(assistant.is_aborted_mid_stream, Some(true)),
        _ => panic!("Expected Assistant variant"),
    }
}

// Pre-2.1.214 assistant records omit `effort`, so it stays `None`.
#[test]
fn test_parse_assistant_without_effort() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "type": "assistant",
        "uuid": "97d081ac-aa71-436d-bda0-0a53b196e6fe",
        "timestamp": "2026-07-09T22:47:12.395Z",
        "message": {
            "id": "msg_011Cctf6yyjGaNhwgqmUY6KP",
            "container": null,
            "model": "claude-opus-4-8",
            "role": "assistant",
            "stop_details": null,
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "type": "message",
            "usage": {
                "input_tokens": 4,
                "output_tokens": 8,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0,
                "server_tool_use": {"web_search_requests": 0, "web_fetch_requests": 0},
                "service_tier": null,
                "cache_creation": {"ephemeral_1h_input_tokens": 0, "ephemeral_5m_input_tokens": 0}
            },
            "content": [{"type": "text", "text": "ok"}],
            "context_management": null
        },
        "requestId": "req_011Cctf6yyjGaNhwgqmUY6KP",
        "userType": "external",
        "cwd": "/test",
        "sessionId": "22222222-2222-2222-2222-222222222222",
        "version": "2.1.104",
        "gitBranch": "HEAD"
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse assistant without effort");
    match line {
        LogLine::Assistant(assistant) => assert_eq!(assistant.effort, None),
        _ => panic!("Expected Assistant variant"),
    }
}

// The effort enum is strict so a genuinely new level surfaces as a parse error rather than being
// silently dropped.
#[test]
fn test_reasoning_effort_rejects_unknown_level() {
    let err = serde_json::from_value::<ReasoningEffort>(serde_json::json!("ludicrous"))
        .expect_err("Should reject an unknown effort level");
    assert!(
        err.to_string().contains("unknown variant"),
        "Error should mention unknown variant, got: {}",
        err
    );
}

// `rename_all = "camelCase"` maps each variant to its lowercase wire token; round-trip guards
// against a serialize-side rename bug (notably `Xhigh` <-> "xhigh").
#[test]
fn test_reasoning_effort_round_trips() {
    for (variant, wire) in [
        (ReasoningEffort::Low, "low"),
        (ReasoningEffort::Medium, "medium"),
        (ReasoningEffort::High, "high"),
        (ReasoningEffort::Xhigh, "xhigh"),
        (ReasoningEffort::Max, "max"),
    ] {
        let json = serde_json::json!(wire);
        let parsed: ReasoningEffort = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(parsed, variant);
        assert_eq!(serde_json::to_value(variant).unwrap(), json);
    }
}

#[test]
fn test_parse_user_with_snake_case_session_id() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        // Distinct values prove the snake_case key populates `session_id_snake` and the camelCase
        // key populates `session_id`; real logs always carry the same value in both.
        "session_id": "33333333-3333-3333-3333-333333333333",
        "sessionId": "44444444-4444-4444-4444-444444444444",
        "version": "2.1.206",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2026-07-09T00:00:00Z"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(
        line.session_id_snake,
        Some(
            "33333333-3333-3333-3333-333333333333"
                .parse::<Uuid>()
                .unwrap()
        )
    );
    assert_eq!(
        line.session_id,
        "44444444-4444-4444-4444-444444444444"
            .parse::<Uuid>()
            .unwrap()
    );
}

#[test]
fn test_parse_attachment_with_snake_case_session_id() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "deferred_tools_delta",
            "addedNames": ["WebFetch"],
            "addedLines": ["WebFetch"],
            "removedNames": []
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2026-07-09T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        // Distinct values prove the snake_case key populates `session_id_snake` independently of
        // the camelCase `sessionId`; real logs always carry the same value in both.
        "session_id": "55555555-5555-5555-5555-555555555555",
        "sessionId": "66666666-6666-6666-6666-666666666666",
        "version": "2.1.206",
        "gitBranch": "main",
        "slug": null
    });
    let line: LogLine = serde_json::from_value(json).unwrap();
    match line {
        LogLine::Attachment(att) => {
            assert_eq!(
                att.session_id_snake,
                Some(
                    "55555555-5555-5555-5555-555555555555"
                        .parse::<Uuid>()
                        .unwrap()
                )
            );
            assert_eq!(
                att.session_id,
                "66666666-6666-6666-6666-666666666666"
                    .parse::<Uuid>()
                    .unwrap()
            );
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_stop_hook_summary_with_snake_case_session_id() {
    let json = serde_json::json!({
        "parentUuid": "5445927e-82b0-4164-91f3-782fafd2a49e",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/home/brendan/src/moriarty",
        // Distinct values prove the snake_case key populates `session_id_snake` independently of
        // the camelCase `sessionId`; real logs always carry the same value in both.
        "session_id": "77777777-7777-7777-7777-777777777777",
        "sessionId": "88888888-8888-8888-8888-888888888888",
        "version": "2.1.206",
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
        "timestamp": "2026-07-09T05:27:44.883Z",
        "uuid": "35c84fed-bf99-42dc-a7bb-eae460cd23ab",
        "toolUseID": "8f3746a9-caa9-4d2d-8e6e-e7a7b005d5d4"
    });
    let line: LogLine = serde_json::from_value(json)
        .expect("Failed to parse stop_hook_summary with snake_case session_id");
    match line {
        LogLine::System(SystemLogLine::StopHookSummary(summary)) => {
            assert_eq!(
                summary.session_id_snake,
                Some(
                    "77777777-7777-7777-7777-777777777777"
                        .parse::<Uuid>()
                        .unwrap()
                )
            );
            assert_eq!(
                summary.session_id,
                "88888888-8888-8888-8888-888888888888"
                    .parse::<Uuid>()
                    .unwrap()
            );
        }
        _ => panic!("Expected System(StopHookSummary) variant"),
    }
}

// Pre-2.1.206 records omit the snake_case duplicate, so `session_id_snake` stays `None`.
#[test]
fn test_parse_user_without_snake_case_session_id_yields_none() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.201",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2026-07-09T00:00:00Z"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.session_id_snake, None);
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
            // Absent key must default to None (pre-2.1.201 logs never had this field).
            assert_eq!(fallback.refused_user_message_uuid, None);
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
            assert_eq!(fallback.refused_user_message_uuid, None);
        }
        _ => panic!("Expected System(ModelRefusalFallback) variant"),
    }
}

#[test]
fn test_parse_model_refusal_fallback_with_refused_user_message_uuid() {
    let json = serde_json::json!({
        "type": "system",
        "subtype": "model_refusal_fallback",
        "parentUuid": "9528e913-20fc-42ea-9e4c-5fb080b07c04",
        "isSidechain": false,
        "direction": "retry",
        "content": "Fable 5's safeguards flagged this message. Switched to Opus 4.8.",
        "level": "warning",
        "trigger": "refusal",
        "originalModel": "claude-fable-5",
        "fallbackModel": "claude-opus-4-8",
        "requestId": "req_011Ccmjmo1wkFV3JyX6W34NT",
        "apiRefusalCategory": "cyber",
        "apiRefusalExplanation": null,
        "isMeta": false,
        "timestamp": "2026-07-07T00:19:18.167Z",
        "uuid": "5eba741a-e6a1-449a-adbe-4d29aaa8468a",
        "retractedMessageUuids": ["490b7142-41ad-4667-8166-469606129093"],
        "refusedUserMessageUuid": "490b7142-41ad-4667-8166-469606129093",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "583790a4-8207-4478-92ee-ebb9538b54dd",
        "version": "2.1.201",
        "gitBranch": "HEAD"
    });

    match serde_json::from_value::<LogLine>(json)
        .expect("Failed to parse model_refusal_fallback with refusedUserMessageUuid")
    {
        LogLine::System(SystemLogLine::ModelRefusalFallback(fallback)) => {
            assert_eq!(
                fallback.refused_user_message_uuid,
                Some(
                    "490b7142-41ad-4667-8166-469606129093"
                        .parse()
                        .expect("valid uuid")
                )
            );
        }
        _ => panic!("Expected System(ModelRefusalFallback) variant"),
    }
}

#[test]
fn test_parse_model_refusal_fallback_with_null_refused_user_message_uuid() {
    // The real 2.1.201 shape records the key present but JSON-null (Claude Code noted no refused
    // user message), distinct from the absent-key path the base fixtures cover.
    let json = serde_json::json!({
        "type": "system",
        "subtype": "model_refusal_fallback",
        "parentUuid": "9528e913-20fc-42ea-9e4c-5fb080b07c04",
        "isSidechain": false,
        "direction": "retry",
        "content": "Fable 5's safeguards flagged this message. Switched to Opus 4.8.",
        "level": "warning",
        "trigger": "refusal",
        "originalModel": "claude-fable-5",
        "fallbackModel": "claude-opus-4-8",
        "requestId": "req_011Ccmjmo1wkFV3JyX6W34NT",
        "apiRefusalCategory": "cyber",
        "apiRefusalExplanation": null,
        "isMeta": false,
        "timestamp": "2026-07-07T00:19:18.167Z",
        "uuid": "5eba741a-e6a1-449a-adbe-4d29aaa8468a",
        "retractedMessageUuids": ["490b7142-41ad-4667-8166-469606129093"],
        "refusedUserMessageUuid": null,
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "583790a4-8207-4478-92ee-ebb9538b54dd",
        "version": "2.1.201",
        "gitBranch": "HEAD"
    });

    match serde_json::from_value::<LogLine>(json)
        .expect("Failed to parse model_refusal_fallback with null refusedUserMessageUuid")
    {
        LogLine::System(SystemLogLine::ModelRefusalFallback(fallback)) => {
            assert_eq!(fallback.refused_user_message_uuid, None);
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

fn model_consent_fallback_json() -> serde_json::Value {
    serde_json::json!({
        "parentUuid": "f37b9468-0ca5-4a8c-a37c-23edfa132e2f",
        "isSidechain": false,
        "type": "system",
        "subtype": "model_consent_fallback",
        "content": "Switched to Opus 4.8 (1M context) for this session · Fable 5 requires usage credits · /model to change",
        "level": "warning",
        "choice": "switch_default",
        "originalModel": "claude-fable-5",
        "fallbackModel": "claude-opus-4-8[1m]",
        "persistedAsDefault": false,
        "isMeta": false,
        "timestamp": "2026-07-15T16:42:33.669Z",
        "uuid": "2fed4bf1-10c6-4bed-b475-c8f26de3d992",
        "session_id": "1bde49fd-c08a-4fe3-8be3-d44c8d858f3e",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "1bde49fd-c08a-4fe3-8be3-d44c8d858f3e",
        "version": "2.1.206",
        "gitBranch": "HEAD",
        "slug": "purring-skipping-mountain"
    })
}

#[test]
fn test_parse_model_consent_fallback() {
    let json = model_consent_fallback_json();
    let line: LogLine =
        serde_json::from_value(json.clone()).expect("Failed to parse model_consent_fallback");
    assert_eq!(
        serde_json::to_value(&line).expect("Failed to serialize model_consent_fallback"),
        json
    );

    match line {
        LogLine::System(SystemLogLine::ModelConsentFallback(fallback)) => {
            assert_eq!(fallback.choice, "switch_default");
            assert_eq!(fallback.original_model.raw(), "claude-fable-5");
            assert_eq!(fallback.fallback_model.raw(), "claude-opus-4-8[1m]");
            assert_eq!(fallback.fallback_model.to_string(), "Opus 4.8");
            assert!(!fallback.persisted_as_default);
            assert_eq!(fallback.session_id, fallback.session_id_snake);
            assert_eq!(fallback.entrypoint.as_deref(), Some("cli"));
            assert_eq!(fallback.slug.as_deref(), Some("purring-skipping-mountain"));
        }
        _ => panic!("Expected System(ModelConsentFallback) variant"),
    }
}

#[test]
fn test_parse_model_consent_fallback_accepts_unknown_choice() {
    let mut json = model_consent_fallback_json();
    json.as_object_mut()
        .expect("fixture must be an object")
        .insert("choice".to_string(), serde_json::json!("ask_again"));

    match serde_json::from_value::<LogLine>(json)
        .expect("model_consent_fallback choice must remain forward-compatible")
    {
        LogLine::System(SystemLogLine::ModelConsentFallback(fallback)) => {
            assert_eq!(fallback.choice, "ask_again");
        }
        _ => panic!("Expected System(ModelConsentFallback) variant"),
    }
}

#[test]
fn test_parse_model_consent_fallback_rejects_unknown_fields() {
    let mut json = model_consent_fallback_json();
    json.as_object_mut()
        .expect("fixture must be an object")
        .insert("unknownField".to_string(), serde_json::json!(true));

    let error = serde_json::from_value::<LogLine>(json)
        .expect_err("model_consent_fallback must reject unknown fields");
    assert!(
        error.to_string().contains("unknown field"),
        "unexpected error: {error}"
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

fn away_summary_json() -> serde_json::Value {
    serde_json::json!({
        "type": "system",
        "subtype": "away_summary",
        "parentUuid": "6616d413-727a-45a6-ab51-348f0b16979b",
        "isSidechain": false,
        "content": "Gathering inputs for pnpm render:root-auth. (disable recaps in /config)",
        "timestamp": "2026-08-24T17:20:55.288Z",
        "uuid": "e08a8aed-a9ce-4146-b783-cf77d1734141",
        "isMeta": false,
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "767a91b4-9f7a-449d-b97d-01a42f5b6fc5",
        "version": "2.1.238",
        "gitBranch": "HEAD"
    })
}

#[test]
fn test_parse_system_log_away_summary() {
    let line: LogLine =
        serde_json::from_value(away_summary_json()).expect("Failed to parse away summary");

    match line {
        LogLine::System(SystemLogLine::AwaySummary(summary)) => {
            assert_eq!(
                summary.content,
                "Gathering inputs for pnpm render:root-auth. (disable recaps in /config)"
            );
            assert_eq!(summary.git_branch.as_deref(), Some("HEAD"));
            assert!(!summary.is_meta);
        }
        _ => panic!("Expected System(AwaySummary) variant"),
    }
}

#[test]
fn test_parse_system_log_away_summary_rejects_unknown_fields() {
    let mut json = away_summary_json();
    json.as_object_mut()
        .expect("fixture is a JSON object")
        .insert("unknownField".to_string(), serde_json::json!("should fail"));

    let err = serde_json::from_value::<LogLine>(json).expect_err("Should reject unknown fields");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
}

fn agents_killed_json() -> serde_json::Value {
    serde_json::json!({
        "type": "system",
        "subtype": "agents_killed",
        "parentUuid": "d9314b52-2937-4d5c-a858-49188f4e851e",
        "isSidechain": false,
        "timestamp": "2026-08-31T16:16:05.906Z",
        "uuid": "932c728c-0711-44be-871e-170444fc29ed",
        "isMeta": false,
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "6d9c57de-81b9-45f1-b047-ec3c10a1583c",
        "version": "2.1.238",
        "gitBranch": "HEAD",
        "slug": "plan-out-deploying-the-wiggly-pnueli"
    })
}

#[test]
fn test_parse_system_log_agents_killed() {
    let line: LogLine =
        serde_json::from_value(agents_killed_json()).expect("Failed to parse agents killed");

    match line {
        LogLine::System(SystemLogLine::AgentsKilled(killed)) => {
            assert_eq!(killed.git_branch.as_deref(), Some("HEAD"));
            assert_eq!(
                killed.slug.as_deref(),
                Some("plan-out-deploying-the-wiggly-pnueli")
            );
            assert_eq!(killed.cwd, "/test");
        }
        _ => panic!("Expected System(AgentsKilled) variant"),
    }
}

#[test]
fn test_parse_system_log_agents_killed_rejects_unknown_fields() {
    assert_log_line_rejects_extra_field(agents_killed_json(), |json| json);
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
fn test_parse_user_log_line_with_tool_ends_turn() {
    let json = serde_json::json!({
        "parentUuid": "b47a9862-3f97-46d5-8d7a-5e6aeaa05a13",
        "isSidechain": true,
        "promptId": "37bc4209-4827-461d-b7d5-6d3a1737da55",
        "agentId": "a14d40e18963bb5c5",
        "type": "user",
        "message": {
            "role": "user",
            "content": [{
                "tool_use_id": "toolu_01BX4nyDkkyCK3pbopv4Ux2R",
                "type": "tool_result",
                "content": "Structured output provided successfully"
            }]
        },
        "uuid": "d800de02-0edd-4606-9a21-13640c3970c9",
        "timestamp": "2026-07-15T15:53:10.905Z",
        "toolEndsTurn": true,
        "sourceToolAssistantUUID": "b47a9862-3f97-46d5-8d7a-5e6aeaa05a13",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "1bde49fd-c08a-4fe3-8be3-d44c8d858f3e",
        "version": "2.1.206",
        "gitBranch": "HEAD"
    });

    let line: LogLine = serde_json::from_value(json).expect("Should parse toolEndsTurn");
    match line {
        LogLine::User(user) => assert_eq!(user.tool_ends_turn, Some(true)),
        other => panic!("Expected User, got {other:?}"),
    }
}

#[test]
fn test_parse_user_log_line_with_tool_ends_turn_false() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": true,
        "agentId": "agent-1",
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.206",
        "gitBranch": "HEAD",
        "message": {"role": "user", "content": "continue"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2026-07-15T15:53:10.905Z",
        "toolEndsTurn": false
    });

    let line: UserLogLine = serde_json::from_value(json).expect("Should preserve false");
    assert_eq!(line.tool_ends_turn, Some(false));
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
            assert_eq!(boundary.is_meta, Some(false));
            assert_eq!(boundary.compact_metadata.trigger, "manual");
            assert_eq!(boundary.compact_metadata.pre_tokens, 100000);
            // Pre-2.1.158 logs omit the preserved-segment metadata entirely.
            assert_eq!(boundary.compact_metadata.post_tokens, None);
            assert_eq!(boundary.compact_metadata.duration_ms, None);
            assert_eq!(boundary.compact_metadata.pre_compact_discovered_tools, None);
            assert_eq!(boundary.compact_metadata.preserved_segment, None);
            assert_eq!(boundary.compact_metadata.preserved_messages, None);
            assert_eq!(boundary.compact_metadata.cumulative_dropped_tokens, None);
            assert_eq!(boundary.compact_metadata.messages_summarized, None);
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
            assert_eq!(meta.cumulative_dropped_tokens, None);
            assert_eq!(meta.messages_summarized, None);
        }
        _ => panic!("Expected System(CompactBoundary) variant"),
    }
}

#[test]
fn test_parse_compact_boundary_with_session_kind() {
    // A backgrounded session's system records carry sessionKind too, so the shared boundary macro
    // must accept it alongside the conversation records.
    let json = serde_json::json!({
        "parentUuid": null,
        "logicalParentUuid": "1d6ea6ce-23d1-47d4-bf8a-65a9b884dc89",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "538faf26-5f15-48a0-be20-20876e5f4f29",
        "version": "2.1.238",
        "gitBranch": "HEAD",
        "type": "system",
        "subtype": "compact_boundary",
        "content": "Conversation compacted",
        "sessionKind": "bg",
        "timestamp": "2026-08-31T15:48:03.553Z",
        "uuid": "c7486d00-8c13-4665-acb0-0c3e5e812cd2",
        "level": "info",
        "compactMetadata": {
            "trigger": "manual",
            "preTokens": 468607,
            "postTokens": 7737
        }
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse compact_boundary with sessionKind");

    match line {
        LogLine::System(SystemLogLine::CompactBoundary(boundary)) => {
            assert_eq!(boundary.session_kind, Some(SessionKind::Bg));
        }
        other => panic!("Expected compact_boundary, got {other:?}"),
    }
}

#[test]
fn test_parse_compact_boundary_with_cumulative_dropped_tokens() {
    // Claude Code 2.1.197 added cumulativeDroppedTokens to compactMetadata.
    let json = serde_json::json!({
        "parentUuid": null,
        "logicalParentUuid": "1d6ea6ce-23d1-47d4-bf8a-65a9b884dc89",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "538faf26-5f15-48a0-be20-20876e5f4f29",
        "version": "2.1.197",
        "gitBranch": "HEAD",
        "slug": "spicy-snuggling-ocean",
        "type": "system",
        "subtype": "compact_boundary",
        "content": "Conversation compacted",
        "isMeta": false,
        "timestamp": "2026-07-06T15:48:03.553Z",
        "uuid": "c7486d00-8c13-4665-acb0-0c3e5e812cd2",
        "level": "info",
        "compactMetadata": {
            "trigger": "manual",
            "preTokens": 922398,
            "durationMs": 172092,
            "preCompactDiscoveredTools": ["Monitor", "WebFetch", "WebSearch"],
            "preservedSegment": {
                "headUuid": "e6cd3a07-837a-4be6-ae4c-ee54fb79bc12",
                "anchorUuid": "0421d5ff-507b-42bb-b799-b434c78f73f8",
                "tailUuid": "1d6ea6ce-23d1-47d4-bf8a-65a9b884dc89"
            },
            "preservedMessages": {
                "anchorUuid": "0421d5ff-507b-42bb-b799-b434c78f73f8",
                "uuids": ["e6cd3a07-837a-4be6-ae4c-ee54fb79bc12", "1d6ea6ce-23d1-47d4-bf8a-65a9b884dc89"],
                "allUuids": ["e6cd3a07-837a-4be6-ae4c-ee54fb79bc12", "1d6ea6ce-23d1-47d4-bf8a-65a9b884dc89"]
            },
            "postTokens": 14546,
            "cumulativeDroppedTokens": 907852
        }
    });

    let line: LogLine = serde_json::from_value(json)
        .expect("Failed to parse compact_boundary with cumulativeDroppedTokens");

    match line {
        LogLine::System(SystemLogLine::CompactBoundary(boundary)) => {
            let meta = boundary.compact_metadata;
            assert_eq!(meta.pre_tokens, 922398);
            assert_eq!(meta.post_tokens, Some(14546));
            assert_eq!(meta.cumulative_dropped_tokens, Some(907852));
        }
        _ => panic!("Expected System(CompactBoundary) variant"),
    }
}

#[test]
fn test_parse_compact_boundary_with_messages_summarized() {
    // Claude Code 2.1.214 added messagesSummarized to compactMetadata.
    let json = serde_json::json!({
        "parentUuid": null,
        "logicalParentUuid": "fe41433a-9ce4-4f68-9a00-decd968be064",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "d7dc9685-4671-4982-8ebc-6bd9d79ad6b0",
        "version": "2.1.214",
        "gitBranch": "HEAD",
        "slug": "partitioned-plotting-firefly",
        "type": "system",
        "subtype": "compact_boundary",
        "content": "Conversation compacted",
        "isMeta": false,
        "timestamp": "2026-07-23T23:53:20.901Z",
        "uuid": "94709d97-84f4-4579-8b95-6b6757672a2f",
        "level": "info",
        "compactMetadata": {
            "trigger": "manual",
            "preTokens": 847997,
            "messagesSummarized": 703,
            "durationMs": 222556,
            "postTokens": 384048,
            "cumulativeDroppedTokens": 463949
        }
    });

    let line: LogLine = serde_json::from_value(json)
        .expect("Failed to parse compact_boundary with messagesSummarized");

    match line {
        LogLine::System(SystemLogLine::CompactBoundary(boundary)) => {
            let meta = boundary.compact_metadata;
            assert_eq!(meta.pre_tokens, 847997);
            assert_eq!(meta.post_tokens, Some(384048));
            assert_eq!(meta.messages_summarized, Some(703));
        }
        _ => panic!("Expected System(CompactBoundary) variant"),
    }
}

// Claude Code 2.1.214 stopped emitting `isMeta` on compact_boundary records, so it must parse when
// the field is absent.
#[test]
fn test_parse_compact_boundary_without_is_meta() {
    let json = serde_json::json!({
        "parentUuid": null,
        "logicalParentUuid": "f0d71e01-ae8d-49a7-8fe3-45171353e8d0",
        "isSidechain": false,
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "33479dd1-b321-4619-b4d1-3c1ebaacb0ca",
        "version": "2.1.214",
        "gitBranch": "HEAD",
        "slug": "binary-skipping-mango",
        "type": "system",
        "subtype": "compact_boundary",
        "content": "Conversation compacted",
        "timestamp": "2026-07-21T17:41:54.339Z",
        "uuid": "bf589097-56a7-45fa-9f80-bf6990bff0e7",
        "level": "info",
        "compactMetadata": {
            "trigger": "manual",
            "preTokens": 997403,
            "postTokens": 7407,
            "cumulativeDroppedTokens": 989996
        }
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse compact_boundary without isMeta");
    match line {
        LogLine::System(SystemLogLine::CompactBoundary(boundary)) => {
            assert_eq!(boundary.is_meta, None);
        }
        _ => panic!("Expected System(CompactBoundary) variant"),
    }
}

#[test]
fn test_parse_user_log_line_with_summarize_metadata() {
    // Claude Code 2.1.214 added summarizeMetadata to the compact-summary user turn.
    let json = serde_json::json!({
        "parentUuid": "f9d262b2-7250-449d-955f-cc4e1fda4f83",
        "isSidechain": false,
        "promptId": "a98be83f-0357-48ba-9e64-27aa66b4cb7b",
        "message": {"role": "user", "content": "This session is being continued..."},
        "isCompactSummary": true,
        "summarizeMetadata": {"messagesSummarized": 703, "direction": "from"},
        "uuid": "0afa2d5b-b030-4aa3-8c2f-599f8bc18262",
        "timestamp": "2026-07-23T23:53:20.903Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "d7dc9685-4671-4982-8ebc-6bd9d79ad6b0",
        "version": "2.1.214",
        "gitBranch": "HEAD",
        "slug": "partitioned-plotting-firefly"
    });

    let line: UserLogLine =
        serde_json::from_value(json).expect("Failed to parse user turn with summarizeMetadata");
    assert_eq!(line.is_compact_summary, Some(true));
    let meta = line
        .summarize_metadata
        .expect("summarizeMetadata should be present");
    assert_eq!(meta.messages_summarized, 703);
    assert_eq!(meta.direction, "from");
}

#[test]
fn test_parse_summarize_metadata_rejects_unknown_fields() {
    let json = serde_json::json!({
        "messagesSummarized": 703,
        "direction": "from",
        "extraField": "should be rejected"
    });

    let err_msg = serde_json::from_value::<SummarizeMetadata>(json)
        .expect_err("Should reject unknown fields in SummarizeMetadata")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("extraField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_summarize_metadata_requires_direction() {
    // `direction` is non-Option, so a payload omitting it must be rejected rather than silently
    // defaulting.
    let json = serde_json::json!({
        "messagesSummarized": 703
    });

    let err_msg = serde_json::from_value::<SummarizeMetadata>(json)
        .expect_err("Should reject SummarizeMetadata missing direction")
        .to_string();
    assert!(
        err_msg.contains("missing field") || err_msg.contains("direction"),
        "Error should mention the missing direction field, got: {}",
        err_msg
    );
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
            assert!(
                boundary
                    .microcompact_metadata
                    .cleared_attachment_uuids
                    .is_empty()
            );
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

// `is_meta` is optional via the shared `define_boundary_log!` macro, so a microcompact_boundary
// without `isMeta` must parse too even though the omission was observed on compact_boundary records.
#[test]
fn test_parse_microcompact_boundary_without_is_meta() {
    let json = serde_json::json!({
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.214",
        "gitBranch": "HEAD",
        "type": "system",
        "subtype": "microcompact_boundary",
        "content": "Context microcompacted",
        "timestamp": "2026-01-18T23:44:09.153Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "level": "info",
        "microcompactMetadata": {
            "trigger": "auto",
            "preTokens": 58482,
            "tokensSaved": 20010,
            "compactedToolIds": [],
            "clearedAttachmentUUIDs": []
        }
    });

    let line: LogLine =
        serde_json::from_value(json).expect("Failed to parse microcompact_boundary without isMeta");
    match line {
        LogLine::System(SystemLogLine::MicrocompactBoundary(boundary)) => {
            assert_eq!(boundary.is_meta, None);
        }
        _ => panic!("Expected System(MicrocompactBoundary) variant"),
    }
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
fn test_parse_assistant_rejects_unknown_top_level_fields() {
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
        "timestamp": "2025-01-01T00:00:00Z",
        "unknown_field": "should fail"
    });

    let err_msg = serde_json::from_value::<AssistantLogLine>(json)
        .expect_err("Should reject unknown top-level fields due to deny_unknown_fields")
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
fn test_parse_user_log_line_with_tool_denial_kind() {
    // Claude Code 2.1.201 added toolDenialKind on the user turn carrying a denied tool's error
    // result (here a Bash `cd` blocked by a permission rule).
    let json = serde_json::json!({
        "parentUuid": "bd9aab76-4d33-4895-b111-b993a7c4ab91",
        "isSidechain": false,
        "promptId": "92e5f63a-6444-4ffa-bce4-620d0e81b1be",
        "message": {"role": "user", "content": [{
            "type": "tool_result",
            "content": "You are not allowed to change directories.",
            "is_error": true,
            "tool_use_id": "toolu_01FYZzge6xHacXuXxY4GF2UG"
        }]},
        "uuid": "6a9597eb-fe7f-4a20-8b33-654bd87fe806",
        "timestamp": "2026-07-06T18:51:55.332Z",
        "toolUseResult": "Error: You are not allowed to change directories.",
        "toolDenialKind": "permission-rule",
        "sourceToolAssistantUUID": "bd9aab76-4d33-4895-b111-b993a7c4ab91",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/Users/brendan/src/h2/h2-iac",
        "sessionId": "538faf26-5f15-48a0-be20-20876e5f4f29",
        "version": "2.1.201",
        "gitBranch": "HEAD",
        "slug": "spicy-snuggling-ocean"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.tool_denial_kind, Some(ToolDenialKind::PermissionRule));
}

#[test]
fn test_parse_user_log_line_with_user_rejected_tool_denial_kind() {
    // Claude Code 2.1.201 also emits toolDenialKind "user-rejected" when the user manually
    // declines the tool call at the permission prompt.
    let json = serde_json::json!({
        "parentUuid": "9b324826-db0c-4613-9658-ab7deb91abb4",
        "isSidechain": false,
        "promptId": "1a749aae-3cad-424c-a945-9469df183d0d",
        "message": {"role": "user", "content": [{
            "type": "tool_result",
            "content": "The user doesn't want to proceed with this tool use.",
            "is_error": true,
            "tool_use_id": "toolu_01Y7KGdNsGpi3x1ajwzmgFUr"
        }]},
        "uuid": "527dc3ba-c146-493e-a202-9716bc1b9e50",
        "timestamp": "2026-07-07T00:23:04.199Z",
        "toolUseResult": "Error: The user doesn't want to proceed with this tool use.",
        "toolDenialKind": "user-rejected",
        "sourceToolAssistantUUID": "9b324826-db0c-4613-9658-ab7deb91abb4",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/Users/brendan/src/switchboard-jj",
        "sessionId": "583790a4-8207-4478-92ee-ebb9538b54dd",
        "version": "2.1.201",
        "gitBranch": "HEAD"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.tool_denial_kind, Some(ToolDenialKind::UserRejected));
}

#[test]
fn test_parse_user_log_line_with_user_feedback() {
    let json = serde_json::json!({
        "parentUuid": "9b324826-db0c-4613-9658-ab7deb91abb4",
        "isSidechain": false,
        "message": {"role": "user", "content": [{
            "type": "tool_result",
            "content": "The user doesn't want to proceed with this tool use.",
            "is_error": true,
            "tool_use_id": "toolu_01Y7KGdNsGpi3x1ajwzmgFUr"
        }]},
        "uuid": "527dc3ba-c146-493e-a202-9716bc1b9e50",
        "timestamp": "2026-07-27T16:38:27.167Z",
        "toolUseResult": "Error: The user doesn't want to proceed with this tool use.",
        "toolDenialKind": "user-rejected",
        "userFeedback": "Use the Write tool",
        "sourceToolAssistantUUID": "9b324826-db0c-4613-9658-ab7deb91abb4",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "583790a4-8207-4478-92ee-ebb9538b54dd",
        "session_id": "583790a4-8207-4478-92ee-ebb9538b54dd",
        "version": "2.1.219",
        "gitBranch": "HEAD"
    });
    let line: UserLogLine =
        serde_json::from_value(json).expect("Failed to parse user record with userFeedback");
    assert_eq!(line.user_feedback.as_deref(), Some("Use the Write tool"));
}

#[test]
fn test_parse_user_log_line_with_automode_unavailable_tool_denial_kind() {
    // Claude Code 2.1.201 emits toolDenialKind "automode-unavailable" (observed on a sidechain
    // agent's turn) when auto mode's safety classifier model is temporarily unavailable and the
    // call is denied rather than approved.
    let json = serde_json::json!({
        "parentUuid": "b594a747-6cac-41ba-a66c-fa751bdf8b90",
        "isSidechain": true,
        "promptId": "acdfb908-1034-496b-8641-335cd0f2f695",
        "agentId": "aa7bae6fc5eeca488",
        "message": {"role": "user", "content": [{
            "type": "tool_result",
            "content": "claude-opus-4-8[1m] is temporarily unavailable, so auto mode cannot determine the safety of WebSearch right now.",
            "is_error": true,
            "tool_use_id": "toolu_013GG8Q3zDB6XBfBZSrgrCbK"
        }]},
        "uuid": "06fe5d3a-928b-4454-b0b8-94f895ee9d53",
        "timestamp": "2026-07-08T18:40:56.155Z",
        "toolUseResult": "Error: claude-opus-4-8[1m] is temporarily unavailable, so auto mode cannot determine the safety of WebSearch right now.",
        "toolDenialKind": "automode-unavailable",
        "sourceToolAssistantUUID": "b594a747-6cac-41ba-a66c-fa751bdf8b90",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/Users/brendan/src/h2/h2-iac",
        "sessionId": "15eb4f99-064c-424f-a7da-ddb39e340c1c",
        "version": "2.1.201",
        "gitBranch": "HEAD",
        "slug": "transient-cooking-thunder"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(
        line.tool_denial_kind,
        Some(ToolDenialKind::AutomodeUnavailable)
    );
}

#[test]
fn test_parse_user_log_line_without_tool_denial_kind() {
    // The field is absent on ordinary user turns, so it must default to None.
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.201",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.tool_denial_kind, None);
}

#[test]
fn test_parse_user_log_line_rejects_unknown_tool_denial_kind() {
    // ToolDenialKind is a closed enum: an unrecognized denial kind must fail to parse so the new
    // value surfaces rather than being silently dropped.
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.201",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "toolDenialKind": "totally-bogus-kind"
    });
    let err_msg = serde_json::from_value::<UserLogLine>(json)
        .expect_err("Should reject unknown toolDenialKind variant")
        .to_string();
    assert!(
        err_msg.contains("unknown variant") || err_msg.contains("totally-bogus-kind"),
        "Error should mention unknown variant, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_user_log_line_with_queue_priority() {
    // Claude Code 2.1.201 added queuePriority on a prompt that was queued (here a meta "Continue"
    // prompt queued with priority "later") rather than sent immediately.
    let json = serde_json::json!({
        "parentUuid": "a828a29c-0171-40f3-be9a-30c141de4aa7",
        "isSidechain": false,
        "promptId": "818b0049-6469-4773-a417-4e4702e8c3db",
        "message": {"role": "user", "content": "Continue: collect the test review pass-2 result."},
        "isMeta": true,
        "uuid": "64c01ff2-4aac-4117-85bf-21fa50c6736f",
        "timestamp": "2026-07-08T00:07:00.977Z",
        "permissionMode": "acceptEdits",
        "promptSource": "system",
        "queuePriority": "later",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/Users/brendan/src/h2/h2-iac",
        "sessionId": "e51c5fa7-4122-484e-8183-8c531ff7b98c",
        "version": "2.1.201",
        "gitBranch": "HEAD"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.queue_priority, Some(QueuePriority::Later));
    assert_eq!(line.turn_companion, None);
}

#[test]
fn test_parse_user_log_line_with_turn_companion() {
    // Claude Code 2.1.238 marks the meta turn carrying an invoked skill's instructions as a
    // companion to the prompt that invoked it.
    let json = serde_json::json!({
        "parentUuid": "c0225c2f-5d39-4a3b-ae03-963bfcd23228",
        "isSidechain": false,
        "promptId": "0cde8170-4ecd-41da-99fd-c1efab7ff6fd",
        "message": {"role": "user", "content": "Base directory for this skill: /skills/real-code-review"},
        "isMeta": true,
        "turnCompanion": true,
        "uuid": "b4e57ed0-4304-413a-9a88-0ee66cd4d783",
        "timestamp": "2026-08-24T17:31:48.949Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "57d72248-a8c6-4b51-81c5-7778851c2a3e",
        "version": "2.1.238",
        "gitBranch": "HEAD"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.turn_companion, Some(true));
}

#[test]
fn test_parse_user_log_line_with_session_kind() {
    // Claude Code 2.1.238 tags every record of a backgrounded session with sessionKind.
    let json = serde_json::json!({
        "parentUuid": "c0225c2f-5d39-4a3b-ae03-963bfcd23228",
        "isSidechain": false,
        "message": {"role": "user", "content": "Continue"},
        "sessionKind": "bg",
        "uuid": "b4e57ed0-4304-413a-9a88-0ee66cd4d783",
        "timestamp": "2026-08-31T17:31:48.949Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "57d72248-a8c6-4b51-81c5-7778851c2a3e",
        "version": "2.1.238",
        "gitBranch": "HEAD"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.session_kind, Some(SessionKind::Bg));
}

#[test]
fn test_parse_user_log_line_rejects_unknown_session_kind() {
    // SessionKind is strict so a new kind surfaces as a parse error instead of being ignored.
    let json = serde_json::json!({
        "parentUuid": "c0225c2f-5d39-4a3b-ae03-963bfcd23228",
        "isSidechain": false,
        "message": {"role": "user", "content": "Continue"},
        "sessionKind": "fg",
        "uuid": "b4e57ed0-4304-413a-9a88-0ee66cd4d783",
        "timestamp": "2026-08-31T17:31:48.949Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "57d72248-a8c6-4b51-81c5-7778851c2a3e",
        "version": "2.1.238",
        "gitBranch": "HEAD"
    });
    let err = serde_json::from_value::<UserLogLine>(json)
        .expect_err("unknown sessionKind must not parse");
    assert!(
        err.to_string().contains("unknown variant `fg`"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_parse_user_log_line_with_classifier_meta_lines() {
    // Claude Code 2.1.238 attaches auto mode's classifier context as an embedded JSON string, so
    // the field must stay a String rather than a nested object.
    let json = serde_json::json!({
        "parentUuid": "f78059a8-6bd0-4c00-a7ad-33369bdbe9d8",
        "isSidechain": true,
        "promptId": "9c024b6d-f9e4-4c8f-8b7a-c72255323d93",
        "agentId": "aa5dc93d927674738",
        "message": {"role": "user", "content": "test"},
        "uuid": "f7c10d66-4699-489b-8f4c-df52f5e0fd34",
        "timestamp": "2026-08-26T15:51:51.767Z",
        "classifierMetaLines": "{\"meta\":{\"gitStatus\":{\"staged\":0,\"modified\":4,\"untracked\":0}}}\n",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "c42a9553-cddb-44db-9b25-2a9e5958f84b",
        "version": "2.1.238",
        "gitBranch": "HEAD"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(
        line.classifier_meta_lines.as_deref(),
        Some("{\"meta\":{\"gitStatus\":{\"staged\":0,\"modified\":4,\"untracked\":0}}}\n")
    );
}

#[test]
fn test_parse_user_log_line_without_queue_priority() {
    // The field is absent on turns sent immediately, so it must default to None.
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.201",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.queue_priority, None);
}

#[test]
fn test_parse_user_log_line_rejects_unknown_queue_priority() {
    // QueuePriority is a closed enum: an unrecognized priority must fail to parse so the new value
    // surfaces rather than being silently dropped.
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440000",
        "version": "2.1.201",
        "gitBranch": "main",
        "message": {"role": "user", "content": "test"},
        "uuid": "550e8400-e29b-41d4-a716-446655440001",
        "timestamp": "2025-01-01T00:00:00Z",
        "queuePriority": "totally-bogus-priority"
    });
    let err_msg = serde_json::from_value::<UserLogLine>(json)
        .expect_err("Should reject unknown queuePriority variant")
        .to_string();
    assert!(
        err_msg.contains("unknown variant") || err_msg.contains("totally-bogus-priority"),
        "Error should mention unknown variant, got: {}",
        err_msg
    );
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
fn test_parse_user_log_line_with_permission_mode_auto() {
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
        "permissionMode": "auto"
    });
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(line.permission_mode, Some(PermissionMode::Auto));
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
    assert_eq!(line.message.usage.output_tokens_details, None);
}

#[test]
fn test_parse_assistant_usage_with_output_tokens_details() {
    let json = serde_json::json!({
        "parentUuid": null,
        "isSidechain": false,
        "userType": "test",
        "cwd": "/test",
        "sessionId": "test-session",
        "version": "2.1.238",
        "gitBranch": "main",
        "message": {
            "id": "msg-1",
            "type": "message",
            "role": "assistant",
            "content": "response",
            "model": "claude-sonnet-5",
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 100,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0,
                "cache_creation": {
                    "ephemeral_5m_input_tokens": 0,
                    "ephemeral_1h_input_tokens": 0
                },
                "output_tokens": 223,
                "output_tokens_details": {
                    "thinking_tokens": 18
                }
            }
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-08-24T19:00:13.354Z"
    });
    let line: AssistantLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(
        line.message.usage.output_tokens_details,
        Some(OutputTokensDetails {
            thinking_tokens: 18
        })
    );
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
fn test_parse_user_log_line_with_permission_mode_bypass_permissions() {
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
    let line: UserLogLine = serde_json::from_value(json).unwrap();
    assert_eq!(
        line.permission_mode,
        Some(PermissionMode::BypassPermissions)
    );
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
        "permissionMode": "totallyBogusMode"
    });
    let err_msg = serde_json::from_value::<UserLogLine>(json)
        .expect_err("Should reject unknown permissionMode variant")
        .to_string();
    assert!(
        err_msg.contains("unknown variant") || err_msg.contains("totallyBogusMode"),
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
    assert_eq!(
        line.message.stop_details,
        Some(StopDetails::EndTurn {
            stop_sequence: None
        })
    );
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
    assert_eq!(
        line.message.stop_details,
        Some(StopDetails::StopSequence {
            stop_sequence: Some("</result>".to_string())
        })
    );
}

#[test]
fn test_parse_assistant_message_with_stop_details_refusal() {
    let json = serde_json::json!({
        "parentUuid": "4d6871d6-3564-4b65-acb7-4f35e2e35fcd",
        "isSidechain": true,
        "agentId": "a10a0c9183a023b6b",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "1bde49fd-c08a-4fe3-8be3-d44c8d858f3e",
        "version": "2.1.206",
        "gitBranch": "HEAD",
        "message": {
            "id": "msg_011Cd49BEfBZCRGtkgZy4Znn",
            "type": "message",
            "role": "assistant",
            "content": [],
            "model": "claude-fable-5",
            "stop_reason": "refusal",
            "stop_sequence": null,
            "stop_details": {
                "type": "refusal",
                "category": "cyber",
                "explanation": "This request triggered restrictions.",
                "fallback_has_prefill_claim": true,
                "recommended_model": null
            },
            "usage": {
                "input_tokens": 2,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0,
                "output_tokens": 1,
                "cache_creation": {
                    "ephemeral_1h_input_tokens": 0,
                    "ephemeral_5m_input_tokens": 0
                }
            }
        },
        "uuid": "be321e14-803d-4b3c-9410-3ef092292eec",
        "timestamp": "2026-07-15T16:16:28.036Z"
    });

    let line: AssistantLogLine = serde_json::from_value(json).expect("Should parse refusal");
    assert_eq!(
        line.message.stop_details,
        Some(StopDetails::Refusal {
            category: "cyber".to_string(),
            explanation: "This request triggered restrictions.".to_string(),
            fallback_has_prefill_claim: true,
            recommended_model: None,
        })
    );
}

#[test]
fn test_parse_refusal_stop_details_with_recommended_model() {
    let json = serde_json::json!({
        "type": "refusal",
        "category": "cyber",
        "explanation": "Retry on the recommended model.",
        "fallback_has_prefill_claim": false,
        "recommended_model": "claude-opus-4-8"
    });

    let stop_details: StopDetails =
        serde_json::from_value(json).expect("Should parse recommended_model");
    match stop_details {
        StopDetails::Refusal {
            recommended_model: Some(model),
            ..
        } => assert_eq!(model.raw(), "claude-opus-4-8"),
        other => panic!("Expected Refusal with a recommended model, got {other:?}"),
    }
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
fn test_parse_refusal_stop_details_requires_refusal_fields() {
    let json = serde_json::json!({
        "type": "refusal",
        "recommended_model": null
    });

    let error = serde_json::from_value::<StopDetails>(json)
        .expect_err("Should require refusal-specific fields")
        .to_string();
    assert!(
        error.contains("missing field"),
        "Error should name a missing refusal field, got: {error}"
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
fn test_parse_permission_mode_change_auto() {
    let json = r#"{"type":"permission-mode","permissionMode":"auto","sessionId":"550e8400-e29b-41d4-a716-446655440000"}"#;
    let log_line: LogLine = serde_json::from_str(json).unwrap();
    match log_line {
        LogLine::PermissionModeChange(pm) => {
            assert_eq!(pm.permission_mode, PermissionMode::Auto);
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
fn test_parse_attachment_read_truncation_notice() {
    let expected_banner = "[Truncated: PARTIAL view — /tmp/task.output: showing lines 1-395 of 1282 total (68943 tokens, cap 25000).]";
    let json = serde_json::json!({
        "parentUuid": "b35b8500-ef4f-4780-8e0a-1b4283ded2a5",
        "isSidechain": false,
        "attachment": {
            "type": "read_truncation_notice",
            "banner": expected_banner,
            "toolUseID": "toolu_01At3K4pi5v6Ejx6tDDzghPy"
        },
        "type": "attachment",
        "uuid": "d4e923c5-4523-486c-b549-e3202e177585",
        "timestamp": "2026-07-10T23:49:39.051Z",
        "session_id": "436afaba-9873-4009-bee2-b858681e6648",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "436afaba-9873-4009-bee2-b858681e6648",
        "version": "2.1.206",
        "gitBranch": "HEAD"
    });

    let line: LogLine = serde_json::from_value(json).expect("Should parse read_truncation_notice");
    match line {
        LogLine::Attachment(attachment) => match attachment.attachment {
            AttachmentData::ReadTruncationNotice(notice) => {
                assert_eq!(notice.banner, expected_banner);
                assert_eq!(notice.tool_use_id, "toolu_01At3K4pi5v6Ejx6tDDzghPy");

                let serialized = serde_json::to_value(notice).expect("Should serialize notice");
                assert_eq!(serialized["toolUseID"], "toolu_01At3K4pi5v6Ejx6tDDzghPy");
                assert!(serialized.get("toolUseId").is_none());
            }
            other => panic!("Expected ReadTruncationNotice, got {other:?}"),
        },
        other => panic!("Expected Attachment, got {other:?}"),
    }
}

#[test]
fn test_parse_read_truncation_notice_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "read_truncation_notice",
        "banner": "truncated",
        "toolUseID": "toolu_123",
        "extraField": true
    });

    let error = serde_json::from_value::<AttachmentData>(json)
        .expect_err("Should reject unknown read_truncation_notice fields")
        .to_string();
    assert!(
        error.contains("extraField"),
        "Error should name the unknown field, got: {error}"
    );
}

#[test]
fn test_parse_attachment_agent_listing_delta() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "agent_listing_delta",
            "addedTypes": ["claude", "Explore"],
            "addedLines": ["- claude: catch-all", "- Explore: read-only search"],
            "removedTypes": [],
            "isInitial": true,
            "showConcurrencyNote": true
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2026-06-15T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.175",
        "gitBranch": "main",
        "slug": null
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => match att.attachment {
            AttachmentData::AgentListingDelta(delta) => {
                assert_eq!(delta.added_types, vec!["claude", "Explore"]);
                assert_eq!(
                    delta.added_lines,
                    vec!["- claude: catch-all", "- Explore: read-only search"]
                );
                assert!(delta.removed_types.is_empty());
                assert!(delta.is_initial);
                assert!(delta.show_concurrency_note);
            }
            other => panic!("Expected AgentListingDelta, got {:?}", other),
        },
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_agent_listing_delta_non_initial() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "agent_listing_delta",
            "addedTypes": ["new-agent"],
            "addedLines": ["- new-agent: added mid-session"],
            "removedTypes": ["old-agent"],
            "isInitial": false,
            "showConcurrencyNote": false
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2026-06-15T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.175",
        "gitBranch": "main",
        "slug": null
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => match att.attachment {
            AttachmentData::AgentListingDelta(delta) => {
                assert_eq!(delta.added_types, vec!["new-agent"]);
                assert_eq!(delta.added_lines, vec!["- new-agent: added mid-session"]);
                assert_eq!(delta.removed_types, vec!["old-agent"]);
                assert!(!delta.is_initial);
                assert!(!delta.show_concurrency_note);
            }
            other => panic!("Expected AgentListingDelta, got {:?}", other),
        },
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_agent_listing_delta_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "agent_listing_delta",
            "addedTypes": ["claude"],
            "addedLines": ["- claude: catch-all"],
            "removedTypes": [],
            "isInitial": true,
            "showConcurrencyNote": true,
            "extraField": "should fail"
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2026-06-15T00:00:00Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.175",
        "gitBranch": "main",
        "slug": null
    });
    let err = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in agent_listing_delta");
    assert!(
        err.to_string().contains("extraField"),
        "error should name the unknown field, got: {err}"
    );
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
                    let FileAttachmentContent::Text { file: body } = file.content else {
                        panic!("Expected Text content");
                    };
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
fn test_parse_attachment_image_file() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": "f79dbdf2-63a8-46c9-9975-6cb9aca8f8b9",
        "isSidechain": false,
        "attachment": {
            "type": "file",
            "filename": "/abs/path/call_starts.png",
            "content": {
                "type": "image",
                "file": {
                    "base64": "iVBORw0KGgo=",
                    "type": "image/png",
                    "originalSize": 95245,
                    "dimensions": {
                        "originalWidth": 1606,
                        "originalHeight": 588,
                        "displayWidth": 803,
                        "displayHeight": 294
                    }
                }
            },
            "displayPath": "call_starts.png"
        },
        "uuid": "898d808b-9583-4be7-80d3-8e4e96f562bf",
        "timestamp": "2026-06-11T00:12:47.506Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.170",
        "gitBranch": "HEAD",
        "slug": "happy-doodling-moonbeam"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => match att.attachment {
            AttachmentData::File(file) => {
                assert_eq!(file.filename, "/abs/path/call_starts.png");
                assert_eq!(file.display_path, "call_starts.png");
                let FileAttachmentContent::Image { file: body } = file.content else {
                    panic!("Expected Image content");
                };
                assert_eq!(body.base64, "iVBORw0KGgo=");
                assert_eq!(body.r#type, "image/png");
                assert_eq!(body.original_size, 95245);
                assert_eq!(body.dimensions.original_width, 1606);
                assert_eq!(body.dimensions.original_height, 588);
                assert_eq!(body.dimensions.display_width, 803);
                assert_eq!(body.dimensions.display_height, 294);
            }
            other => panic!("Expected File attachment, got {:?}", other),
        },
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_image_file_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "image",
        "file": {
            "base64": "iVBORw0KGgo=",
            "type": "image/png",
            "originalSize": 95245,
            "dimensions": {
                "originalWidth": 1606,
                "originalHeight": 588,
                "displayWidth": 1606,
                "displayHeight": 588,
                "unknownField": "should fail"
            }
        }
    });

    let err_msg = serde_json::from_value::<FileAttachmentContent>(json)
        .expect_err("Should reject unknown fields due to deny_unknown_fields")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("unknownField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

#[test]
fn test_parse_attachment_image_body_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "image",
        "file": {
            "base64": "iVBORw0KGgo=",
            "type": "image/png",
            "originalSize": 95245,
            "dimensions": {
                "originalWidth": 1606,
                "originalHeight": 588,
                "displayWidth": 1606,
                "displayHeight": 588
            },
            "extraField": "should fail"
        }
    });

    let err_msg = serde_json::from_value::<FileAttachmentContent>(json)
        .expect_err("Should reject unknown fields in FileAttachmentImageBody")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("extraField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
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
fn test_parse_attachment_context_tip() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": "3f2db2c5-ade2-4192-bdb0-c5697e6e565d",
        "isSidechain": false,
        "attachment": {
            "type": "context_tip",
            "tip": {
                "tip": "You're searching across multiple directories outside your working directory. You can grant Claude access to those paths with /add-dir so you don't have to manually search — just read the file directly",
                "featureId": "outside-working-dir",
                "action": "/add-dir /Users/brendan/src/h2/h2-root-auth"
            }
        },
        "uuid": "a340a6a7-125e-44c9-ab4d-69596ca5c4ae",
        "timestamp": "2026-07-01T20:55:09.945Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/Users/brendan/src/h2",
        "sessionId": "a5f871fa-d7d6-44c0-a68c-6227535e1afd",
        "version": "2.1.197",
        "gitBranch": "main"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => match att.attachment {
            AttachmentData::ContextTip(tip) => {
                assert_eq!(
                    tip.tip.tip,
                    "You're searching across multiple directories outside your working directory. You can grant Claude access to those paths with /add-dir so you don't have to manually search — just read the file directly"
                );
                assert_eq!(tip.tip.feature_id, "outside-working-dir");
                assert_eq!(
                    tip.tip.action.as_deref(),
                    Some("/add-dir /Users/brendan/src/h2/h2-root-auth")
                );
            }
            other => panic!("Expected ContextTip attachment, got {:?}", other),
        },
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_context_tip_without_action() {
    let json = serde_json::json!({
        "type": "context_tip",
        "tip": {
            "tip": "A tip that carries no suggested command",
            "featureId": "some-feature"
        }
    });
    let att: AttachmentData = serde_json::from_value(json).unwrap();
    match att {
        AttachmentData::ContextTip(tip) => {
            assert_eq!(tip.tip.tip, "A tip that carries no suggested command");
            assert_eq!(tip.tip.feature_id, "some-feature");
            assert_eq!(tip.tip.action, None);
        }
        other => panic!("Expected ContextTip attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_context_tip_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "context_tip",
        "tip": {
            "tip": "A tip",
            "featureId": "some-feature",
            "extraField": "should be rejected"
        }
    });
    let err_msg = serde_json::from_value::<AttachmentData>(json)
        .expect_err("Should reject unknown fields in ContextTip")
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
fn test_parse_attachment_invoked_skills() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": "c64f3f86-4c80-4731-867c-f5edd1ca017d",
        "isSidechain": false,
        "attachment": {
            "type": "invoked_skills",
            "skills": [{
                "name": "code-review",
                "path": "userSettings:code-review",
                "content": "The following is feedback from a code review"
            }]
        },
        "uuid": "ec995960-3e87-49be-aa57-e57baba03f9e",
        "timestamp": "2026-06-24T23:07:08.822Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.179",
        "gitBranch": "HEAD",
        "slug": "elegant-yawning-steele"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => match att.attachment {
            AttachmentData::InvokedSkills(invoked) => {
                assert_eq!(invoked.skills.len(), 1);
                assert_eq!(invoked.skills[0].name, "code-review");
                assert_eq!(invoked.skills[0].path, "userSettings:code-review");
                assert_eq!(
                    invoked.skills[0].content,
                    "The following is feedback from a code review"
                );
            }
            other => panic!("Expected InvokedSkills attachment, got {:?}", other),
        },
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

// A turn can record zero invoked skills, so an empty `skills` array must parse cleanly.
#[test]
fn test_parse_attachment_invoked_skills_empty() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "invoked_skills",
            "skills": []
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2026-06-24T23:07:08.822Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.179",
        "gitBranch": "HEAD",
        "slug": null
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => match att.attachment {
            AttachmentData::InvokedSkills(invoked) => assert!(invoked.skills.is_empty()),
            other => panic!("Expected InvokedSkills attachment, got {:?}", other),
        },
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

// `deny_unknown_fields` on the outer `InvokedSkills` envelope is a separate code path from the
// per-skill struct, so guard it independently.
#[test]
fn test_parse_attachment_invoked_skills_rejects_unknown_envelope_fields() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "invoked_skills",
            "skills": [],
            "extraField": "should fail"
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2026-06-24T23:07:08.822Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.179",
        "gitBranch": "HEAD",
        "slug": null
    });
    let err = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields on InvokedSkills");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
}

#[test]
fn test_parse_attachment_invoked_skills_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "invoked_skills",
            "skills": [{
                "name": "code-review",
                "path": "userSettings:code-review",
                "content": "feedback",
                "extraField": "should fail"
            }]
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2026-06-24T23:07:08.822Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.179",
        "gitBranch": "HEAD",
        "slug": null
    });
    let err = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields on InvokedSkill");
    assert!(
        err.to_string().contains("unknown field"),
        "Error should mention unknown field, got: {}",
        err
    );
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

// `task_status` attachment (Claude Code 2.1.214+) tracks a spawned background agent's progress.
#[test]
fn test_parse_attachment_task_status() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": "acc69b3e-00e9-4fe5-8feb-3a44658dae15",
        "isSidechain": false,
        "attachment": {
            "type": "task_status",
            "taskId": "a1155314b8a6f5c42",
            "taskType": "local_agent",
            "description": "Code-quality review pass 2",
            "status": "running",
            "deltaSummary": "Reading config shape-rejection cases",
            "outputFilePath": "/tmp/tasks/a1155314b8a6f5c42.output"
        },
        "uuid": "b03318a9-c802-41c5-8aec-4a06f78ca35f",
        "timestamp": "2026-07-21T18:03:47.391Z",
        "userType": "external",
        "cwd": "/test",
        "sessionId": "33479dd1-b321-4619-b4d1-3c1ebaacb0ca",
        "version": "2.1.214",
        "gitBranch": "HEAD"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::TaskStatus(task) = &att.attachment {
                assert_eq!(task.task_id, "a1155314b8a6f5c42");
                assert_eq!(task.task_type, "local_agent");
                assert_eq!(task.status, "running");
            } else {
                panic!("Expected TaskStatus, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_task_status_rejects_unknown_fields() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "task_status",
            "taskId": "a1155314b8a6f5c42",
            "taskType": "local_agent",
            "description": "review",
            "status": "running",
            "deltaSummary": "working",
            "outputFilePath": "/tmp/tasks/x.output",
            "unknownField": "should be rejected"
        },
        "uuid": "b03318a9-c802-41c5-8aec-4a06f78ca35f",
        "timestamp": "2026-07-21T18:03:47.391Z",
        "userType": "external",
        "cwd": "/test",
        "sessionId": "33479dd1-b321-4619-b4d1-3c1ebaacb0ca",
        "version": "2.1.214",
        "gitBranch": "HEAD"
    });
    let err_msg = serde_json::from_value::<LogLine>(json)
        .expect_err("Should reject unknown fields in task_status attachment")
        .to_string();
    assert!(
        err_msg.contains("unknown field") || err_msg.contains("unknownField"),
        "Error should mention unknown field, got: {}",
        err_msg
    );
}

// `total_tokens_reminder` attachment (Claude Code 2.1.226+) carries the token-budget reminder
// injected into a turn.
#[test]
fn test_parse_attachment_total_tokens_reminder() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": "f6221c76-a900-483e-aaec-68f20e63d421",
        "isSidechain": false,
        "attachment": {
            "type": "total_tokens_reminder",
            "text": "<total_tokens>15000000 tokens left</total_tokens>"
        },
        "uuid": "11dadebc-702c-41a8-a8b2-c25b8663ad0c",
        "timestamp": "2026-08-15T18:13:27.286Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "e08517b4-1b95-4443-ae90-15654bc60c49",
        "version": "2.1.226",
        "gitBranch": "HEAD"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::TotalTokensReminder(reminder) = &att.attachment {
                assert_eq!(
                    reminder.text,
                    "<total_tokens>15000000 tokens left</total_tokens>"
                );
            } else {
                panic!("Expected TotalTokensReminder, got {:?}", att.attachment);
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
                assert_eq!(cmd.origin, None);
            } else {
                panic!("Expected QueuedCommand, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_queued_command_with_origin() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "queued_command",
            "prompt": "be sure to use the gitattributes to mark the crds as generated.",
            "commandMode": "prompt",
            "origin": {"kind": "human"},
            "timestamp": "2026-07-01T18:50:39.389Z"
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2026-07-01T18:50:39.389Z",
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.197",
        "gitBranch": "HEAD"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::QueuedCommand(cmd) = &att.attachment {
                assert_eq!(
                    cmd.prompt,
                    "be sure to use the gitattributes to mark the crds as generated."
                );
                assert_eq!(cmd.command_mode, "prompt");
                assert_eq!(
                    cmd.origin,
                    Some(MessageOrigin {
                        kind: "human".to_string()
                    })
                );
                assert_eq!(
                    cmd.timestamp,
                    Some("2026-07-01T18:50:39.389Z".parse::<DateTime<Utc>>().unwrap())
                );
            } else {
                panic!("Expected QueuedCommand, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_queued_command_with_source_uuid() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "queued_command",
            "prompt": "queued while busy",
            "source_uuid": "1f438c56-2f6c-4cf8-a2a5-d0c13f1e5ff2",
            "commandMode": "prompt",
            "origin": {"kind": "human"},
            "timestamp": "2026-08-24T18:50:38.260Z"
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2026-08-24T18:50:38.260Z",
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.238",
        "gitBranch": "HEAD"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::QueuedCommand(cmd) = &att.attachment {
                assert_eq!(
                    cmd.source_uuid,
                    Some(
                        "1f438c56-2f6c-4cf8-a2a5-d0c13f1e5ff2"
                            .parse::<Uuid>()
                            .unwrap()
                    )
                );
            } else {
                panic!("Expected QueuedCommand, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_queued_command_with_null_origin() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "queued_command",
            "prompt": "Run the tests",
            "commandMode": "prompt",
            "origin": null,
            "timestamp": null
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440000",
        "timestamp": "2025-01-01T00:00:00Z",
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.197",
        "gitBranch": "main"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::QueuedCommand(cmd) = &att.attachment {
                assert_eq!(cmd.origin, None);
                assert_eq!(cmd.timestamp, None);
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
            if let AttachmentData::AutoMode(AutoMode::Reminder(reminder)) = &att.attachment {
                assert_eq!(reminder.reminder_type, "full");
            } else {
                panic!("Expected AutoMode, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_auto_mode_behavior_flags() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "auto_mode",
            "autoModeConsentFlow": false,
            "bashFirst": true,
            "steerOnly": true
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-08-18T21:23:05.353Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.226",
        "gitBranch": "HEAD"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::AutoMode(AutoMode::BehaviorFlags(flags)) = &att.attachment {
                assert!(!flags.auto_mode_consent_flow);
                assert!(flags.bash_first);
                assert!(flags.steer_only);
                assert_eq!(flags.bypass, None);
            } else {
                panic!("Expected AutoMode, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_auto_mode_behavior_flags_with_bypass() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "auto_mode",
            "autoModeConsentFlow": false,
            "bashFirst": true,
            "steerOnly": true,
            "bypass": false
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-08-24T18:31:00.945Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.238",
        "gitBranch": "HEAD"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::AutoMode(AutoMode::BehaviorFlags(flags)) = &att.attachment {
                assert_eq!(flags.bypass, Some(false));
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
            assert!(matches!(
                att.attachment,
                AttachmentData::AutoModeExit(AutoModeExit::Bare(_))
            ));
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_auto_mode_exit_with_behavior_flags() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "auto_mode_exit",
            "bashFirst": true,
            "steerOnly": true
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-08-24T22:56:36.647Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.238",
        "gitBranch": "HEAD"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::AutoModeExit(AutoModeExit::BehaviorFlags(flags)) =
                &att.attachment
            {
                assert!(flags.bash_first);
                assert!(flags.steer_only);
            } else {
                panic!(
                    "Expected AutoModeExit behavior flags, got {:?}",
                    att.attachment
                );
            }
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
                assert_eq!(edited.display_path, None);
            } else {
                panic!("Expected EditedTextFile, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_edited_text_file_with_display_path() {
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": null,
        "isSidechain": false,
        "attachment": {
            "type": "edited_text_file",
            "filename": "/Users/brendan/src/h2/h2-iac/.specs/foundation.md",
            "snippet": "1\t# foundation",
            "displayPath": ".specs/foundation.md"
        },
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "timestamp": "2026-07-09T20:59:11.764Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/Users/brendan/src/h2/h2-iac",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.201",
        "gitBranch": "HEAD"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::EditedTextFile(edited) = &att.attachment {
                assert_eq!(
                    edited.filename,
                    "/Users/brendan/src/h2/h2-iac/.specs/foundation.md"
                );
                assert_eq!(edited.snippet, "1\t# foundation");
                assert_eq!(edited.display_path.as_deref(), Some(".specs/foundation.md"));
            } else {
                panic!("Expected EditedTextFile, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

// `rename_all = "camelCase"` and `deny_unknown_fields` interact: a serialize-side rename bug would
// make a serialized record fail to parse back, so assert the wire keys explicitly via round-trip.
#[test]
fn test_edited_text_file_round_trips() {
    let attachment = serde_json::json!({
        "type": "edited_text_file",
        "filename": "/Users/brendan/src/h2/h2-iac/.specs/foundation.md",
        "snippet": "1\t# foundation",
        "displayPath": ".specs/foundation.md"
    });
    let parsed: AttachmentData = serde_json::from_value(attachment.clone()).unwrap();
    assert_eq!(serde_json::to_value(&parsed).unwrap(), attachment);
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
                assert_eq!(cancelled.timed_out, None);
                assert_eq!(cancelled.timeout_ms, None);
            } else {
                panic!("Expected HookCancelled, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_hook_cancelled_with_timeout_fields() {
    // Claude Code 2.1.201 added timedOut/timeoutMs to hook_cancelled attachments.
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": "b0d3f006-e1dd-4c60-bcff-af3ecd19b5d7",
        "isSidechain": false,
        "attachment": {
            "type": "hook_cancelled",
            "hookName": "Stop",
            "toolUseID": "68798b35-9bd5-45b1-a1f6-dea7a7c162b6",
            "hookEvent": "Stop",
            "command": "moriarty hooks exec",
            "durationMs": 11734,
            "timedOut": false,
            "timeoutMs": 300000
        },
        "uuid": "72d63f46-b398-474e-83e9-50dfb1a15493",
        "timestamp": "2026-07-06T21:41:57.253Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "538faf26-5f15-48a0-be20-20876e5f4f29",
        "version": "2.1.201",
        "gitBranch": "HEAD",
        "slug": "spicy-snuggling-ocean"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::HookCancelled(cancelled) = &att.attachment {
                assert_eq!(cancelled.duration_ms, 11734);
                assert_eq!(cancelled.timed_out, Some(false));
                assert_eq!(cancelled.timeout_ms, Some(300000));
            } else {
                panic!("Expected HookCancelled, got {:?}", att.attachment);
            }
        }
        other => panic!("Expected Attachment, got {:?}", other),
    }
}

#[test]
fn test_parse_attachment_hook_cancelled_timed_out_true() {
    // The interesting timeout case: a hook actually cancelled for hitting its timeout.
    let json = serde_json::json!({
        "type": "attachment",
        "parentUuid": "b0d3f006-e1dd-4c60-bcff-af3ecd19b5d7",
        "isSidechain": false,
        "attachment": {
            "type": "hook_cancelled",
            "hookName": "Stop",
            "toolUseID": "68798b35-9bd5-45b1-a1f6-dea7a7c162b6",
            "hookEvent": "Stop",
            "command": "moriarty hooks exec",
            "durationMs": 300001,
            "timedOut": true,
            "timeoutMs": 300000
        },
        "uuid": "72d63f46-b398-474e-83e9-50dfb1a15493",
        "timestamp": "2026-07-06T21:41:57.253Z",
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "538faf26-5f15-48a0-be20-20876e5f4f29",
        "version": "2.1.201",
        "gitBranch": "HEAD",
        "slug": "spicy-snuggling-ocean"
    });
    let log_line: LogLine = serde_json::from_value(json).unwrap();
    match log_line {
        LogLine::Attachment(att) => {
            if let AttachmentData::HookCancelled(cancelled) = &att.attachment {
                assert_eq!(cancelled.timed_out, Some(true));
                assert_eq!(cancelled.timeout_ms, Some(300000));
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
fn test_parse_turn_duration_with_pending_background_agent_count() {
    let json = serde_json::json!({
        "type": "system",
        "subtype": "turn_duration",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.170",
        "gitBranch": "main",
        "durationMs": 1223071,
        "timestamp": "2026-06-11T17:59:54.862Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "isMeta": false,
        "entrypoint": "cli",
        "messageCount": 501,
        "pendingBackgroundAgentCount": 3
    });
    let line: LogLine = serde_json::from_value(json)
        .expect("Failed to parse turn_duration with pendingBackgroundAgentCount");
    match line {
        LogLine::System(SystemLogLine::TurnDuration(duration)) => {
            assert_eq!(duration.duration_ms, 1223071);
            assert_eq!(duration.message_count, Some(501));
            assert_eq!(duration.pending_background_agent_count, Some(3));
        }
        _ => panic!("Expected System(TurnDuration) variant"),
    }
}

#[test]
fn test_parse_turn_duration_pending_background_agent_count_zero() {
    // A zero count must deserialize to Some(0), distinct from the field being absent (None).
    let json = serde_json::json!({
        "type": "system",
        "subtype": "turn_duration",
        "parentUuid": "550e8400-e29b-41d4-a716-446655440000",
        "isSidechain": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "550e8400-e29b-41d4-a716-446655440001",
        "version": "2.1.170",
        "gitBranch": "main",
        "durationMs": 100,
        "timestamp": "2026-06-11T17:59:54.862Z",
        "uuid": "550e8400-e29b-41d4-a716-446655440002",
        "isMeta": false,
        "entrypoint": "cli",
        "pendingBackgroundAgentCount": 0
    });
    let line: LogLine = serde_json::from_value(json)
        .expect("Failed to parse turn_duration with pendingBackgroundAgentCount of 0");
    match line {
        LogLine::System(SystemLogLine::TurnDuration(duration)) => {
            assert_eq!(duration.pending_background_agent_count, Some(0));
        }
        _ => panic!("Expected System(TurnDuration) variant"),
    }
}

#[test]
fn test_parse_turn_duration_with_pending_workflow_count() {
    let json = serde_json::json!({
        "type": "system",
        "subtype": "turn_duration",
        "parentUuid": "85dac7ae-e8cc-46ad-a221-787f6896a532",
        "isSidechain": false,
        "durationMs": 34964,
        "messageCount": 20,
        "pendingWorkflowCount": 1,
        "timestamp": "2026-07-15T15:51:25.784Z",
        "uuid": "f192fd34-7613-4b6b-833f-cf9c591766cd",
        "isMeta": false,
        "userType": "external",
        "entrypoint": "cli",
        "cwd": "/test",
        "sessionId": "1bde49fd-c08a-4fe3-8be3-d44c8d858f3e",
        "version": "2.1.206",
        "gitBranch": "HEAD"
    });

    let line: LogLine = serde_json::from_value(json).expect("Should parse pendingWorkflowCount");
    match line {
        LogLine::System(SystemLogLine::TurnDuration(duration)) => {
            assert_eq!(duration.pending_workflow_count, Some(1));
        }
        other => panic!("Expected System(TurnDuration), got {other:?}"),
    }
}

#[test]
fn test_parse_turn_duration_with_zero_pending_workflows() {
    let json = serde_json::json!({
        "type": "system",
        "subtype": "turn_duration",
        "parentUuid": "85dac7ae-e8cc-46ad-a221-787f6896a532",
        "isSidechain": false,
        "durationMs": 100,
        "pendingWorkflowCount": 0,
        "timestamp": "2026-07-15T15:51:25.784Z",
        "uuid": "f192fd34-7613-4b6b-833f-cf9c591766cd",
        "isMeta": false,
        "userType": "external",
        "cwd": "/test",
        "sessionId": "1bde49fd-c08a-4fe3-8be3-d44c8d858f3e",
        "version": "2.1.206",
        "gitBranch": "HEAD"
    });

    let line: LogLine = serde_json::from_value(json).expect("Should preserve zero");
    match line {
        LogLine::System(SystemLogLine::TurnDuration(duration)) => {
            assert_eq!(duration.pending_workflow_count, Some(0));
        }
        other => panic!("Expected System(TurnDuration), got {other:?}"),
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
