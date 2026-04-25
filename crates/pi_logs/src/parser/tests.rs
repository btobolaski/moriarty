//! Unit tests for the pi session log parser.
//!
//! These tests pin the current on-disk format. Each test feeds a small,
//! representative JSON snippet through [`parse_line`] and asserts on the
//! typed result.

use serde_json::json;

use super::*;

fn parse(value: serde_json::Value) -> PiLogLine {
    let raw = value.to_string();
    let parsed = parse_line(raw.as_str());
    parsed.unwrap_or_else(|e| panic!("failed to parse: {e}\nJSON: {raw}"))
}

fn parse_err(value: serde_json::Value) -> ParseError {
    let raw = value.to_string();
    let parsed = parse_line(raw.as_str());
    parsed.unwrap_err()
}

#[test]
fn session_line() {
    let line = parse(json!({
        "type": "session",
        "version": 1,
        "id": "019dc252-e50e-766c-8182-d654b46881af",
        "timestamp": "2026-04-25T01:48:25.742Z",
        "cwd": "/home/brendan/src/moriarty",
    }));
    match line {
        PiLogLine::Session(s) => {
            assert_eq!(s.version, 1);
            assert_eq!(s.cwd, PathBuf::from("/home/brendan/src/moriarty"));
        }
        other => panic!("expected Session, got {other:?}"),
    }
}

#[test]
fn model_change_optional_parent() {
    let line = parse(json!({
        "type": "model_change",
        "id": "m1",
        "parentId": null,
        "timestamp": "2026-04-25T01:48:25.742Z",
        "provider": "anthropic",
        "modelId": "claude-sonnet-4-5",
    }));
    match line {
        PiLogLine::ModelChange(m) => {
            assert_eq!(m.parent_id, None);
            assert_eq!(m.provider, Provider::Anthropic);
        }
        other => panic!("expected ModelChange, got {other:?}"),
    }
}

#[test]
fn thinking_level_change() {
    let line = parse(json!({
        "type": "thinking_level_change",
        "id": "t1",
        "parentId": "m1",
        "timestamp": "2026-04-25T01:48:25.742Z",
        "thinkingLevel": "high",
    }));
    match line {
        PiLogLine::ThinkingLevelChange(t) => {
            assert_eq!(t.thinking_level, ThinkingLevel::High);
        }
        other => panic!("expected ThinkingLevelChange, got {other:?}"),
    }
}

#[test]
fn user_message() {
    let line = parse(json!({
        "type": "message",
        "id": "u1",
        "parentId": "p1",
        "timestamp": "2026-04-25T01:48:25.742Z",
        "message": {
            "role": "user",
            "content": [{"type": "text", "text": "hello"}],
            "timestamp": 1_700_000_000,
        },
    }));
    match line {
        PiLogLine::Message(m) => match m.message {
            RoleMessage::User(u) => {
                assert_eq!(u.content.len(), 1);
                assert!(matches!(
                    &u.content[0],
                    UserContentItem::Text { text } if text == "hello"
                ));
            }
            other => panic!("expected User, got {other:?}"),
        },
        other => panic!("expected Message, got {other:?}"),
    }
}

fn assistant_usage_json() -> serde_json::Value {
    json!({
        "input": 10,
        "output": 5,
        "cacheRead": 0,
        "cacheWrite": 0,
        "totalTokens": 15,
        "cost": {
            "input": "0.00003",
            "output": "0.000075",
            "cacheRead": "0",
            "cacheWrite": "0",
            "total": "0.000105",
        },
    })
}

fn parse_first_assistant_content(
    content_item: serde_json::Value,
    api: &str,
    provider: &str,
    model: &str,
    stop_reason: &str,
) -> AssistantContentItem {
    let line = parse(json!({
        "type": "message",
        "id": "a1",
        "parentId": "u1",
        "timestamp": "2026-04-25T01:48:25.742Z",
        "message": {
            "role": "assistant",
            "content": [content_item],
            "api": api,
            "provider": provider,
            "model": model,
            "usage": assistant_usage_json(),
            "stopReason": stop_reason,
            "timestamp": 1_700_000_000,
        },
    }));
    let PiLogLine::Message(m) = line else {
        panic!("expected Message")
    };
    let RoleMessage::Assistant(a) = m.message else {
        panic!("expected Assistant")
    };

    a.content
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("expected assistant content item"))
}

