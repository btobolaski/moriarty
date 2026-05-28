//! Unit tests for the pi session log parser.
//!
//! These tests pin the current on-disk format and related serde contracts.
//! Most feed a small, representative JSON snippet through [`parse_line`] and
//! assert on the typed result, while others pin serialization behavior or
//! shape-routing assumptions that the parser relies on.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
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
    response_model: Option<&'a str>,
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
            response_model: None,
            error_message: None,
        }
    }

    fn with_response_id(self, response_id: &'a str) -> Self {
        Self {
            response_id: Some(response_id),
            ..self
        }
    }

    fn with_response_model(self, response_model: &'a str) -> Self {
        Self {
            response_model: Some(response_model),
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
    let err = parsed.expect_err(&format!("expected parse error\nJSON: {raw}"));
    err
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

fn session_info_json(name: &str) -> Value {
    json!({
        "type": "session_info",
        "id": "child-info-1",
        "parentId": "parent-message-1",
        "timestamp": FIXED_TIMESTAMP,
        "name": name,
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

fn compaction_json(from_hook: bool) -> Value {
    json!({
        "type": "compaction",
        "id": "c1",
        "parentId": "p1",
        "timestamp": FIXED_TIMESTAMP,
        "summary": "Compacted earlier work",
        "firstKeptEntryId": "e1",
        "tokensBefore": 12345,
        "details": {
            "readFiles": ["src/main.rs", "/tmp/output.log"],
            "modifiedFiles": ["crates/pi_logs/src/parser.rs"]
        },
        "fromHook": from_hook,
    })
}

fn branch_summary_json(from_hook: bool) -> Value {
    json!({
        "type": "branch_summary",
        "id": "b1",
        "parentId": "p1",
        "timestamp": FIXED_TIMESTAMP,
        "fromId": "branch-root-1",
        "summary": "The user explored a different branch before returning here.",
        "details": {
            "readFiles": ["references/hydrogen-rtc/Dockerfile"],
            "modifiedFiles": ["plans/ci-cd-monorepo.md"]
        },
        "fromHook": from_hook,
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
    insert_optional_field(
        &mut message,
        "responseModel",
        fixture.response_model.map(Value::from),
    );

    message_line_json("a1", "u1", message)
}

fn assistant_tool_call_json(tool_name: &str, arguments: Value) -> Value {
    json!({
        "type": "toolCall",
        "id": "call_1",
        "name": tool_name,
        "arguments": arguments,
    })
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

fn bash_execution_message_json(
    command: &str,
    output: &str,
    exit_code: i32,
    cancelled: bool,
    truncated: bool,
    exclude_from_context: bool,
    full_output_path: Option<&str>,
) -> Value {
    let mut message = json!({
        "role": "bashExecution",
        "command": command,
        "output": output,
        "exitCode": exit_code,
        "cancelled": cancelled,
        "truncated": truncated,
        "timestamp": MESSAGE_TIMESTAMP,
        "excludeFromContext": exclude_from_context,
    });

    insert_optional_field(
        &mut message,
        "fullOutputPath",
        full_output_path.map(Value::from),
    );

    message_line_json("b1", "p1", message)
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
    let AssistantContentItem::Thinking(ThinkingAssistantContent {
        thinking_signature: Some(parsed_signature),
        ..
    }) = parse_first_assistant_content(
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

fn parse_tool_call(tool_name: &str, arguments: Value) -> ToolCallContent {
    let AssistantContentItem::ToolCall(tool_call) = parse_first_assistant_content(
        assistant_tool_call_json(tool_name, arguments),
        AssistantFixture::new("openai-responses", "openai", "gpt-5.4", "toolUse"),
    ) else {
        panic!("expected ToolCall")
    };

    *tool_call
}

fn parse_mcp_details(content: Vec<Value>, details: Value) -> McpDetails {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "mcp",
        content,
        false,
        Some(details),
    ));

    let Some(ToolResultDetails::Mcp(details)) = tool_result.details else {
        panic!("expected Mcp details")
    };

    details
}

fn parse_tool_result_message(value: Value) -> ToolResultMessage {
    let RoleMessage::ToolResult(tool_result) = parse_role_message(value) else {
        panic!("expected ToolResult")
    };

    *tool_result
}

// Keep this overlapping Ls/Find details shape shared so the augmentation
// and dispatch tests cannot drift away from the same serde case.
fn parse_ls_lean_ctx_fixture(truncated: bool, compression: Value) -> ToolResultMessage {
    parse_tool_result_message(tool_result_message_json(
        "ls",
        vec![json!({"type": "text", "text": "listing"})],
        false,
        Some(json!({
            "path": "crates",
            "source": "lean-ctx",
            "truncated": truncated,
            "compression": compression,
        })),
    ))
}

fn parse_bash_execution_message(value: Value) -> BashExecutionMessage {
    let RoleMessage::BashExecution(bash_execution) = parse_role_message(value) else {
        panic!("expected BashExecution")
    };

    *bash_execution
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

fn assert_parse_error_contains_all(name: &str, value: Value, expected_fragments: &[&str]) {
    let msg = format!("{}", parse_err(value));
    for fragment in expected_fragments {
        assert!(
            msg.contains(fragment),
            "case {name} expected error to mention {fragment:?}: {msg}"
        );
    }
}

#[test]
fn session_line() {
    let line = parse(session_json("/home/brendan/src/moriarty"));

    match line {
        PiLogLine::Session(session) => {
            assert_eq!(session.version, 1);
            assert_eq!(session.cwd, PathBuf::from("/home/brendan/src/moriarty"));
            assert_eq!(session.parent_session, None);
        }
        other => panic!("expected Session, got {other:?}"),
    }
}

#[test]
fn session_info_line() {
    let line = parse(session_info_json("subagent-code-quality-reviewer-run-1"));

    match line {
        PiLogLine::SessionInfo(session_info) => {
            assert_eq!(session_info.id, "child-info-1");
            assert_eq!(session_info.parent_id.as_deref(), Some("parent-message-1"));
            assert_eq!(session_info.name, "subagent-code-quality-reviewer-run-1");
        }
        other => panic!("expected SessionInfo, got {other:?}"),
    }
}

#[test]
fn session_info_line_accepts_missing_parent_id() {
    let line = parse(json!({
        "type": "session_info",
        "id": "child-info-2",
        "timestamp": FIXED_TIMESTAMP,
        "name": "subagent-test-quality-reviewer-run-1"
    }));

    let PiLogLine::SessionInfo(session_info) = line else {
        panic!("expected SessionInfo")
    };
    assert_eq!(session_info.id, "child-info-2");
    assert_eq!(session_info.parent_id, None);
    assert_eq!(session_info.name, "subagent-test-quality-reviewer-run-1");
}

#[test]
fn compatibility_smoke_set_parses_through_entry_point() {
    let lines = [
        session_info_json("subagent-documentation-reviewer-run-1"),
        assistant_message_json(
            vec![assistant_tool_call_json(
                "fact_write",
                json!({
                    "id": "parser-compatibility",
                    "title": "Parser compatibility",
                    "content": "Newer schemas parse successfully",
                    "confidence": 0.8,
                    "domain": "logs",
                    "scope": "project",
                    "observation_count": 3,
                    "confirmed_count": 2,
                    "contradicted_count": 1,
                    "inactive_count": 0
                }),
            )],
            AssistantFixture::new("openai-responses", "openai", "gpt-5.5", "stop"),
        ),
        assistant_message_json(
            vec![assistant_tool_call_json(
                "todo",
                json!({
                    "action": "create",
                    "subject": "Track feedback",
                    "description": "Follow up on review items",
                    "metadata": {
                        "source": "reviewer",
                        "priority": "medium"
                    }
                }),
            )],
            AssistantFixture::new("openai-responses", "openai", "gpt-5.5", "stop"),
        ),
        assistant_message_json(
            vec![assistant_tool_call_json("grep", json!({}))],
            AssistantFixture::new("openai-responses", "openai", "gpt-5.5", "stop"),
        ),
        custom_json(
            "intercom_sent",
            json!({
                "to": "subagent-chat-019dfe82",
                "messageId": "msg-1",
                "timestamp": 1_746_000_000,
                "message": {
                    "text": "Blocked on output requirement",
                    "reason": "need_decision"
                },
                "subagent": {
                    "name": "documentation-reviewer"
                }
            }),
        ),
        custom_message_json(
            "subagent needs attention",
            "intercom_message",
            Some(json!({
                "from": {
                    "type": "session",
                    "id": "subagent-chat-019dfe82"
                },
                "message": {
                    "text": "Need tool access"
                },
                "replyCommand": "pi intercom reply subagent-chat-019dfe82",
                "bodyText": "Need tool access"
            })),
        ),
        tool_result_message_json(
            "subagent",
            vec![json!({"type": "text", "text": "subagent completed"})],
            false,
            Some(json!({
                "mode": "single",
                "results": [{
                    "agent": "scout",
                    "task": "Inspect parser compatibility",
                    "response": "No issues found"
                }],
                "runId": "run-1"
            })),
        ),
        tool_result_message_json(
            "fact_list",
            vec![json!({"type": "text", "text": "1 fact(s)"})],
            false,
            Some(json!({"count": 1})),
        ),
        tool_result_message_json(
            "intercom",
            vec![json!({"type": "text", "text": "Failed: no reply within 10 minutes"})],
            true,
            Some(json!({"error": true})),
        ),
    ];

    for value in lines {
        parse(value);
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
fn model_change_with_parent() {
    let line = parse(model_change_json(Some("session-root"), "openai", "gpt-5.4"));

    match line {
        PiLogLine::ModelChange(model_change) => {
            assert_eq!(model_change.parent_id.as_deref(), Some("session-root"));
            assert_eq!(model_change.provider, Provider::OpenAi);
        }
        other => panic!("expected ModelChange, got {other:?}"),
    }
}

#[test]
fn model_change_with_openrouter() {
    let line = parse(model_change_json(
        Some("session-root"),
        "openrouter",
        "openai/gpt-5.4",
    ));

    match line {
        PiLogLine::ModelChange(model_change) => {
            assert_eq!(model_change.parent_id.as_deref(), Some("session-root"));
            assert_eq!(model_change.provider, Provider::OpenRouter);
        }
        other => panic!("expected ModelChange, got {other:?}"),
    }
}

#[test]
fn provider_order_is_stable() {
    // `cost_analyzer::logs::PiModel` derives `Ord`, so reordering `Provider` variants would
    // silently change model ordering and any APIs that rely on that derived sort behavior.
    assert!(Provider::Anthropic < Provider::OpenAi);
    assert!(Provider::OpenAi < Provider::OpenRouter);
    assert!(Provider::OpenRouter < Provider::Faux);
}

#[test]
fn model_change_with_faux() {
    let line = parse(model_change_json(Some("session-root"), "faux", "faux-1"));

    match line {
        PiLogLine::ModelChange(model_change) => {
            assert_eq!(model_change.parent_id.as_deref(), Some("session-root"));
            assert_eq!(model_change.provider, Provider::Faux);
            assert_eq!(model_change.model_id, "faux-1");
        }
        other => panic!("expected ModelChange, got {other:?}"),
    }
}

#[test]
fn model_change_rejects_unknown_provider() {
    let err = parse_err(model_change_json(
        Some("session-root"),
        "unknown-provider",
        "model-1",
    ));
    let msg = format!("{err}");
    assert!(
        msg.contains("unknown variant `unknown-provider`") || msg.contains("unknown-provider"),
        "expected rejection of unknown provider, got: {msg}"
    );
}

#[test]
fn model_change_asserts_model_id() {
    let line = parse(model_change_json(None, "anthropic", "claude-sonnet-4-5"));

    match line {
        PiLogLine::ModelChange(model_change) => {
            assert_eq!(model_change.model_id, "claude-sonnet-4-5");
        }
        other => panic!("expected ModelChange, got {other:?}"),
    }
}

#[test]
fn thinking_level_order_is_stable() {
    // `ThinkingLevel` derives `Ord` through multiple parser structs, so reordering variants would
    // silently change any persisted or sorted comparisons that depend on the intensity ladder.
    assert!(ThinkingLevel::Off < ThinkingLevel::Minimal);
    assert!(ThinkingLevel::Minimal < ThinkingLevel::Low);
    assert!(ThinkingLevel::Low < ThinkingLevel::Medium);
    assert!(ThinkingLevel::Medium < ThinkingLevel::High);
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
fn thinking_level_change_medium() {
    let line = parse(thinking_level_change_json("m1", "medium"));

    match line {
        PiLogLine::ThinkingLevelChange(thinking_level) => {
            assert_eq!(thinking_level.thinking_level, ThinkingLevel::Medium);
        }
        other => panic!("expected ThinkingLevelChange, got {other:?}"),
    }
}

#[test]
fn thinking_level_change_minimal() {
    let line = parse(thinking_level_change_json("m1", "minimal"));

    match line {
        PiLogLine::ThinkingLevelChange(thinking_level) => {
            assert_eq!(thinking_level.thinking_level, ThinkingLevel::Minimal);
        }
        other => panic!("expected ThinkingLevelChange, got {other:?}"),
    }
}

#[test]
fn thinking_level_change_low() {
    let line = parse(thinking_level_change_json("m1", "low"));

    match line {
        PiLogLine::ThinkingLevelChange(thinking_level) => {
            assert_eq!(thinking_level.thinking_level, ThinkingLevel::Low);
        }
        other => panic!("expected ThinkingLevelChange, got {other:?}"),
    }
}

#[test]
fn compaction_line() {
    let line = parse(compaction_json(false));

    match line {
        PiLogLine::Compaction(compaction) => {
            assert_eq!(compaction.summary, "Compacted earlier work");
            assert_eq!(compaction.first_kept_entry_id, "e1");
            assert_eq!(compaction.tokens_before, 12345);
            assert_eq!(
                compaction.details.read_files,
                vec![
                    PathBuf::from("src/main.rs"),
                    PathBuf::from("/tmp/output.log")
                ]
            );
            assert_eq!(
                compaction.details.modified_files,
                vec![PathBuf::from("crates/pi_logs/src/parser.rs")]
            );
            assert!(!compaction.from_hook);
        }
        other => panic!("expected Compaction, got {other:?}"),
    }
}

#[test]
fn compaction_line_from_hook() {
    let line = parse(compaction_json(true));

    match line {
        PiLogLine::Compaction(compaction) => assert!(compaction.from_hook),
        other => panic!("expected Compaction, got {other:?}"),
    }
}

#[test]
fn branch_summary_line() {
    let line = parse(branch_summary_json(false));

    match line {
        PiLogLine::BranchSummary(branch_summary) => {
            assert_eq!(branch_summary.id, "b1");
            assert_eq!(branch_summary.parent_id, "p1");
            assert_eq!(
                branch_summary.timestamp,
                FIXED_TIMESTAMP
                    .parse::<DateTime<Utc>>()
                    .expect("valid timestamp")
            );
            assert_eq!(branch_summary.from_id, "branch-root-1");
            assert_eq!(
                branch_summary.summary,
                "The user explored a different branch before returning here."
            );
            assert_eq!(
                branch_summary.details.read_files,
                vec![PathBuf::from("references/hydrogen-rtc/Dockerfile")]
            );
            assert_eq!(
                branch_summary.details.modified_files,
                vec![PathBuf::from("plans/ci-cd-monorepo.md")]
            );
            assert!(!branch_summary.from_hook);
        }
        other => panic!("expected BranchSummary, got {other:?}"),
    }
}

#[test]
fn branch_summary_line_from_hook() {
    let line = parse(branch_summary_json(true));

    match line {
        PiLogLine::BranchSummary(branch_summary) => assert!(branch_summary.from_hook),
        other => panic!("expected BranchSummary, got {other:?}"),
    }
}

#[test]
fn rejects_unknown_branch_summary_field() {
    let mut bad_branch_summary = branch_summary_json(false);
    bad_branch_summary
        .as_object_mut()
        .unwrap()
        .insert("bogus".to_string(), json!("value"));

    assert_parse_error_contains_any(
        "rejects unknown branch_summary field",
        bad_branch_summary,
        &["bogus"],
    );
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
fn bash_execution_message() {
    let bash_execution = parse_bash_execution_message(bash_execution_message_json(
        "cargo run --bin parse_pi_sessions",
        "parsed 595 line(s) across 87 file(s)",
        1,
        false,
        true,
        false,
        Some("/tmp/pi-bash.log"),
    ));

    assert_eq!(bash_execution.command, "cargo run --bin parse_pi_sessions");
    assert_eq!(
        bash_execution.output,
        "parsed 595 line(s) across 87 file(s)"
    );
    assert_eq!(bash_execution.exit_code, 1);
    assert!(!bash_execution.cancelled);
    assert!(bash_execution.truncated);
    assert!(!bash_execution.exclude_from_context);
    assert_eq!(
        bash_execution.full_output_path,
        Some(PathBuf::from("/tmp/pi-bash.log"))
    );
}

#[test]
fn fact_list_tool_call_stays_tied_to_tool_name() {
    let tool_call = parse_tool_call("fact_list", json!({}));

    assert_eq!(tool_call.name(), ToolName::FactList);
}

#[test]
fn tool_call_preserves_arguments_as_a_raw_json_map() {
    let raw_arguments = json!({
        "arbitraryString": "value",
        "arbitraryNumber": 42,
        "arbitraryBool": true,
        "arbitraryNull": null,
        "arbitraryArray": [1, {"nested": false}],
        "arbitraryObject": {
            "inner": {"stillRaw": true}
        }
    });
    let tool_call = parse_tool_call("bash", raw_arguments.clone());

    assert_eq!(tool_call.name(), ToolName::Bash);
    assert_eq!(
        serde_json::to_value(&tool_call.arguments).expect("serialize raw arguments"),
        raw_arguments
    );
}

#[test]
fn tool_call_rejects_non_object_arguments() {
    let line = assistant_message_json(
        vec![json!({
            "type": "toolCall",
            "id": "call_1",
            "name": "bash",
            "arguments": ["not", "an", "object"]
        })],
        AssistantFixture::new("openai-responses", "openai", "gpt-5.4", "toolUse"),
    );

    let err = parse_err(line);
    let msg = err.to_string();
    assert!(
        msg.contains("invalid type") || msg.contains("map"),
        "expected non-object arguments to be rejected, got: {msg}"
    );
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

    assert_eq!(
        assistant.api,
        AssistantApi::Known(ApiKind::AnthropicMessages)
    );
    assert_eq!(assistant.provider, Provider::Anthropic);
    assert_eq!(assistant.stop_reason, AssistantStopReason::ToolUse);
    assert_eq!(assistant.response_id.as_deref(), Some("resp_1"));
    assert_eq!(assistant.response_model, None);
    assert_eq!(assistant.content.len(), 2);

    match &assistant.content[0] {
        AssistantContentItem::Text(text) => {
            assert_eq!(text.text, "I will read the file.");
            assert!(text.text_signature.is_none());
        }
        other => panic!("expected Text, got {other:?}"),
    }

    match &assistant.content[1] {
        AssistantContentItem::ToolCall(tool_call) => {
            assert_eq!(tool_call.name(), ToolName::Read);
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
}

#[test]
fn assistant_message_accepts_openrouter_openai_completions_response_model() {
    let assistant = parse_assistant_message(
        vec![json!({"type": "text", "text": "done"})],
        AssistantFixture::new("openai-completions", "openrouter", "openai/gpt-5.4", "stop")
            .with_response_model("openai/gpt-5.4-20260305"),
    );

    assert_eq!(
        assistant.api,
        AssistantApi::Known(ApiKind::OpenAiCompletions)
    );
    assert_eq!(assistant.provider, Provider::OpenRouter);
    assert_eq!(assistant.model, "openai/gpt-5.4");
    assert_eq!(
        assistant.response_model.as_deref(),
        Some("openai/gpt-5.4-20260305")
    );
}

#[test]
fn assistant_message_without_response_model_defaults_to_none() {
    let assistant = parse_assistant_message(
        vec![json!({"type": "text", "text": "hello"})],
        AssistantFixture::new(
            "anthropic-messages",
            "anthropic",
            "claude-sonnet-4-5",
            "stop",
        ),
    );

    assert_eq!(assistant.response_model, None);
}

#[test]
fn assistant_api_known_values_parse_as_known_variants() {
    let cases = [
        ("anthropic-messages", ApiKind::AnthropicMessages),
        ("openai-responses", ApiKind::OpenAiResponses),
        ("openai-completions", ApiKind::OpenAiCompletions),
    ];

    for (api, expected) in cases {
        let assistant = parse_assistant_message(
            vec![json!({"type": "text", "text": "ok"})],
            AssistantFixture::new(api, "openai", "gpt-5.4", "stop"),
        );
        assert_eq!(
            assistant.api,
            AssistantApi::Known(expected),
            "expected Known({expected:?}) for api={api:?}"
        );
    }
}

#[test]
fn assistant_message_accepts_faux_api_identifier() {
    let assistant = parse_assistant_message(
        vec![json!({"type": "text", "text": "simulated"})],
        AssistantFixture::new("faux:1779726916919:3u6tax2mqmp", "faux", "faux-1", "stop"),
    );

    assert_eq!(
        assistant.api,
        AssistantApi::Faux("faux:1779726916919:3u6tax2mqmp".to_string())
    );
    assert_eq!(assistant.provider, Provider::Faux);
    assert_eq!(assistant.model, "faux-1");
}

#[test]
fn assistant_api_rejects_unknown_identifier() {
    let err = parse_err(assistant_message_json(
        vec![json!({"type": "text", "text": "ok"})],
        AssistantFixture::new("unknown-api", "openai", "gpt-5.4", "stop"),
    ));
    let msg = format!("{err}");
    assert!(msg.contains("unknown variant `unknown-api`"), "{msg}");
}

#[test]
fn usage_cost_accepts_source_field() {
    // Pi now emits a `source` field inside the cost object.
    let line = r#"{"type":"message","id":"m1","parentId":"root","timestamp":"2026-04-25T01:48:25.742Z","message":{"role":"assistant","content":[{"type":"text","text":"ok"}],"api":"anthropic-messages","provider":"anthropic","model":"claude-sonnet-4-5","usage":{"input":10,"output":20,"cacheRead":0,"cacheWrite":0,"totalTokens":30,"cost":{"input":0.001,"output":0.002,"cacheRead":0,"cacheWrite":0,"total":0.003,"source":"provider"}},"stopReason":"stop","timestamp":1700000000}}"#;
    let parsed: PiLogLine = serde_json::from_str(line).unwrap();
    match parsed {
        PiLogLine::Message(m) => match &m.message {
            RoleMessage::Assistant(msg) => {
                let source = &msg.usage.cost.source;
                assert_eq!(
                    source.as_deref(),
                    Some("provider"),
                    "unexpected source: {source:?}"
                );
            }
            other => panic!("expected Assistant, got {other:?}"),
        },
        other => panic!("expected Message, got {other:?}"),
    }
}

#[test]
fn usage_cost_omits_source_when_absent() {
    // Old log files without `source` should still parse.
    let line = r#"{"type":"message","id":"m1","parentId":"root","timestamp":"2026-04-25T01:48:25.742Z","message":{"role":"assistant","content":[{"type":"text","text":"ok"}],"api":"anthropic-messages","provider":"anthropic","model":"claude-sonnet-4-5","usage":{"input":10,"output":20,"cacheRead":0,"cacheWrite":0,"totalTokens":30,"cost":{"input":0.001,"output":0.002,"cacheRead":0,"cacheWrite":0,"total":0.003}},"stopReason":"stop","timestamp":1700000000}}"#;
    let parsed: PiLogLine = serde_json::from_str(line).unwrap();
    match parsed {
        PiLogLine::Message(m) => match &m.message {
            RoleMessage::Assistant(msg) => {
                let source = &msg.usage.cost.source;
                assert_eq!(source, &None, "unexpected source: {source:?}");
            }
            other => panic!("expected Assistant, got {other:?}"),
        },
        other => panic!("expected Message, got {other:?}"),
    }
}

#[test]
fn edit_details_accepts_patch_field() {
    // Ni emits a `patch` field alongside `diff` in edit tool results.
    let line = r#"{"type":"message","id":"m1","parentId":"root","timestamp":"2026-04-25T01:48:25.742Z","message":{"role":"toolResult","toolCallId":"tc1","toolName":"edit","content":[{"type":"text","text":"ok"}],"details":{"diff":"--- a\n+++ b\n@@ -1 +1 @@\n-old\n+new\n","patch":"--- a\n+++ b\n@@ -1 +1 @@\n-foo\n+bar\n","firstChangedLine":1},"isError":false,"timestamp":1700000000}}"#;
    let parsed: PiLogLine = serde_json::from_str(line).unwrap();
    match parsed {
        PiLogLine::Message(m) => match m.message {
            RoleMessage::ToolResult(tool_result) => {
                let details = tool_result.details.unwrap();
                match details {
                    ToolResultDetails::Edit(edit) => {
                        assert!(edit.patch.is_some(), "expected patch to be present");
                        assert!(edit.patch.as_deref().unwrap().contains("@@ -1 +1 @@"));
                        assert_eq!(edit.first_changed_line, Some(1));
                    }
                    other => panic!("expected Edit, got {other:?}"),
                }
            }
            other => panic!("expected ToolResult, got {other:?}"),
        },
        other => panic!("expected Message, got {other:?}"),
    }
}

#[test]
fn edit_details_patch_absent_by_default() {
    // Upstream pi logs without `patch` should still parse.
    let line = r#"{"type":"message","id":"m1","parentId":"root","timestamp":"2026-04-25T01:48:25.742Z","message":{"role":"toolResult","toolCallId":"tc1","toolName":"edit","content":[{"type":"text","text":"ok"}],"details":{"diff":"--- a\n+++ b\n@@ -1 +1 @@\n-old\n+new\n","firstChangedLine":1},"isError":false,"timestamp":1700000000}}"#;
    let parsed: PiLogLine = serde_json::from_str(line).unwrap();
    match parsed {
        PiLogLine::Message(m) => match m.message {
            RoleMessage::ToolResult(tool_result) => {
                let details = tool_result.details.unwrap();
                match details {
                    ToolResultDetails::Edit(edit) => {
                        assert_eq!(edit.patch, None);
                        assert_eq!(edit.first_changed_line, Some(1));
                    }
                    other => panic!("expected Edit, got {other:?}"),
                }
            }
            other => panic!("expected ToolResult, got {other:?}"),
        },
        other => panic!("expected Message, got {other:?}"),
    }
}

#[test]
fn assistant_api_round_trips_known_variants() {
    let cases = [
        ("anthropic-messages", ApiKind::AnthropicMessages),
        ("openai-responses", ApiKind::OpenAiResponses),
        ("openai-completions", ApiKind::OpenAiCompletions),
    ];
    for (expected_str, kind) in cases {
        let api = AssistantApi::Known(kind);
        let serialized = serde_json::to_value(&api).unwrap();
        assert_eq!(serialized, json!(expected_str), "Known({kind:?})");

        let deserialized: AssistantApi = serde_json::from_value(serialized).unwrap();
        assert_eq!(deserialized, api, "round-trip for Known({kind:?})");
    }
}

#[test]
fn assistant_api_round_trips_faux() {
    let id = "faux:session-id:worker-id";
    let api = AssistantApi::Faux(id.to_string());
    let serialized = serde_json::to_value(&api).unwrap();
    assert_eq!(serialized, json!(id));

    let deserialized: AssistantApi = serde_json::from_value(serialized).unwrap();
    assert_eq!(deserialized, api);
}

#[test]
fn assistant_text_content_rejects_unknown_field() {
    let err = parse_err(assistant_message_json(
        vec![json!({
            "type": "text",
            "text": "hello",
            "unexpected": true
        })],
        AssistantFixture::new(
            "anthropic-messages",
            "anthropic",
            "claude-sonnet-4-5",
            "stop",
        ),
    ));
    let msg = err.to_string();
    assert!(
        msg.contains("unexpected"),
        "expected parse error to mention unexpected, got: {msg}"
    );
}

#[test]
fn assistant_thinking_content_rejects_unknown_field() {
    let err = parse_err(assistant_message_json(
        vec![json!({
            "type": "thinking",
            "thinking": "hmm",
            "thinkingSignature": "sig_1",
            "unexpected": true
        })],
        AssistantFixture::new(
            "anthropic-messages",
            "anthropic",
            "claude-sonnet-4-5",
            "stop",
        ),
    ));
    let msg = err.to_string();
    assert!(
        msg.contains("unexpected"),
        "expected parse error to mention unexpected, got: {msg}"
    );
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
fn assistant_error_stop_reason_with_error_message() {
    let assistant = parse_assistant_message(
        Vec::new(),
        AssistantFixture::new("openai-responses", "openai", "gpt-5.4", "error")
            .with_response_id("resp_1")
            .with_error_message("quota exceeded"),
    );

    assert_eq!(assistant.stop_reason, AssistantStopReason::Error);
    assert_eq!(assistant.response_id.as_deref(), Some("resp_1"));
    assert_eq!(assistant.error_message.as_deref(), Some("quota exceeded"));
}

#[test]
fn assistant_stop_reason_stop() {
    let assistant = parse_assistant_message(
        vec![json!({"type": "text", "text": "done"})],
        AssistantFixture::new(
            "anthropic-messages",
            "anthropic",
            "claude-sonnet-4-5",
            "stop",
        ),
    );

    assert_eq!(assistant.stop_reason, AssistantStopReason::Stop);
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
fn subagent_tool_result_accepts_error_fields() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "subagent",
        vec![json!({"type": "text", "text": "subagent failed"})],
        false,
        Some(json!({
            "mode": "single",
            "results": [{
                "agent": "scout",
                "task": "Inspect duplication hotspots",
                "exitCode": 1,
                "usage": {
                    "input": 0,
                    "output": 0,
                    "cacheRead": 0,
                    "cacheWrite": 0,
                    "cost": 0,
                    "turns": 0
                },
                "model": "openai-codex/gpt-5.4-mini",
                "artifactPaths": {
                    "inputPath": "/tmp/input.md",
                    "outputPath": "/tmp/output.md",
                    "jsonlPath": "/tmp/session.jsonl",
                    "metadataPath": "/tmp/meta.json"
                },
                "error": "No API key found",
                "progressSummary": {
                    "toolCount": 0,
                    "tokens": 0,
                    "durationMs": 4136
                },
                "finalOutput": "",
                "savedOutputPath": "/tmp/quality.md",
                "attemptedModels": ["openai-codex/gpt-5.4-mini"],
                "sessionFile": "/tmp/subagent-session.jsonl",
                "toolCalls": [{
                    "text": "grep {\"pattern\":\"duplication\"}",
                    "expandedText": "grep {\"pattern\":\"duplication\"}"
                }],
                "modelAttempts": [{
                    "model": "openai-codex/gpt-5.4-mini",
                    "success": false,
                    "exitCode": 1,
                    "error": "No API key found",
                    "usage": {
                        "input": 0,
                        "output": 0,
                        "cacheRead": 0,
                        "cacheWrite": 0,
                        "cost": 0,
                        "turns": 0
                    }
                }]
            }],
            "artifacts": {
                "dir": "/tmp/subagent-artifacts",
                "files": [{
                    "inputPath": "/tmp/input.md",
                    "outputPath": "/tmp/output.md",
                    "jsonlPath": "/tmp/session.jsonl",
                    "metadataPath": "/tmp/meta.json"
                }]
            }
        })),
    ));

    let Some(ToolResultDetails::Subagent(details)) = tool_result.details else {
        panic!("expected Subagent details")
    };

    assert_eq!(
        details.results[0].error.as_deref(),
        Some("No API key found")
    );
    assert_eq!(
        details.results[0].saved_output_path,
        Some(PathBuf::from("/tmp/quality.md"))
    );
    assert_eq!(
        details.results[0].model_attempts.as_ref().unwrap()[0]
            .error
            .as_deref(),
        Some("No API key found")
    );
    assert_eq!(
        details.results[0].session_file,
        Some(PathBuf::from("/tmp/subagent-session.jsonl"))
    );
    assert_eq!(
        details.results[0].tool_calls.as_ref().unwrap()[0].text,
        "grep {\"pattern\":\"duplication\"}"
    );
    assert_eq!(
        details.results[0].task.as_deref(),
        Some("Inspect duplication hotspots")
    );
    assert_eq!(details.results[0].final_output.as_deref(), Some(""));

    // The wire fixture carries a non-zero `durationMs`, so asserting
    // `progress_summary` value-by-value pins both the camelCase rename and the
    // numeric meaning. A swap of `tool_count`/`tokens`/`duration_ms` would
    // otherwise survive parsing because the underlying struct is
    // `deny_unknown_fields` only, not value-checked.
    let progress_summary = details.results[0]
        .progress_summary
        .as_ref()
        .expect("expected progress_summary");
    assert_eq!(progress_summary.tool_count, 0);
    assert_eq!(progress_summary.tokens, 0);
    assert_eq!(progress_summary.duration_ms, 4136);
}

#[test]
fn subagent_tool_result_accepts_result_response() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "subagent",
        vec![json!({"type": "text", "text": "subagent completed"})],
        false,
        Some(json!({
            "mode": "single",
            "results": [{
                "agent": "scout",
                "task": "Inspect parser compatibility",
                "response": "No issues found"
            }]
        })),
    ));

    let Some(ToolResultDetails::Subagent(details)) = tool_result.details else {
        panic!("expected Subagent details")
    };
    assert_eq!(
        details.results[0].task.as_deref(),
        Some("Inspect parser compatibility")
    );
    assert_eq!(
        details.results[0].response.as_deref(),
        Some("No issues found")
    );
}

#[test]
fn todo_tool_result_accepts_error_field() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "todo",
        vec![json!({"type": "text", "text": "Error: addBlockedBy: #6 not found"})],
        false,
        Some(json!({
            "action": "update",
            "params": {
                "action": "update",
                "id": 5,
                "status": "pending",
                "addBlockedBy": [6]
            },
            "tasks": [{
                "id": 5,
                "subject": "Run review agents",
                "status": "in_progress",
                "description": "Invoke review agents",
                "activeForm": "running review agents",
                "metadata": {"source": "review"}
            }],
            "nextId": 6,
            "error": "addBlockedBy: #6 not found"
        })),
    ));

    let Some(ToolResultDetails::Todo(details)) = tool_result.details else {
        panic!("expected Todo details")
    };

    assert_eq!(details.error.as_deref(), Some("addBlockedBy: #6 not found"));
    assert_eq!(details.params.add_blocked_by, Some(vec![6]));
    assert_eq!(
        details.tasks[0]
            .metadata
            .as_ref()
            .map(|value| value.0.clone()),
        Some(json!({"source": "review"}))
    );
}

#[test]
fn fact_list_tool_result_accepts_count() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "fact_list",
        vec![json!({"type": "text", "text": "No facts found matching the given filters."})],
        false,
        Some(json!({
            "count": 0
        })),
    ));

    let Some(ToolResultDetails::Count(details)) = tool_result.details else {
        panic!("expected Count details")
    };
    assert_eq!(details.count, 0);
}

#[test]
fn intercom_tool_result_accepts_delivery_status() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "intercom",
        vec![json!({
            "type": "text",
            "text": "Reply to \"subagent-documentation-reviewer-4194b4bf-3\" was not delivered: Session not found"
        })],
        false,
        Some(json!({
            "messageId": "ca42f2de-5fe9-4920-be2f-396cb2917bb8",
            "delivered": false,
            "reason": "Session not found"
        })),
    ));

    let Some(ToolResultDetails::Intercom(details)) = tool_result.details else {
        panic!("expected Intercom details")
    };
    assert_eq!(
        details.message_id.as_deref(),
        Some("ca42f2de-5fe9-4920-be2f-396cb2917bb8")
    );
    assert_eq!(details.delivered, Some(false));
    assert_eq!(details.reason.as_deref(), Some("Session not found"));
    assert_eq!(details.error, None);
}

#[test]
fn intercom_tool_result_accepts_error_flag() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "intercom",
        vec![json!({
            "type": "text",
            "text": "Failed to reply: No active intercom context to reply to"
        })],
        false,
        Some(json!({"error": true})),
    ));

    let Some(ToolResultDetails::Intercom(details)) = tool_result.details else {
        panic!("expected Intercom details")
    };
    assert_eq!(details.error, Some(true));
    assert_eq!(details.message_id, None);
    assert_eq!(details.delivered, None);
    assert_eq!(details.reason, None);
}

