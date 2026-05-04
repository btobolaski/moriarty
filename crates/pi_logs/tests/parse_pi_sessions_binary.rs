use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn temp_sessions_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pi_logs_parse_pi_sessions_{name}_{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&dir).expect("expected temp sessions dir");
    dir
}

fn write_jsonl(path: &Path, lines: &[&str]) {
    let content = lines.join("\n") + "\n";
    fs::write(path, content).expect("expected jsonl fixture write");
}

#[test]
fn parse_pi_sessions_binary_succeeds_on_checked_in_fixture_dir() {
    let fixture_dir = fixture_path("tests/fixtures/pi_sessions_ok");
    let output = Command::new(env!("CARGO_BIN_EXE_parse_pi_sessions"))
        .arg(&fixture_dir)
        .output()
        .expect("expected parse_pi_sessions to run");

    assert!(
        output.status.success(),
        "expected success, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("scanning 1 *.jsonl file(s) under"));
    assert!(stdout.contains("parsed 5 line(s) across 1 file(s); 0 failure(s)"));
}

#[test]
fn parse_pi_sessions_binary_reports_failures() {
    let dir = temp_sessions_dir("failure");
    let good = dir.join("good.jsonl");
    let bad = dir.join("bad.jsonl");

    write_jsonl(
        &good,
        &[
            r#"{"type":"session","version":1,"id":"019dc252-e50e-766c-8182-d654b46881af","timestamp":"2026-05-04T20:00:00Z","cwd":"/tmp/moriarty"}"#,
        ],
    );
    write_jsonl(
        &bad,
        &[
            r#"{"type":"session","version":1,"id":"not-a-uuid","timestamp":"2026-05-04T20:00:00Z","cwd":"/tmp/moriarty"}"#,
        ],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_parse_pi_sessions"))
        .arg(&dir)
        .output()
        .expect("expected parse_pi_sessions to run");

    fs::remove_dir_all(&dir).expect("expected temp sessions dir cleanup");

    assert!(
        !output.status.success(),
        "expected parse_pi_sessions failure"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("scanning 2 *.jsonl file(s) under"));
    assert!(stdout.contains("FAIL "));
    assert!(stdout.contains("parsed 1 line(s) across 2 file(s); 1 failure(s)"));
    assert!(
        stderr.contains("invalid character") || stderr.contains("UUID") || stderr.contains("uuid")
    );
}
