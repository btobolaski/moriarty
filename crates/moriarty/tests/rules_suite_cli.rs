use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use serde_json::{Value, json};
use tempfile::TempDir;

struct Fixture {
    root: TempDir,
    suite: PathBuf,
    policy: PathBuf,
    xdg_config: PathBuf,
}

impl Fixture {
    fn new(suite: &str, policy: &str) -> Self {
        let root = TempDir::new().unwrap();
        let suite_path = root.path().join("suite.toml");
        let policy_path = root.path().join("policy.toml");
        let xdg_config = root.path().join("xdg-config");
        fs::create_dir_all(xdg_config.join("moriarty")).unwrap();
        fs::create_dir_all(root.path().join("xdg-state")).unwrap();
        fs::create_dir_all(root.path().join("xdg-cache")).unwrap();
        fs::create_dir_all(root.path().join("home")).unwrap();
        fs::write(&suite_path, suite).unwrap();
        fs::write(&policy_path, policy).unwrap();
        Self {
            root,
            suite: suite_path,
            policy: policy_path,
            xdg_config,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_moriarty"));
        command
            .current_dir(self.root.path())
            .env("HOME", self.root.path().join("home"))
            .env("XDG_CONFIG_HOME", &self.xdg_config)
            .env("XDG_STATE_HOME", self.root.path().join("xdg-state"))
            .env("XDG_CACHE_HOME", self.root.path().join("xdg-cache"))
            .env("RUST_LOG", "warn");
        command
    }

    fn rules(&self, json: bool) -> Command {
        self.rules_with("suite.toml", "policy.toml", json)
    }

    fn rules_with(&self, suite: &str, policy: &str, json: bool) -> Command {
        let mut command = self.command();
        command.args(["test", "rules", suite, "-c", policy]);
        if json {
            command.arg("--json");
        }
        command
    }

    fn install_policy(&self, source: &str) {
        fs::write(self.xdg_config.join("moriarty/tool_rules.toml"), source).unwrap();
    }
}

fn run(mut command: Command) -> Output {
    command.output().unwrap()
}

fn run_with_stdin(mut command: Command, input: &Value) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(serde_json::to_string(input).unwrap().as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn assert_status(output: &Output, status: i32) {
    assert_eq!(
        output.status.code(),
        Some(status),
        "stdout:\n{}\nstderr:\n{}",
        text(&output.stdout),
        text(&output.stderr)
    );
}

fn hook_event(cwd: &Path, mode: &str, tool: &str, input: Value) -> Value {
    json!({
        "session_id": "synthetic-session",
        "transcript_path": "/tmp/synthetic-transcript.jsonl",
        "cwd": cwd,
        "permission_mode": mode,
        "hook_event_name": "PreToolUse",
        "tool_name": tool,
        "tool_input": input,
    })
}

#[test]
fn success_reports_order_expansion_cwds_and_explicit_policy_in_both_formats() {
    let suite = r#"format_version = 1
cases = [
  { id = "bash", name = "expanded bash", request = { kind = "bash", command = "echo {{cwd}}" }, modes = ["plan", "default"], expect = "allowed", expand_cwd = true },
  { id = "tool", name = "case-local tool cwd", request = { kind = "tool", tool = "Read", input_json = '{"path":"{{cwd}}/input","null":null}' }, modes = ["auto"], cwd = "case", expect = "ask", expand_cwd = true },
  { id = "last", name = "mode-less pass-through", request = { kind = "tool", tool = "Unknown", input_json = '{}' }, expect = "no_match" },
]
"#;
    let policy = r#"bash_rules = [{ name = "allow-echo", pattern = "^echo ", action = { type = "Allow" } }]
tool_rules = [{ name = "ask-read", tool = "Read", action = { type = "Ask" } }]
"#;
    let fixture = Fixture::new(suite, policy);
    fs::create_dir_all(fixture.root.path().join("eval/case")).unwrap();
    fixture.install_policy(
        r#"bash_rules = [{ name = "deployed-deny", pattern = ".*", action = { type = "Deny", value = "wrong policy" } }]
tool_rules = [{ name = "deployed-allow", tool = "*", action = { type = "Allow" } }]
"#,
    );

    let mut json_command = fixture.rules(true);
    json_command.args(["--cwd", "eval"]);
    let output = run(json_command);
    assert_status(&output, 0);
    assert!(output.stderr.is_empty(), "{}", text(&output.stderr));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["format_version"], 1);
    assert_eq!(report["source_case_count"], 3);
    assert_eq!(report["evaluation_count"], 4);
    assert_eq!(report["passed_count"], 4);
    assert_eq!(report["failed_count"], 0);
    assert_eq!(
        report["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["bash", "bash", "tool", "last"]
    );
    assert_eq!(report["rows"][0]["mode"], "plan");
    assert_eq!(report["rows"][1]["mode"], "default");
    assert_eq!(report["rows"][2]["mode"], "auto");
    assert!(report["rows"][3]["mode"].is_null());
    let default_cwd = fs::canonicalize(fixture.root.path().join("eval"))
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let case_cwd = fs::canonicalize(fixture.root.path().join("eval/case"))
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(report["rows"][0]["effective_cwd"], default_cwd);
    assert_eq!(
        report["rows"][0]["request"]["command"],
        format!("echo {default_cwd}")
    );
    assert_eq!(report["rows"][2]["effective_cwd"], case_cwd);
    assert_eq!(
        report["rows"][2]["request"]["input"]["path"],
        format!("{case_cwd}/input")
    );
    assert!(report["rows"][2]["request"]["input"]["null"].is_null());

    let mut human_command = fixture.rules(false);
    human_command.args(["--cwd", "eval"]);
    let output = run(human_command);
    assert_status(&output, 0);
    assert_eq!(
        text(&output.stdout),
        "3 cases, 4 evaluations: 4 passed, 0 failed\n"
    );
}

#[test]
fn mismatches_are_complete_and_human_rows_are_escaped() {
    let suite = r#"format_version = 1
cases = [
  { id = "reason\nfirst", name = "quoted \"reason\"", request = { kind = "bash", command = "deny" }, expect = "denied", reason = "" },
  { id = "pass", name = "passing row", request = { kind = "bash", command = "allow" }, expect = "allowed" },
  { id = "absent-reason", name = "denial without exact reason", request = { kind = "bash", command = "deny" }, expect = "denied" },
  { id = "late", name = "modified is always a v1 mismatch", request = { kind = "bash", command = "rewrite" }, expect = "allowed" },
]
"#;
    let policy = r#"bash_rules = [
  { name = "deny", pattern = "^deny$", action = { type = "Deny", value = "actual reason\nnext" } },
  { name = "allow", pattern = "^allow$", action = { type = "Allow" } },
  { name = "rewrite", pattern = "^rewrite$", action = { type = "Modify", value = "echo safe" } },
]
"#;
    let fixture = Fixture::new(suite, policy);

    let output = run(fixture.rules(true));
    assert_status(&output, 1);
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["passed_count"], 2);
    assert_eq!(report["failed_count"], 2);
    assert_eq!(report["rows"].as_array().unwrap().len(), 4);
    assert_eq!(report["rows"][0]["expected_reason"], "");
    assert_eq!(report["rows"][0]["actual"]["reason"], "actual reason\nnext");
    assert!(report["rows"][2].get("expected_reason").is_none());
    assert_eq!(report["rows"][3]["actual"]["result"], "modified");
    assert_eq!(report["rows"][3]["actual"]["rewrite"], "echo safe");