#[test]
fn instinct_list_tool_result_accepts_count() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "instinct_list",
        vec![json!({"type": "text", "text": "1 instinct(s)"})],
        false,
        Some(json!({
            "count": 1
        })),
    ));

    let Some(ToolResultDetails::Count(details)) = tool_result.details else {
        panic!("expected Count details")
    };
    assert_eq!(details.count, 1);
}

#[test]
fn ask_user_tool_result_accepts_selection_response() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "ask_user",
        vec![json!({
            "type": "text",
            "text": "User answered: Continue - No need to run the review agents again"
        })],
        false,
        Some(json!({
            "question": "We\'ve reached the third review-agent pass. Should I continue and make the last two small code-review fixes?",
            "context": "Current state: cargo nextest passed for cost_analyzer + pi_logs.",
            "options": [
                {
                    "title": "Continue",
                    "description": "Apply the two follow-up fixes."
                },
                {
                    "title": "Stop here",
                    "description": "Leave the current code as-is."
                }
            ],
            "response": {
                "kind": "selection",
                "selections": ["Continue"],
                "comment": "No need to run the review agents again"
            },
            "cancelled": false
        })),
    ));

    let Some(ToolResultDetails::AskUser(details)) = tool_result.details else {
        panic!("expected AskUser details")
    };

    assert_eq!(
        details.context.as_deref(),
        Some("Current state: cargo nextest passed for cost_analyzer + pi_logs.")
    );
    assert_eq!(
        details.options,
        vec![
            AskUserOption::Detailed(AskUserDetailedOption {
                title: "Continue".to_string(),
                description: Some("Apply the two follow-up fixes.".to_string()),
            }),
            AskUserOption::Detailed(AskUserDetailedOption {
                title: "Stop here".to_string(),
                description: Some("Leave the current code as-is.".to_string()),
            })
        ]
    );
    match details.response {
        Some(AskUserResponse::Selection {
            selections,
            comment,
        }) => {
            assert_eq!(selections, vec!["Continue".to_string()]);
            assert_eq!(
                comment.as_deref(),
                Some("No need to run the review agents again")
            );
        }
        other => panic!("expected selection response, got {other:?}"),
    }
    assert!(!details.cancelled);
}

