//! Unit tests for the pi session log parser.
//!
//! These tests pin the current on-disk format. Each test feeds a small,
//! representative JSON snippet through [`parse_line`] and asserts on the
//! typed result.

use std::path::PathBuf;

use serde_json::{json, Value};

use super::*;

const FIXED_TIMESTAMP: &str = "2026-04-25T01:48:25.742Z";
const MESSAGE_TIMESTAMP: i64 = 1_700_000_000;
const SESSION_ID: &str = "019dc252-e50e-766c-8182-d654b46881af";

#[derive(Clone, Copy)]
struct AssistantFixture<'a> {
    api: &'a str,
    provider: &'a str,
    model: &'a str,
    stop_reason: &'a str,
    response_id: Option<&'a str>,
    error_message: Option<&'a str>,
}

impl<'a> AssistantFixture<'a> {
    fn new(api: &'a str, provider: &'a str, model: &'a str, stop_reason: &'a str) -> Self {
        Self {
            api,
            provider,
            model,
            stop_reason,
            response_id: None,
            error_message: None,
        }
    }

    fn with_response_id(self, response_id: &'a str) -> Self {
        Self {
            response_id: Some(response_id),
            ..self
        }
    }

    fn with_error_message(self, error_message: &'a str) -> Self {
        Self {
            error_message: Some(error_message),
            ..self
        }
    }
}

fn parse(value: Value) -> PiLogLine {
    let raw = value.to_string();
    let parsed = parse_line(raw.as_str());
    parsed.unwrap_or_else(|e| panic!("failed to parse: {e}\nJSON: {raw}"))
}

fn parse_err(value: Value) -> ParseError {
    let raw = value.to_string();
    let parsed = parse_line(raw.as_str());
    parsed.unwrap_err()
}

fn message_line_json(id: &str, parent_id: &str, message: Value) -> Value {
    json!({
        "type": "message",
        "id": id,
        "parentId": parent_id,
        "timestamp": FIXED_TIMESTAMP,
        "message": message,
    })
}

fn session_json(cwd: &str) -> Value {
    json!({
        "type": "session",
        "version": 1,
        "id": SESSION_ID,
        "timestamp": FIXED_TIMESTAMP,
        "cwd": cwd,
    })
}

fn model_change_json(parent_id: Option<&str>, provider: &str, model_id: &str) -> Value {
    json!({
        "type": "model_change",
        "id": "m1",
        "parentId": parent_id,
        "timestamp": FIXED_TIMESTAMP,
        "provider": provider,
        "modelId": model_id,
    })
}

fn thinking_level_change_json(parent_id: &str, thinking_level: &str) -> Value {
    json!({
        "type": "thinking_level_change",
        "id": "t1",
        "parentId": parent_id,
        "timestamp": FIXED_TIMESTAMP,
        "thinkingLevel": thinking_level,
    })
}

fn user_message_json(text: &str) -> Value {
    message_line_json(
        "u1",
        "p1",
        json!({
            "role": "user",
            "content": [{"type": "text", "text": text}],
            "timestamp": MESSAGE_TIMESTAMP,
        }),
    )
}

fn assistant_usage_json() -> Value {
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

fn insert_optional_field(message: &mut Value, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        message
            .as_object_mut()
            .unwrap()
            .insert(key.to_string(), value);
    }
}

fn assistant_message_json(content: Vec<Value>, fixture: AssistantFixture<'_>) -> Value {
    let mut message = json!({
        "role": "assistant",
        "content": content,
        "api": fixture.api,
        "provider": fixture.provider,
        "model": fixture.model,
        "usage": assistant_usage_json(),
        "stopReason": fixture.stop_reason,
        "timestamp": MESSAGE_TIMESTAMP,
    });

    insert_optional_field(
        &mut message,
        "responseId",
        fixture.response_id.map(Value::from),
    );
    insert_optional_field(
        &mut message,
        "errorMessage",
        fixture.error_message.map(Value::from),
    );

    message_line_json("a1", "u1", message)
}

fn tool_result_message_json(
    tool_name: &str,
    content: Vec<Value>,
    is_error: bool,
    details: Option<Value>,
) -> Value {
    let mut message = json!({
        "role": "toolResult",
        "toolCallId": "call_1",
        "toolName": tool_name,
        "content": content,
        "isError": is_error,
        "timestamp": MESSAGE_TIMESTAMP,
    });

    insert_optional_field(&mut message, "details", details);

    message_line_json("r1", "a1", message)
}

fn custom_json(custom_type: &str, data: Value) -> Value {
    json!({
        "type": "custom",
        "id": "c1",
        "parentId": "p1",
        "timestamp": FIXED_TIMESTAMP,
        "customType": custom_type,
        "data": data,
    })
}

fn custom_message_json(content: &str, custom_type: &str, details: Option<Value>) -> Value {
    let mut message = json!({
        "type": "custom_message",
        "id": "cm1",
        "parentId": "p1",
        "timestamp": FIXED_TIMESTAMP,
        "content": content,
        "display": true,
        "customType": custom_type,
    });

    insert_optional_field(&mut message, "details", details);

    message
}

