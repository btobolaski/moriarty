use std::{fs, path::Path, process::Command};

use rust_decimal::Decimal;
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

fn session_line(session_id: &str, timestamp: &str) -> Value {
    json!({
        "type": "session",
        "version": 1,
        "id": session_id,
        "timestamp": timestamp,
        "cwd": "/tmp/moriarty-test"
    })
}

fn decimal_total(input: &str, output: &str, cache_write: &str, cache_read: &str) -> String {
    (Decimal::from_str_exact(input).unwrap()
        + Decimal::from_str_exact(output).unwrap()
        + Decimal::from_str_exact(cache_write).unwrap()
        + Decimal::from_str_exact(cache_read).unwrap())
    .to_string()
}

#[allow(clippy::too_many_arguments)]
fn assistant_line(
    id: &str,
    timestamp: &str,
    provider: &str,
    api: &str,
    model: &str,
    input: &str,
    output: &str,
    cache_write: &str,
    cache_read: &str,
) -> Value {
    assistant_line_with_tokens(
        id,
        timestamp,
        provider,
        api,
        model,
        input,
        output,
        cache_write,
        cache_read,
        10,
        5,
        1,
        2,
    )
}

#[allow(clippy::too_many_arguments)]
fn assistant_line_with_tokens(
    id: &str,
    timestamp: &str,
    provider: &str,
    api: &str,
    model: &str,
    input: &str,
    output: &str,
    cache_write: &str,
    cache_read: &str,
    input_tokens: i64,
    output_tokens: i64,
    cache_write_tokens: i64,
    cache_read_tokens: i64,
) -> Value {
    let total = decimal_total(input, output, cache_write, cache_read);
    json!({
        "type": "message",
        "id": id,
        "parentId": "u1",
        "timestamp": timestamp,
        "message": {
            "role": "assistant",
            "content": [{"type": "text", "text": "hello"}],
            "api": api,
            "provider": provider,
            "model": model,
            "usage": {
                "input": input_tokens,
                "output": output_tokens,
                "cacheRead": cache_read_tokens,
                "cacheWrite": cache_write_tokens,
                "totalTokens": input_tokens + output_tokens + cache_write_tokens + cache_read_tokens,
                "cost": {
                    "input": input,
                    "output": output,
                    "cacheRead": cache_read,
                    "cacheWrite": cache_write,
                    "total": total
                }
            },
            "stopReason": "stop",
            "timestamp": 1700000000
        }
    })
}

fn anthropic_line(id: &str, timestamp: &str, model: &str, input: &str, output: &str) -> Value {
    assistant_line(
        id,
        timestamp,
        "anthropic",
        "anthropic-messages",
        model,
        input,
        output,
        "0",
        "0",
    )
}

fn openai_line(id: &str, timestamp: &str, model: &str, input: &str, output: &str) -> Value {
    assistant_line(
        id,
        timestamp,
        "openai",
        "openai-responses",
        model,
        input,
        output,
        "0",
        "0",
    )
}

fn moriarty_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_moriarty"))
}