#[test]
fn ask_user_tool_result_rejects_unknown_option_field() {
    let err = parse_err(tool_result_message_json(
        "ask_user",
        vec![json!({"type": "text", "text": "invalid option"})],
        false,
        Some(json!({
            "question": "Continue?",
            "options": [{
                "title": "Yes",
                "description": "Proceed",
                "unexpected": true
            }],
            "cancelled": false
        })),
    ));
    let msg = err.to_string();
    assert!(
        msg.contains("unexpected"),
        "expected parse error to mention unexpected, got: {msg}"
    );
}

#[test]
fn contact_supervisor_tool_result_accepts_error_flag() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "contact_supervisor",
        vec![json!({"type": "text", "text": "Failed: no reply within 10 minutes"})],
        false,
        Some(json!({"error": true})),
    ));

    let Some(ToolResultDetails::ContactSupervisor(details)) = tool_result.details else {
        panic!("expected ContactSupervisor details")
    };

    assert_eq!(details.error, Some(true));
}

#[test]
fn mcp_tool_result_accepts_call_result() {
    let details = parse_mcp_details(
        vec![json!({
            "type": "text",
            "text": "{\"exit_code\":0,\"stderr\":\"\",\"stdout\":\"working tree clean\\n\"}"
        })],
        json!({
            "mode": "call",
            "mcpResult": {
                "content": [{
                    "type": "text",
                    "text": "{\"exit_code\":0,\"stderr\":\"\",\"stdout\":\"working tree clean\\n\"}"
                }],
                "structuredContent": {
                    "exit_code": 0,
                    "stderr": "",
                    "stdout": "working tree clean\n"
                },
                "isError": false
            },
            "server": "git-read-only",
            "tool": "status"
        }),
    );

    assert_eq!(details.mode, McpMode::Call);
    assert_eq!(details.server.as_deref(), Some("git-read-only"));
    assert_eq!(details.tool.as_ref().map(McpTool::name), Some("status"));

    let mcp_result = details.mcp_result.expect("expected mcp result");
    assert!(!mcp_result.is_error);
    assert_eq!(
        mcp_result.structured_content,
        Some(JsonBlob::from(json!({
            "exit_code": 0,
            "stderr": "",
            "stdout": "working tree clean\n"
        })))
    );
}

#[test]
fn mcp_tool_result_accepts_arbitrary_structured_content() {
    let details = parse_mcp_details(
        vec![json!({
            "type": "text",
            "text": "resource payload available"
        })],
        json!({
            "mode": "call",
            "mcpResult": {
                "content": [{
                    "type": "resource",
                    "resource": {"uri": "mcp://example/items/1", "text": "payload"}
                }],
                "structuredContent": {
                    "items": [{"id": 1, "name": "example"}],
                    "nextCursor": "cursor-2"
                },
                "isError": false
            },
            "server": "project-tools",
            "tool": "list_items"
        }),
    );

    let mcp_result = details.mcp_result.expect("expected mcp result");
    assert_eq!(
        mcp_result.content,
        vec![JsonBlob::from(json!({
            "type": "resource",
            "resource": {"uri": "mcp://example/items/1", "text": "payload"}
        }))]
    );
    assert_eq!(
        mcp_result.structured_content,
        Some(JsonBlob::from(json!({
            "items": [{"id": 1, "name": "example"}],
            "nextCursor": "cursor-2"
        })))
    );
}

#[test]
fn mcp_tool_result_accepts_missing_structured_content() {
    let details = parse_mcp_details(
        vec![
            json!({"type": "text", "text": "stdout: \n\n "}),
            json!({
                "type": "text",
                "text": "stderr: \n\n warning: workspace hack crate has no edition"
            }),
        ],
        json!({
            "mode": "call",
            "mcpResult": {
                "content": [
                    {"type": "text", "text": "stdout: \n\n "},
                    {
                        "type": "text",
                        "text": "stderr: \n\n warning: workspace hack crate has no edition"
                    }
                ],
                "isError": false
            },
            "server": "project-tools",
            "tool": "run_tests"
        }),
    );

    assert_eq!(details.mode, McpMode::Call);
    let mcp_result = details.mcp_result.expect("expected mcp result");
    assert!(!mcp_result.is_error);
    assert!(mcp_result.structured_content.is_none());
    assert_eq!(details.server.as_deref(), Some("project-tools"));
    assert_eq!(details.tool.as_ref().map(McpTool::name), Some("run_tests"));
}

#[test]
fn mcp_tool_result_accepts_describe_mode() {
    let details = parse_mcp_details(
        vec![json!({
            "type": "text",
            "text": "jj_read_only_run\nServer: jj-read-only"
        })],
        json!({
            "mode": "describe",
            "server": "jj-read-only",
            "tool": {
                "name": "jj_read_only_run",
                "originalName": "run",
                "description": "Runs a jj command",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project_dir": {"type": "string"}
                    },
                    "required": ["project_dir"]
                }
            }
        }),
    );

    assert_eq!(details.mode, McpMode::Describe);
    assert_eq!(details.server.as_deref(), Some("jj-read-only"));
    let described = details
        .tool
        .as_ref()
        .and_then(McpTool::described)
        .expect("expected described tool");
    assert_eq!(described.name, "jj_read_only_run");
    assert_eq!(described.original_name.as_deref(), Some("run"));
    assert_eq!(described.description, "Runs a jj command");
    assert_eq!(
        described.input_schema,
        JsonBlob::from(json!({
            "type": "object",
            "properties": {
                "project_dir": {"type": "string"}
            },
            "required": ["project_dir"]
        }))
    );
}

#[test]
fn mcp_tool_result_accepts_call_failure() {
    let details = parse_mcp_details(
        vec![json!({
            "type": "text",
            "text": "Failed to call tool: MCP error -32600: Project tools not approved"
        })],
        json!({
            "mode": "call",
            "error": "call_failed",
            "message": "MCP error -32600: Project tools not approved"
        }),
    );

    assert_eq!(details.mode, McpMode::Call);
    assert_eq!(details.error.as_deref(), Some("call_failed"));
    assert_eq!(
        details.message.as_deref(),
        Some("MCP error -32600: Project tools not approved")
    );
    assert!(details.mcp_result.is_none());
}

#[test]
fn mcp_tool_result_rejects_unknown_mode() {
    let err = parse_err(tool_result_message_json(
        "mcp",
        vec![json!({"type": "text", "text": "unknown mode"})],
        false,
        Some(json!({"mode": "probe"})),
    ));
    let msg = err.to_string();
    assert!(
        msg.contains("probe"),
        "expected parse error to mention probe, got: {msg}"
    );
}

#[test]
fn code_search_tool_result_accepts_error_details() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "code_search",
        vec![json!({
            "type": "text",
            "text": "Error: MCP error -32602: Tool get_code_context_exa not found"
        })],
        false,
        Some(json!({
            "query": "jscpd ignore comment syntax ignore-start ignore-end",
            "maxTokens": 2000,
            "error": "MCP error -32602: Tool get_code_context_exa not found"
        })),
    ));

    let Some(ToolResultDetails::CodeSearch(details)) = tool_result.details else {
        panic!("expected CodeSearch details")
    };

    assert_eq!(
        details.query,
        "jscpd ignore comment syntax ignore-start ignore-end"
    );
    assert_eq!(details.max_tokens, 2000);
    assert_eq!(details.mode, None);
    assert_eq!(
        details.error.as_deref(),
        Some("MCP error -32602: Tool get_code_context_exa not found")
    );
}

#[test]
fn code_search_tool_result_accepts_fallback_mode_details() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "code_search",
        vec![json!({
            "type": "text",
            "text": "code search fallback results"
        })],
        false,
        Some(json!({
            "query": "sidecar ordering fallback",
            "maxTokens": 3000,
            "mode": "web-search-fallback"
        })),
    ));

    let Some(ToolResultDetails::CodeSearch(details)) = tool_result.details else {
        panic!("expected CodeSearch details")
    };

    assert_eq!(details.query, "sidecar ordering fallback");
    assert_eq!(details.max_tokens, 3000);
    assert_eq!(details.mode.as_deref(), Some("web-search-fallback"));
    assert_eq!(details.error, None);
}

#[test]
fn read_tool_result_accepts_lines_truncated() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "read",
        vec![json!({
            "type": "text",
            "text": "{\n  \"defaultProvider\": \"openai\"\n}"
        })],
        false,
        Some(json!({
            "truncation": {
                "content": "{\n  \"defaultProvider\": \"openai\"\n}",
                "truncated": true,
                "truncatedBy": "bytes",
                "totalLines": 100,
                "totalBytes": 64226,
                "outputLines": 79,
                "outputBytes": 51016,
                "lastLinePartial": false,
                "firstLineExceedsLimit": false,
                "maxLines": 9007199254740991u64,
                "maxBytes": 51200
            },
            "linesTruncated": true,
            "matchLimitReached": 100
        })),
    ));

    let Some(ToolResultDetails::Read(details)) = tool_result.details else {
        panic!("expected Read details")
    };

    let truncation = details
        .truncation
        .as_ref()
        .expect("expected truncation details");
    assert!(truncation.truncated);
    assert_eq!(truncation.truncated_by, TruncatedBy::Bytes);
    assert_eq!(details.lines_truncated, Some(true));
    assert_eq!(details.match_limit_reached, Some(100));
}

#[test]
fn web_search_tool_result_accepts_details() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "web_search",
        vec![json!({"type": "text", "text": "Found 3 results"})],
        false,
        Some(json!({
            "searchId": "search_1",
            "fetchId": "fetch_1",
            "queryCount": 2,
            "successfulQueries": 1,
            "totalResults": 3,
            "includeContent": true,
            "queries": ["rust serde deny_unknown_fields", "pi log parser"]
        })),
    ));

    let Some(ToolResultDetails::WebSearch(details)) = tool_result.details else {
        panic!("expected WebSearch details")
    };

    assert_eq!(details.search_id.as_deref(), Some("search_1"));
    assert_eq!(details.fetch_id.as_deref(), Some("fetch_1"));
    assert_eq!(details.total_results, 3);
    assert!(details.include_content);

    assert!(!details.curated);
    assert_eq!(details.curated_from, None);
    assert_eq!(details.curated_queries, None);
    assert_eq!(details.summary, None);
}

#[test]
fn web_search_tool_result_accepts_curated_details() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "web_search",
        vec![json!({"type": "text", "text": "Found curated results"})],
        false,
        Some(json!({
            "searchId": "search_curated",
            "fetchId": "fetch_curated",
            "queryCount": 3,
            "successfulQueries": 2,
            "totalResults": 10,
            "includeContent": true,
            "queries": ["initial query", "follow up", "third query"],
            "curated": true,
            "curatedFrom": 3,
            "curatedQueries": [{
                "query": "curated query one",
                "provider": "exa",
                "answer": "Curated answer one",
                "sources": [{
                    "title": "Example Source",
                    "url": "https://example.com/source1"
                }]
            }, {
                "query": "curated query two",
                "provider": "perplexity",
                "answer": "Curated answer two",
                "sources": [{
                    "title": "Another Source",
                    "url": "https://example.com/source2"
                }, {
                    "title": "Third Source",
                    "url": "https://example.com/source3"
                }]
            }],
            "summary": {
                "text": "Curated search summary text",
                "workflow": "curated-search",
                "model": "claude-sonnet-4-5",
                "durationMs": 1234,
                "tokenEstimate": 567,
                "fallbackUsed": true,
                "edited": true
            }
        })),
    ));

    let Some(ToolResultDetails::WebSearch(details)) = tool_result.details else {
        panic!("expected WebSearch details")
    };

    assert!(details.curated);
    assert_eq!(details.curated_from, Some(3));

    let curated_queries = details
        .curated_queries
        .as_ref()
        .expect("expected curated queries");
    assert_eq!(curated_queries.len(), 2);

    assert_eq!(curated_queries[0].query, "curated query one");
    assert_eq!(curated_queries[0].provider.as_deref(), Some("exa"));
    assert_eq!(
        curated_queries[0].answer.as_deref(),
        Some("Curated answer one")
    );
    let sources_0 = curated_queries[0]
        .sources
        .as_ref()
        .expect("expected sources");
    assert_eq!(sources_0.len(), 1);
    assert_eq!(sources_0[0].title, "Example Source");
    assert_eq!(sources_0[0].url, "https://example.com/source1");
    assert_eq!(curated_queries[0].error, None);

    assert_eq!(curated_queries[1].query, "curated query two");
    assert_eq!(curated_queries[1].provider.as_deref(), Some("perplexity"));
    assert_eq!(
        curated_queries[1].answer.as_deref(),
        Some("Curated answer two")
    );
    let sources_1 = curated_queries[1]
        .sources
        .as_ref()
        .expect("expected sources");
    assert_eq!(sources_1.len(), 2);
    assert_eq!(sources_1[0].title, "Another Source");
    assert_eq!(sources_1[1].title, "Third Source");
    assert_eq!(curated_queries[1].error, None);

    let summary = details.summary.as_ref().expect("expected summary");
    assert_eq!(summary.text, "Curated search summary text");
    assert_eq!(summary.workflow, "curated-search");
    assert_eq!(summary.model.as_deref(), Some("claude-sonnet-4-5"));
    assert_eq!(summary.duration_ms, Some(1234));
    assert_eq!(summary.token_estimate, Some(567));
    assert!(summary.fallback_used);
    assert!(summary.edited);
}

#[test]
fn web_search_tool_result_summary_defaults() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "web_search",
        vec![json!({"type": "text", "text": "Found results"})],
        false,
        Some(json!({
            "searchId": "search_1",
            "queryCount": 1,
            "successfulQueries": 1,
            "totalResults": 5,
            "includeContent": false,
            "queries": ["rust serde"],
            "curated": true,
            "summary": {
                "text": "Summary text",
                "workflow": "curated-web-search"
            }
        })),
    ));

    let Some(ToolResultDetails::WebSearch(details)) = tool_result.details else {
        panic!("expected WebSearch details")
    };

    let summary = details.summary.as_ref().expect("expected summary");
    assert_eq!(summary.text, "Summary text");
    assert_eq!(summary.workflow, "curated-web-search");
    assert_eq!(summary.model, None);
    assert_eq!(summary.duration_ms, None);
    assert_eq!(summary.token_estimate, None);
    assert!(!summary.fallback_used);
    assert!(!summary.edited);
}

#[test]
fn web_search_tool_result_rejects_unknown_curated_query_source_field() {
    let err = parse_err(tool_result_message_json(
        "web_search",
        vec![json!({"type": "text", "text": "curated"})],
        false,
        Some(json!({
            "searchId": "search_1",
            "queryCount": 1,
            "successfulQueries": 1,
            "totalResults": 1,
            "includeContent": false,
            "queries": ["query"],
            "curated": true,
            "curatedQueries": [{
                "query": "q",
                "sources": [{
                    "title": "Test",
                    "url": "https://example.com",
                    "unexpected": true
                }]
            }]
        })),
    ))
    .to_string();

    assert!(
        err.contains("unknown field") || err.contains("unexpected"),
        "expected parse error mentioning unknown source field, got: {err}"
    );
}

#[test]
fn web_search_tool_result_rejects_unknown_curated_query_field() {
    let err = parse_err(tool_result_message_json(
        "web_search",
        vec![json!({"type": "text", "text": "curated"})],
        false,
        Some(json!({
            "searchId": "search_1",
            "queryCount": 1,
            "successfulQueries": 1,
            "totalResults": 1,
            "includeContent": false,
            "queries": ["query"],
            "curated": true,
            "curatedQueries": [{
                "query": "q",
                "unexpected": true
            }]
        })),
    ))
    .to_string();

    assert!(
        err.contains("unknown field") || err.contains("unexpected"),
        "expected parse error mentioning unknown curated query field, got: {err}"
    );
}

#[test]
fn web_search_tool_result_rejects_unknown_summary_field() {
    let err = parse_err(tool_result_message_json(
        "web_search",
        vec![json!({"type": "text", "text": "curated"})],
        false,
        Some(json!({
            "searchId": "search_1",
            "queryCount": 1,
            "successfulQueries": 1,
            "totalResults": 1,
            "includeContent": false,
            "queries": ["query"],
            "curated": true,
            "summary": {
                "text": "summary",
                "workflow": "search",
                "unexpected": true
            }
        })),
    ))
    .to_string();

    assert!(
        err.contains("unknown field") || err.contains("unexpected"),
        "expected parse error mentioning unknown summary field, got: {err}"
    );
}

#[test]
fn web_search_tool_result_accepts_curated_query_error_entry() {
    // A curated query with only query + error (no provider/answer/sources)
    // exercises the optional success-only fields on CuratedQueryInfo.
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "web_search",
        vec![json!({"type": "text", "text": "Found partial curated results"})],
        false,
        Some(json!({
            "searchId": "search_1",
            "fetchId": "fetch_1",
            "queryCount": 1,
            "successfulQueries": 0,
            "totalResults": 0,
            "includeContent": true,
            "queries": ["failed query"],
            "curated": true,
            "curatedFrom": 1,
            "curatedQueries": [{
                "query": "failed query",
                "error": "This operation was aborted"
            }]
        })),
    ));

    let Some(ToolResultDetails::WebSearch(details)) = tool_result.details else {
        panic!("expected WebSearch details")
    };

    let curated_queries = details
        .curated_queries
        .as_ref()
        .expect("expected curated queries");

    assert_eq!(curated_queries.len(), 1);
    assert_eq!(curated_queries[0].query, "failed query");
    assert_eq!(
        curated_queries[0].error.as_deref(),
        Some("This operation was aborted")
    );
    assert_eq!(curated_queries[0].provider, None);
    assert_eq!(curated_queries[0].answer, None);
    assert_eq!(curated_queries[0].sources, None);
}

#[test]
fn web_search_tool_result_rejects_unknown_details_field() {
    assert_parse_error_contains_any(
        "web_search rejects unknown details field",
        tool_result_message_json(
            "web_search",
            vec![json!({"type": "text", "text": "Found results"})],
            false,
            Some(json!({
                "searchId": "search_1",
                "queryCount": 1,
                "successfulQueries": 1,
                "totalResults": 1,
                "includeContent": false,
                "queries": ["query"],
                "curated": true,
                "unexpected": true
            })),
        ),
        &["unknown field", "unexpected"],
    );
}

#[test]
fn grep_tool_result_accepts_both_limits() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "grep",
        vec![json!({"type": "text", "text": "too many matches"})],
        false,
        Some(json!({
            "matchLimitReached": 100,
            "linesTruncated": true
        })),
    ));

    let Some(ToolResultDetails::Grep(details)) = tool_result.details else {
        panic!("expected Grep details")
    };

    assert_eq!(details.match_limit_reached, Some(100));
    assert_eq!(details.lines_truncated, Some(true));
}

#[test]
fn grep_tool_result_accepts_truncation_details() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "grep",
        vec![json!({"type": "text", "text": "truncated output"})],
        false,
        Some(json!({
            "path": "README.md",
            "pattern": "cache",
            "source": "lean-ctx",
            "truncation": {
                "content": "truncated output",
                "truncated": true,
                "truncatedBy": "bytes",
                "totalLines": 4000,
                "totalBytes": 204800,
                "outputLines": 2000,
                "outputBytes": 51200,
                "lastLinePartial": false,
                "firstLineExceedsLimit": false,
                "maxLines": 2000,
                "maxBytes": 51200
            }
        })),
    ));

    let Some(ToolResultDetails::Grep(details)) = tool_result.details else {
        panic!("expected Grep details")
    };

    assert_eq!(details.path, Some(PathBuf::from("README.md")));
    assert_eq!(details.pattern.as_deref(), Some("cache"));
    assert_eq!(details.source, Some(ToolResultSource::LeanCtx));
    assert_eq!(
        details
            .truncation
            .as_ref()
            .map(|truncation| truncation.truncated_by),
        Some(TruncatedBy::Bytes)
    );
}