fn loaded_tool_json(name: &str) -> Value {
    json!({
        "name": name,
        "description": "Read a file",
        "active": true,
        "source": "builtin",
        "scope": "user",
        "origin": "top-level",
    })
}

fn parse_role_message(value: Value) -> RoleMessage {
    let PiLogLine::Message(message) = parse(value) else {
        panic!("expected Message")
    };

    message.message
}

fn parse_assistant_message(content: Vec<Value>, fixture: AssistantFixture<'_>) -> AssistantMessage {
    let RoleMessage::Assistant(assistant) =
        parse_role_message(assistant_message_json(content, fixture))
    else {
        panic!("expected Assistant")
    };

    *assistant
}

fn parse_first_assistant_content(
    content_item: Value,
    fixture: AssistantFixture<'_>,
) -> AssistantContentItem {
    parse_assistant_message(vec![content_item], fixture)
        .content
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("expected assistant content item"))
}

fn parse_assistant_thinking_signature(
    signature: Value,
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
        AssistantFixture::new(api, provider, model, "stop"),
    )
    else {
        panic!("expected thinking signature")
    };

    parsed_signature
}

fn parse_tool_result_message(value: Value) -> ToolResultMessage {
    let RoleMessage::ToolResult(tool_result) = parse_role_message(value) else {
        panic!("expected ToolResult")
    };

    *tool_result
}

fn parse_custom_payload(custom_type: &str, data: Value) -> CustomPayload {
    let line = parse(custom_json(custom_type, data));
    let PiLogLine::Custom(custom) = line else {
        panic!("expected Custom")
    };

    custom.payload
}

fn parse_custom_message_payload(
    content: &str,
    custom_type: &str,
    details: Option<Value>,
) -> CustomMessagePayload {
    let line = parse(custom_message_json(content, custom_type, details));
    let PiLogLine::CustomMessage(custom_message) = line else {
        panic!("expected CustomMessage")
    };

    custom_message.payload
}

fn assert_parse_error_contains_any(name: &str, value: Value, expected_fragments: &[&str]) {
    let msg = format!("{}", parse_err(value));
    assert!(
        expected_fragments
            .iter()
            .any(|fragment| msg.contains(fragment)),
        "case {name} expected error to mention one of {expected_fragments:?}: {msg}"
    );
}

#[test]
fn session_line() {
    let line = parse(session_json("/home/brendan/src/moriarty"));

    match line {
        PiLogLine::Session(session) => {
            assert_eq!(session.version, 1);
            assert_eq!(session.cwd, PathBuf::from("/home/brendan/src/moriarty"));
        }
        other => panic!("expected Session, got {other:?}"),
    }
}

#[test]
fn model_change_optional_parent() {
    let line = parse(model_change_json(None, "anthropic", "claude-sonnet-4-5"));

    match line {
        PiLogLine::ModelChange(model_change) => {
            assert_eq!(model_change.parent_id, None);
            assert_eq!(model_change.provider, Provider::Anthropic);
        }
        other => panic!("expected ModelChange, got {other:?}"),
    }
}

#[test]
fn provider_order_is_stable() {
    // `cost_analyzer::logs::PiModel` derives `Ord`, so reordering `Provider` variants would
    // silently change model ordering and any APIs that rely on that derived sort behavior.
    assert!(Provider::Anthropic < Provider::OpenAi);
}

#[test]
fn thinking_level_change() {
    let line = parse(thinking_level_change_json("m1", "high"));

    match line {
        PiLogLine::ThinkingLevelChange(thinking_level) => {
            assert_eq!(thinking_level.thinking_level, ThinkingLevel::High);
        }
        other => panic!("expected ThinkingLevelChange, got {other:?}"),
    }
}

#[test]
fn user_message() {
    let line = parse(user_message_json("hello"));

    match line {
        PiLogLine::Message(message) => match message.message {
            RoleMessage::User(user) => {
                assert_eq!(user.content.len(), 1);
                assert!(matches!(
                    &user.content[0],
                    UserContentItem::Text { text } if text == "hello"
                ));
            }
            other => panic!("expected User, got {other:?}"),
        },
        other => panic!("expected Message, got {other:?}"),
    }
}