fn parse_assistant_thinking_signature(
    signature: serde_json::Value,
    api: &str,
    provider: &str,
    model: &str,
) -> ThinkingSignature {
    let AssistantContentItem::Thinking {
        thinking_signature: Some(parsed_signature),
        ..
    } = parse_first_assistant_content(
        json!({
            "type": "thinking",
            "thinking": "hmm",
            "thinkingSignature": signature,
        }),
        api,
        provider,
        model,
        "stop",
    )
    else {
        panic!("expected thinking signature")
    };

    parsed_signature
}

#[test]
fn assistant_message_with_text_and_tool_call() {
    let line = parse(json!({
        "type": "message",
        "id": "a1",
        "parentId": "u1",
        "timestamp": "2026-04-25T01:48:25.742Z",
        "message": {
            "role": "assistant",
            "content": [
                {"type": "text", "text": "I will read the file."},
                {
                    "type": "toolCall",
                    "id": "call_1",
                    "name": "read",
                    "arguments": {"path": "/tmp/x.txt"},
                },
            ],
            "api": "anthropic-messages",
            "provider": "anthropic",
            "model": "claude-sonnet-4-5",
            "usage": assistant_usage_json(),
            "stopReason": "toolUse",
            "timestamp": 1_700_000_000,
            "responseId": "resp_1",
        },
    }));
    match line {
        PiLogLine::Message(m) => match m.message {
            RoleMessage::Assistant(a) => {
                assert_eq!(a.api, AssistantApi::AnthropicMessages);
                assert_eq!(a.provider, Provider::Anthropic);
                assert_eq!(a.stop_reason, AssistantStopReason::ToolUse);
                assert_eq!(a.response_id.as_deref(), Some("resp_1"));
                assert_eq!(a.content.len(), 2);
                match &a.content[1] {
                    AssistantContentItem::ToolCall(tc) => {
                        assert_eq!(tc.name, ToolName::Read);
                        assert!(matches!(tc.arguments, ToolCallArguments::Read(_)));
                    }
                    other => panic!("expected ToolCall, got {other:?}"),
                }
            }
            other => panic!("expected Assistant, got {other:?}"),
        },
        other => panic!("expected Message, got {other:?}"),
    }
}

#[test]
fn assistant_aborted_with_error_message() {
    let line = parse(json!({
        "type": "message",
        "id": "a1",
        "parentId": "u1",
        "timestamp": "2026-04-25T01:48:25.742Z",
        "message": {
            "role": "assistant",
            "content": [],
            "api": "openai-responses",
            "provider": "openai",
            "model": "gpt-5",
            "usage": assistant_usage_json(),
            "stopReason": "aborted",
            "timestamp": 1_700_000_000,
            "errorMessage": "user aborted",
        },
    }));
    match line {
        PiLogLine::Message(m) => match m.message {
            RoleMessage::Assistant(a) => {
                assert_eq!(a.stop_reason, AssistantStopReason::Aborted);
                assert_eq!(a.error_message.as_deref(), Some("user aborted"));
                assert_eq!(a.response_id, None);
            }
            other => panic!("expected Assistant, got {other:?}"),
        },
        other => panic!("expected Message, got {other:?}"),
    }
}

#[test]
fn assistant_thinking_opaque_signature() {
    let signature = parse_assistant_thinking_signature(
        json!("opaque-sig"),
        "anthropic-messages",
        "anthropic",
        "claude-sonnet-4-5",
    );

    match signature {
        ThinkingSignature::Opaque(s) => {
            assert_eq!(s, "opaque-sig");
        }
        other => panic!("expected opaque thinking signature, got {other:?}"),
    }
}

#[test]
fn assistant_thinking_structured_signature() {
    let signature = parse_assistant_thinking_signature(
        json!({
            "id": "thk_1",
            "type": "reasoning",
            "encrypted_content": "abc",
            "summary": ["a", "b"],
        }),
        "openai-responses",
        "openai",
        "gpt-5",
    );

    match signature {
        ThinkingSignature::Structured(s) => {
            assert_eq!(s.id, "thk_1");
            assert_eq!(s.summary, vec!["a".to_string(), "b".to_string()]);
        }
        other => panic!("expected structured signature, got {other:?}"),
    }
}