#[test]
fn bash_tool_result_accepts_truncation_and_full_output_path() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "bash",
        vec![json!({"type": "text", "text": "truncated output"})],
        false,
        Some(json!({
            "truncation": {
                "content": "truncated output",
                "truncated": true,
                "truncatedBy": "lines",
                "totalLines": 4000,
                "totalBytes": 204800,
                "outputLines": 2000,
                "outputBytes": 51200,
                "lastLinePartial": false,
                "firstLineExceedsLimit": false,
                "maxLines": 2000,
                "maxBytes": 51200
            },
            "fullOutputPath": "/tmp/pi-bash.log"
        })),
    ));

    let Some(ToolResultDetails::Bash(details)) = tool_result.details else {
        panic!("expected Bash details")
    };

    assert_eq!(
        details.full_output_path,
        Some(PathBuf::from("/tmp/pi-bash.log"))
    );
    assert_eq!(
        details
            .truncation
            .as_ref()
            .map(|truncation| truncation.truncated_by),
        Some(TruncatedBy::Lines)
    );
}

#[test]
fn bash_details_serialize_full_output_path_as_camel_case() {
    let value = serde_json::to_value(BashDetails {
        truncation: None,
        full_output_path: Some(PathBuf::from("/tmp/pi-bash.log")),
        compression: None,
    })
    .expect("serialize bash details");

    assert_eq!(
        value.get("fullOutputPath"),
        Some(&Value::from("/tmp/pi-bash.log"))
    );
    assert!(value.get("full_output_path").is_none());
}

#[test]
fn plannotator_submit_plan_tool_result_accepts_feedback() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "plannotator_submit_plan",
        vec![json!({"type": "text", "text": "Plan denied"})],
        false,
        Some(json!({
            "approved": false,
            "feedback": "Please split the migration into two phases"
        })),
    ));

    let Some(ToolResultDetails::PlannotatorSubmitPlan(details)) = tool_result.details else {
        panic!("expected PlannotatorSubmitPlan details")
    };

    assert!(!details.approved);
    assert_eq!(
        details.feedback.as_deref(),
        Some("Please split the migration into two phases")
    );
}

#[test]
fn compress_tool_result_accepts_superseded_block_ids() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "compress",
        vec![json!({"type": "text", "text": "Compressed 2 ranges"})],
        false,
        Some(json!({
            "blockIds": [1, 2],
            "topic": "Parser incremental fixes",
            "supersededBlockIds": [9, 10]
        })),
    ));

    let Some(ToolResultDetails::Compress(details)) = tool_result.details else {
        panic!("expected Compress details")
    };

    assert_eq!(details.block_ids, vec![1, 2]);
    assert_eq!(details.topic, "Parser incremental fixes");
    assert_eq!(details.superseded_block_ids, vec![9, 10]);
}

#[test]
fn edit_tool_result_accepts_missing_first_changed_line() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "edit",
        vec![json!({"type": "text", "text": "ok"})],
        false,
        Some(json!({
            "diff": "--- a\n+++ b\n"
        })),
    ));

    let Some(ToolResultDetails::Edit(details)) = tool_result.details else {
        panic!("expected Edit details")
    };

    assert_eq!(details.first_changed_line, None);
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
                "startTimestamp": 1777084923000.5,
                "endTimestamp": 1777084924000_i64,
                "anchorTimestamp": 1777084924000.5,
                "active": true,
                "summaryTokenEstimate": 100,
                "createdAt": 1777084924500_i64,
                "tokensSavedEstimate": 4096,
                "savingsApplied": true,
                "supersededByBlockId": 2,
                "supersededAt": 1777084925000_i64,
                "supersedesBlockIds": [7, 8]
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
            assert_eq!(state.pruned_tool_ids, vec!["call_1"]);
            assert_eq!(state.tokens_saved, 1000);
            assert_eq!(state.total_prune_count, 3);
            assert!(!state.manual_mode);
            assert_eq!(state.compression_blocks.len(), 1);
            assert_eq!(state.compression_blocks[0].id, 1);
            assert_eq!(
                state.compression_blocks[0].tokens_saved_estimate,
                Some(4096)
            );
            assert_eq!(state.compression_blocks[0].topic, "Test topic");
            assert_eq!(state.compression_blocks[0].summary, "Test summary");
            assert_eq!(state.compression_blocks[0].savings_applied, Some(true));
            assert_eq!(state.compression_blocks[0].superseded_by_block_id, Some(2));
            assert_eq!(
                state.compression_blocks[0].superseded_at,
                Some(1777084925000)
            );
            assert_eq!(state.compression_blocks[0].supersedes_block_ids, vec![7, 8]);
            assert_eq!(
                state.compression_blocks[0].start_timestamp.to_string(),
                "1777084923000.5"
            );

            let serialized = serde_json::to_value(&state.compression_blocks[0])
                .expect("serialize compression block");
            assert_eq!(serialized.get("savingsApplied"), Some(&Value::from(true)));
            assert_eq!(serialized.get("supersededByBlockId"), Some(&Value::from(2)));
            assert_eq!(
                serialized.get("supersededAt"),
                Some(&Value::from(1777084925000_i64))
            );
            assert_eq!(serialized.get("supersedesBlockIds"), Some(&json!([7, 8])));
            assert!(serialized.get("savings_applied").is_none());
            assert!(serialized.get("superseded_by_block_id").is_none());
            assert!(serialized.get("superseded_at").is_none());
            assert!(serialized.get("supersedes_block_ids").is_none());
            assert_eq!(
                state.compression_blocks[0].end_timestamp.to_string(),
                "1777084924000"
            );
            assert_eq!(
                state.compression_blocks[0].anchor_timestamp.to_string(),
                "1777084924000.5"
            );
            assert!(state.compression_blocks[0].active);
            assert_eq!(state.compression_blocks[0].summary_token_estimate, 100);
            assert_eq!(state.compression_blocks[0].created_at, 1777084924500);
        }
        other => panic!("expected DcpState, got {other:?}"),
    }
}

#[test]
fn custom_intercom_sent() {
    match parse_custom_payload(
        "intercom_sent",
        json!({
            "to": "subagent-chat-019dfe82",
            "message": {
                "text": "Blocked on output requirement",
                "reason": "need_decision"
            },
            "messageId": "722b737e-1077-4be3-9d4e-9615d74a236d",
            "timestamp": 1778091848539_i64,
            "subagent": {
                "runId": "c14f72b8",
                "agent": "documentation-reviewer",
                "index": "2"
            }
        }),
    ) {
        CustomPayload::IntercomSent(details) => {
            assert_eq!(details.to, "subagent-chat-019dfe82");
            assert_eq!(details.message_id, "722b737e-1077-4be3-9d4e-9615d74a236d");
            assert_eq!(details.timestamp, 1778091848539);
            assert_eq!(
                details.message.0,
                json!({
                    "text": "Blocked on output requirement",
                    "reason": "need_decision"
                })
            );
            assert_eq!(
                details.subagent.as_ref().expect("expected subagent").0,
                json!({
                    "runId": "c14f72b8",
                    "agent": "documentation-reviewer",
                    "index": "2"
                })
            );
        }
        other => panic!("expected IntercomSent, got {other:?}"),
    }
}

#[test]
fn custom_plannotator() {
    match parse_custom_payload(
        "plannotator",
        json!({
            "phase": "planning",
            "planFilePath": "/tmp/PLAN.md",
            "lastSubmittedPath": "/tmp/submitted.md",
            "savedState": "draft"
        }),
    ) {
        CustomPayload::Plannotator(details) => {
            assert_eq!(details.phase, PlannotatorPhase::Planning);
            assert_eq!(details.plan_file_path, Some(PathBuf::from("/tmp/PLAN.md")));
            assert_eq!(
                details.last_submitted_path,
                Some(PathBuf::from("/tmp/submitted.md"))
            );
            assert_eq!(
                details.saved_state,
                Some(PlannotatorSavedState::Legacy("draft".to_string()))
            );
        }
        other => panic!("expected Plannotator, got {other:?}"),
    }
}

#[test]
fn custom_web_search_results() {
    match parse_custom_payload(
        "web-search-results",
        json!({
            "id": "search_1",
            "timestamp": MESSAGE_TIMESTAMP,
            "type": "search",
            "queries": [{
                "query": "rust serde deny_unknown_fields",
                "answer": "Use strict structs.",
                "results": [{
                    "title": "Serde docs",
                    "url": "https://serde.rs",
                    "snippet": "deny_unknown_fields"
                }],
                "provider": "exa"
            }]
        }),
    ) {
        CustomPayload::WebSearchResults(results) => {
            let WebSearchResultsPayload::Search(search) = &results.payload else {
                panic!("expected Search payload, got {:?}", results.payload);
            };
            assert_eq!(search.queries.len(), 1);
            assert_eq!(search.queries[0].provider.as_deref(), Some("exa"));
        }
        other => panic!("expected WebSearchResults, got {other:?}"),
    }
}

#[test]
fn custom_web_search_results_fetch() {
    match parse_custom_payload(
        "web-search-results",
        json!({
            "id": "fetch_1",
            "timestamp": MESSAGE_TIMESTAMP,
            "type": "fetch",
            "urls": [{
                "url": "https://example.com",
                "title": "Example",
                "content": "# Example",
                "error": null
            }]
        }),
    ) {
        CustomPayload::WebSearchResults(results) => {
            let WebSearchResultsPayload::Fetch(fetch) = &results.payload else {
                panic!("expected Fetch payload, got {:?}", results.payload);
            };
            assert_eq!(fetch.urls.len(), 1);
            assert_eq!(fetch.urls[0].url, "https://example.com");
            assert_eq!(fetch.urls[0].error, None);
        }
        other => panic!("expected WebSearchResults, got {other:?}"),
    }
}

#[test]
fn web_search_results_data_serializes_and_roundtrips() {
    for data in [
        WebSearchResultsData {
            id: "search_1".to_string(),
            timestamp: MESSAGE_TIMESTAMP,
            payload: WebSearchResultsPayload::Search(WebSearchResultsSearch {
                queries: vec![WebSearchQueryResult {
                    query: "rust serde".to_string(),
                    answer: "Use deny_unknown_fields.".to_string(),
                    results: vec![WebSearchResult {
                        title: "Serde docs".to_string(),
                        url: "https://serde.rs".to_string(),
                        snippet: "Strict parsing".to_string(),
                    }],
                    error: None,
                    provider: Some("exa".to_string()),
                }],
            }),
        },
        WebSearchResultsData {
            id: "fetch_1".to_string(),
            timestamp: MESSAGE_TIMESTAMP,
            payload: WebSearchResultsPayload::Fetch(WebSearchResultsFetch {
                urls: vec![WebFetchResult {
                    url: "https://example.com".to_string(),
                    title: "Example".to_string(),
                    content: "Body".to_string(),
                    error: None,
                }],
            }),
        },
    ] {
        let value = serde_json::to_value(&data).expect("expected serialization");
        assert!(value.get("type").is_some());
        assert!(value.get("id").is_some());
        assert!(value.get("timestamp").is_some());
        let roundtrip: WebSearchResultsData =
            serde_json::from_value(value.clone()).expect("expected roundtrip parse");
        assert_eq!(roundtrip, data);
        assert!(value.get("payload").is_none());
    }
}

#[test]
fn custom_web_search_results_fetch_rejects_missing_error_key() {
    let err = parse_err(custom_json(
        "web-search-results",
        json!({
            "id": "fetch_1",
            "timestamp": MESSAGE_TIMESTAMP,
            "type": "fetch",
            "urls": [{
                "url": "https://example.com",
                "title": "Example",
                "content": "# Example"
            }]
        }),
    ))
    .to_string();

    assert!(
        err.contains("missing field") && err.contains("error"),
        "expected parse error to mention missing error field, got: {err}"
    );
}

#[test]
fn custom_web_search_results_rejects_unknown_outer_field() {
    let err = parse_err(custom_json(
        "web-search-results",
        json!({
            "id": "search_1",
            "timestamp": MESSAGE_TIMESTAMP,
            "type": "search",
            "queries": [],
            "unexpected": true
        }),
    ))
    .to_string();

    assert!(
        err.contains("unknown field") || err.contains("unexpected"),
        "expected parse error to mention unexpected outer field, got: {err}"
    );
}

#[test]
fn custom_web_search_results_rejects_missing_or_unknown_type() {
    for (name, data, expected) in [
        (
            "missing type",
            json!({
                "id": "search_1",
                "timestamp": MESSAGE_TIMESTAMP,
                "queries": []
            }),
            vec!["missing field", "type"],
        ),
        (
            "unknown type",
            json!({
                "id": "search_1",
                "timestamp": MESSAGE_TIMESTAMP,
                "type": "stream",
                "queries": []
            }),
            vec!["unknown variant", "stream"],
        ),
    ] {
        assert_parse_error_contains_all(
            name,
            custom_json("web-search-results", data),
            expected.as_slice(),
        );
    }
}

#[test]
fn custom_web_search_results_rejects_wrong_variant_keys_or_missing_payload() {
    for (name, data, expected) in [
        (
            "search with urls",
            json!({
                "id": "search_1",
                "timestamp": MESSAGE_TIMESTAMP,
                "type": "search",
                "urls": []
            }),
            vec!["unknown field", "urls"],
        ),
        (
            "fetch with queries",
            json!({
                "id": "fetch_1",
                "timestamp": MESSAGE_TIMESTAMP,
                "type": "fetch",
                "queries": []
            }),
            vec!["unknown field", "queries"],
        ),
        (
            "search missing queries",
            json!({
                "id": "search_1",
                "timestamp": MESSAGE_TIMESTAMP,
                "type": "search"
            }),
            vec!["missing field", "queries"],
        ),
        (
            "fetch missing urls",
            json!({
                "id": "fetch_1",
                "timestamp": MESSAGE_TIMESTAMP,
                "type": "fetch"
            }),
            vec!["missing field", "urls"],
        ),
    ] {
        assert_parse_error_contains_all(
            name,
            custom_json("web-search-results", data),
            expected.as_slice(),
        );
    }
}

#[test]
fn custom_web_search_results_rejects_unknown_nested_search_field() {
    assert_parse_error_contains_all(
        "unknown WebSearchQueryResult field",
        custom_json(
            "web-search-results",
            json!({
                "id": "search_1",
                "timestamp": MESSAGE_TIMESTAMP,
                "type": "search",
                "queries": [{
                    "query": "rust serde deny_unknown_fields",
                    "answer": "Use strict structs.",
                    "results": [{
                        "title": "Serde docs",
                        "url": "https://serde.rs",
                        "snippet": "deny_unknown_fields"
                    }],
                    "provider": "exa",
                    "unexpected": true
                }]
            }),
        ),
        &["unknown field", "unexpected"],
    );
}

#[test]
fn custom_web_search_results_rejects_unknown_nested_fetch_field() {
    assert_parse_error_contains_all(
        "unknown WebFetchResult field",
        custom_json(
            "web-search-results",
            json!({
                "id": "fetch_1",
                "timestamp": MESSAGE_TIMESTAMP,
                "type": "fetch",
                "urls": [{
                    "url": "https://example.com",
                    "title": "Example",
                    "content": "# Example",
                    "error": null,
                    "unexpected": true
                }]
            }),
        ),
        &["unknown field", "unexpected"],
    );
}

#[test]
fn custom_plannotator_execute() {
    match parse_custom_payload(
        "plannotator-execute",
        json!({
            "lastSubmittedPath": "/tmp/PLAN.md"
        }),
    ) {
        CustomPayload::PlannotatorExecute(details) => {
            assert_eq!(
                details.last_submitted_path,
                Some(PathBuf::from("/tmp/PLAN.md"))
            );
            assert_eq!(details.plan_file_path, None);
        }
        other => panic!("expected PlannotatorExecute, got {other:?}"),
    }
}

#[test]
fn custom_message_dcp_compress_trigger() {
    assert!(matches!(
        parse_custom_message_payload(
            "Compress the oldest closed section.",
            "dcp-compress-trigger",
            None,
        ),
        CustomMessagePayload::DcpCompressTrigger
    ));
}

#[test]
fn custom_message_dcp_compress_trigger_rejects_details() {
    assert_parse_error_contains_any(
        "dcp-compress-trigger rejects details",
        custom_message_json(
            "Compress the oldest closed section.",
            "dcp-compress-trigger",
            Some(json!({"unexpected": true})),
        ),
        &[
            "unknown variant",
            "did not match any variant",
            "dcp-compress-trigger",
        ],
    );
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
fn custom_message_plannotator_complete_rejects_details() {
    assert_parse_error_contains_any(
        "plannotator-complete rejects details",
        custom_message_json(
            "Plan complete",
            "plannotator-complete",
            Some(json!({"unexpected": true})),
        ),
        &[
            "unknown variant",
            "did not match any variant",
            "plannotator-complete",
        ],
    );
}

#[test]
fn rejects_unknown_session_field() {
    let mut bad_session = session_json("/tmp");
    bad_session
        .as_object_mut()
        .unwrap()
        .insert("bogus".to_string(), json!("value"));

    assert_parse_error_contains_any("rejects unknown session field", bad_session, &["bogus"]);
}

#[test]
fn rejects_unknown_loaded_tool_name() {
    let unknown_loaded_tool = loaded_tool_json("mystery_tool");

    assert_parse_error_contains_any(
        "rejects unknown loaded tool name",
        custom_message_json(
            "loaded",
            "pi-loaded-tools",
            Some(json!({
                "tools": [unknown_loaded_tool],
            })),
        ),
        &["mystery_tool", "unknown variant"],
    );
}

#[test]
fn parse_file_smoke_parses_multiple_line_types() {
    let tmp = std::env::temp_dir().join(format!("pi_logs_smoke_{}.jsonl", uuid::Uuid::new_v4()));
    std::fs::write(
        &tmp,
        format!(
            "{}\n{}\n{}\n{}\n",
            session_json("/tmp"),
            model_change_json(Some("session"), "anthropic", "claude-sonnet-4-5"),
            branch_summary_json(false),
            user_message_json("hello")
        ),
    )
    .unwrap();

    let parsed = parse_file(&tmp).expect("expected parse success");
    let _ = std::fs::remove_file(&tmp);

    assert_eq!(parsed.len(), 4);
    assert!(matches!(parsed[0], PiLogLine::Session(_)));
    assert!(matches!(parsed[1], PiLogLine::ModelChange(_)));
    assert!(matches!(parsed[2], PiLogLine::BranchSummary(_)));
    assert!(matches!(parsed[3], PiLogLine::Message(_)));
}

#[test]
fn parse_file_ignores_blank_and_whitespace_only_lines() {
    let tmp = std::env::temp_dir().join(format!("pi_logs_blank_{}.jsonl", uuid::Uuid::new_v4()));
    std::fs::write(
        &tmp,
        format!(
            "{}\n\n   \n\t\n{}\n",
            session_json("/tmp"),
            session_json("/tmp/project")
        ),
    )
    .unwrap();

    let parsed = parse_file(&tmp).expect("expected parse success");
    let _ = std::fs::remove_file(&tmp);

    assert_eq!(parsed.len(), 2);
}

#[test]
fn parse_file_accepts_empty_file() {
    let tmp = std::env::temp_dir().join(format!("pi_logs_empty_{}.jsonl", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, "").unwrap();

    let parsed = parse_file(&tmp).expect("expected parse success");
    let _ = std::fs::remove_file(&tmp);

    assert!(parsed.is_empty());
}

#[test]
fn parse_file_reports_missing_file() {
    let tmp = std::env::temp_dir().join(format!("pi_logs_missing_{}.jsonl", uuid::Uuid::new_v4()));

    let err = parse_file(&tmp).expect_err("expected missing file failure");
    match err {
        ParseError::Open { path, .. } => assert_eq!(path, tmp),
        other => panic!("expected Open, got {other:?}"),
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

#[test]
fn find_tool_result_accepts_lean_ctx_augmentation() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "find",
        vec![json!({"type": "text", "text": "matches"})],
        false,
        Some(json!({
            "path": "crates",
            "pattern": "*.rs",
            "source": "lean-ctx",
            "truncated": false,
            "compression": {"originalTokens": 1234, "compressedTokens": 456, "percentSaved": 63}
        })),
    ));
    let Some(ToolResultDetails::Find(details)) = tool_result.details else {
        panic!("expected Find details")
    };
    assert_eq!(details.path, Some(PathBuf::from("crates")));
    assert_eq!(details.pattern.as_deref(), Some("*.rs"));
    assert_eq!(details.source, Some(ToolResultSource::LeanCtx));
    assert_eq!(details.truncated, Some(false));
    let compression = details.compression.expect("expected compression");
    assert_eq!(compression.original_tokens, 1234);
    assert_eq!(compression.compressed_tokens, 456);
    assert_eq!(compression.percent_saved, 63);
}

#[test]
fn find_tool_result_accepts_legacy_result_limit() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "find",
        vec![json!({"type": "text", "text": "..."})],
        false,
        Some(json!({"resultLimitReached": 250})),
    ));
    let Some(ToolResultDetails::Find(details)) = tool_result.details else {
        panic!("expected Find details")
    };
    assert_eq!(details.result_limit_reached, Some(250));
    assert!(details.truncation.is_none());
    assert!(details.compression.is_none());
}