#[test]
fn assistant_message_with_text_and_tool_call() {
    let assistant = parse_assistant_message(
        vec![
            json!({"type": "text", "text": "I will read the file."}),
            json!({
                "type": "toolCall",
                "id": "call_1",
                "name": "read",
                "arguments": {"path": "/tmp/x.txt"},
            }),
        ],
        AssistantFixture::new(
            "anthropic-messages",
            "anthropic",
            "claude-sonnet-4-5",
            "toolUse",
        )
        .with_response_id("resp_1"),
    );

    assert_eq!(assistant.api, AssistantApi::AnthropicMessages);
    assert_eq!(assistant.provider, Provider::Anthropic);
    assert_eq!(assistant.stop_reason, AssistantStopReason::ToolUse);
    assert_eq!(assistant.response_id.as_deref(), Some("resp_1"));
    assert_eq!(assistant.content.len(), 2);

    match &assistant.content[1] {
        AssistantContentItem::ToolCall(tool_call) => {
            assert_eq!(tool_call.name, ToolName::Read);
            assert!(matches!(tool_call.arguments, ToolCallArguments::Read(_)));
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
}

#[test]
fn assistant_aborted_with_error_message() {
    let assistant = parse_assistant_message(
        Vec::new(),
        AssistantFixture::new("openai-responses", "openai", "gpt-5", "aborted")
            .with_error_message("user aborted"),
    );

    assert_eq!(assistant.stop_reason, AssistantStopReason::Aborted);
    assert_eq!(assistant.error_message.as_deref(), Some("user aborted"));
    assert_eq!(assistant.response_id, None);
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
        ThinkingSignature::Opaque(signature) => {
            assert_eq!(signature, "opaque-sig");
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
        ThinkingSignature::Structured(signature) => {
            assert_eq!(signature.id, "thk_1");
            assert_eq!(signature.summary, vec!["a".to_string(), "b".to_string()]);
        }
        other => panic!("expected structured signature, got {other:?}"),
    }
}

#[test]
fn tool_result_with_edit_details() {
    match parse_tool_result_message(tool_result_message_json(
        "edit",
        vec![json!({"type": "text", "text": "ok"})],
        false,
        Some(json!({
            "diff": "--- a\n+++ b\n",
            "firstChangedLine": 3,
        })),
    )) {
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
    }
}

#[test]
fn tool_result_without_details() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "bash",
        vec![json!({"type": "text", "text": "hi"})],
        false,
        None,
    ));

    assert!(tool_result.details.is_none());
}

#[test]
fn custom_dcp_state() {
    match parse_custom_payload(
        "dcp-state",
        json!({
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
        }),
    ) {
        CustomPayload::DcpState(state) => {
            assert_eq!(state.next_block_id, 2);
            assert_eq!(state.compression_blocks.len(), 1);
            assert_eq!(state.compression_blocks[0].id, 1);
            assert_eq!(state.compression_blocks[0].topic, "Test topic");
            assert!(state.compression_blocks[0].active);
        }
        other => panic!("expected DcpState, got {other:?}"),
    }
}

#[test]
fn custom_message_pi_loaded_tools() {
    match parse_custom_message_payload(
        "Loaded tools",
        "pi-loaded-tools",
        Some(json!({
            "tools": [loaded_tool_json("read")],
        })),
    ) {
        CustomMessagePayload::PiLoadedTools(details) => {
            assert_eq!(details.tools.len(), 1);
            assert_eq!(details.tools[0].name, ToolName::Read);
            assert_eq!(details.tools[0].origin, ToolOrigin::TopLevel);
        }
        other => panic!("expected PiLoadedTools, got {other:?}"),
    }
}

#[test]
fn custom_message_plannotator_complete_without_details() {
    assert!(matches!(
        parse_custom_message_payload("Plan complete", "plannotator-complete", None),
        CustomMessagePayload::PlannotatorComplete
    ));
}

#[test]
fn parse_rejects_unknown_or_malformed_fields() {
    let mut bad_session = session_json("/tmp");
    bad_session
        .as_object_mut()
        .unwrap()
        .insert("bogus".to_string(), json!("value"));

    let unknown_loaded_tool = loaded_tool_json("mystery_tool");

    let cases = [
        ("rejects unknown session field", bad_session, vec!["bogus"]),
        (
            "rejects unknown loaded tool name",
            // Use LoadedTool (a strict ToolName) to provoke the error — ToolCall
            // itself has a fallback through `ToolCallArguments` that would mask the
            // tool-name mismatch with a structural error instead.
            custom_message_json(
                "loaded",
                "pi-loaded-tools",
                Some(json!({
                    "tools": [unknown_loaded_tool],
                })),
            ),
            vec!["mystery_tool", "unknown variant"],
        ),
        (
            "rejects malformed tool-call arguments",
            assistant_message_json(
                vec![json!({
                    "type": "toolCall",
                    "id": "call_1",
                    "name": "bash",
                    "arguments": {"unknown_field": "x"},
                })],
                AssistantFixture::new(
                    "anthropic-messages",
                    "anthropic",
                    "claude-sonnet-4-5",
                    "toolUse",
                ),
            ),
            vec!["did not match any variant", "unknown field"],
        ),
    ];

    for (name, value, expected_fragments) in cases {
        assert_parse_error_contains_any(name, value, &expected_fragments);
    }
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
        AssistantFixture::new(
            "anthropic-messages",
            "anthropic",
            "claude-sonnet-4-5",
            "toolUse",
        ),
    );
    let AssistantContentItem::ToolCall(tool_call) = content else {
        panic!("expected ToolCall")
    };
    assert_eq!(
        tool_call.partial_json.as_deref(),
        Some("{\"command\": \"ls\"")
    );
}

#[test]
fn parse_file_reports_path_and_line() {
    let tmp = std::env::temp_dir().join(format!("pi_logs_test_{}.jsonl", uuid::Uuid::new_v4()));
    let good = session_json("/tmp").to_string();
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