fn assert_daily_token_columns(stdout: &str) {
    for expected in ["123", "456", "90", "12", "681"] {
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
fn pi_cost_cli_renders_daily_report_and_incomplete_warning() {
    let dir = TempDir::new().unwrap();
    let session = "019dc252-e50e-766c-8182-d654b46881b0";
    write_log(
        dir.path(),
        "valid.jsonl",
        &[
            session_line(session, "2026-04-16T00:00:00Z"),
            anthropic_line(
                "anthropic-1",
                "2026-04-16T09:00:00Z",
                "claude-sonnet-4-5",
                "1.0",
                "2.0",
            ),
            openai_line("openai-1", "2026-04-16T10:00:00Z", "gpt-5", "0.5", "0.5"),
        ],
    );
    fs::write(dir.path().join("invalid.jsonl"), "not json at all").unwrap();

    let output = moriarty_command()
        .args(["pi", "cost", "--dir"])
        .arg(dir.path())
        .args(["--timezone", "utc"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert!(stdout.contains("Pi Cost Report"));
    assert!(stdout.contains("2026-04"));
    assert!(stdout.contains("gpt-5"));
    assert!(stdout.contains("claude-sonnet"));
    assert!(stdout.contains("Summary"));
    assert!(stdout.contains("Grand Total"));
    assert!(stdout.contains("$4.0000"));
    assert!(stderr.contains("Warning: some log files could not be read or parsed"));
}

#[test]
fn pi_cost_cli_renders_daily_token_report() {
    let dir = TempDir::new().unwrap();
    let session = "019dc252-e50e-766c-8182-d654b46881b0";
    write_log(
        dir.path(),
        "session.jsonl",
        &[
            session_line(session, "2026-04-16T00:00:00Z"),
            assistant_line_with_tokens(
                "openai-1",
                "2026-04-16T09:00:00Z",
                "openai",
                "openai-responses",
                "x",
                "1.0",
                "2.0",
                "0",
                "0",
                123,
                456,
                90,
                12,
            ),
        ],
    );

    let output = moriarty_command()
        .args(["pi", "cost", "--dir"])
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

    assert!(stdout.contains("Pi Token Report"));
    assert!(stdout.contains("Grand Total"));
    assert_daily_token_columns(&stdout);
}

#[test]
fn pi_cost_cli_renders_conversation_token_report_in_utc() {
    let dir = TempDir::new().unwrap();
    let session = "019dc252-e50e-766c-8182-d654b46881b0";
    write_log(
        dir.path(),
        "session.jsonl",
        &[
            session_line(session, "2026-04-16T00:00:00Z"),
            assistant_line_with_tokens(
                "openai-1",
                "2026-04-16T09:00:00Z",
                "openai",
                "openai-responses",
                "x",
                "1.0",
                "2.0",
                "0",
                "0",
                1_234,
                5_678,
                90,
                12,
            ),
        ],
    );

    let output = moriarty_command()
        .args(["pi", "cost", "--dir"])
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

    assert!(stdout.contains("Pi Token Report by Conversation"));
    assert!(stdout.contains("2026-04-16"));
    assert!(stdout.contains("7,014"));
    assert!(!stdout.contains('$'));
}

#[test]
fn pi_cost_cli_renders_conversation_report_in_utc() {
    let dir = TempDir::new().unwrap();
    let session = "019dc252-e50e-766c-8182-d654b46881b0";
    write_log(
        dir.path(),
        "session.jsonl",
        &[
            session_line(session, "2026-04-16T00:00:00Z"),
            anthropic_line(
                "anthropic-1",
                "2026-04-16T09:00:00Z",
                "claude-sonnet-4-5",
                "1.0",
                "2.0",
            ),
            openai_line("openai-1", "2026-04-16T10:30:00Z", "gpt-5", "0.5", "0.5"),
        ],
    );

    let output = moriarty_command()
        .args(["pi", "cost", "--dir"])
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

    assert!(stdout.contains("Pi Cost Report by Conversation"));
    assert!(stdout.contains("2026-04-16 09:00"));
    assert!(stdout.contains("1 hr"));
    assert!(stdout.contains("gpt-5"));
    assert!(stdout.contains("Summary"));
    assert!(stdout.contains("Grand Total"));
    assert!(stdout.contains("$4.0000"));
}

#[test]
fn pi_cost_cli_conversation_report_warns_when_incomplete() {
    let dir = TempDir::new().unwrap();
    let session = "019dc252-e50e-766c-8182-d654b46881b0";
    write_log(
        dir.path(),
        "session.jsonl",
        &[
            session_line(session, "2026-04-16T00:00:00Z"),
            anthropic_line(
                "anthropic-1",
                "2026-04-16T09:00:00Z",
                "claude-sonnet-4-5",
                "1.0",
                "2.0",
            ),
        ],
    );
    fs::write(dir.path().join("invalid.jsonl"), "not json at all").unwrap();

    let output = moriarty_command()
        .args(["pi", "cost", "--dir"])
        .arg(dir.path())
        .args(["--timezone", "utc", "--conversations"])
        .output()
        .unwrap();

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert!(stdout.contains("Pi Cost Report by Conversation"));
    assert!(stdout.contains("$3.0000"));
    assert!(stderr.contains("Warning: some log files could not be read or parsed"));
}

#[test]
fn pi_cost_cli_conversation_filter_keeps_matching_rows() {
    let dir = TempDir::new().unwrap();
    let session = "019dc252-e50e-766c-8182-d654b46881b0";
    write_log(
        dir.path(),
        "session.jsonl",
        &[
            session_line(session, "2026-04-16T00:00:00Z"),
            anthropic_line(
                "out-of-window",
                "2026-04-15T11:00:00Z",
                "claude-sonnet-4-5",
                "1.0",
                "0.0",
            ),
            openai_line("in-window", "2026-04-16T14:00:00Z", "gpt-5", "0.5", "1.5"),
        ],
    );

    let output = moriarty_command()
        .args(["pi", "cost", "--dir"])
        .arg(dir.path())
        .args([
            "--timezone",
            "utc",
            "--conversations",
            "--start-time",
            "2026-04-16",
            "--end-time",
            "2026-04-16",
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
    assert!(stdout.contains("Pi Cost Report by Conversation"));
    assert!(stdout.contains("2026-04-16 14:00"));
    assert!(!stdout.contains("2026-04-15 11:00"));
    assert!(!stdout.contains("$1.0000"));
    assert!(stdout.contains("Summary"));
    assert!(stdout.contains("$2.0000"));
}

#[test]
fn pi_cost_cli_daily_filter_keeps_matching_rows() {
    let dir = TempDir::new().unwrap();
    let session = "019dc252-e50e-766c-8182-d654b46881b0";
    write_log(
        dir.path(),
        "session.jsonl",
        &[
            session_line(session, "2026-04-16T00:00:00Z"),
            anthropic_line(
                "out-of-window",
                "2026-04-15T11:00:00Z",
                "claude-sonnet-4-5",
                "1.0",
                "0.0",
            ),
            openai_line("in-window", "2026-04-16T14:00:00Z", "gpt-5", "0.5", "1.5"),
        ],
    );

    let output = moriarty_command()
        .args(["pi", "cost", "--dir"])
        .arg(dir.path())
        .args([
            "--timezone",
            "utc",
            "--start-time",
            "2026-04-16",
            "--end-time",
            "2026-04-16",
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
    assert!(stdout.contains("Pi Cost Report"));
    assert!(stdout.contains("2026-04-"));
    assert!(stdout.contains("OpenAI"));
    assert!(!stdout.contains("Anthro"));
    assert!(!stdout.contains("$1.0000"));
    assert!(stdout.contains("Summary"));
    assert!(stdout.contains("$2.0000"));
}

#[test]
fn pi_cost_cli_defaults_to_local_timezone() {
    let dir = TempDir::new().unwrap();
    let session = "019dc252-e50e-766c-8182-d654b46881b0";
    write_log(
        dir.path(),
        "session.jsonl",
        &[
            session_line(session, "2026-04-16T00:00:00Z"),
            openai_line("boundary", "2026-04-16T01:30:00Z", "gpt-5", "0.5", "1.5"),
        ],
    );

    let output = moriarty_command()
        .env("TZ", "America/New_York")
        .args(["pi", "cost", "--dir"])
        .arg(dir.path())
        .args(["--conversations"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();

    assert!(stdout.contains("Pi Cost Report by Conversation"));
    assert!(stdout.contains("2026-04-15 21:30"));
    assert!(!stdout.contains("2026-04-16 01:30"));
    assert!(stdout.contains("$2.0000"));
}

#[test]
fn pi_cost_cli_prints_filter_banner_and_empty_state() {
    let dir = TempDir::new().unwrap();
    let session = "019dc252-e50e-766c-8182-d654b46881b0";
    write_log(
        dir.path(),
        "session.jsonl",
        &[
            session_line(session, "2026-04-16T00:00:00Z"),
            anthropic_line(
                "anthropic-1",
                "2026-04-16T09:00:00Z",
                "claude-sonnet-4-5",
                "1.0",
                "2.0",
            ),
        ],
    );
    fs::write(dir.path().join("invalid.jsonl"), "not json at all").unwrap();

    let output = moriarty_command()
        .args(["pi", "cost", "--dir"])
        .arg(dir.path())
        .args([
            "--timezone",
            "utc",
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

    assert!(stdout.contains("Applying time range filter:"));
    assert!(stdout.contains("Start: 2026-04-17T00:00:00+00:00"));
    assert!(stdout.contains("End:   2026-04-18T00:00:00+00:00"));
    assert!(stdout.contains("No usage data found."));
    assert!(stderr.contains("Warning: some log files could not be read or parsed"));
}

#[test]
fn pi_cost_cli_renders_conversation_graphs() {
    let dir = TempDir::new().unwrap();
    let session = "019dc252-e50e-766c-8182-d654b46881b0";
    write_log(
        dir.path(),
        "session.jsonl",
        &[
            session_line(session, "2026-04-16T00:00:00Z"),
            assistant_line_with_tokens(
                "anthropic-1",
                "2026-04-16T09:00:00Z",
                "anthropic",
                "anthropic-messages",
                "claude-sonnet-4-5",
                "1.0",
                "2.0",
                "0",
                "0",
                1_000,
                2_000,
                0,
                0,
            ),
            assistant_line_with_tokens(
                "openai-1",
                "2026-04-16T10:00:00Z",
                "openai",
                "openai-responses",
                "gpt-5",
                "0.5",
                "0.5",
                "0",
                "0",
                500,
                250,
                0,
                0,
            ),
        ],
    );

    let output = moriarty_command()
        .args(["graphs", "pi", "--dir"])
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

    assert!(stdout.contains("Pi Token Graphs by Conversation"));
    assert!(stdout.contains("Conversation total tokens by provider/model"));
    assert!(stdout.contains("Token share by provider/model"));
    assert!(stdout.contains("Anthropic / claude-sonnet-4-5"));
    assert!(stdout.contains("OpenAI / gpt-5"));
    assert!(stdout.contains("019dc252"));
    assert!(stdout.contains("Grand Total: 3,750"));
    assert!(!stdout.contains('$'));
    assert_has_graph_bar(&stdout, "019dc252");
}

#[test]
fn pi_cost_cli_renders_daily_cost_graphs() {
    let dir = TempDir::new().unwrap();
    let session = "019dc252-e50e-766c-8182-d654b46881b0";
    write_log(
        dir.path(),
        "session.jsonl",
        &[
            session_line(session, "2026-04-16T00:00:00Z"),
            anthropic_line(
                "anthropic-1",
                "2026-04-16T09:00:00Z",
                "claude-sonnet-4-5",
                "1.0",
                "2.0",
            ),
            openai_line("openai-1", "2026-04-17T10:00:00Z", "gpt-5", "0.5", "0.5"),
        ],
    );

    let output = moriarty_command()
        .args(["graphs", "pi", "--dir"])
        .arg(dir.path())
        .args(["--timezone", "utc"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();

    assert!(stdout.contains("Pi Cost Graphs"));
    assert!(stdout.contains("Daily total cost by provider/model"));
    assert!(stdout.contains("Cost share by provider/model"));
    assert!(stdout.contains("Anthropic / claude-sonnet-4-5"));
    assert!(stdout.contains("OpenAI / gpt-5"));
    assert!(stdout.contains("2026-04-16"));
    assert!(stdout.contains("2026-04-17"));
    assert!(stdout.contains("Grand Total: $4.0000"));
    assert_has_graph_bar(&stdout, "2026-04-16");
}

#[test]
fn pi_cost_cli_graphs_print_filter_banner_empty_state_and_warning() {
    let dir = TempDir::new().unwrap();
    let session = "019dc252-e50e-766c-8182-d654b46881b0";
    write_log(
        dir.path(),
        "session.jsonl",
        &[
            session_line(session, "2026-04-16T00:00:00Z"),
            anthropic_line(
                "anthropic-1",
                "2026-04-16T09:00:00Z",
                "claude-sonnet-4-5",
                "1.0",
                "2.0",
            ),
        ],
    );
    fs::write(dir.path().join("invalid.jsonl"), "not json at all").unwrap();

    let output = moriarty_command()
        .args(["graphs", "pi", "--dir"])
        .arg(dir.path())
        .args([
            "--timezone",
            "utc",
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

    assert!(stdout.contains("Applying time range filter:"));
    assert!(stdout.contains("Start: 2026-04-17T00:00:00+00:00"));
    assert!(stdout.contains("End:   2026-04-18T00:00:00+00:00"));
    assert!(stdout.contains("No usage data found."));
    assert!(stderr.contains("Warning: some log files could not be read or parsed"));
}
