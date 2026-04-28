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
    parsed.expect_err(&format!("expected parse error\nJSON: {raw}"))
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

fn parse_tool_call(tool_name: &str, arguments: Value) -> ToolCallContent {
    let AssistantContentItem::ToolCall(tool_call) = parse_first_assistant_content(
        assistant_tool_call_json(tool_name, arguments),
        AssistantFixture::new("openai-responses", "openai", "gpt-5.4", "toolUse"),
    ) else {
        panic!("expected ToolCall")
    };

    *tool_call
}

fn parse_subagent_args(arguments: Value) -> SubagentArgs {
    let tool_call = parse_tool_call("subagent", arguments);
    let ToolCallArguments::Subagent(args) = tool_call.tool else {
        panic!("expected Subagent args")
    };
    args
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
fn model_change_with_parent() {
    let line = parse(model_change_json(
        Some("session-root"),
        "openai",
        "gpt-5.4",
    ));

    match line {
        PiLogLine::ModelChange(model_change) => {
            assert_eq!(model_change.parent_id.as_deref(), Some("session-root"));
            assert_eq!(model_change.provider, Provider::OpenAi);
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
fn compaction_line() {
    let line = parse(compaction_json(false));

    match line {
        PiLogLine::Compaction(compaction) => {
            assert_eq!(compaction.summary, "Compacted earlier work");
            assert_eq!(compaction.first_kept_entry_id, "e1");
            assert_eq!(compaction.tokens_before, 12345);
            assert_eq!(
                compaction.details.read_files,
                vec![PathBuf::from("src/main.rs"), PathBuf::from("/tmp/output.log")]
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
    assert_eq!(bash_execution.output, "parsed 595 line(s) across 87 file(s)");
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
fn subagent_tool_call_accepts_output_variants() {
    let cases = [
        (
            json!({
                "agent": "scout",
                "task": "Inspect duplication hotspots",
                "cwd": "/home/brendan/src/moriarty",
                "output": false
            }),
            Some("scout"),
            Some("Inspect duplication hotspots"),
            Some(SubagentOutput::Enabled(false)),
        ),
        (
            json!({
                "agent": "writer",
                "task": "Draft reviewer summary",
                "output": "artifacts/review.md"
            }),
            Some("writer"),
            Some("Draft reviewer summary"),
            Some(SubagentOutput::Path("artifacts/review.md".to_string())),
        ),
    ];

    for (arguments, expected_agent, expected_task, expected_output) in cases {
        let args = parse_subagent_args(arguments);
        assert_eq!(args.agent.as_deref(), expected_agent);
        assert_eq!(args.task.as_deref(), expected_task);
        assert_eq!(args.output, expected_output);
    }
}

#[test]
fn subagent_tool_call_accepts_artifacts_flag() {
    let args = parse_subagent_args(json!({
        "tasks": [{
            "agent": "code-quality-reviewer",
            "task": "Review the change"
        }],
        "concurrency": 3,
        "context": "fresh",
        "cwd": "/home/brendan/.flk",
        "artifacts": true,
        "includeProgress": false
    }));

    assert_eq!(args.artifacts, Some(true));
    assert_eq!(args.include_progress, Some(false));
    assert_eq!(args.concurrency, Some(3));
}

#[test]
fn subagent_status_tool_call_accepts_action() {
    let tool_call = parse_tool_call("subagent_status", json!({ "action": "list" }));

    assert_eq!(tool_call.name(), ToolName::SubagentStatus);
    let ToolCallArguments::SubagentStatus(args) = tool_call.tool else {
        panic!("expected SubagentStatus args")
    };
    assert_eq!(args.action.as_deref(), Some("list"));
}

#[test]
fn fact_list_tool_call_stays_tied_to_tool_name() {
    let tool_call = parse_tool_call("fact_list", json!({}));

    assert_eq!(tool_call.name(), ToolName::FactList);
    assert!(matches!(tool_call.tool, ToolCallArguments::FactList(_)));
}

#[test]
fn ask_user_tool_call_accepts_title_option() {
    let tool_call = parse_tool_call(
        "ask_user",
        json!({
            "question": "Continue?",
            "options": ["Continue"]
        }),
    );

    let ToolCallArguments::AskUser(args) = tool_call.tool else {
        panic!("expected AskUser args")
    };

    assert_eq!(args.options, Some(vec![AskUserOption::Title("Continue".to_string())]));
}

#[test]
fn compress_tool_call_accepts_ranges() {
    let tool_call = parse_tool_call(
        "compress",
        json!({
            "topic": "Auth system exploration",
            "ranges": [
                {
                    "startId": "m001",
                    "endId": "m010",
                    "summary": "Explored OAuth flow"
                },
                {
                    "startId": "m015",
                    "endId": "m020",
                    "summary": "Reviewed token refresh"
                }
            ]
        }),
    );

    assert_eq!(tool_call.name(), ToolName::Compress);
    let ToolCallArguments::Compress(args) = tool_call.tool else {
        panic!("expected Compress args")
    };

    assert_eq!(args.topic, "Auth system exploration");
    assert_eq!(args.ranges.len(), 2);
    assert_eq!(args.ranges[0].start_id, "m001");
    assert_eq!(args.ranges[0].end_id, "m010");
    assert_eq!(args.ranges[1].summary, "Reviewed token refresh");
}

#[test]
fn code_search_tool_call_accepts_max_tokens() {
    let tool_call = parse_tool_call(
        "code_search",
        json!({
            "query": "jscpd ignore comment syntax ignore-start ignore-end",
            "maxTokens": 2000
        }),
    );

    let ToolCallArguments::CodeSearch(args) = &tool_call.tool else {
        panic!("expected CodeSearch args")
    };

    assert_eq!(
        args.query,
        "jscpd ignore comment syntax ignore-start ignore-end"
    );
    assert_eq!(args.max_tokens, 2000);
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
            assert_eq!(tool_call.name(), ToolName::Read);
            assert!(matches!(tool_call.tool, ToolCallArguments::Read(_)));
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
        AssistantFixture::new("anthropic-messages", "anthropic", "claude-sonnet-4-5", "stop"),
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

    assert_eq!(details.results[0].error.as_deref(), Some("No API key found"));
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
                "activeForm": "running review agents"
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
}

#[test]
fn ask_user_tool_result_accepts_selection_response() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "ask_user",
        vec![json!({
            "type": "text",
            "text": "User answered: Continue — No need to run the review agents again"
        })],
        false,
        Some(json!({
            "question": "We’ve reached the third review-agent pass. Should I continue and make the last two small code-review fixes?",
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

    assert_eq!(details.mode, "call");
    assert_eq!(details.server.as_deref(), Some("git-read-only"));
    assert_eq!(details.tool.as_deref(), Some("status"));

    let mcp_result = details.mcp_result.expect("expected mcp result");
    assert!(!mcp_result.is_error);
    let structured_content = mcp_result
        .structured_content
        .expect("expected structured content");
    assert_eq!(structured_content.exit_code, 0);
    assert_eq!(structured_content.stderr, "");
    assert_eq!(structured_content.stdout, "working tree clean\n");
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

    let mcp_result = details.mcp_result.expect("expected mcp result");
    assert!(!mcp_result.is_error);
    assert!(mcp_result.structured_content.is_none());
    assert_eq!(details.server.as_deref(), Some("project-tools"));
    assert_eq!(details.tool.as_deref(), Some("run_tests"));
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

    assert_eq!(details.mode, "call");
    assert_eq!(details.error.as_deref(), Some("call_failed"));
    assert_eq!(
        details.message.as_deref(),
        Some("MCP error -32600: Project tools not approved")
    );
    assert!(details.mcp_result.is_none());
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
    assert_eq!(
        details.error.as_deref(),
        Some("MCP error -32602: Tool get_code_context_exa not found")
    );
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

    assert!(details.truncation.truncated);
    assert_eq!(details.truncation.truncated_by, TruncatedBy::Bytes);
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

    assert_eq!(details.full_output_path, Some(PathBuf::from("/tmp/pi-bash.log")));
    assert_eq!(
        details.truncation.as_ref().map(|truncation| truncation.truncated_by),
        Some(TruncatedBy::Lines)
    );
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
fn compress_tool_result_accepts_block_ids() {
    let tool_result = parse_tool_result_message(tool_result_message_json(
        "compress",
        vec![json!({"type": "text", "text": "Compressed 2 ranges"})],
        false,
        Some(json!({
            "blockIds": [1, 2],
            "topic": "Parser incremental fixes"
        })),
    ));

    let Some(ToolResultDetails::Compress(details)) = tool_result.details else {
        panic!("expected Compress details")
    };

    assert_eq!(details.block_ids, vec![1, 2]);
    assert_eq!(details.topic, "Parser incremental fixes");
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
            assert_eq!(state.pruned_tool_ids, vec!["call_1"]);
            assert_eq!(state.tokens_saved, 1000);
            assert_eq!(state.total_prune_count, 3);
            assert!(!state.manual_mode);
            assert_eq!(state.compression_blocks.len(), 1);
            assert_eq!(state.compression_blocks[0].id, 1);
            assert_eq!(state.compression_blocks[0].topic, "Test topic");
            assert_eq!(state.compression_blocks[0].summary, "Test summary");
            assert_eq!(state.compression_blocks[0].start_timestamp.to_string(), "1777084923000.5");
            assert_eq!(state.compression_blocks[0].end_timestamp.to_string(), "1777084924000");
            assert_eq!(state.compression_blocks[0].anchor_timestamp.to_string(), "1777084924000.5");
            assert!(state.compression_blocks[0].active);
            assert_eq!(state.compression_blocks[0].summary_token_estimate, 100);
            assert_eq!(state.compression_blocks[0].created_at, 1777084924500);
        }
        other => panic!("expected DcpState, got {other:?}"),
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
            assert_eq!(details.saved_state.as_deref(), Some("draft"));
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
            assert_eq!(results.kind, WebSearchResultsKind::Search);
            assert_eq!(results.queries.len(), 1);
            assert_eq!(results.queries[0].provider, "exa");
        }
        other => panic!("expected WebSearchResults, got {other:?}"),
    }
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
            assert_eq!(details.last_submitted_path, Some(PathBuf::from("/tmp/PLAN.md")));
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
fn rejects_malformed_tool_call_arguments() {
    assert_parse_error_contains_any(
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
        &["did not match any variant", "unknown field"],
    );
}

#[test]
fn parse_file_ignores_blank_lines() {
    let tmp = std::env::temp_dir().join(format!("pi_logs_blank_{}.jsonl", uuid::Uuid::new_v4()));
    std::fs::write(
        &tmp,
        format!("{}\n\n{}\n", session_json("/tmp"), session_json("/tmp/project")),
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