#[test]
fn tool_result_with_edit_details() {
    let line = parse(json!({
        "type": "message",
        "id": "r1",
        "parentId": "a1",
        "timestamp": "2026-04-25T01:48:25.742Z",
        "message": {
            "role": "toolResult",
            "toolCallId": "call_1",
            "toolName": "edit",
            "content": [{"type": "text", "text": "ok"}],
            "isError": false,
            "timestamp": 1_700_000_000,
            "details": {
                "diff": "--- a\n+++ b\n",
                "firstChangedLine": 3,
            },
        },
    }));
    match line {
        PiLogLine::Message(m) => match *match m.message {
            RoleMessage::ToolResult(t) => t,
            other => panic!("expected ToolResult, got {other:?}"),
        } {
            ToolResultMessage {
                tool_name,
                is_error,
                details:
                    Some(ToolResultDetails::Edit(EditDetails {
                        first_changed_line, ..
                    })),
                ..
            } => {
                assert_eq!(tool_name, ToolName::Edit);
                assert!(!is_error);
                assert_eq!(first_changed_line, Some(3));
            }
            other => panic!("expected Edit details, got {other:?}"),
        },
        other => panic!("expected Message, got {other:?}"),
    }
}

#[test]
fn tool_result_without_details() {
    let line = parse(json!({
        "type": "message",
        "id": "r1",
        "parentId": "a1",
        "timestamp": "2026-04-25T01:48:25.742Z",
        "message": {
            "role": "toolResult",
            "toolCallId": "call_1",
            "toolName": "bash",
            "content": [{"type": "text", "text": "hi"}],
            "isError": false,
            "timestamp": 1_700_000_000,
        },
    }));
    match line {
        PiLogLine::Message(m) => match m.message {
            RoleMessage::ToolResult(t) => {
                assert!(t.details.is_none());
            }
            other => panic!("expected ToolResult, got {other:?}"),
        },
        other => panic!("expected Message, got {other:?}"),
    }
}

#[test]
fn custom_dcp_state() {
    let line = parse(json!({
        "type": "custom",
        "id": "c1",
        "parentId": "p1",
        "timestamp": "2026-04-25T01:48:25.742Z",
        "customType": "dcp-state",
        "data": {
            "compressionBlocks": [{
                "id": 1,
                "topic": "Test topic",
                "summary": "Test summary",
                "startTimestamp": 1777084923000_i64,
                "endTimestamp": 1777084924000_i64,
                "anchorTimestamp": 1777084924000_i64,
                "active": true,
                "summaryTokenEstimate": 100,
                "createdAt": 1777084924500_i64
            }],
            "nextBlockId": 2,
            "prunedToolIds": ["call_1"],
            "tokensSaved": 1000,
            "totalPruneCount": 3,
            "manualMode": false,
        },
    }));
    match line {
        PiLogLine::Custom(c) => match c.payload {
            CustomPayload::DcpState(d) => {
                assert_eq!(d.next_block_id, 2);
                assert_eq!(d.compression_blocks.len(), 1);
                assert_eq!(d.compression_blocks[0].id, 1);
                assert_eq!(d.compression_blocks[0].topic, "Test topic");
                assert!(d.compression_blocks[0].active);
            }
            other => panic!("expected DcpState, got {other:?}"),
        },
        other => panic!("expected Custom, got {other:?}"),
    }
}

#[test]
fn custom_message_pi_loaded_tools() {
    let line = parse(json!({
        "type": "custom_message",
        "id": "cm1",
        "parentId": "p1",
        "timestamp": "2026-04-25T01:48:25.742Z",
        "content": "Loaded tools",
        "display": true,
        "customType": "pi-loaded-tools",
        "details": {
            "tools": [{
                "name": "read",
                "description": "Read a file",
                "active": true,
                "source": "builtin",
                "scope": "user",
                "origin": "top-level",
            }],
        },
    }));
    match line {
        PiLogLine::CustomMessage(cm) => match cm.payload {
            CustomMessagePayload::PiLoadedTools(details) => {
                assert_eq!(details.tools.len(), 1);
                assert_eq!(details.tools[0].name, ToolName::Read);
                assert_eq!(details.tools[0].origin, ToolOrigin::TopLevel);
            }
            other => panic!("expected PiLoadedTools, got {other:?}"),
        },
        other => panic!("expected CustomMessage, got {other:?}"),
    }
}

