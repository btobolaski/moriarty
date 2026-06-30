use std::{fs, path::Path, process::Command};

use chrono::{DateTime, TimeZone, Utc};
use serde_json::{Value, json};
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

fn assert_has_graph_bar(stdout: &str, row_prefix: &str) {
    let line = stdout
        .lines()
        .find(|line| line.starts_with(row_prefix))
        .unwrap_or_else(|| panic!("missing graph row {row_prefix:?} in:\n{stdout}"));

    assert!(
        ['█', '▓', '▒', '░', '▇', '▆']
            .into_iter()
            .any(|glyph| line.contains(glyph)),
        "expected graph row {row_prefix:?} to include a bar in:\n{line}"
    );
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

#[test]
fn api_pricing_cli_renders_daily_graphs() {
    let dir = TempDir::new().unwrap();
    let session = "019dc252-e50e-766c-8182-d654b46881af";
    write_log(
        dir.path(),
        "tokens.jsonl",
        &[
            assistant_line(
                session,
                timestamp(2026, 4, 16, 9, 0),
                "claude-sonnet-4-20250514",
                "req-token-1",
                usage_json(1_234, 5_678, 90, 12),
            ),
            assistant_line(
                session,
                timestamp(2026, 4, 17, 9, 0),
                "claude-opus-4-20250514",
                "req-token-2",
                usage_json(100, 200, 0, 0),
            ),
        ],
    );

    let output = moriarty_command()
        .args(["graphs", "claude", "--dir"])
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

    assert!(stdout.contains("API Token Graphs"));
    assert!(stdout.contains("Daily total tokens by model"));
    assert!(stdout.contains("Token share by model"));
    assert!(stdout.contains("Legend:"));
    assert!(stdout.contains("Sonnet"));
    assert!(stdout.contains("Opus 4"));
    assert!(stdout.contains("2026-04-16"));
    assert!(stdout.contains("2026-04-17"));
    assert!(stdout.contains("Grand Total: 7,314"));
    assert_has_graph_bar(&stdout, "2026-04-16");
}

#[test]
fn api_pricing_cli_renders_conversation_cost_graphs() {
    let dir = TempDir::new().unwrap();
    let session = "019dc252-e50e-766c-8182-d654b46881af";
    write_log(
        dir.path(),
        "costs.jsonl",
        &[
            assistant_line(
                session,
                timestamp(2026, 4, 16, 9, 0),
                "claude-sonnet-4-20250514",
                "req-cost-1",
                usage_json(1_000, 500, 0, 0),
            ),
            assistant_line(
                session,
                timestamp(2026, 4, 16, 10, 0),
                "claude-opus-4-20250514",
                "req-cost-2",
                usage_json(250, 125, 0, 0),
            ),
        ],
    );

    let output = moriarty_command()
        .args(["graphs", "claude", "--dir"])
        .arg(dir.path())
        .args(["--timezone", "utc", "--conversations"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();

    assert!(stdout.contains("API Cost Graphs by Conversation"));
    assert!(stdout.contains("Conversation total cost by model"));
    assert!(stdout.contains("Cost share by model"));
    assert!(stdout.contains("019dc252"));
    assert!(stdout.contains("Sonnet"));
    assert!(stdout.contains("Opus 4"));
    assert!(stdout.contains("Grand Total: $"));
    assert_has_graph_bar(&stdout, "019dc252");
}

#[test]
fn api_pricing_cli_graphs_apply_time_filter() {
    let dir = TempDir::new().unwrap();
    let session = "019dc252-e50e-766c-8182-d654b46881af";
    write_log(
        dir.path(),
        "tokens.jsonl",
        &[
            assistant_line(
                session,
                timestamp(2026, 4, 16, 9, 0),
                "claude-sonnet-4-20250514",
                "req-token-1",
                usage_json(1_234, 5_678, 90, 12),
            ),
            assistant_line(
                session,
                timestamp(2026, 4, 17, 9, 0),
                "claude-opus-4-20250514",
                "req-token-2",
                usage_json(100, 200, 0, 0),
            ),
        ],
    );

    let output = moriarty_command()
        .args(["graphs", "claude", "--dir"])
        .arg(dir.path())
        .args([
            "--timezone",
            "utc",
            "--tokens",
            "--start-time",
            "2026-04-17",
            "--end-time",
            "2026-04-17",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();

    assert!(stdout.contains("Applying time range filter:"));
    assert!(stdout.contains("Start: 2026-04-17T00:00:00+00:00"));
    assert!(stdout.contains("End:   2026-04-18T00:00:00+00:00"));
    assert!(!stdout.contains("2026-04-16"));
    assert!(stdout.contains("2026-04-17"));
    assert!(stdout.contains("Grand Total: 300"));
    assert_has_graph_bar(&stdout, "2026-04-17");
}

#[test]
fn api_pricing_cli_graphs_last_days_filters_by_recent_days() {
    let dir = TempDir::new().unwrap();
    let session = "019dc252-e50e-766c-8182-d654b46881af";

    let now = Utc::now();
    let today = now.date_naive();
    let ten_days_ago = today - chrono::Duration::days(10);
    let today_9am = today.and_hms_opt(9, 0, 0).unwrap().and_utc();
    let old_noon = ten_days_ago.and_hms_opt(12, 0, 0).unwrap().and_utc();

    write_log(
        dir.path(),
        "tokens.jsonl",
        &[
            // A log from ~10 days ago — should be excluded by --last-days 7.
            assistant_line(
                session,
                old_noon,
                "claude-sonnet-4-20250514",
                "req-old-1",
                usage_json(5_000, 10_000, 0, 0),
            ),
            // A log from today — should be included.
            assistant_line(
                session,
                today_9am,
                "claude-opus-4-20250514",
                "req-new-1",
                usage_json(100, 200, 0, 0),
            ),
        ],
    );

    let output = moriarty_command()
        .args(["graphs", "claude", "--dir"])
        .arg(dir.path())
        .args(["--last-days", "7", "--timezone", "utc", "--tokens"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();

    assert!(stdout.contains("Applying time range filter:"));
    let expected_start_date = today - chrono::Duration::days(6);
    let expected_start_str = format!("Start: {}T00:00:00+00:00", expected_start_date);
    assert!(
        stdout.contains(&expected_start_str),
        "expected start banner with '{expected_start_str}' in:\n{stdout}"
    );

    assert!(stdout.contains(&today.format("%Y-%m-%d").to_string()));
    assert!(!stdout.contains(&ten_days_ago.format("%Y-%m-%d").to_string()));
    assert!(stdout.contains("Grand Total: 300"));
    assert_has_graph_bar(&stdout, &today.format("%Y-%m-%d").to_string());
}

#[test]
fn api_pricing_cli_graphs_print_empty_state_and_warning() {
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
    fs::write(dir.path().join("invalid.jsonl"), "not json at all").unwrap();

    let output = moriarty_command()
        .args(["graphs", "claude", "--dir"])
        .arg(dir.path())
        .args([
            "--timezone",
            "utc",
            "--tokens",
            "--start-time",
            "2026-04-17",
            "--end-time",
            "2026-04-17",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert!(stdout.contains("No usage data found."));
    assert!(stderr.contains("Warning: some log files could not be read or parsed"));
}