#[test]
fn find_tool_result_accepts_result_limit_and_truncation_details() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "find",
        vec![json!({"type": "text", "text": "sessions/..."})],
        false,
        Some(json!({
            "resultLimitReached": 500,
            "truncation": {
                "content": "sessions/...",
                "truncated": true,
                "truncatedBy": "bytes",
                "totalLines": 500,
                "totalBytes": 59789,
                "outputLines": 427,
                "outputBytes": 51147,
                "lastLinePartial": false,
                "firstLineExceedsLimit": false,
                "maxLines": 9007199254740991u64,
                "maxBytes": 51200
            }
        })),
    ));
    let Some(ToolResultDetails::Find(details)) = tool_result.details else {
        panic!("expected Find details")
    };
    assert_eq!(details.result_limit_reached, Some(500));
    let truncation = details
        .truncation
        .as_ref()
        .expect("expected truncation details");
    assert!(truncation.truncated);
    assert_eq!(truncation.truncated_by, TruncatedBy::Bytes);
    assert_eq!(truncation.total_bytes, 59789);
    assert_eq!(truncation.output_bytes, 51147);
    assert!(details.compression.is_none());
}

#[test]
fn find_tool_result_accepts_truncation_details_without_result_limit() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "find",
        vec![json!({"type": "text", "text": "sessions/..."})],
        false,
        Some(json!({
            "truncation": {
                "content": "sessions/...",
                "truncated": true,
                "truncatedBy": "bytes",
                "totalLines": 500,
                "totalBytes": 59789,
                "outputLines": 427,
                "outputBytes": 51147,
                "lastLinePartial": false,
                "firstLineExceedsLimit": false,
                "maxLines": 9007199254740991u64,
                "maxBytes": 51200
            }
        })),
    ));
    let Some(ToolResultDetails::Find(details)) = tool_result.details else {
        panic!("expected Find details")
    };
    assert!(details.result_limit_reached.is_none());
    let truncation = details
        .truncation
        .as_ref()
        .expect("expected truncation details");
    assert!(truncation.truncated);
    assert_eq!(truncation.truncated_by, TruncatedBy::Bytes);
    assert_eq!(truncation.content, "sessions/...");
    assert_eq!(truncation.output_bytes, 51147);
    assert!(details.compression.is_none());
}

#[test]
fn ls_tool_result_accepts_lean_ctx_augmentation() {
    let tool_result = parse_ls_lean_ctx_fixture(
        true,
        json!({"originalTokens": 100, "compressedTokens": 40, "percentSaved": 60}),
    );
    let Some(ToolResultDetails::Ls(details)) = tool_result.details else {
        panic!("expected Ls details")
    };
    assert_eq!(details.path, Some(PathBuf::from("crates")));
    assert_eq!(details.source, Some(ToolResultSource::LeanCtx));
    assert_eq!(details.truncated, Some(true));
    let compression = details.compression.expect("expected compression");
    assert_eq!(compression.original_tokens, 100);
    assert_eq!(compression.compressed_tokens, 40);
    assert_eq!(compression.percent_saved, 60);
    assert!(details.entry_limit_reached.is_none());
}

#[test]
fn ls_tool_result_accepts_entry_limit_only() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "ls",
        vec![json!({"type": "text", "text": "..."})],
        false,
        Some(json!({"entryLimitReached": 500})),
    ));
    let Some(ToolResultDetails::Ls(details)) = tool_result.details else {
        panic!("expected Ls details")
    };
    assert_eq!(details.entry_limit_reached, Some(500));
    assert!(details.compression.is_none());
}

#[test]
fn git_read_only_tool_results_route_all_flat_tool_names() {
    for (tool_name, expected_tool) in [
        ("git_read_only_diff", "diff"),
        ("git_read_only_log", "log"),
        ("git_read_only_show", "show"),
        ("git_read_only_status", "status"),
    ] {
        let tool_result = parse_tool_result_message(tool_result_message_json(
            tool_name,
            vec![json!({"type": "text", "text": "git output"})],
            false,
            Some(json!({"server": "git-read-only", "tool": expected_tool})),
        ));
        let Some(ToolResultDetails::GitReadOnly(details)) = tool_result.details else {
            panic!("expected GitReadOnly details for {tool_name}")
        };
        assert_eq!(details.server, "git-read-only");
        assert_eq!(details.tool, expected_tool);
    }
}

#[test]
fn fetch_content_tool_result_accepts_details() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "fetch_content",
        vec![json!({"type": "text", "text": "fetched"})],
        false,
        Some(json!({
            "urls": ["https://example.com/a", "https://example.com/b"],
            "urlCount": 2,
            "successful": 2,
            "totalChars": 12000u64,
            "title": "Example",
            "responseId": "resp_1",
            "truncated": false,
            "hasImage": false,
            "imageCount": 0,
            "prompt": "Summarize this page"
        })),
    ));
    let Some(ToolResultDetails::FetchContent(details)) = tool_result.details else {
        panic!("expected FetchContent details")
    };
    assert_eq!(details.urls.len(), 2);
    assert_eq!(details.url_count, 2);
    assert_eq!(details.successful, 2);
    assert_eq!(details.total_chars, 12000);
    assert_eq!(details.title.as_deref(), Some("Example"));
    assert_eq!(details.response_id, "resp_1");
    assert!(!details.truncated);
    assert!(!details.has_image);
    assert_eq!(details.image_count, 0);
    assert_eq!(details.prompt.as_deref(), Some("Summarize this page"));
}

#[test]
fn fetch_content_tool_result_accepts_missing_optional_metadata() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "fetch_content",
        vec![json!({"type": "text", "text": "fetched"})],
        false,
        Some(json!({
            "urls": ["https://example.com/a"],
            "urlCount": 1,
            "successful": 1,
            "totalChars": 12000u64,
            "responseId": "resp_1"
        })),
    ));
    let Some(ToolResultDetails::FetchContent(details)) = tool_result.details else {
        panic!("expected FetchContent details")
    };
    assert_eq!(details.urls, ["https://example.com/a"]);
    assert_eq!(details.url_count, 1);
    assert_eq!(details.successful, 1);
    assert_eq!(details.total_chars, 12000);
    assert!(details.title.is_none());
    assert_eq!(details.response_id, "resp_1");
    assert!(!details.truncated);
    assert!(!details.has_image);
    assert_eq!(details.image_count, 0);
    assert!(details.prompt.is_none());
    assert!(details.error.is_none());
}

#[test]
fn fetch_content_tool_result_accepts_top_level_error_summary() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "fetch_content",
        vec![json!({"type": "text", "text": "Error: fetch failed"})],
        true,
        Some(json!({
            "urls": ["file:///tmp/example.html"],
            "urlCount": 1,
            "successful": 0,
            "responseId": "resp_1",
            "error": "fetch failed"
        })),
    ));
    let Some(ToolResultDetails::FetchContent(details)) = tool_result.details else {
        panic!("expected FetchContent details")
    };
    assert_eq!(details.urls, ["file:///tmp/example.html"]);
    assert_eq!(details.url_count, 1);
    assert_eq!(details.successful, 0);
    assert_eq!(details.total_chars, 0);
    assert_eq!(details.response_id, "resp_1");
    assert_eq!(details.error.as_deref(), Some("fetch failed"));
}

#[test]
fn fetch_content_tool_result_rejects_unknown_details_field() {
    assert_parse_error_contains_any(
        "fetch_content rejects unknown details field",
        tool_result_message_json(
            "fetch_content",
            vec![json!({"type": "text", "text": "fetched"})],
            false,
            Some(json!({
                "urls": ["https://example.com/a"],
                "urlCount": 1,
                "successful": 1,
                "responseId": "resp_1",
                "unexpected": true
            })),
        ),
        &["unexpected", "unknown field"],
    );
}

#[test]
fn null_tool_result_details_are_treated_as_absent() {
    for tool_name in [
        "compress",
        "edit",
        "fetch_content",
        "git_read_only_log",
        "mcp",
        "web_search",
    ] {
        let tool_result = parse_tool_result_message(tool_result_message_json(
            tool_name,
            vec![json!({"type": "text", "text": "permission denied"})],
            true,
            Some(Value::Null),
        ));
        assert!(
            tool_result.details.is_none(),
            "expected null details to be dropped for {tool_name}"
        );
    }
}

#[test]
fn empty_error_tool_result_details_are_treated_as_absent() {
    for tool_name in [
        "compress",
        "edit",
        "fetch_content",
        "git_read_only_log",
        "mcp",
        "web_search",
    ] {
        let tool_result = parse_tool_result_message(tool_result_message_json(
            tool_name,
            vec![json!({"type": "text", "text": "permission denied"})],
            true,
            Some(json!({})),
        ));
        assert!(
            tool_result.details.is_none(),
            "expected empty error details to be dropped for {tool_name}"
        );
    }
}

#[test]
fn empty_success_tool_result_details_stay_strict() {
    assert_parse_error_contains_any(
        "empty success details stay strict",
        tool_result_message_json(
            "compress",
            vec![json!({"type": "text", "text": "Compressed 1 range"})],
            false,
            Some(json!({})),
        ),
        &["did not match any variant", "required field", "blockIds"],
    );
}

#[test]
fn get_search_content_tool_result_accepts_success_details() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "get_search_content",
        vec![json!({"type": "text", "text": "cached body"})],
        false,
        Some(json!({"url": "https://example.com", "title": "Example", "contentLength": 4096u64})),
    ));
    let Some(ToolResultDetails::GetSearchContent(GetSearchContentDetails::Success(details))) =
        tool_result.details
    else {
        panic!("expected GetSearchContent success details")
    };
    assert_eq!(details.url, "https://example.com");
    assert_eq!(details.title, "Example");
    assert_eq!(details.content_length, 4096);
}

#[test]
fn get_search_content_tool_result_accepts_error_details() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "get_search_content",
        vec![json!({"type": "text", "text": "URL not found. Available:\n  https://example.com"})],
        false,
        Some(json!({"error": "URL not found"})),
    ));
    let Some(ToolResultDetails::GetSearchContent(GetSearchContentDetails::Error(details))) =
        tool_result.details
    else {
        panic!("expected GetSearchContent error details")
    };
    assert_eq!(details.error, "URL not found");
}

// Pins the strict-variant invariant of GetSearchContentDetails: a payload
// that mixes success fields with the error field must not be silently
// accepted by either inner variant. Both inner structs use
// `deny_unknown_fields`, so the untagged enum has no valid match.
#[test]
fn get_search_content_tool_result_rejects_mixed_success_and_error_details() {
    assert_parse_error_contains_any(
        "rejects mixed get_search_content details",
        tool_result_message_json(
            "get_search_content",
            vec![json!({"type": "text", "text": "ambiguous"})],
            false,
            Some(json!({
                "url": "https://example.com",
                "title": "Example",
                "contentLength": 4096u64,
                "error": "URL not found",
            })),
        ),
        &["did not match any variant", "unknown field", "error"],
    );
}

// Pins that GetSearchContentErrorDetails honors the project-wide
// `deny_unknown_fields` strictness contract so a future protocol
// extension surfaces as a loud parse error rather than silently dropping
// fields.
#[test]
fn get_search_content_tool_result_rejects_unknown_error_detail_field() {
    assert_parse_error_contains_any(
        "rejects unknown get_search_content error detail field",
        tool_result_message_json(
            "get_search_content",
            vec![json!({"type": "text", "text": "URL not found"})],
            false,
            Some(json!({
                "error": "URL not found",
                "code": "not_found",
            })),
        ),
        &["did not match any variant", "unknown field", "code"],
    );
}

// Bash is the only currently-modeled tool whose lean-ctx breadcrumb can be
// just `{compression}` with no path or pattern discriminator.
#[test]
fn bash_tool_result_accepts_compression_only_lean_ctx_details() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "bash",
        vec![json!({"type": "text", "text": "compressed output"})],
        false,
        Some(json!({
            "compression": {"originalTokens": 5000, "compressedTokens": 1500, "percentSaved": 70}
        })),
    ));
    let Some(ToolResultDetails::Bash(details)) = tool_result.details else {
        panic!("expected Bash details")
    };
    let compression = details.compression.expect("expected compression");
    assert_eq!(compression.original_tokens, 5000);
    assert_eq!(compression.compressed_tokens, 1500);
    assert_eq!(compression.percent_saved, 70);
    assert!(details.full_output_path.is_none());
    assert!(details.truncation.is_none());
}

#[test]
fn bash_tool_result_accepts_lean_ctx_compression_with_full_output_path() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "bash",
        vec![json!({"type": "text", "text": "compressed output"})],
        false,
        Some(json!({
            "fullOutputPath": "/tmp/bash-output.log",
            "compression": {"originalTokens": 5000, "compressedTokens": 1500, "percentSaved": 70}
        })),
    ));
    let Some(ToolResultDetails::Bash(details)) = tool_result.details else {
        panic!("expected Bash details")
    };
    let compression = details.compression.expect("expected compression");
    assert_eq!(compression.original_tokens, 5000);
    assert_eq!(compression.compressed_tokens, 1500);
    assert_eq!(compression.percent_saved, 70);
    assert_eq!(
        details.full_output_path,
        Some(PathBuf::from("/tmp/bash-output.log"))
    );
    assert!(details.truncation.is_none());
}

#[test]
fn read_tool_result_accepts_lean_ctx_only() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "read",
        vec![json!({"type": "text", "text": "lean-ctx wrapped read"})],
        false,
        Some(json!({
            "path": "src/lib.rs",
            "source": "lean-ctx",
            "mode": "full",
            "lines": 123,
            "compression": {"originalTokens": 800, "compressedTokens": 600, "percentSaved": 25}
        })),
    ));
    let Some(ToolResultDetails::Read(details)) = tool_result.details else {
        panic!("expected Read details")
    };
    assert!(details.truncation.is_none());
    assert_eq!(details.path, Some(PathBuf::from("src/lib.rs")));
    assert_eq!(details.source, Some(ToolResultSource::LeanCtx));
    assert_eq!(details.mode.as_deref(), Some("full"));
    assert_eq!(details.lines, Some(123));
    assert!(details.compression.is_some());
}

#[test]
fn read_tool_result_accepts_pattern_caps() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "read",
        vec![json!({"type": "text", "text": "pattern-limited read"})],
        false,
        Some(json!({"matchLimitReached": 50, "linesTruncated": true})),
    ));
    let Some(ToolResultDetails::Read(details)) = tool_result.details else {
        panic!("expected Read details")
    };
    assert_eq!(details.match_limit_reached, Some(50));
    assert_eq!(details.lines_truncated, Some(true));
    assert!(details.path.is_none());
    assert!(details.compression.is_none());
}

#[test]
fn ask_user_tool_result_accepts_freeform_response() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "ask_user",
        vec![json!({"type": "text", "text": "User typed: hello"})],
        false,
        Some(json!({
            "question": "What's your favorite editor?",
            "context": "Tooling preference",
            "options": [{"title": "neovim"}, {"title": "emacs"}],
            "response": {"kind": "freeform", "text": "helix"},
            "cancelled": false
        })),
    ));
    let Some(ToolResultDetails::AskUser(details)) = tool_result.details else {
        panic!("expected AskUser details")
    };
    match details.response {
        Some(AskUserResponse::Freeform { text }) => assert_eq!(text, "helix"),
        other => panic!("expected freeform response, got {other:?}"),
    }
}

// `ToolResultMessage` routes known tool names explicitly, so enum ordering
// only matters when `ToolResultDetails` is deserialized directly by shape.
// This pins the Grep-vs-Read overlap on `{matchLimitReached, linesTruncated}`.
#[test]
fn bare_limit_shape_lands_in_grep_during_direct_details_deserialization() {
    let details: ToolResultDetails =
        serde_json::from_value(json!({"matchLimitReached": 50, "linesTruncated": true}))
            .expect("expected direct ToolResultDetails parse");
    assert!(matches!(details, ToolResultDetails::Grep(_)));
}

#[test]
fn ctx_cache_tool_call_keeps_declared_name() {
    let tool_call = parse_tool_call(
        "ctx_cache",
        json!({
            "action": "invalidate",
            "path": "crates/moriarty/src/api_pricing/analyzer_tests.rs"
        }),
    );

    assert_eq!(tool_call.name(), ToolName::CtxCache);
}

#[test]
fn git_read_only_tool_calls_keep_declared_names() {
    for (tool_name, expected) in [
        ("git_read_only_diff", ToolName::GitReadOnlyDiff),
        ("git_read_only_log", ToolName::GitReadOnlyLog),
        ("git_read_only_show", ToolName::GitReadOnlyShow),
        ("git_read_only_status", ToolName::GitReadOnlyStatus),
    ] {
        let tool_call = parse_tool_call(tool_name, json!({"project_dir": "/tmp/repo", "args": []}));
        assert_eq!(tool_call.name(), expected);
    }
}

#[test]
fn hermes_memory_tool_calls_keep_declared_names() {
    for (tool_name, expected) in [
        ("memory", ToolName::Memory),
        ("memory_search", ToolName::MemorySearch),
        ("session_search", ToolName::SessionSearch),
        ("skill", ToolName::Skill),
    ] {
        let tool_call = parse_tool_call(tool_name, json!({"query": "auth"}));
        assert_eq!(tool_call.name(), expected);
    }
}