#[test]
fn custom_message_plannotator_complete_without_details() {
    let line = parse(json!({
        "type": "custom_message",
        "id": "cm1",
        "parentId": "p1",
        "timestamp": "2026-04-25T01:48:25.742Z",
        "content": "Plan complete",
        "display": true,
        "customType": "plannotator-complete",
    }));
    match line {
        PiLogLine::CustomMessage(cm) => {
            assert!(matches!(
                cm.payload,
                CustomMessagePayload::PlannotatorComplete
            ));
        }
        other => panic!("expected CustomMessage, got {other:?}"),
    }
}

#[test]
fn unknown_session_field_is_rejected() {
    let err = parse_err(json!({
        "type": "session",
        "version": 1,
        "id": "019dc252-e50e-766c-8182-d654b46881af",
        "timestamp": "2026-04-25T01:48:25.742Z",
        "cwd": "/tmp",
        "bogus": "value",
    }));
    let msg = format!("{err}");
    assert!(
        msg.contains("bogus"),
        "expected error to mention bogus: {msg}"
    );
}

#[test]
fn unknown_tool_name_is_rejected_in_loaded_tools() {
    // Use LoadedTool (a strict ToolName) to provoke the error — ToolCall
    // itself has a fallback through `ToolCallArguments` that would mask the
    // tool-name mismatch with a structural error instead.
    let err = parse_err(json!({
        "type": "custom_message",
        "id": "cm1",
        "parentId": "p1",
        "timestamp": "2026-04-25T01:48:25.742Z",
        "content": "loaded",
        "display": true,
        "customType": "pi-loaded-tools",
        "details": {
            "tools": [{
                "name": "mystery_tool",
                "description": "",
                "active": true,
                "source": "builtin",
                "scope": "user",
                "origin": "top-level",
            }],
        },
    }));
    let msg = format!("{err}");
    assert!(
        msg.contains("mystery_tool") || msg.contains("unknown variant"),
        "expected error to mention unknown tool: {msg}"
    );
}

#[test]
fn bash_args_unknown_field_is_rejected() {
    let err = parse_err(json!({
        "type": "message",
        "id": "a1",
        "parentId": "u1",
        "timestamp": "2026-04-25T01:48:25.742Z",
        "message": {
            "role": "assistant",
            "content": [{
                "type": "toolCall",
                "id": "call_1",
                "name": "bash",
                "arguments": {"unknown_field": "x"},
            }],
            "api": "anthropic-messages",
            "provider": "anthropic",
            "model": "claude-sonnet-4-5",
            "usage": assistant_usage_json(),
            "stopReason": "toolUse",
            "timestamp": 1_700_000_000,
        },
    }));
    let msg = format!("{err}");
    assert!(
        msg.contains("did not match any variant") || msg.contains("unknown field"),
        "expected structural mismatch error, got: {msg}"
    );
}

#[test]
fn tool_call_partial_json_preserved() {
    let content = parse_first_assistant_content(
        json!({
            "type": "toolCall",
            "id": "call_1",
            "name": "bash",
            "arguments": {"command": "ls"},
            "partialJson": "{\"command\": \"ls\"",
        }),
        "anthropic-messages",
        "anthropic",
        "claude-sonnet-4-5",
        "toolUse",
    );
    let AssistantContentItem::ToolCall(tc) = content else {
        panic!("expected ToolCall")
    };
    assert_eq!(tc.partial_json.as_deref(), Some("{\"command\": \"ls\""));
}

#[test]
fn parse_file_reports_path_and_line() {
    let tmp = std::env::temp_dir().join(format!("pi_logs_test_{}.jsonl", uuid::Uuid::new_v4()));
    let good = json!({
        "type": "session",
        "version": 1,
        "id": "019dc252-e50e-766c-8182-d654b46881af",
        "timestamp": "2026-04-25T01:48:25.742Z",
        "cwd": "/tmp",
    })
    .to_string();
    let bad = "{not-json}";
    std::fs::write(&tmp, format!("{good}\n{bad}\n")).unwrap();

    let err = parse_file(&tmp).expect_err("expected parse failure");
    let _ = std::fs::remove_file(&tmp);
    match err {
        ParseError::LineParse { line, path, .. } => {
            assert_eq!(line, 2);
            assert!(path.to_string_lossy().contains("pi_logs_test_"));
        }
        other => panic!("expected LineParse, got {other:?}"),
    }
}
