use std::{fs, path::Path, process::Command};

use chrono::{DateTime, TimeZone, Utc};
use serde_json::{json, Value};
use tempfile::TempDir;

fn write_log(dir: &Path, name: &str, lines: &[Value]) {
    let body = lines
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(dir.join(name), body).unwrap();
}

fn timestamp(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
        .single()
        .unwrap()
}

fn usage_json(
    input_tokens: usize,
    output_tokens: usize,
    cache_write_tokens: usize,
    cache_read_tokens: usize,
) -> Value {
    json!({
        "input_tokens": input_tokens,
        "cache_creation_input_tokens": cache_write_tokens,
        "cache_read_input_tokens": cache_read_tokens,
        "cache_creation": {
            "ephemeral_5m_input_tokens": 0,
            "ephemeral_1h_input_tokens": 0,
        },
        "output_tokens": output_tokens,
        "service_tier": null,
        "server_tool_use": null,
        "inference_geo": null,
        "iterations": null,
    })
}

fn assistant_line(
    session_id: &str,
    ts: DateTime<Utc>,
    model: &str,
    request_id: &str,
    usage: Value,
) -> Value {
    json!({
        "parentUuid": null,
        "isSidechain": false,
        "agentId": null,
        "userType": "external",
        "cwd": "/tmp/moriarty-test",
        "sessionId": session_id,
        "version": "2.1.104",
        "gitBranch": "main",
        "slug": null,
        "type": "assistant",
        "message": {
            "id": format!("msg-{request_id}"),
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
        },
        "requestId": request_id,
        "uuid": "00000000-0000-4000-8000-000000000000",
        "timestamp": ts.to_rfc3339(),
        "isApiErrorMessage": null,
        "error": null,
        "entrypoint": null,
    })
}

fn moriarty_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_moriarty"))
}

fn assert_token_columns(stdout: &str) {
    for expected in ["1,234", "5,678", "90", "12", "7,014"] {
        assert!(
            stdout.contains(expected),
            "missing token value {expected} in:\n{stdout}"
        );
    }
    assert!(!stdout.contains('$'));
}

#[test]
fn api_pricing_cli_renders_daily_token_report() {
    let dir = TempDir::new().unwrap();
    let session = "019dc252-e50e-766c-8182-d654b46881af";
    write_log(
        dir.path(),
        "tokens.jsonl",
        &[assistant_line(
            session,
            timestamp(2026, 4, 16, 9, 0),
            "claude-sonnet-4-20250514",
            "req-token-1",
            usage_json(1_234, 5_678, 90, 12),
        )],
    );

    let output = moriarty_command()
        .args(["api-pricing", "--dir"])
        .arg(dir.path())
        .args(["--timezone", "utc", "--tokens"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();

    assert!(stdout.contains("API Token Report"));
    assert!(stdout.contains("Grand Total"));
    assert_token_columns(&stdout);
}

#[test]
fn api_pricing_cli_renders_conversation_token_report() {
    let dir = TempDir::new().unwrap();
    let session = "019dc252-e50e-766c-8182-d654b46881af";
    write_log(
        dir.path(),
        "tokens.jsonl",
        &[assistant_line(
            session,
            timestamp(2026, 4, 16, 9, 0),
            "claude-sonnet-4-20250514",
            "req-token-1",
            usage_json(1_234, 5_678, 90, 12),
        )],
    );

    let output = moriarty_command()
        .args(["api-pricing", "--dir"])
        .arg(dir.path())
        .args(["--timezone", "utc", "--conversations", "--tokens"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();

    assert!(stdout.contains("API Token Report by Conversation"));
    assert!(stdout.contains("2026-04-16"));
    assert!(stdout.contains("7,014"));
    assert!(!stdout.contains('$'));
}