#[test]
fn pi_lens_tool_calls_keep_declared_names() {
    for (tool_name, expected) in [
        ("ast_grep_search", ToolName::AstGrepSearch),
        ("ast_grep_replace", ToolName::AstGrepReplace),
        ("lsp_diagnostics", ToolName::LspDiagnostics),
        ("lsp_navigation", ToolName::LspNavigation),
    ] {
        let tool_call = parse_tool_call(tool_name, json!({"path": "src/main.rs"}));
        assert_eq!(tool_call.name(), expected);
    }
}

#[test]
fn git_read_only_tool_call_preserves_raw_snake_case_arguments() {
    let raw_arguments = json!({
        "project_dir": "/tmp/repo",
        "args": ["--oneline", "-n", "5"]
    });
    let tool_call = parse_tool_call("git_read_only_log", raw_arguments.clone());

    assert_eq!(tool_call.name(), ToolName::GitReadOnlyLog);
    assert_eq!(
        serde_json::to_value(&tool_call.arguments).expect("serialize raw arguments"),
        raw_arguments
    );
}

#[test]
fn custom_plannotator_accepts_snapshot_saved_state() {
    match parse_custom_payload(
        "plannotator",
        json!({
            "phase": "planning",
            "savedState": {
                "activeTools": ["read", "bash"],
                "model": {"provider": "anthropic", "id": "claude-opus-4-6"},
                "thinkingLevel": "medium"
            }
        }),
    ) {
        CustomPayload::Plannotator(details) => {
            let Some(PlannotatorSavedState::Snapshot(snapshot)) = details.saved_state else {
                panic!("expected snapshot saved_state")
            };
            assert_eq!(snapshot.active_tools, vec![ToolName::Read, ToolName::Bash]);
            assert_eq!(snapshot.model.provider, Provider::Anthropic);
            assert_eq!(snapshot.model.id, "claude-opus-4-6");
            assert_eq!(snapshot.thinking_level, ThinkingLevel::Medium);
        }
        other => panic!("expected Plannotator, got {other:?}"),
    }
}

#[test]
fn custom_plannotator_accepts_snapshot_saved_state_with_minimal_thinking() {
    match parse_custom_payload(
        "plannotator",
        json!({
            "phase": "planning",
            "savedState": {
                "activeTools": ["read"],
                "model": {"provider": "openai", "id": "gpt-5.5"},
                "thinkingLevel": "minimal"
            }
        }),
    ) {
        CustomPayload::Plannotator(details) => {
            let Some(PlannotatorSavedState::Snapshot(snapshot)) = details.saved_state else {
                panic!("expected snapshot saved_state")
            };
            assert_eq!(snapshot.active_tools, vec![ToolName::Read]);
            assert_eq!(snapshot.model.provider, Provider::OpenAi);
            assert_eq!(snapshot.model.id, "gpt-5.5");
            assert_eq!(snapshot.thinking_level, ThinkingLevel::Minimal);
        }
        other => panic!("expected Plannotator, got {other:?}"),
    }
}

#[test]
fn custom_plannotator_accepts_snapshot_saved_state_with_openrouter() {
    match parse_custom_payload(
        "plannotator",
        json!({
            "phase": "planning",
            "savedState": {
                "activeTools": ["read", "bash"],
                "model": {"provider": "openrouter", "id": "openai/gpt-5.4"},
                "thinkingLevel": "medium"
            }
        }),
    ) {
        CustomPayload::Plannotator(details) => {
            let Some(PlannotatorSavedState::Snapshot(snapshot)) = details.saved_state else {
                panic!("expected snapshot saved_state")
            };
            assert_eq!(snapshot.active_tools, vec![ToolName::Read, ToolName::Bash]);
            assert_eq!(snapshot.model.provider, Provider::OpenRouter);
            assert_eq!(snapshot.model.id, "openai/gpt-5.4");
            assert_eq!(snapshot.thinking_level, ThinkingLevel::Medium);
        }
        other => panic!("expected Plannotator, got {other:?}"),
    }
}

#[test]
fn custom_plannotator_accepts_snapshot_saved_state_with_pi_lens_tools() {
    match parse_custom_payload(
        "plannotator",
        json!({
            "phase": "planning",
            "savedState": {
                "activeTools": [
                    "read",
                    "ast_grep_search",
                    "ast_grep_replace",
                    "lsp_diagnostics",
                    "lsp_navigation"
                ],
                "model": {"provider": "anthropic", "id": "claude-sonnet-4-5"},
                "thinkingLevel": "high"
            }
        }),
    ) {
        CustomPayload::Plannotator(details) => {
            let Some(PlannotatorSavedState::Snapshot(snapshot)) = details.saved_state else {
                panic!("expected snapshot saved_state")
            };
            assert_eq!(
                snapshot.active_tools,
                vec![
                    ToolName::Read,
                    ToolName::AstGrepSearch,
                    ToolName::AstGrepReplace,
                    ToolName::LspDiagnostics,
                    ToolName::LspNavigation,
                ]
            );
            assert_eq!(snapshot.model.provider, Provider::Anthropic);
            assert_eq!(snapshot.model.id, "claude-sonnet-4-5");
            assert_eq!(snapshot.thinking_level, ThinkingLevel::High);
        }
        other => panic!("expected Plannotator, got {other:?}"),
    }
}

#[test]
fn custom_message_subagent_notify_has_no_details() {
    assert!(matches!(
        parse_custom_message_payload("Background task failed: timeout", "subagent-notify", None,),
        CustomMessagePayload::SubagentNotify
    ));
}

#[test]
fn custom_message_subagent_notify_rejects_details() {
    assert_parse_error_contains_any(
        "subagent-notify rejects details",
        custom_message_json(
            "Background task failed: timeout",
            "subagent-notify",
            Some(json!({"unexpected": true})),
        ),
        &[
            "unknown variant",
            "did not match any variant",
            "subagent-notify",
        ],
    );
}

#[test]
fn custom_message_intercom_message() {
    let payload = parse_custom_message_payload(
        "subagent needs attention",
        "intercom_message",
        Some(json!({
            "from": {
                "id": "subagent-control",
                "name": "subagent-control",
                "cwd": "/Users/brendan/src/hydrogen-cloud",
                "model": "subagent-control",
                "pid": 78321,
                "startedAt": 1778087846388_i64,
                "lastActivity": 1778087846388_i64,
                "status": "needs_attention"
            },
            "message": {
                "id": "0fde0e5d-9914-4cc0-b203-54dedc736a3c",
                "timestamp": 1778087846388_i64,
                "content": { "text": "subagent needs attention" }
            },
            "replyCommand": "intercom({ action: \"reply\", message: \"...\" })",
            "bodyText": "subagent needs attention"
        })),
    );

    let CustomMessagePayload::IntercomMessage(details) = payload else {
        panic!("expected IntercomMessage payload")
    };
    assert_eq!(
        details.from.0,
        json!({
            "id": "subagent-control",
            "name": "subagent-control",
            "cwd": "/Users/brendan/src/hydrogen-cloud",
            "model": "subagent-control",
            "pid": 78321,
            "startedAt": 1778087846388_i64,
            "lastActivity": 1778087846388_i64,
            "status": "needs_attention"
        })
    );
    assert_eq!(
        details.message.0,
        json!({
            "id": "0fde0e5d-9914-4cc0-b203-54dedc736a3c",
            "timestamp": 1778087846388_i64,
            "content": { "text": "subagent needs attention" }
        })
    );
    assert_eq!(
        details.reply_command.as_deref(),
        Some("intercom({ action: \"reply\", message: \"...\" })")
    );
    assert_eq!(details.body_text, "subagent needs attention");
}

#[test]
fn custom_message_subagent_control_notice_accepts_needs_attention_event() {
    match parse_custom_message_payload(
        "Subagent needs attention: documentation-reviewer",
        "subagent_control_notice",
        Some(json!({
            "event": {
                "type": "needs_attention",
                "from": "active_long_running",
                "to": "needs_attention",
                "ts": 1777921594147_u64,
                "runId": "8784581c",
                "agent": "documentation-reviewer",
                "index": 2,
                "message": "documentation-reviewer needs attention (no observed activity for 60s)",
                "reason": "idle",
                "turns": 12,
                "tokens": 71740,
                "toolCount": 54,
                "currentPath": "review-documentation-3.md",
                "elapsedMs": 60887
            },
            "source": "foreground",
            "childIntercomTarget": "subagent-documentation-reviewer-8784581c-3",
            "noticeText": "Subagent needs attention: documentation-reviewer"
        })),
    ) {
        CustomMessagePayload::SubagentControlNotice(details) => {
            assert_eq!(details.source, "foreground");
            assert_eq!(
                details.child_intercom_target.as_deref(),
                Some("subagent-documentation-reviewer-8784581c-3")
            );
            assert_eq!(
                details.notice_text,
                "Subagent needs attention: documentation-reviewer"
            );
            let SubagentControlEvent::NeedsAttention(event) = details.event else {
                panic!("expected needs_attention event")
            };
            assert_eq!(event.from.as_deref(), Some("active_long_running"));
            assert_eq!(event.to, "needs_attention");
            assert_eq!(event.ts, 1777921594147);
            assert_eq!(event.run_id, "8784581c");
            assert_eq!(event.agent, "documentation-reviewer");
            assert_eq!(event.index, 2);
            assert_eq!(
                event.message,
                "documentation-reviewer needs attention (no observed activity for 60s)"
            );
            assert_eq!(event.reason, "idle");
            assert_eq!(event.turns, 12);
            assert_eq!(event.tokens, 71740);
            assert_eq!(event.tool_count, 54);
            assert_eq!(
                event.current_path,
                Some(PathBuf::from("review-documentation-3.md"))
            );
            assert_eq!(event.elapsed_ms, 60887);
        }
        other => panic!("expected SubagentControlNotice, got {other:?}"),
    }
}

#[test]
fn custom_message_subagent_control_notice_accepts_active_long_running_event() {
    match parse_custom_message_payload(
        "Subagent is still active: code-quality-reviewer",
        "subagent_control_notice",
        Some(json!({
            "event": {
                "type": "active_long_running",
                "to": "active_long_running",
                "ts": 1777657840252_u64,
                "runId": "b48327c8",
                "agent": "code-quality-reviewer",
                "index": 0,
                "message": "code-quality-reviewer is still active but long-running",
                "reason": "turn_threshold",
                "turns": 15,
                "tokens": 121069,
                "toolCount": 44,
                "elapsedMs": 97198
            },
            "source": "foreground",
            "noticeText": "Subagent is still active: code-quality-reviewer"
        })),
    ) {
        CustomMessagePayload::SubagentControlNotice(details) => {
            assert_eq!(details.source, "foreground");
            assert_eq!(
                details.notice_text,
                "Subagent is still active: code-quality-reviewer"
            );
            let SubagentControlEvent::ActiveLongRunning(event) = details.event else {
                panic!("expected active_long_running event")
            };
            assert_eq!(event.from, None);
            assert_eq!(event.run_id, "b48327c8");
            assert_eq!(event.agent, "code-quality-reviewer");
            assert_eq!(event.reason, "turn_threshold");
        }
        other => panic!("expected SubagentControlNotice, got {other:?}"),
    }
}

#[test]
fn custom_message_subagent_control_notice_rejects_unknown_details_field() {
    assert_parse_error_contains_any(
        "rejects unknown subagent control notice field",
        custom_message_json(
            "Subagent needs attention: documentation-reviewer",
            "subagent_control_notice",
            Some(json!({
                "event": {
                    "type": "needs_attention",
                    "to": "needs_attention",
                    "ts": 1777921594147_u64,
                    "runId": "8784581c",
                    "agent": "documentation-reviewer",
                    "index": 2,
                    "message": "documentation-reviewer needs attention (no observed activity for 60s)",
                    "reason": "idle",
                    "turns": 12,
                    "tokens": 71740,
                    "toolCount": 54,
                    "elapsedMs": 60887
                },
                "source": "foreground",
                "noticeText": "Subagent needs attention: documentation-reviewer",
                "unexpected": true
            })),
        ),
        &["unexpected"],
    );
}

#[test]
fn custom_message_subagent_control_notice_requires_details() {
    assert_parse_error_contains_any(
        "subagent control notice requires details",
        custom_message_json(
            "Subagent needs attention: documentation-reviewer",
            "subagent_control_notice",
            None,
        ),
        &["details", "subagent_control_notice"],
    );
}

#[test]
fn custom_message_pi_loaded_tools_accepts_modeled_manifest_names() {
    let builtin_cases = [("read", ToolName::Read)];
    let intercom_cases = [("contact_supervisor", ToolName::ContactSupervisor)];
    let lean_ctx_cases = [
        ("ctx_agent", ToolName::CtxAgent),
        ("ctx_analyze", ToolName::CtxAnalyze),
        ("ctx_architecture", ToolName::CtxArchitecture),
        ("ctx_benchmark", ToolName::CtxBenchmark),
        ("ctx_cache", ToolName::CtxCache),
        ("ctx_callees", ToolName::CtxCallees),
        ("ctx_callers", ToolName::CtxCallers),
        ("ctx_compress", ToolName::CtxCompress),
        ("ctx_compress_memory", ToolName::CtxCompressMemory),
        ("ctx_context", ToolName::CtxContext),
        ("ctx_cost", ToolName::CtxCost),
        ("ctx_dedup", ToolName::CtxDedup),
        ("ctx_delta", ToolName::CtxDelta),
        ("ctx_discover", ToolName::CtxDiscover),
        ("ctx_edit", ToolName::CtxEdit),
        ("ctx_execute", ToolName::CtxExecute),
        ("ctx_expand", ToolName::CtxExpand),
        ("ctx_feedback", ToolName::CtxFeedback),
        ("ctx_fill", ToolName::CtxFill),
        ("ctx_gain", ToolName::CtxGain),
        ("ctx_graph", ToolName::CtxGraph),
        ("ctx_graph_diagram", ToolName::CtxGraphDiagram),
        ("ctx_handoff", ToolName::CtxHandoff),
        ("ctx_heatmap", ToolName::CtxHeatmap),
        ("ctx_impact", ToolName::CtxImpact),
        ("ctx_intent", ToolName::CtxIntent),
        ("ctx_knowledge", ToolName::CtxKnowledge),
        ("ctx_metrics", ToolName::CtxMetrics),
        ("ctx_outline", ToolName::CtxOutline),
        ("ctx_overview", ToolName::CtxOverview),
        ("ctx_prefetch", ToolName::CtxPrefetch),
        ("ctx_preload", ToolName::CtxPreload),
        ("ctx_response", ToolName::CtxResponse),
        ("ctx_routes", ToolName::CtxRoutes),
        ("ctx_semantic_search", ToolName::CtxSemanticSearch),
        ("ctx_session", ToolName::CtxSession),
        ("ctx_share", ToolName::CtxShare),
        ("ctx_smart_read", ToolName::CtxSmartRead),
        ("ctx_symbol", ToolName::CtxSymbol),
        ("ctx_task", ToolName::CtxTask),
        ("ctx_workflow", ToolName::CtxWorkflow),
        ("ctx_wrapped", ToolName::CtxWrapped),
    ];
    let git_cases = [
        ("git_read_only_diff", ToolName::GitReadOnlyDiff),
        ("git_read_only_log", ToolName::GitReadOnlyLog),
        ("git_read_only_show", ToolName::GitReadOnlyShow),
        ("git_read_only_status", ToolName::GitReadOnlyStatus),
    ];
    let hermes_cases = [
        ("memory", ToolName::Memory),
        ("memory_search", ToolName::MemorySearch),
        ("session_search", ToolName::SessionSearch),
        ("skill", ToolName::Skill),
    ];
    let pi_lens_cases = [
        ("ast_grep_search", ToolName::AstGrepSearch),
        ("ast_grep_replace", ToolName::AstGrepReplace),
        ("lsp_diagnostics", ToolName::LspDiagnostics),
        ("lsp_navigation", ToolName::LspNavigation),
    ];

    let mut tools = Vec::new();
    for (wire_name, _) in &builtin_cases {
        tools.push(json!({
            "name": wire_name,
            "description": "builtin tool",
            "active": true,
            "source": "builtin",
            "scope": "temporary",
            "origin": "top-level"
        }));
    }
    for (wire_name, _) in &intercom_cases {
        tools.push(json!({
            "name": wire_name,
            "description": "intercom tool",
            "active": true,
            "source": "extension",
            "scope": "user",
            "origin": "package",
            "extensionPath": "npm:pi-intercom@0.6.0"
        }));
    }
    for (wire_name, _) in &lean_ctx_cases {
        tools.push(json!({
            "name": wire_name,
            "description": "lean ctx tool",
            "active": true,
            "source": "extension",
            "scope": "user",
            "origin": "package",
            "extensionPath": "npm:pi-lean-ctx@3.3.6"
        }));
    }
    for (wire_name, _) in &git_cases {
        tools.push(json!({
            "name": wire_name,
            "description": "git tool",
            "active": true,
            "source": "extension",
            "scope": "user",
            "origin": "package",
            "extensionPath": "npm:pi-mcp-adapter@2.5.1"
        }));
    }
    for (wire_name, _) in &hermes_cases {
        tools.push(json!({
            "name": wire_name,
            "description": "hermes tool",
            "active": true,
            "source": "extension",
            "scope": "user",
            "origin": "package",
            "extensionPath": "npm:pi-hermes-memory@0.7.10"
        }));
    }
    for (wire_name, _) in &pi_lens_cases {
        tools.push(json!({
            "name": wire_name,
            "description": "pi lens tool",
            "active": true,
            "source": "extension",
            "scope": "user",
            "origin": "package",
            "extensionPath": "npm:pi-lens@3.8.44"
        }));
    }

    match parse_custom_message_payload(
        "Loaded tools",
        "pi-loaded-tools",
        Some(json!({"tools": tools})),
    ) {
        CustomMessagePayload::PiLoadedTools(details) => {
            for (index, (_, expected_name)) in builtin_cases.iter().enumerate() {
                let tool = &details.tools[index];
                assert_eq!(tool.name, *expected_name);
                assert_eq!(tool.source, ToolSource::Builtin);
                assert_eq!(tool.scope, ToolScope::Temporary);
                assert_eq!(tool.origin, ToolOrigin::TopLevel);
                assert!(tool.extension_path.is_none());
            }

            let mut index = builtin_cases.len();
            for (_, expected_name) in &intercom_cases {
                let tool = &details.tools[index];
                assert_eq!(tool.name, *expected_name);
                assert_eq!(tool.source, ToolSource::Extension);
                assert_eq!(tool.scope, ToolScope::User);
                assert_eq!(tool.origin, ToolOrigin::Package);
                assert_eq!(
                    tool.extension_path.as_deref(),
                    Some("npm:pi-intercom@0.6.0")
                );
                index += 1;
            }

            for (_, expected_name) in &lean_ctx_cases {
                let tool = &details.tools[index];
                assert_eq!(tool.name, *expected_name);
                assert_eq!(tool.source, ToolSource::Extension);
                assert_eq!(tool.scope, ToolScope::User);
                assert_eq!(tool.origin, ToolOrigin::Package);
                assert_eq!(
                    tool.extension_path.as_deref(),
                    Some("npm:pi-lean-ctx@3.3.6")
                );
                index += 1;
            }

            for (_, expected_name) in &git_cases {
                let tool = &details.tools[index];
                assert_eq!(tool.name, *expected_name);
                assert_eq!(tool.source, ToolSource::Extension);
                assert_eq!(tool.scope, ToolScope::User);
                assert_eq!(tool.origin, ToolOrigin::Package);
                assert_eq!(
                    tool.extension_path.as_deref(),
                    Some("npm:pi-mcp-adapter@2.5.1")
                );
                index += 1;
            }

            for (_, expected_name) in &hermes_cases {
                let tool = &details.tools[index];
                assert_eq!(tool.name, *expected_name);
                assert_eq!(tool.source, ToolSource::Extension);
                assert_eq!(tool.scope, ToolScope::User);
                assert_eq!(tool.origin, ToolOrigin::Package);
                assert_eq!(
                    tool.extension_path.as_deref(),
                    Some("npm:pi-hermes-memory@0.7.10")
                );
                index += 1;
            }

            for (_, expected_name) in &pi_lens_cases {
                let tool = &details.tools[index];
                assert_eq!(tool.name, *expected_name);
                assert_eq!(tool.source, ToolSource::Extension);
                assert_eq!(tool.scope, ToolScope::User);
                assert_eq!(tool.origin, ToolOrigin::Package);
                assert_eq!(tool.extension_path.as_deref(), Some("npm:pi-lens@3.8.44"));
                index += 1;
            }
        }
        other => panic!("expected PiLoadedTools, got {other:?}"),
    }
}

