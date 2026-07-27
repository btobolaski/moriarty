use std::{fs, path::Path, process::Command};

use tempfile::TempDir;

const CLAUDE_SESSION: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const PI_SESSION: &str = "bbbbbbbb-0000-4000-8000-000000000002";
const CLAUDE_LINE: &str = r#"{"parentUuid":null,"isSidechain":false,"agentId":null,"userType":"external","cwd":"/tmp/moriarty-test","sessionId":"$SESSION","version":"2.1.104","gitBranch":"main","slug":null,"type":"assistant","message":{"id":"msg-combined","type":"message","role":"assistant","model":"claude-sonnet-4-20250514","container":null,"content":[{"type":"text","text":"hello"}],"stop_reason":"end_turn","stop_sequence":null,"stop_details":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":200,"service_tier":null,"server_tool_use":null,"inference_geo":null,"iterations":null},"context_management":null},"requestId":"req-combined","uuid":"00000000-0000-4000-8000-000000000000","timestamp":"2026-04-16T09:00:00Z","isApiErrorMessage":null,"error":null,"entrypoint":null}"#;
const PI_SESSION_LINE: &str = r#"{"type":"session","version":1,"id":"$SESSION","timestamp":"2026-04-16T08:00:00Z","cwd":"/tmp/moriarty-test"}"#;
const PI_MESSAGE_LINE: &str = r#"{"type":"message","id":"pi-combined","parentId":"u1","timestamp":"2026-04-16T10:00:00Z","message":{"role":"assistant","content":[{"type":"text","text":"hello"}],"api":"anthropic-messages","provider":"anthropic","model":"claude-sonnet-4-5","usage":{"input":300,"output":400,"cacheRead":0,"cacheWrite":0,"totalTokens":700,"cost":{"input":"1.0","output":"2.0","cacheRead":"0","cacheWrite":"0","total":"3.0"}},"stopReason":"stop","timestamp":1700000000}}"#;

fn write_fixtures(claude_dir: &Path, pi_dir: &Path) {
    fs::write(
        claude_dir.join("claude.jsonl"),
        CLAUDE_LINE.replace("$SESSION", CLAUDE_SESSION),
    )
    .unwrap();
    fs::write(
        pi_dir.join("pi.jsonl"),
        format!(
            "{}\n{}",
            PI_SESSION_LINE.replace("$SESSION", PI_SESSION),
            PI_MESSAGE_LINE
        ),
    )
    .unwrap();
}

fn moriarty() -> Command {
    Command::new(env!("CARGO_BIN_EXE_moriarty"))
}

fn combined(claude_dir: &Path, pi_dir: &Path) -> Command {
    let mut command = moriarty();
    command
        .args(["graphs", "all", "--claude-dir"])
        .arg(claude_dir)
        .arg("--pi-dir")
        .arg(pi_dir);
    command
}

fn success(command: &mut Command) -> (String, String) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    (
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

#[test]
fn daily_cost_merges_sources_and_retains_data_after_partial_failure() {
    let claude = TempDir::new().unwrap();
    let pi = TempDir::new().unwrap();
    write_fixtures(claude.path(), pi.path());
    fs::write(pi.path().join("malformed.jsonl"), "not json").unwrap();

    let mut command = combined(claude.path(), pi.path());
    command.args(["--timezone", "utc"]);
    let (stdout, stderr) = success(&mut command);

    assert!(stdout.contains("Combined Cost Graphs"), "{stdout}");
    assert!(stdout.contains("Claude Code / Sonnet 4"), "{stdout}");
    assert!(
        stdout.contains("pi / Anthropic / claude-sonnet-4-5"),
        "{stdout}"
    );
    assert_eq!(stdout.matches("2026-04-16").count(), 1, "{stdout}");
    assert!(stdout.contains("Grand Total: $3.0060"), "{stdout}");
    assert_eq!(
        stderr.matches("totals may be incomplete").count(),
        1,
        "{stderr}"
    );
}

#[test]
fn conversation_tokens_show_both_sessions_without_currency() {
    let claude = TempDir::new().unwrap();
    let pi = TempDir::new().unwrap();
    write_fixtures(claude.path(), pi.path());

    let mut command = combined(claude.path(), pi.path());
    command.args(["--timezone", "utc", "--conversations", "--tokens"]);
    let (stdout, _) = success(&mut command);

    assert!(
        stdout.contains("Combined Token Graphs by Conversation"),
        "{stdout}"
    );
    assert!(stdout.contains("Claude Code / Sonnet 4"), "{stdout}");
    assert!(
        stdout.contains("pi / Anthropic / claude-sonnet-4-5"),
        "{stdout}"
    );
    assert!(
        stdout.contains("aaaaaaaa") && stdout.contains("bbbbbbbb"),
        "{stdout}"
    );
    assert!(stdout.contains("Grand Total: 1,900"), "{stdout}");
    assert!(!stdout.contains('$'), "{stdout}");
}

#[test]
fn directory_resolution_and_empty_state() {
    let claude = TempDir::new().unwrap();
    let unused_pi = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    write_fixtures(claude.path(), unused_pi.path());

    let mut skipped = moriarty();
    skipped
        .env("HOME", home.path())
        .args(["graphs", "all", "--claude-dir"])
        .arg(claude.path());
    let (stdout, stderr) = success(&mut skipped);
    assert!(
        stderr.contains("skipping this source") && stderr.contains("--pi-dir"),
        "{stderr}"
    );
    assert!(stdout.contains("Claude Code / Sonnet 4"), "{stdout}");

    let missing = home.path().join("missing");
    let output = combined(claude.path(), &missing).output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not exist"));

    let output = moriarty()
        .env("HOME", home.path())
        .args(["graphs", "all"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("No log directories to analyze"));

    let empty_claude = TempDir::new().unwrap();
    let empty_pi = TempDir::new().unwrap();
    let mut empty = combined(empty_claude.path(), empty_pi.path());
    let (stdout, _) = success(&mut empty);
    assert_eq!(
        stdout.matches("No usage data found.").count(),
        1,
        "{stdout}"
    );
}
