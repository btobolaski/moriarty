use serde_json::{json, Map, Value};

pub(crate) const CLAUDE_ASSISTANT_UUID: &str = "22222222-2222-4222-8222-222222222222";
pub(crate) const CLAUDE_BRANCH: &str = "main";
pub(crate) const CLAUDE_CWD: &str = "/home/brendan/src/moriarty";
pub(crate) const CLAUDE_PARENT_UUID: &str = "11111111-1111-4111-8111-111111111111";
pub(crate) const CLAUDE_SESSION_ID: &str = "019dc252-e50e-766c-8182-d654b46881af";
pub(crate) const CLAUDE_TIMESTAMP: &str = "2026-04-25T01:48:25.742Z";
pub(crate) const CLAUDE_USER_UUID: &str = "33333333-3333-4333-8333-333333333333";
pub(crate) const CLAUDE_LEAF_UUID: &str = "44444444-4444-4444-8444-444444444444";
pub(crate) const CLAUDE_VERSION: &str = "2.1.104";

pub(crate) fn claude_usage_json(
    input_tokens: usize,
    output_tokens: usize,
    cache_creation_input_tokens: usize,
    cache_read_input_tokens: usize,
) -> Value {
    json!({
        "input_tokens": input_tokens,
        "cache_creation_input_tokens": cache_creation_input_tokens,
        "cache_read_input_tokens": cache_read_input_tokens,
        "cache_creation": {
            "ephemeral_5m_input_tokens": 0,
            "ephemeral_1h_input_tokens": 0,
        },
        "output_tokens": output_tokens,
        "service_tier": null,
        "server_tool_use": null,
        "inference_geo": null,
        "iterations": null,
        "speed": null,
    })
}

pub(crate) fn claude_transcript_envelope(parent_uuid: Option<&str>) -> Map<String, Value> {
    let mut metadata = Map::new();
    metadata.insert("parentUuid".to_string(), json!(parent_uuid));
    metadata.insert("isSidechain".to_string(), json!(false));
    metadata.insert("agentId".to_string(), Value::Null);
    metadata.insert("userType".to_string(), json!("external"));
    metadata.insert("cwd".to_string(), json!(CLAUDE_CWD));
    metadata.insert("sessionId".to_string(), json!(CLAUDE_SESSION_ID));
    metadata.insert("version".to_string(), json!(CLAUDE_VERSION));
    metadata.insert("gitBranch".to_string(), json!(CLAUDE_BRANCH));
    metadata.insert("slug".to_string(), Value::Null);
    metadata
}

pub(crate) fn claude_assistant_json(
    parent_uuid: Option<&str>,
    request_id: Option<&str>,
    message_id: &str,
    uuid: &str,
    model: &str,
    usage: Value,
) -> Value {
    let mut metadata = claude_transcript_envelope(parent_uuid);
    metadata.insert("type".to_string(), json!("assistant"));
    metadata.insert(
        "message".to_string(),
        json!({
            "id": message_id,
            "type": "message",
            "role": "assistant",
            "model": model,
            "container": null,
            "content": [{"type": "text", "text": "hello"}],
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "stop_details": null,
            "usage": usage,
            "context_management": null,
        }),
    );
    metadata.insert("requestId".to_string(), json!(request_id));
    metadata.insert("uuid".to_string(), json!(uuid));
    metadata.insert("timestamp".to_string(), json!(CLAUDE_TIMESTAMP));
    metadata.insert("isApiErrorMessage".to_string(), Value::Null);
    metadata.insert("error".to_string(), Value::Null);
    metadata.insert("entrypoint".to_string(), Value::Null);
    Value::Object(metadata)
}