#[test]
fn custom_web_search_results_accepts_aborted_query_without_provider() {
    match parse_custom_payload(
        "web-search-results",
        json!({
            "id": "search_aborted",
            "timestamp": MESSAGE_TIMESTAMP,
            "type": "search",
            "queries": [{
                "query": "anything",
                "answer": "",
                "results": [],
                "error": "This operation was aborted"
            }]
        }),
    ) {
        CustomPayload::WebSearchResults(results) => {
            let WebSearchResultsPayload::Search(search) = results.payload else {
                panic!("expected Search payload")
            };
            assert!(search.queries[0].provider.is_none());
            assert_eq!(
                search.queries[0].error.as_deref(),
                Some("This operation was aborted")
            );
        }
        other => panic!("expected WebSearchResults, got {other:?}"),
    }
}

#[test]
fn assistant_thinking_without_signature_parses() {
    let content = parse_first_assistant_content(
        json!({"type": "thinking", "thinking": "Hmm..."}),
        AssistantFixture::new(
            "anthropic-messages",
            "anthropic",
            "claude-sonnet-4-5",
            "aborted",
        ),
    );
    let AssistantContentItem::Thinking(ThinkingAssistantContent {
        thinking,
        thinking_signature,
    }) = content
    else {
        panic!("expected Thinking content")
    };
    assert_eq!(thinking, "Hmm...");
    assert!(thinking_signature.is_none());
}

/// Grep also accepts the lean-ctx augmented shape
/// `{path, pattern, source, compression}` when the surrounding tool result
/// is routed by `toolName: "grep"`.
#[test]
fn grep_tool_result_accepts_full_lean_ctx_augmentation() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "grep",
        vec![json!({"type": "text", "text": "hits"})],
        false,
        Some(json!({
            "path": "crates",
            "pattern": "fn parse",
            "source": "lean-ctx",
            "compression": {
                "originalTokens": 4000,
                "compressedTokens": 1000,
                "percentSaved": 75
            }
        })),
    ));
    let Some(ToolResultDetails::Grep(details)) = tool_result.details else {
        panic!("expected Grep details")
    };
    assert_eq!(details.path, Some(PathBuf::from("crates")));
    assert_eq!(details.pattern.as_deref(), Some("fn parse"));
    assert_eq!(details.source, Some(ToolResultSource::LeanCtx));
    let compression = details.compression.expect("expected compression");
    assert_eq!(compression.original_tokens, 4000);
    assert_eq!(compression.compressed_tokens, 1000);
    assert_eq!(compression.percent_saved, 75);
}

/// ThinkingLevel::Off is a real wire value (`"off"`). High, Medium, Minimal,
/// and Low already have coverage; this pins the fifth arm so a typo in the
/// rename (e.g. `"none"`/`"disabled"`) fails noisily.
#[test]
fn thinking_level_change_off() {
    let line = parse(thinking_level_change_json("m1", "off"));
    match line {
        PiLogLine::ThinkingLevelChange(thinking_level) => {
            assert_eq!(thinking_level.thinking_level, ThinkingLevel::Off);
        }
        other => panic!("expected ThinkingLevelChange, got {other:?}"),
    }
}

/// Ls and Find overlap on the lean-ctx fields except for Find's optional
/// `pattern`. Direct `ToolResultDetails` parsing therefore depends on enum
/// order to keep a payload without `pattern` landing in `Ls`.
#[test]
fn ls_shaped_payload_lands_in_ls_during_direct_details_deserialization() {
    let details: ToolResultDetails = serde_json::from_value(json!({
        "path": "crates",
        "source": "lean-ctx",
        "truncated": false,
        "compression": {
            "originalTokens": 50,
            "compressedTokens": 25,
            "percentSaved": 50
        }
    }))
    .expect("expected direct ToolResultDetails parse");
    assert!(matches!(details, ToolResultDetails::Ls(_)));
}

#[test]
fn find_truncation_payload_lands_in_find_during_direct_details_deserialization() {
    let details: ToolResultDetails = serde_json::from_value(json!({
        "resultLimitReached": 500,
        "truncation": {
            "content": "sessions/...",
            "truncated": true,
            "truncatedBy": "bytes",
            "totalLines": 500,
            "totalBytes": 59789,
            "outputLines": 427,
            "outputBytes": 51147,
            "lastLinePartial": false,
            "firstLineExceedsLimit": false,
            "maxLines": 9007199254740991u64,
            "maxBytes": 51200
        }
    }))
    .expect("expected direct ToolResultDetails parse");
    assert!(matches!(details, ToolResultDetails::Find(_)));
}

/// McpDetails carries strict `deny_unknown_fields`, so a silent rename
/// of `servers` / `connectedCount` / `totalTools` would leave callers parsing
/// status responses with empty data. This pins all three plus the
/// McpServerStatus shape.
#[test]
fn mcp_tool_result_accepts_status_mode() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "mcp",
        vec![json!({"type": "text", "text": "status"})],
        false,
        Some(json!({
            "mode": "status",
            "servers": [
                {"name": "git-read-only", "status": "connected", "toolCount": 4},
                {"name": "flaky", "status": "failed", "toolCount": 0, "failedAgo": 12}
            ],
            "totalTools": 4,
            "connectedCount": 1
        })),
    ));
    let Some(ToolResultDetails::Mcp(details)) = tool_result.details else {
        panic!("expected Mcp details")
    };
    assert_eq!(details.mode, McpMode::Status);
    assert_eq!(details.total_tools, Some(4));
    assert_eq!(details.connected_count, Some(1));
    let servers = details.servers.expect("expected servers");
    assert_eq!(servers.len(), 2);
    assert_eq!(servers[0].name, "git-read-only");
    assert_eq!(servers[0].status, "connected");
    assert_eq!(servers[0].tool_count, 4);
    assert!(servers[0].failed_ago.is_none());
    assert_eq!(servers[1].name, "flaky");
    assert_eq!(servers[1].status, "failed");
    assert_eq!(servers[1].tool_count, 0);
    assert_eq!(servers[1].failed_ago, Some(12));
}

/// `mode: "list"` populates `tools` and `count` instead
/// of the status fields. A field rename or a discriminator swap on either
/// would silently leave callers parsing list responses with `None` while
/// the data was on the wire.
#[test]
fn mcp_tool_result_accepts_list_mode() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "mcp",
        vec![json!({"type": "text", "text": "list"})],
        false,
        Some(json!({
            "mode": "list",
            "server": "git-read-only",
            "tools": ["status", "diff", "log", "show"],
            "count": 4
        })),
    ));
    let Some(ToolResultDetails::Mcp(details)) = tool_result.details else {
        panic!("expected Mcp details")
    };
    assert_eq!(details.mode, McpMode::List);
    assert_eq!(details.server.as_deref(), Some("git-read-only"));
    assert_eq!(
        details.tools,
        Some(vec![
            "status".to_string(),
            "diff".to_string(),
            "log".to_string(),
            "show".to_string()
        ])
    );
    assert_eq!(details.count, Some(4));
    assert!(details.servers.is_none());
}

/// `mode: "call"` errors of kind `tool_not_found`
/// surface the missing tool name in `requested_tool`. Newer logs can also
/// attach `hintServer` to point the caller at the right server namespace.
#[test]
fn mcp_tool_result_accepts_tool_not_found_error() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "mcp",
        vec![json!({"type": "text", "text": "tool not found"})],
        true,
        Some(json!({
            "mode": "call",
            "server": "git-read-only",
            "tool": "rebase",
            "error": "tool_not_found",
            "message": "Server 'git-read-only' does not expose tool 'rebase'",
            "requestedTool": "rebase",
            "hintServer": "git-read-only"
        })),
    ));
    let Some(ToolResultDetails::Mcp(details)) = tool_result.details else {
        panic!("expected Mcp details")
    };
    assert_eq!(details.mode, McpMode::Call);
    assert_eq!(details.server.as_deref(), Some("git-read-only"));
    assert_eq!(details.tool.as_ref().map(McpTool::name), Some("rebase"));
    assert_eq!(details.error.as_deref(), Some("tool_not_found"));
    assert_eq!(details.requested_tool.as_deref(), Some("rebase"));
    assert_eq!(details.hint_server.as_deref(), Some("git-read-only"));
    // This fixture models the tool-not-found shape, which should not carry a
    // nested call result payload.
    assert!(details.mcp_result.is_none());
}

#[test]
fn mcp_details_serialize_hint_server_as_camel_case() {
    let value = serde_json::to_value(McpDetails {
        mode: McpMode::Call,
        mcp_result: None,
        server: Some("git-read-only".to_string()),
        tool: Some(McpTool::Name("rebase".to_string())),
        error: Some("tool_not_found".to_string()),
        message: Some("missing tool".to_string()),
        requested_tool: Some("rebase".to_string()),
        hint_server: Some("project-tools".to_string()),
        servers: None,
        total_tools: None,
        connected_count: None,
        tools: None,
        count: None,
    })
    .expect("serialize mcp details");

    assert_eq!(value.get("hintServer"), Some(&Value::from("project-tools")));
    assert!(value.get("hint_server").is_none());
}

/// Pins the routed async-only fields on subagent results.
#[test]
fn subagent_tool_result_accepts_async_progress() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "subagent",
        vec![json!({"type": "text", "text": "queued"})],
        false,
        Some(json!({
            "mode": "async",
            "results": [{"agent": "scout"}],
            "asyncId": "run_42",
            "asyncDir": "/tmp/scout-run",
            "progress": [{
                "index": 0,
                "agent": "scout",
                "status": "running",
                "task": "inspect",
                "toolCount": 3,
                "tokens": 1024,
                "durationMs": 500,
                "recentTools": ["read", "grep"],
                "recentOutput": ["matches found"]
            }]
        })),
    ));
    let Some(ToolResultDetails::Subagent(details)) = tool_result.details else {
        panic!("expected Subagent details")
    };
    assert_eq!(details.mode, SubagentResultMode::Async);
    assert_eq!(details.async_id.as_deref(), Some("run_42"));
    assert_eq!(details.async_dir, Some(PathBuf::from("/tmp/scout-run")));
    let progress = details.progress.expect("expected progress entries");
    assert_eq!(progress.len(), 1);
    assert_eq!(progress[0].agent, "scout");
    assert_eq!(progress[0].status, "running");
    assert_eq!(progress[0].tool_count, 3);
    assert_eq!(progress[0].tokens, 1024);
    assert_eq!(progress[0].duration_ms, 500);
    assert_eq!(progress[0].recent_tools, vec!["read", "grep"]);
    assert_eq!(progress[0].recent_output, vec!["matches found"]);
    assert_eq!(progress[0].task, "inspect");
    assert_eq!(progress[0].index, 0);
}

/// `parent_session` is the Rust field, but pi writes the camelCase wire key
/// `parentSession`. This test pins that rename mapping in a successful parse.
#[test]
fn session_line_with_parent_session() {
    let line = parse(json!({
        "type": "session",
        "version": 1,
        "id": SESSION_ID,
        "timestamp": FIXED_TIMESTAMP,
        "cwd": "/home/brendan/src/moriarty",
        "parentSession": "/home/brendan/.flk/sessions/parent.jsonl"
    }));
    match line {
        PiLogLine::Session(session) => {
            assert_eq!(
                session.parent_session,
                Some(PathBuf::from("/home/brendan/.flk/sessions/parent.jsonl"))
            );
        }
        other => panic!("expected Session, got {other:?}"),
    }
}

#[test]
fn session_line_serializes_parent_session_as_camel_case() {
    let value = serde_json::to_value(SessionLine {
        version: 1,
        id: uuid::Uuid::parse_str(SESSION_ID).expect("valid session id"),
        timestamp: FIXED_TIMESTAMP.parse().expect("valid timestamp"),
        cwd: PathBuf::from("/home/brendan/src/moriarty"),
        parent_session: Some(PathBuf::from("/home/brendan/.flk/sessions/parent.jsonl")),
    })
    .expect("serialize session line");

    assert_eq!(
        value.get("parentSession"),
        Some(&Value::from("/home/brendan/.flk/sessions/parent.jsonl"))
    );
    assert!(value.get("parent_session").is_none());
}

// Pins the strict outer envelope on ToolCallContent: raw `arguments` do not
// relax `deny_unknown_fields` for sibling keys like `extraUnknown`.
#[test]
fn tool_call_content_rejects_unknown_top_level_field() {
    let line = assistant_message_json(
        vec![json!({
            "type": "toolCall",
            "id": "call_1",
            "name": "bash",
            "arguments": {"command": "ls"},
            "extraUnknown": "should be rejected"
        })],
        AssistantFixture::new("openai-responses", "openai", "gpt-5.4", "toolUse"),
    );
    let err = parse_err(line);
    let msg = err.to_string();
    assert!(
        msg.contains("extraUnknown"),
        "expected parse error to mention extraUnknown, got: {msg}"
    );
}

/// Pins each assistant usage field so same-typed token and cost fields cannot
/// silently swap meanings under serde rename drift.
#[test]
fn assistant_usage_preserves_field_meaning() {
    let assistant = parse_assistant_message(
        vec![json!({"type": "text", "text": "reply"})],
        AssistantFixture::new(
            "anthropic-messages",
            "anthropic",
            "claude-sonnet-4-5",
            "stop",
        ),
    );
    let usage = &assistant.usage;
    assert_eq!(usage.input, 10);
    assert_eq!(usage.output, 5);
    assert_eq!(usage.cache_read, 0);
    assert_eq!(usage.cache_write, 0);
    assert_eq!(usage.total_tokens, 15);
    // Comparing via `to_string()` keeps the test free of a fresh `FromStr`
    // import while still pinning each cost component to its exact wire repr,
    // including the trailing zeros on the zero-cost fields.
    assert_eq!(usage.cost.input.to_string(), "0.00003");
    assert_eq!(usage.cost.output.to_string(), "0.000075");
    assert_eq!(usage.cost.cache_read.to_string(), "0");
    assert_eq!(usage.cost.cache_write.to_string(), "0");
    assert_eq!(usage.cost.total.to_string(), "0.000105");
}

/// The cancelled path through `ask_user` omits `response` entirely
/// while setting `cancelled: true`. Without an explicit test, a regression
/// that swallowed the `cancelled` flag (or made `response` required) would
/// only be caught by users hitting the cancellation path in the wild.
#[test]
fn ask_user_tool_result_accepts_cancelled() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "ask_user",
        vec![json!({"type": "text", "text": "User cancelled"})],
        false,
        Some(json!({
            "question": "Continue?",
            "options": [{"title": "Yes"}, {"title": "No"}],
            "cancelled": true
        })),
    ));
    let Some(ToolResultDetails::AskUser(details)) = tool_result.details else {
        panic!("expected AskUser details")
    };
    assert!(details.cancelled);
    assert!(details.response.is_none());
    assert!(details.context.is_none());
    assert_eq!(details.options.len(), 2);
}

#[test]
fn memory_search_tool_result_accepts_search_summary() {
    for (tool_name, output) in [
        ("memory_search", None),
        ("session_search", Some("session-123")),
    ] {
        let tool_result = parse_tool_result_message(tool_result_message_json(
            tool_name,
            vec![json!({"type": "text", "text": "no results"})],
            false,
            Some(json!({
                "success": true,
                "count": 0,
                "message": "No results found.",
                "output": output
            })),
        ));
        let Some(ToolResultDetails::SearchResult(details)) = tool_result.details else {
            panic!("expected SearchResult details for {tool_name}")
        };
        assert!(details.success);
        assert_eq!(details.count, Some(0));
        assert_eq!(details.message.as_deref(), Some("No results found."));
        assert_eq!(details.output.as_deref(), output);
    }
}

#[test]
fn direct_tool_result_details_bare_error_stays_get_search_content() {
    let details = serde_json::from_value::<ToolResultDetails>(json!({
        "error": "URL not found"
    }))
    .expect("expected bare error payload to deserialize");

    let ToolResultDetails::GetSearchContent(GetSearchContentDetails::Error(error)) = details else {
        panic!("expected GetSearchContent error details")
    };
    assert_eq!(error.error, "URL not found");
}

#[test]
fn memory_tool_result_accepts_hermes_result_details() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "memory",
        vec![json!({"type": "text", "text": "Entry added."})],
        false,
        Some(json!({
            "success": true,
            "target": "user",
            "entries": ["Remember this."],
            "usage": "6% — 303/5000 chars",
            "entry_count": 1,
            "message": "Entry added."
        })),
    ));
    let Some(ToolResultDetails::Memory(details)) = tool_result.details else {
        panic!("expected Memory details")
    };
    assert_eq!(details.success, Some(true));
    assert_eq!(details.target.as_deref(), Some("user"));
    assert_eq!(details.entry_count, Some(1));
    assert_eq!(details.message.as_deref(), Some("Entry added."));
}

#[test]
fn memory_tool_result_routes_bare_error_to_memory_details() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "memory",
        vec![json!({"type": "text", "text": "memory failed"})],
        true,
        Some(json!({
            "error": "Content is required."
        })),
    ));
    let Some(ToolResultDetails::Memory(details)) = tool_result.details else {
        panic!("expected Memory details")
    };
    assert_eq!(details.error.as_deref(), Some("Content is required."));
}

#[test]
fn memory_tool_result_accepts_empty_details() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "memory",
        vec![json!({"type": "text", "text": "Content is required."})],
        false,
        Some(json!({})),
    ));
    assert!(matches!(
        tool_result.details,
        Some(ToolResultDetails::Empty(_))
    ));
}

#[test]
fn memory_tool_result_preserves_empty_error_details() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "memory",
        vec![json!({"type": "text", "text": "Content is required."})],
        true,
        Some(json!({})),
    ));
    assert!(matches!(
        tool_result.details,
        Some(ToolResultDetails::Empty(_))
    ));
}

#[test]
fn skill_tool_result_accepts_skill_index_details() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "skill",
        vec![json!({"type": "text", "text": "{\"success\":true,\"skills\":[]}"})],
        false,
        Some(json!({
            "skills": [
                {
                    "skillId": "global:debug-typescript-errors",
                    "scope": "global",
                    "fileName": "SKILL.md",
                    "path": "/tmp/skills/debug-typescript-errors/SKILL.md",
                    "name": "debug-typescript-errors",
                    "description": "Use when TypeScript errors need debugging"
                }
            ]
        })),
    ));
    let Some(ToolResultDetails::Skill(details)) = tool_result.details else {
        panic!("expected Skill details")
    };
    let skills = details.skills.expect("expected skills");
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].skill_id, "global:debug-typescript-errors");
    assert_eq!(skills[0].scope, "global");
    assert_eq!(skills[0].name, "debug-typescript-errors");
}