    let output = run(fixture.rules(false));
    assert_status(&output, 1);
    let stdout = text(&output.stdout);
    assert!(
        stdout.contains("FAIL \"reason\\nfirst\" (\"quoted \\\"reason\\\"\")"),
        "{stdout}"
    );
    assert!(stdout.contains("expected reason: \"\""), "{stdout}");
    assert!(stdout.contains("actual reason\\nnext"), "{stdout}");
    assert!(stdout.contains("\"rewrite\":\"echo safe\""), "{stdout}");
    assert!(!stdout.contains("passing row"), "{stdout}");
    assert!(stdout.ends_with("4 cases, 4 evaluations: 2 passed, 2 failed\n"));
}

const BASE_SUITE: &str = r#"format_version = 1
cases = [{ id = "ok", name = "ok", request = { kind = "bash", command = "echo" }, expect = "no_match" }]
"#;

#[test]
fn argument_errors_exit_two_without_reports() {
    let fixture = Fixture::new(BASE_SUITE, "");
    for args in [
        vec!["test", "rules", "suite.toml"],
        vec!["test", "rules", "-c", "policy.toml"],
        vec![
            "test",
            "rules",
            "suite.toml",
            "-c",
            "policy.toml",
            "--explain",
        ],
    ] {
        let mut command = fixture.command();
        command.args(args);
        let output = run(command);
        assert_status(&output, 2);
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn input_and_compilation_failures_exit_two_without_reports() {
    let fixture = Fixture::new(BASE_SUITE, "");
    let cases = [
        ("missing-suite.toml", "policy.toml", None, None),
        (
            "malformed-suite.toml",
            "policy.toml",
            Some("not toml"),
            None,
        ),
        (
            "version-suite.toml",
            "policy.toml",
            Some("format_version = 2\ncases = []"),
            None,
        ),
        ("suite.toml", "missing-policy.toml", None, None),
        (
            "suite.toml",
            "malformed-policy.toml",
            None,
            Some("[[bash_rules"),
        ),
        (
            "late-suite.toml",
            "policy.toml",
            Some(
                r#"format_version = 1
cases = [
  { id = "first", name = "first", request = { kind = "bash", command = "echo" }, expect = "no_match" },
  { id = "late", name = "late invalid", request = { kind = "tool", tool = "Read", input_json = "{" }, expect = "no_match" },
]
"#,
            ),
            None,
        ),
        (
            "suite.toml",
            "bad-bash.toml",
            None,
            Some(
                r#"bash_rules = [{ name = "bad-bash", pattern = "[", action = { type = "Allow" } }]"#,
            ),
        ),
        (
            "suite.toml",
            "bad-tool.toml",
            None,
            Some(
                r#"tool_rules = [{ name = "bad-tool", tool = "Read", field = "path", action = { type = "Allow" } }]"#,
            ),
        ),
    ];
    for (suite_name, policy_name, suite_source, policy_source) in cases {
        if let Some(source) = suite_source {
            fs::write(fixture.root.path().join(suite_name), source).unwrap();
        }
        if let Some(source) = policy_source {
            fs::write(fixture.root.path().join(policy_name), source).unwrap();
        }
        let output = run(fixture.rules_with(suite_name, policy_name, false));
        assert_status(&output, 2);
        assert!(output.stdout.is_empty(), "{}", text(&output.stdout));
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn invalid_default_cwd_exits_two_without_a_report() {
    let fixture = Fixture::new(BASE_SUITE, "");
    let mut command = fixture.rules(false);
    command.args(["--cwd", "missing-directory"]);
    let output = run(command);
    assert_status(&output, 2);
    assert!(output.stdout.is_empty());
}

#[test]
fn successful_reports_never_execute_bash_redirects_or_tool_requests() {
    let fixture = Fixture::new("format_version = 1\ncases = []", "");
    let sentinel = fixture.root.path().join("sentinel");
    let absent = fixture.root.path().join("absent");
    fs::write(&sentinel, "original").unwrap();
    fs::write(
        &fixture.policy,
        r#"bash_rules = [
  { name = "allow-rm", pattern = "^rm ", action = { type = "Allow" } },
  { name = "allow-printf", pattern = "^printf ", action = { type = "Allow" } },
  { name = "allow-touch", pattern = "^touch ", action = { type = "Allow" } },
  { name = "allow-output", pattern = ".*", action = { type = "AllowRedirect" } },
]
tool_rules = [{ name = "allow-write", tool = "Write", action = { type = "Allow" } }]
"#,
    )
    .unwrap();
    fs::write(
        &fixture.suite,
        format!(
            r#"format_version = 1
cases = [
  {{ id = "remove", name = "destructive-looking command", request = {{ kind = "bash", command = "rm {}" }}, expect = "allowed" }},
  {{ id = "redirect", name = "output redirect", request = {{ kind = "bash", command = "printf changed > {}" }}, expect = "allowed" }},
  {{ id = "touch", name = "absent target", request = {{ kind = "bash", command = "touch {}" }}, expect = "allowed" }},
  {{ id = "write", name = "write-like tool", request = {{ kind = "tool", tool = "Write", input_json = '{{"file_path":"{}","content":"changed"}}' }}, expect = "allowed" }},
]
"#,
            sentinel.display(),
            sentinel.display(),
            absent.display(),
            absent.display()
        ),
    )
    .unwrap();

    for json in [false, true] {
        let output = run(fixture.rules(json));
        assert_status(&output, 0);
        assert_eq!(fs::read_to_string(&sentinel).unwrap(), "original");
        assert!(!absent.exists());
    }
}

const PARITY_SUITE: &str = r#"format_version = 1
cases = [
  { id = "mode-less", name = "mode-less differs from default", request = { kind = "bash", command = "default-only" }, expect = "no_match" },
  { id = "default", name = "explicit default", request = { kind = "bash", command = "default-only" }, modes = ["default"], expect = "allowed" },
  { id = "compound", name = "compound allow", request = { kind = "bash", command = "echo hi && true" }, modes = ["default"], expect = "allowed" },
  { id = "filter", name = "argument filter", request = { kind = "bash", command = "cargo doc --open" }, modes = ["default"], expect = "allowed" },
  { id = "tool-deny", name = "exact tool denial", request = { kind = "tool", tool = "Write", input_json = '{"file_path":"x"}' }, modes = ["default"], expect = "denied", reason = "blocked exactly" },
  { id = "pass-through", name = "non-Bash pass-through", request = { kind = "tool", tool = "Read", input_json = '{}' }, modes = ["default"], expect = "no_match" },
]
"#;
const PARITY_POLICY: &str = r#"bash_rules = [
  { name = "default-only", pattern = "^default-only$", modes = ["default"], action = { type = "Allow" } },
  { name = "echo", pattern = "^echo ", action = { type = "Allow" } },
  { name = "true", pattern = "^true$", action = { type = "Allow" } },
  { name = "filter-doc", pattern = "^cargo doc --open$", action = { type = "ArgumentFilter", remove = ["--open"], reason = "removed" } },
  { name = "allow-doc", pattern = "^cargo doc$", action = { type = "Allow" } },
]
tool_rules = [{ name = "deny-write", tool = "Write", action = { type = "Deny", value = "blocked exactly" } }]
"#;

fn parity_fixture() -> Fixture {
    let fixture = Fixture::new(PARITY_SUITE, PARITY_POLICY);
    fixture.install_policy(PARITY_POLICY);
    fixture
}

#[test]
fn suite_rows_preserve_bounded_parity_cases() {
    let fixture = parity_fixture();
    let output = run(fixture.rules(true));
    assert_status(&output, 0);
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let rows = &report["rows"];
    assert_eq!(rows[0]["actual"]["result"], "no_match");
    assert_eq!(rows[1]["actual"]["result"], "allowed");
    assert_eq!(rows[2]["actual"]["result"], "allowed");
    assert_eq!(rows[3]["actual"]["result"], "argument_filtered");
    assert_eq!(rows[3]["actual"]["rewrite"], "cargo doc");
    assert_eq!(rows[4]["actual"]["result"], "denied");
    assert_eq!(rows[5]["actual"]["result"], "no_match");
}

#[test]
fn compound_and_filter_results_match_one_shot_bash_testing() {
    let fixture = parity_fixture();
    for (command_text, expected) in [
        ("echo hi && true", "allowed"),
        ("cargo doc --open", "argument_filtered"),
    ] {
        let mut command = fixture.command();
        command
            .args(["test", "bash-rules", command_text, "-c"])
            .arg(&fixture.policy)
            .args(["--cwd"])
            .arg(fixture.root.path())
            .args(["--mode", "default", "--json"]);
        let output = run(command);
        assert_status(&output, 0);
        let actual: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(actual["result"], expected);
    }
}

#[test]
fn bash_tool_and_passthrough_results_match_live_hook_meanings() {
    let fixture = parity_fixture();
    let hook_cases = [
        (
            "Bash",
            json!({"command": "echo hi && true"}),
            Some("allow"),
            None,
        ),
        (
            "Bash",
            json!({"command": "cargo doc --open"}),
            Some("allow"),
            Some("cargo doc"),
        ),
        ("Write", json!({"file_path": "x"}), Some("deny"), None),
        ("Read", json!({}), None, None),
    ];
    for (tool, input, expected, expected_rewrite) in hook_cases {
        let mut command = fixture.command();
        command.args(["hooks", "exec"]);
        let output = run_with_stdin(
            command,
            &hook_event(fixture.root.path(), "default", tool, input),
        );
        assert_status(&output, 0);
        let actual: Value = serde_json::from_slice(&output.stdout).unwrap();
        let pretool = actual.get("hookSpecificOutput");
        assert_eq!(
            pretool.and_then(|value| value["permissionDecision"].as_str()),
            expected
        );
        if tool == "Write" {
            assert_eq!(
                pretool.unwrap()["permissionDecisionReason"],
                "blocked exactly"
            );
        }
        if let Some(expected_rewrite) = expected_rewrite {
            assert_eq!(
                pretool.unwrap()["updatedInput"]["command"],
                expected_rewrite
            );
        }
    }
}