#[test]
fn skill_tool_result_routes_bare_error_to_skill_details() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "skill",
        vec![json!({"type": "text", "text": "skill failed"})],
        true,
        Some(json!({
            "error": "skill_id is required."
        })),
    ));
    let Some(ToolResultDetails::Skill(details)) = tool_result.details else {
        panic!("expected Skill details")
    };
    assert_eq!(details.error.as_deref(), Some("skill_id is required."));
}

#[test]
fn skill_tool_result_accepts_document_details() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "skill",
        vec![json!({"type": "text", "text": "skill document"})],
        false,
        Some(json!({
            "success": true,
            "skillId": "project:moriarty:parse-pi-sessions",
            "scope": "project",
            "fileName": "SKILL.md",
            "path": "/tmp/skills/parse-pi-sessions/SKILL.md",
            "projectName": "moriarty",
            "name": "parse-pi-sessions",
            "displayName": "Parse Pi Sessions",
            "description": "Use when replaying pi session fixtures",
            "body": "## When to Use\nWhen parsing pi sessions.",
            "version": 3,
            "created": "2026-05-19T00:00:00.000Z",
            "updated": "2026-05-19T01:00:00.000Z"
        })),
    ));
    let Some(ToolResultDetails::Skill(details)) = tool_result.details else {
        panic!("expected Skill details")
    };
    assert_eq!(details.success, Some(true));
    assert_eq!(
        details.skill_id.as_deref(),
        Some("project:moriarty:parse-pi-sessions")
    );
    assert_eq!(details.file_name.as_deref(), Some("SKILL.md"));
    assert_eq!(details.project_name.as_deref(), Some("moriarty"));
    assert_eq!(details.version, Some(3));
    assert_eq!(
        details.body.as_deref(),
        Some("## When to Use\nWhen parsing pi sessions.")
    );
}

#[test]
fn skill_tool_result_accepts_conflict_error_details() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "skill",
        vec![json!({"type": "text", "text": "conflict"})],
        true,
        Some(json!({
            "success": false,
            "error": "Similar skill already exists.",
            "skillId": "global:parse-pi-sessions",
            "scope": "global",
            "fileName": "SKILL.md",
            "path": "/tmp/skills/parse-pi-sessions/SKILL.md",
            "conflictType": "similar-name",
            "similarSkillIds": ["global:parse-pi-sessions-v2"],
            "suggestedAction": "patch"
        })),
    ));
    let Some(ToolResultDetails::Skill(details)) = tool_result.details else {
        panic!("expected Skill details")
    };
    assert_eq!(details.success, Some(false));
    assert_eq!(
        details.error.as_deref(),
        Some("Similar skill already exists.")
    );
    assert_eq!(details.conflict_type.as_deref(), Some("similar-name"));
    assert_eq!(
        details.similar_skill_ids,
        Some(vec!["global:parse-pi-sessions-v2".to_string()])
    );
    assert_eq!(details.suggested_action.as_deref(), Some("patch"));
}

#[test]
fn skill_tool_result_accepts_empty_details() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "skill",
        vec![json!({"type": "text", "text": "skill_id is required."})],
        false,
        Some(json!({})),
    ));
    assert!(matches!(
        tool_result.details,
        Some(ToolResultDetails::Empty(_))
    ));
}

#[test]
fn skill_tool_result_preserves_empty_error_details() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "skill",
        vec![json!({"type": "text", "text": "skill_id is required."})],
        true,
        Some(json!({})),
    ));
    assert!(matches!(
        tool_result.details,
        Some(ToolResultDetails::Empty(_))
    ));
}

#[test]
fn subagent_tool_result_accepts_all_mode_values() {
    for (wire_mode, expected_mode) in [
        ("async", SubagentResultMode::Async),
        ("management", SubagentResultMode::Management),
        ("parallel", SubagentResultMode::Parallel),
        ("single", SubagentResultMode::Single),
    ] {
        let tool_result = parse_tool_result_message(tool_result_message_json(
            "subagent",
            vec![json!({"type": "text", "text": "finished"})],
            false,
            Some(json!({
                "mode": wire_mode,
                "results": [{"agent": "code-quality-reviewer"}]
            })),
        ));
        let Some(ToolResultDetails::Subagent(details)) = tool_result.details else {
            panic!("expected Subagent details for mode {wire_mode}")
        };
        assert_eq!(details.mode, expected_mode);
    }
}

/// Parallel subagent runs populate the closed `mode` enum and can return
/// more than one result summary.
#[test]
fn subagent_tool_result_accepts_parallel_mode() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "subagent",
        vec![json!({"type": "text", "text": "parallel run complete"})],
        false,
        Some(json!({
            "mode": "parallel",
            "results": [
                {"agent": "alpha"},
                {"agent": "beta"}
            ]
        })),
    ));
    let Some(ToolResultDetails::Subagent(details)) = tool_result.details else {
        panic!("expected Subagent details")
    };
    assert_eq!(details.mode, SubagentResultMode::Parallel);
    assert_eq!(details.results.len(), 2);
    assert_eq!(details.results[0].agent.as_deref(), Some("alpha"));
    assert_eq!(details.results[1].agent.as_deref(), Some("beta"));
}

#[test]
fn subagent_tool_result_rejects_unknown_mode() {
    let err = parse_err(tool_result_message_json(
        "subagent",
        vec![json!({"type": "text", "text": "unknown mode"})],
        false,
        Some(json!({
            "mode": "queued",
            "results": [{"agent": "scout"}]
        })),
    ));
    let msg = err.to_string();
    assert!(
        msg.contains("queued"),
        "expected parse error to mention queued, got: {msg}"
    );
}

/// `instinct_write` emits a small `details` payload that the parser must route
/// to its own variant rather than letting the untagged dispatch fall through
/// to a different `Details` shape. Pinning the closed enum on `action` also
/// guards against silent drops if pi adds a new outcome string upstream.
#[test]
fn instinct_write_tool_result_routes_to_dedicated_details_variant() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "instinct_write",
        vec![json!({
            "type": "text",
            "text": "Created instinct: pulumi-vitest-unhandled-rejection-cascade"
        })],
        false,
        Some(json!({
            "id": "pulumi-vitest-unhandled-rejection-cascade",
            "action": "created"
        })),
    ));
    let Some(ToolResultDetails::InstinctWrite(details)) = tool_result.details else {
        panic!("expected InstinctWrite details")
    };
    assert_eq!(details.id, "pulumi-vitest-unhandled-rejection-cascade");
    assert_eq!(details.action, InstinctWriteAction::Created);
}

/// Companion to `instinct_write_tool_result_routes_to_dedicated_details_variant`:
/// the closed `InstinctWriteAction` enum has two arms (`Created`, `Updated`),
/// and a positive test that only exercises `"created"` would not catch the
/// accidental removal or rename of the `Updated` arm.
#[test]
fn instinct_write_tool_result_accepts_updated_action() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "instinct_write",
        vec![json!({
            "type": "text",
            "text": "Updated instinct: pulumi-vitest-unhandled-rejection-cascade"
        })],
        false,
        Some(json!({
            "id": "pulumi-vitest-unhandled-rejection-cascade",
            "action": "updated"
        })),
    ));
    let Some(ToolResultDetails::InstinctWrite(details)) = tool_result.details else {
        panic!("expected InstinctWrite details")
    };
    assert_eq!(details.action, InstinctWriteAction::Updated);
}

/// Pins the closed-enum claim: an unknown `action` string must fail loudly
/// rather than be silently absorbed by a relaxed `String` fallback.
#[test]
fn instinct_write_tool_result_rejects_unknown_action() {
    assert_parse_error_contains_any(
        "rejects unknown instinct_write action",
        tool_result_message_json(
            "instinct_write",
            vec![json!({"type": "text", "text": "Unknown instinct_write action"})],
            false,
            Some(json!({
                "id": "pulumi-vitest-unhandled-rejection-cascade",
                "action": "unchanged"
            })),
        ),
        &["did not match any variant", "unknown variant", "unchanged"],
    );
}

/// Pins `deny_unknown_fields` on `InstinctWriteDetails`. Without this test
/// a regression that loosened the struct (or the untagged dispatch silently
/// re-routing the payload) could pass unnoticed.
#[test]
fn instinct_write_tool_result_rejects_unknown_detail_field() {
    assert_parse_error_contains_any(
        "rejects unknown instinct_write detail field",
        tool_result_message_json(
            "instinct_write",
            vec![json!({"type": "text", "text": "Created instinct"})],
            false,
            Some(json!({
                "id": "pulumi-vitest-unhandled-rejection-cascade",
                "action": "created",
                "scope": "user"
            })),
        ),
        &["did not match any variant", "unknown field", "scope"],
    );
}

/// Pins the camelCase `outputReference` field name on subagent tool results.
#[test]
fn subagent_tool_result_accepts_output_reference() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "subagent",
        vec![json!({"type": "text", "text": "saved output"})],
        false,
        Some(json!({
            "mode": "parallel",
            "runId": "e6e4aed9",
            "results": [{
                "agent": "code-quality-reviewer",
                "outputMode": "inline",
                "outputReference": {
                    "path": "/tmp/review.md",
                    "bytes": 3179,
                    "lines": 62,
                    "message": "Output saved to: /tmp/review.md"
                }
            }]
        })),
    ));
    let Some(ToolResultDetails::Subagent(details)) = tool_result.details else {
        panic!("expected Subagent details")
    };
    assert_eq!(details.run_id.as_deref(), Some("e6e4aed9"));
    let summary = &details.results[0];
    assert_eq!(summary.output_mode.as_deref(), Some("inline"));
    assert_eq!(
        summary.output_reference,
        Some(SubagentOutputReference {
            path: PathBuf::from("/tmp/review.md"),
            bytes: 3179,
            lines: 62,
            message: "Output saved to: /tmp/review.md".to_string(),
        })
    );
}

#[test]
fn subagent_tool_result_accepts_active_long_running_control_event() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "subagent",
        vec![json!({"type": "text", "text": "3/3 succeeded"})],
        false,
        Some(json!({
            "mode": "parallel",
            "results": [{
                "agent": "code-quality-reviewer",
                "controlEvents": [{
                    "type": "active_long_running",
                    "to": "active_long_running",
                    "ts": 1777657840252_u64,
                    "runId": "b48327c8",
                    "agent": "code-quality-reviewer",
                    "index": 0,
                    "message": "code-quality-reviewer is still active but long-running",
                    "reason": "turn_threshold",
                    "turns": 15,
                    "tokens": 121069,
                    "toolCount": 44,
                    "currentTool": "read",
                    "currentToolDurationMs": 1500,
                    "currentPath": "charts/temporal/values.yaml",
                    "elapsedMs": 97198
                }]
            }]
        })),
    ));
    let Some(ToolResultDetails::Subagent(details)) = tool_result.details else {
        panic!("expected Subagent details")
    };
    let events = details.results[0]
        .control_events
        .as_ref()
        .expect("expected controlEvents");
    assert_eq!(events.len(), 1);
    let SubagentControlEvent::ActiveLongRunning(event) = &events[0] else {
        panic!("expected active_long_running event")
    };
    assert_eq!(event.from, None);
    assert_eq!(event.to, "active_long_running");
    assert_eq!(event.ts, 1777657840252);
    assert_eq!(event.run_id, "b48327c8");
    assert_eq!(event.agent, "code-quality-reviewer");
    assert_eq!(event.index, 0);
    assert_eq!(
        event.message,
        "code-quality-reviewer is still active but long-running"
    );
    assert_eq!(event.reason, "turn_threshold");
    assert_eq!(event.turns, 15);
    assert_eq!(event.tokens, 121069);
    assert_eq!(event.tool_count, 44);
    assert_eq!(event.current_tool.as_deref(), Some("read"));
    assert_eq!(event.current_tool_duration_ms, Some(1500));
    assert_eq!(
        event.current_path,
        Some(PathBuf::from("charts/temporal/values.yaml"))
    );
    assert_eq!(event.elapsed_ms, 97198);
}

#[test]
fn subagent_tool_result_accepts_needs_attention_control_event() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "subagent",
        vec![json!({"type": "text", "text": "needs attention"})],
        false,
        Some(json!({
            "mode": "parallel",
            "results": [{
                "agent": "documentation-reviewer",
                "controlEvents": [{
                    "type": "needs_attention",
                    "from": "active_long_running",
                    "to": "needs_attention",
                    "ts": 1777921594147_u64,
                    "runId": "8784581c",
                    "agent": "documentation-reviewer",
                    "index": 2,
                    "message": "documentation-reviewer needs attention (no observed activity for 60s)",
                    "reason": "idle",
                    "turns": 12,
                    "tokens": 71740,
                    "toolCount": 54,
                    "currentTool": "intercom",
                    "currentToolDurationMs": 60617,
                    "currentPath": "/Users/brendan/src/hydrogen-cloud/review-documentation-3.md",
                    "elapsedMs": 60887
                }]
            }]
        })),
    ));
    let Some(ToolResultDetails::Subagent(details)) = tool_result.details else {
        panic!("expected Subagent details")
    };
    let events = details.results[0]
        .control_events
        .as_ref()
        .expect("expected controlEvents");
    assert_eq!(events.len(), 1);
    let SubagentControlEvent::NeedsAttention(event) = &events[0] else {
        panic!("expected needs_attention event")
    };
    assert_eq!(event.from.as_deref(), Some("active_long_running"));
    assert_eq!(event.to, "needs_attention");
    assert_eq!(event.ts, 1777921594147);
    assert_eq!(event.run_id, "8784581c");
    assert_eq!(event.agent, "documentation-reviewer");
    assert_eq!(event.index, 2);
    assert_eq!(
        event.message,
        "documentation-reviewer needs attention (no observed activity for 60s)"
    );
    assert_eq!(event.reason, "idle");
    assert_eq!(event.turns, 12);
    assert_eq!(event.tokens, 71740);
    assert_eq!(event.tool_count, 54);
    assert_eq!(event.current_tool.as_deref(), Some("intercom"));
    assert_eq!(event.current_tool_duration_ms, Some(60617));
    assert_eq!(
        event.current_path,
        Some(PathBuf::from(
            "/Users/brendan/src/hydrogen-cloud/review-documentation-3.md"
        ))
    );
    assert_eq!(event.elapsed_ms, 60887);
}

#[test]
fn subagent_result_summary_serializes_control_events_as_camel_case() {
    let value = serde_json::to_value(SubagentResultSummary {
        agent: Some("code-quality-reviewer".to_string()),
        task: None,
        response: None,
        exit_code: None,
        usage: None,
        model: None,
        artifact_paths: None,
        error: None,
        progress_summary: None,
        final_output: None,
        saved_output_path: None,
        output_mode: None,
        output_reference: None,
        attempted_models: None,
        model_attempts: None,
        session_file: None,
        tool_calls: None,
        control_events: Some(vec![SubagentControlEvent::ActiveLongRunning(
            SubagentControlEventPayload {
                to: "active_long_running".to_string(),
                from: None,
                ts: 1777657840252,
                run_id: "b48327c8".to_string(),
                agent: "code-quality-reviewer".to_string(),
                index: 0,
                message: "still active".to_string(),
                reason: "turn_threshold".to_string(),
                turns: 15,
                tokens: 121069,
                tool_count: 44,
                current_tool: None,
                current_tool_duration_ms: None,
                current_path: Some(PathBuf::from("charts/temporal/values.yaml")),
                elapsed_ms: 97198,
            },
        )]),
    })
    .expect("serialize subagent result summary");

    let events = value
        .get("controlEvents")
        .and_then(Value::as_array)
        .expect("expected controlEvents array");
    assert_eq!(events[0].get("runId"), Some(&Value::from("b48327c8")));
    assert!(events[0].get("from").is_none());
    assert_eq!(events[0].get("toolCount"), Some(&Value::from(44)));
    assert_eq!(
        events[0].get("currentPath"),
        Some(&Value::from("charts/temporal/values.yaml"))
    );
    assert!(value.get("control_events").is_none());
}

/// Pins strict rejection of extra fields on `SubagentControlEventPayload`.
#[test]
fn subagent_tool_result_rejects_unknown_active_long_running_control_event_field() {
    assert_parse_error_contains_any(
        "rejects unknown subagent active_long_running control event field",
        tool_result_message_json(
            "subagent",
            vec![json!({"type": "text", "text": "still running"})],
            false,
            Some(json!({
                "mode": "parallel",
                "results": [{
                    "agent": "code-quality-reviewer",
                    "controlEvents": [{
                        "type": "active_long_running",
                        "to": "active_long_running",
                        "ts": 1777657840252_u64,
                        "runId": "b48327c8",
                        "agent": "code-quality-reviewer",
                        "index": 0,
                        "message": "code-quality-reviewer is still active but long-running",
                        "reason": "turn_threshold",
                        "turns": 15,
                        "tokens": 121069,
                        "toolCount": 44,
                        "elapsedMs": 97198,
                        "unexpected": true
                    }]
                }]
            })),
        ),
        &["did not match any variant", "unknown field", "unexpected"],
    );
}

#[test]
fn subagent_tool_result_rejects_unknown_needs_attention_control_event_field() {
    assert_parse_error_contains_any(
        "rejects unknown subagent needs_attention control event field",
        tool_result_message_json(
            "subagent",
            vec![json!({"type": "text", "text": "needs attention"})],
            false,
            Some(json!({
                "mode": "parallel",
                "results": [{
                    "agent": "documentation-reviewer",
                    "controlEvents": [{
                        "type": "needs_attention",
                        "to": "needs_attention",
                        "ts": 1777921594147_u64,
                        "runId": "8784581c",
                        "agent": "documentation-reviewer",
                        "index": 2,
                        "message": "documentation-reviewer needs attention (no observed activity for 60s)",
                        "reason": "idle",
                        "turns": 12,
                        "tokens": 71740,
                        "toolCount": 54,
                        "elapsedMs": 60887,
                        "unexpected": true
                    }]
                }]
            })),
        ),
        &["did not match any variant", "unknown field", "unexpected"],
    );
}

/// Pins the closed-`type` contract on the `SubagentControlEvent` enum so
/// that a new upstream variant (for example a hypothetical `"paused"`
/// transition) surfaces as a loud parse failure rather than a silent drop.
#[test]
fn subagent_tool_result_rejects_unknown_control_event_type() {
    assert_parse_error_contains_any(
        "rejects unknown subagent control event type",
        tool_result_message_json(
            "subagent",
            vec![json!({"type": "text", "text": "unknown control event"})],
            false,
            Some(json!({
                "mode": "parallel",
                "results": [{
                    "agent": "code-quality-reviewer",
                    "controlEvents": [{
                        "type": "paused",
                        "to": "paused",
                        "ts": 1777657840252_u64,
                        "runId": "b48327c8",
                        "agent": "code-quality-reviewer",
                        "index": 0,
                        "message": "paused",
                        "reason": "manual",
                        "turns": 15,
                        "tokens": 121069,
                        "toolCount": 44,
                        "elapsedMs": 97198
                    }]
                }]
            })),
        ),
        &["did not match any variant", "unknown variant", "paused"],
    );
}
