//! Tests for hooks module

use std::{fs, io::Cursor};

use serde_json::Value;
use tempfile::TempDir;

use super::*;
use crate::test_helpers::{setup_isolated_xdg_config, setup_isolated_xdg_state};

async fn setup_user_bash_rules(rules_toml: &str) -> TempDir {
    let temp_dir = setup_isolated_xdg_config();

    let moriarty_dir = temp_dir.path().join("moriarty");
    tokio::fs::create_dir_all(&moriarty_dir).await.unwrap();
    tokio::fs::write(moriarty_dir.join("tool_rules.toml"), rules_toml)
        .await
        .unwrap();

    temp_dir
}

/// Creates a temp dir with `.config/tools.toml` and sets CLAUDE_PROJECT_DIR.
fn setup_project_with_config(toml_content: &str) -> TempDir {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path().join(".config");
    std::fs::create_dir(&config_dir).expect("Failed to create .config dir");
    std::fs::write(config_dir.join("tools.toml"), toml_content)
        .expect("Failed to write tools.toml");
    std::env::set_var("CLAUDE_PROJECT_DIR", temp_dir.path());
    temp_dir
}

/// A cross-platform `(command, args)` pair whose exit code matches `exit_ok`.
///
/// On Unix uses `true`/`false`; on Windows uses `cmd /c exit 0|1`. The string
/// data is `'static`, so callers can copy the references freely without
/// duplicating the underlying bytes (the `Vec<&str>` container itself still
/// allocates, as always).
fn check_cmd(exit_ok: bool) -> (&'static str, Vec<&'static str>) {
    #[cfg(unix)]
    {
        if exit_ok {
            ("true", vec![])
        } else {
            ("false", vec![])
        }
    }
    #[cfg(windows)]
    {
        if exit_ok {
            ("cmd", vec!["/c", "exit 0"])
        } else {
            ("cmd", vec!["/c", "exit 1"])
        }
    }
}

/// Runs `handle_stop_hook` for a project set up with the given checks (all approved).
/// Returns the temp dir (kept alive by caller) and the hook result.
async fn run_stop_hook_with_checks(checks: Vec<(&str, &str, Vec<&str>)>) -> (TempDir, HookOutput) {
    let temp_dir = setup_approved_project_with_checks(checks).await;
    std::env::set_var("CLAUDE_PROJECT_DIR", temp_dir.path());
    let result = handle_stop_hook().await.expect("Should succeed");
    (temp_dir, result)
}

/// Assert a stop-hook result is `Block` and the reason contains each substring.
/// Also verifies that `system_message == reason` (the public contract).
fn assert_stop_blocked_with(result: &HookOutput, substrs: &[&str]) {
    assert_eq!(result.decision, Some(HookDecision::Block));
    let reason = result.reason.as_ref().expect("reason should be set");
    for substr in substrs {
        assert!(
            reason.contains(substr),
            "reason {reason:?} should contain {substr:?}"
        );
    }
    assert_eq!(
        result.system_message, result.reason,
        "system_message should match reason for user feedback"
    );
}

/// Extract the PreToolUseOutput from a HookOutput, panicking if not present
fn unwrap_pretool_output(result: &HookOutput) -> &PreToolUseOutput {
    match &result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => output,
        _ => panic!("Expected PreToolUse hook specific output"),
    }
}

fn assert_pretool_allow(result: &HookOutput) {
    let output = unwrap_pretool_output(result);
    assert_eq!(output.permission_decision, Some(PermissionDecision::Allow));
    assert_eq!(output.updated_input, None);
}

fn assert_pretool_deny(result: &HookOutput, reason_contains: &str) {
    let output = unwrap_pretool_output(result);
    assert_eq!(output.permission_decision, Some(PermissionDecision::Deny));
    assert!(
        output
            .permission_decision_reason
            .as_ref()
            .expect("Deny decision should always carry a reason")
            .contains(reason_contains),
        "Expected reason to contain '{}', got: {:?}",
        reason_contains,
        output.permission_decision_reason
    );
}

fn assert_pretool_ask(result: &HookOutput) {
    let output = unwrap_pretool_output(result);
    assert_eq!(output.permission_decision, Some(PermissionDecision::Ask));
    assert_eq!(output.permission_decision_reason, None);
    assert_eq!(output.updated_input, None);
}

async fn run_bash_hook(config: &str, command: &str) -> HookOutput {
    let _xdg_config = setup_user_bash_rules(config).await;
    let tool_input = serde_json::json!({"command": command});
    handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed")
}

async fn run_pretool_hook(
    config: &str,
    tool: &str,
    input: &serde_json::Value,
    cwd: &str,
) -> HookOutput {
    let _xdg_config = setup_user_bash_rules(config).await;
    handle_pretool_hook(tool, input, cwd)
        .await
        .expect("Should succeed")
}

/// Variant-test wrapper for `assert_pretool_modified` that threads a case label
/// into every assertion failure.
fn assert_pretool_modified_case(result: &HookOutput, expected_command: &str, label: &str) {
    let output = unwrap_pretool_output(result);
    assert_eq!(
        output.permission_decision,
        Some(PermissionDecision::Allow),
        "case {label}"
    );
    let updated = output
        .updated_input
        .as_ref()
        .expect("Should have updated input");
    assert_eq!(
        updated["command"],
        serde_json::Value::String(expected_command.to_string()),
        "case {label}"
    );
}

/// Variant-test wrapper for `assert_pretool_modified_contains` that keeps the
/// case label visible in fragment-match failures.
fn assert_pretool_modified_contains_case(
    result: &HookOutput,
    expected_fragments: &[&str],
    label: &str,
) {
    let output = unwrap_pretool_output(result);
    assert_eq!(
        output.permission_decision,
        Some(PermissionDecision::Allow),
        "case {label}"
    );
    let command = output
        .updated_input
        .as_ref()
        .expect("Expected updated input")["command"]
        .as_str()
        .expect("Expected command string");

    for fragment in expected_fragments {
        assert!(
            command.contains(fragment),
            "case {label}: expected command {command:?} to contain {fragment:?}"
        );
    }
}

/// Asserts a deprecated `decision` enum value is rejected during JSON parsing
/// and that serde surfaces the invalid value in its error text.
fn assert_invalid_decision_value(decision: &str) {
    let err = serde_json::from_str::<HookOutput>(&format!(r#"{{"decision": "{decision}"}}"#))
        .expect_err("deprecated decision value should fail to parse");
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("unknown variant") || err_msg.contains(decision),
        "Error should mention invalid decision value {decision:?}, got: {err_msg}"
    );
}

/// Sets up isolated XDG state, then runs `setup` so callers can configure
/// `CLAUDE_PROJECT_DIR`; the returned `TempDir` guard is held alive until the
/// stop hook has been asserted as approved.
async fn assert_stop_approved_case(label: &str, setup: impl FnOnce() -> Option<TempDir>) {
    let _xdg_dir = setup_isolated_xdg_state();
    let _guard = setup();
    let result = run_stop_hook().await;
    assert_eq!(
        result.decision,
        Some(HookDecision::Approve),
        "case: {label}"
    );
    assert_eq!(result.reason, None, "case: {label}");
}

/// Build a single-entry `[[tool_rules]]` TOML fragment for the given fields.
///
/// `name` and `tool` are inserted into TOML *literal* strings (single-quoted),
/// so they must not contain single quotes. Pass `action_toml` exactly as it
/// should appear after `action = ` (e.g. `r#"{ type = "Allow" }"#`). Used to
/// collapse test configs that differ only in the tool name and action.
fn cfg_tool_rule(name: &str, tool: &str, action_toml: &str) -> String {
    format!("[[tool_rules]]\nname = '{name}'\ntool = '{tool}'\naction = {action_toml}\n")
}

/// Build a single-entry `[[bash_rules]]` TOML fragment for the given fields.
///
/// `pattern` is inserted into a TOML *literal* string (single-quoted) so regex
/// backslashes are passed through verbatim without TOML-level escaping. The
/// pattern must therefore not contain single quotes. `action_toml` is the
/// value after `action = ` (e.g. `r#"{ type = "Ask" }"#`).
fn cfg_bash_rule(name: &str, pattern: &str, action_toml: &str) -> String {
    format!("[[bash_rules]]\nname = '{name}'\npattern = '{pattern}'\naction = {action_toml}\n")
}

/// Build an `[[tool_rules]]` fragment with `allow_local = true` plus a
/// field+pattern constraint and an action. Used by the allow-local integration
/// tests that differ only in which field/pattern they assert on.
fn cfg_allow_local_rule(tool: &str, field: &str, pattern: &str, action_toml: &str) -> String {
    format!(
        "[[tool_rules]]\nname = 'allow-local-test'\ntool = '{tool}'\nallow_local = true\nfield = '{field}'\npattern = '{pattern}'\naction = {action_toml}\n",
    )
}

/// Runs `handle_stop_hook` and unwraps the result. Assumes callers have already
/// set up `CLAUDE_PROJECT_DIR` and any required config.
async fn run_stop_hook() -> HookOutput {
    handle_stop_hook().await.expect("Should succeed")
}

/// Runs a Bash pretool hook against an empty XDG config dir (no rules file),
/// with a default `ls -la` command. Used by the "no config" passthrough tests.
async fn run_bash_hook_empty_xdg() -> HookOutput {
    let _xdg_config = setup_isolated_xdg_config();
    let tool_input = serde_json::json!({"command": "ls -la"});
    handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed")
}

/// Runs `exec_hook_impl` with the given JSON input and expects success. Used by
/// the PreToolUse passthrough tests that share a literal event payload.
async fn run_exec_hook_expect_ok(input: &str, ctx: &str) {
    let cursor = Cursor::new(input);
    exec_hook_impl(cursor).await.expect(ctx);
}

async fn read_hooks_log_file() -> String {
    let log_path = crate::hooks::tracing::get_current_log_path()
        .await
        .expect("Log path should resolve");
    let log_dir = log_path.parent().expect("Log path should have parent");

    let log_entry = fs::read_dir(log_dir)
        .expect("Log directory should be readable")
        .filter_map(|entry| entry.ok())
        .find(|entry| entry.file_name().to_string_lossy().starts_with("hooks.log"))
        .expect("Hooks log file should exist");

    fs::read_to_string(log_entry.path()).expect("Hooks log file should be readable")
}

#[tokio::test]
async fn test_exec_hook_empty_input_returns_error() {
    let _xdg_dir = setup_isolated_xdg_state();

    let input = "";
    let cursor = Cursor::new(input);
    let err = exec_hook_impl(cursor)
        .await
        .expect_err("Should fail with empty input");
    assert!(err.to_string().contains("No input received"));
}

#[tokio::test]
async fn test_exec_hook_malformed_json_returns_error() {
    let _xdg_dir = setup_isolated_xdg_state();

    let input = "{invalid json";
    let cursor = Cursor::new(input);
    let err = exec_hook_impl(cursor)
        .await
        .expect_err("Should fail with malformed JSON");
    assert!(err.to_string().contains("Failed to parse hook input"));
}

#[tokio::test]
async fn test_exec_hook_valid_input_succeeds() {
    let _xdg_dir = setup_isolated_xdg_state();

    let input = r#"{
        "session_id": "test-session",
        "transcript_path": "/tmp/transcript.json",
        "cwd": "/tmp/project",
        "permission_mode": "default",
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "echo test"}
    }"#;

    let cursor = Cursor::new(input);
    let result = exec_hook_impl(cursor).await;

    result.unwrap(); // Panics with full error details if it fails
}

#[tokio::test]
async fn test_exec_hook_pretool_completion_log_includes_tool_context() {
    let _xdg_state = setup_isolated_xdg_state();
    let _xdg_config = setup_isolated_xdg_config();

    let input = r#"{
        "session_id": "test-session",
        "transcript_path": "/tmp/transcript.json",
        "cwd": "/tmp/project",
        "permission_mode": "default",
        "hook_event_name": "PreToolUse",
        "tool_name": "Read",
        "tool_input": {
            "file_path": "/tmp/foo.rs",
            "api_key": "secret-value",
            "command": "echo secret-value"
        }
    }"#;

    exec_hook_impl(Cursor::new(input))
        .await
        .expect("PreToolUse Read event should succeed");

    let content = read_hooks_log_file().await;
    let completion_event = content
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|event| {
            event.pointer("/fields/message").and_then(Value::as_str)
                == Some("PreToolUse hook completed")
        })
        .expect("PreToolUse completion log event should be present");
    let fields = completion_event
        .get("fields")
        .expect("Tracing JSON event should contain fields");

    assert_eq!(
        fields.get("tool_name").and_then(Value::as_str),
        Some("Read")
    );

    let tool_args = fields
        .get("tool_args")
        .and_then(Value::as_str)
        .expect("tool_args should be logged as a string field");
    let parsed_tool_args: Value =
        serde_json::from_str(tool_args).expect("tool_args should be valid JSON text");

    assert_eq!(
        parsed_tool_args.get("file_path").and_then(Value::as_str),
        Some("/tmp/foo.rs")
    );
    assert_eq!(
        parsed_tool_args.get("api_key").and_then(Value::as_str),
        Some("secret-value")
    );
    assert_eq!(
        parsed_tool_args.get("command").and_then(Value::as_str),
        Some("echo secret-value")
    );
    assert!(tool_args.contains("secret-value"));
    assert!(
        fields.get("hook_output").is_some(),
        "hook_output should remain present on completion log events"
    );
}

#[test]
fn test_tool_args_for_log_truncates_large_input() {
    let mut paths = serde_json::Map::new();
    for index in 0..20 {
        paths.insert(
            format!("file_{index}_path"),
            Value::String(format!(
                "/tmp/{}",
                "x".repeat(SAFE_LOG_STRING_TRUNCATE_SIZE + 100)
            )),
        );
    }
    let tool_input = Value::Object(paths);
    let full_input = serde_json::to_string(&tool_input).expect("Tool input should serialize");

    let tool_args = tool_args_for_log(&tool_input);

    assert!(tool_args.len() < full_input.len());
    assert!(tool_args.contains("... [truncated "));
}

#[tokio::test]
async fn test_exec_hook_large_input_within_limit() {
    let _xdg_dir = setup_isolated_xdg_state();

    // Create valid input close to but under 1MB limit to verify we don't crash
    let large_command = "x".repeat(1024 * 900); // 900KB of padding
    let input = format!(
        r#"{{"session_id":"test","transcript_path":"/t","cwd":"/c","permission_mode":"default","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{{"command":"{}"}}}}"#,
        large_command
    );

    let cursor = Cursor::new(input);
    let result = exec_hook_impl(cursor).await;

    // Should succeed - large but valid input
    result.unwrap();
}

#[tokio::test]
async fn test_exec_hook_truncates_long_invalid_input() {
    let _xdg_dir = setup_isolated_xdg_state();

    // Create 1000-byte invalid JSON
    let input = format!("{{\"invalid\": \"{}", "x".repeat(1000));

    let cursor = Cursor::new(input);

    // Should return error without panicking
    let err = exec_hook_impl(cursor)
        .await
        .expect_err("Should fail with large invalid JSON");
    assert!(err.to_string().contains("Failed to parse hook input"));
}

#[tokio::test]
async fn test_xdg_isolation_verification() {
    let temp_dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_STATE_HOME", temp_dir.path());

    // Verify the xdg crate sees our isolated directory
    use crate::persistence::FileType;
    let log_path = FileType::State.build_path("hooks/hooks.log").await.unwrap();

    assert!(
        log_path.starts_with(temp_dir.path()),
        "Log path {:?} should be under temp dir {:?}",
        log_path,
        temp_dir.path()
    );
}

#[tokio::test]
async fn test_exec_hook_all_event_types() {
    let _xdg_dir = setup_isolated_xdg_state();

    // Prevent infinite recursion: if this test is running inside a hook's test check,
    // the Stop event would try to run checks again, including this test, causing deadlock.
    // Clear CLAUDE_PROJECT_DIR to disable check execution in the Stop hook.
    std::env::remove_var("CLAUDE_PROJECT_DIR");

    let test_cases = vec![
        r#"{"session_id":"s","transcript_path":"/t","cwd":"/c","permission_mode":"default","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"ls"}}"#,
        r#"{"session_id":"s","transcript_path":"/t","cwd":"/c","permission_mode":"default","hook_event_name":"PostToolUse","tool_name":"Bash","tool_input":{},"tool_response":""}"#,
        r#"{"session_id":"s","transcript_path":"/t","cwd":"/c","permission_mode":"default","hook_event_name":"SessionStart","matcher":"startup"}"#,
        r#"{"session_id":"s","transcript_path":"/t","cwd":"/c","permission_mode":"default","hook_event_name":"SessionEnd","reason":"logout"}"#,
        r#"{"session_id":"s","transcript_path":"/t","cwd":"/c","permission_mode":"default","hook_event_name":"Stop"}"#,
    ];

    for json in test_cases {
        let cursor = Cursor::new(json);
        let result = exec_hook_impl(cursor).await;
        result.unwrap_or_else(|e| panic!("Failed for input {}: {}", json, e));
    }
}

#[test]
fn test_hook_output_rejects_old_decision_values() {
    for decision in ["allow", "deny"] {
        assert_invalid_decision_value(decision);
    }

    // 'ask' is now a valid decision value
    let ask_decision = r#"{"decision": "ask"}"#;
    let output = serde_json::from_str::<HookOutput>(ask_decision)
        .expect("Should accept 'ask' decision value");
    assert_eq!(output.decision, Some(HookDecision::Ask));
}

#[tokio::test]
async fn test_stop_hook_no_env_var() {
    let _xdg_dir = setup_isolated_xdg_state();

    // Ensure CLAUDE_PROJECT_DIR is not set
    std::env::remove_var("CLAUDE_PROJECT_DIR");

    let result = handle_stop_hook().await.expect("Should succeed");

    assert_eq!(result.decision, Some(HookDecision::Approve));
    assert_eq!(result.reason, None);
}

#[tokio::test]
async fn test_stop_hook_approval_variants() {
    let cases = [
        (
            "no config file",
            Box::new(|| {
                let temp_dir = TempDir::new().expect("Failed to create temp dir");
                std::env::set_var("CLAUDE_PROJECT_DIR", temp_dir.path());
                Some(temp_dir)
            }) as Box<dyn FnOnce() -> Option<TempDir>>,
        ),
        (
            "no checks defined",
            Box::new(|| {
                Some(setup_project_with_config(
                    r#"
[commands]
lint = ["echo", "lint"]
"#,
                ))
            }),
        ),
        (
            // TOML parse fails on incomplete `[[checks]]` tables.
            // Tests the intentional fail-open behavior documented in the security model.
            "empty checks",
            Box::new(|| {
                Some(setup_project_with_config(
                    r#"
[commands]
lint = ["echo", "lint"]

[[checks]]
"#,
                ))
            }),
        ),
    ];

    for (label, setup) in cases {
        assert_stop_approved_case(label, setup).await;
    }
}

#[tokio::test]
async fn test_stop_hook_empty_command_array() {
    let _xdg_dir = setup_isolated_xdg_state();

    // Create project with check that has empty command array
    let _temp_dir = setup_project_with_config(
        r#"
[commands]
lint = ["echo", "lint"]

[[checks]]
name = "empty-command"
command = []
"#,
    );

    let result = handle_stop_hook().await.expect("Should succeed");

    assert_eq!(result.decision, Some(HookDecision::Block));
    let reason = result.reason.unwrap();
    assert!(
        reason.contains("empty-command"),
        "Expected check name in error. Reason: {}",
        reason
    );
    assert!(
        reason.contains("empty command array"),
        "Expected error about empty command. Reason: {}",
        reason
    );
    assert!(
        reason.contains("Expected format"),
        "Expected format guidance. Reason: {}",
        reason
    );
}

#[tokio::test]
async fn test_stop_hook_unapproved_check() {
    let _xdg_dir = setup_isolated_xdg_state();
    // Approval lookups go through `FileType::Config.load`, which falls back
    // to `$HOME/.config` when XDG_CONFIG_HOME is unset. Under the Nix sandbox
    // that resolves to `/homeless-shelter/.config` and `place_config_file`
    // returns `Permission denied`, so the assertion below never runs.
    let _xdg_config = setup_isolated_xdg_config();

    // Create project with a check
    let _temp_dir = setup_project_with_config(
        r#"
[commands]

[[checks]]
name = "test-check"
command = ["echo", "test"]
"#,
    );

    // Don't approve the project, so it should be denied
    let result = handle_stop_hook().await.expect("Should succeed");

    assert_eq!(result.decision, Some(HookDecision::Block));

    // Verify both reason and system_message contain the error
    let message = result.reason.as_ref().expect("reason should be set");
    assert!(message.contains("Check 'test-check' is not approved"));

    assert_eq!(
        result.system_message, result.reason,
        "system_message should match reason for user feedback"
    );
}

#[tokio::test]
async fn test_stop_hook_check_result_variants() {
    #[derive(Clone, Copy)]
    enum Case {
        ApprovedOne,
        ApprovedMany,
        Failed,
    }

    for case in [Case::ApprovedOne, Case::ApprovedMany, Case::Failed] {
        let _xdg_dir = setup_isolated_xdg_state();
        let _config_dir = setup_isolated_xdg_config();

        let label = match case {
            Case::ApprovedOne => "approved-one",
            Case::ApprovedMany => "approved-many",
            Case::Failed => "failed",
        };
        let checks = match case {
            Case::ApprovedOne => {
                let (command, args) = check_cmd(true);
                vec![("passing-check", command, args)]
            }
            Case::ApprovedMany => {
                let (command, args) = check_cmd(true);
                vec![("check1", command, args.clone()), ("check2", command, args)]
            }
            Case::Failed => {
                let (command, args) = check_cmd(false);
                vec![("failing-check", command, args)]
            }
        };

        let (_temp_dir, result) = run_stop_hook_with_checks(checks).await;
        match case {
            Case::ApprovedOne | Case::ApprovedMany => {
                assert_eq!(
                    result.decision,
                    Some(HookDecision::Approve),
                    "case: {label}"
                );
                assert_eq!(result.reason, None, "case: {label}");
            }
            Case::Failed => {
                assert_eq!(result.decision, Some(HookDecision::Block), "case: {label}");
                let reason = result.reason.as_ref().expect("reason should be set");
                for needle in ["Checks failed", "failing-check"] {
                    assert!(
                        reason.contains(needle),
                        "case {label}: {reason:?} should contain {needle:?}"
                    );
                }
                assert_eq!(result.system_message, result.reason, "case: {label}");
            }
        }
    }
}

#[tokio::test]
async fn test_stop_hook_check_binary_hash_mismatch() {
    let _xdg_dir = setup_isolated_xdg_state();
    let _config_dir = setup_isolated_xdg_config();
    let (c, a) = check_cmd(true);
    let temp_dir = setup_approved_project_with_checks(vec![("test-check", c, a)]).await;

    // Manually corrupt the binary hash in approvals
    let mut approvals = ProjectApprovals::load()
        .await
        .expect("Failed to load approvals");
    let canonical_dir = temp_dir
        .path()
        .canonicalize()
        .expect("Failed to canonicalize");
    let project_key = canonical_dir.to_string_lossy().to_string();

    if let Some(project) = approvals.projects.get_mut(&project_key) {
        if let Some(check_approval) = project.checks.get_mut("test-check") {
            check_approval.binary_hash = "corrupted_hash_value".to_string();
        } else {
            panic!("test-check not found in approvals");
        }
    } else {
        panic!("Project not found in approvals");
    }

    approvals
        .save()
        .await
        .expect("Failed to save corrupted approvals");

    std::env::set_var("CLAUDE_PROJECT_DIR", temp_dir.path());
    let result = handle_stop_hook().await.expect("Should succeed");
    assert_stop_blocked_with(&result, &["binary changed", "test-check"]);
}

/// Helper to set up an approved project with checks
///
/// Creates a project directory with `.config/tools.toml` containing the specified checks,
/// approves all checks, and returns the temp directory for cleanup.
async fn setup_approved_project_with_checks(
    check_specs: Vec<(&str, &str, Vec<&str>)>, // (name, command, args)
) -> TempDir {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path().join(".config");
    std::fs::create_dir(&config_dir).expect("Failed to create .config dir");

    // Build config content
    let mut config_content = String::from("[commands]\n\n");
    for (name, command, args) in &check_specs {
        let mut full_command = vec![command.to_string()];
        full_command.extend(args.iter().map(|s| s.to_string()));
        config_content.push_str(&format!(
            "[[checks]]\nname = \"{}\"\ncommand = {:?}\n\n",
            name, full_command
        ));
    }

    std::fs::write(config_dir.join("tools.toml"), &config_content)
        .expect("Failed to write tools.toml");

    // Set up approvals
    use crate::hashing;
    use crate::project_config::approvals::{CommandApproval, ProjectApproval, ProjectApprovals};
    use crate::project_config::resolve_binary_path_with_original;

    let canonical_dir = temp_dir.path().canonicalize().unwrap();
    let tools_config_hash = hashing::hash_string(&config_content);

    let mut check_approvals = std::collections::HashMap::new();
    for (name, command, _args) in check_specs {
        let (original_path, canonical_path) =
            resolve_binary_path_with_original(command, &canonical_dir).unwrap();
        let binary_hash = hashing::hash_file(&canonical_path).await.unwrap();

        check_approvals.insert(
            name.to_string(),
            CommandApproval {
                original_path: original_path.to_string_lossy().to_string(),
                canonical_path: canonical_path.to_string_lossy().to_string(),
                binary_hash,
            },
        );
    }

    let mut approvals = ProjectApprovals::default();
    approvals.projects.insert(
        canonical_dir.to_string_lossy().to_string(),
        ProjectApproval {
            tools_config_hash,
            last_approved: chrono::Utc::now(),
            commands: std::collections::HashMap::new(),
            checks: check_approvals,
        },
    );

    approvals.save().await.expect("Failed to save approvals");

    temp_dir
}

#[tokio::test]
async fn test_bash_hook_ask_variants() {
    #[derive(Clone, Copy)]
    enum Case {
        NoUserConfig,
        NoConfigFile,
        NoRulesConfigured,
        RuleAsks,
    }

    for case in [
        Case::NoUserConfig,
        Case::NoConfigFile,
        Case::NoRulesConfigured,
        Case::RuleAsks,
    ] {
        let label = match case {
            Case::NoUserConfig => "no-user-config",
            Case::NoConfigFile => "no-config-file",
            Case::NoRulesConfigured => "no-rules-configured",
            Case::RuleAsks => "rule-asks",
        };
        let result = match case {
            Case::NoUserConfig | Case::NoConfigFile => run_bash_hook_empty_xdg().await,
            Case::NoRulesConfigured => run_bash_hook("# Empty config file\n", "ls -la").await,
            Case::RuleAsks => {
                let config = cfg_bash_rule("ask-docker", "^docker", r#"{ type = "Ask" }"#);
                run_bash_hook(&config, "docker build .").await
            }
        };
        let output = unwrap_pretool_output(&result);
        assert_eq!(
            output.permission_decision,
            Some(PermissionDecision::Ask),
            "case {label}"
        );
        assert_eq!(output.permission_decision_reason, None, "case {label}");
        assert_eq!(output.updated_input, None, "case {label}");
    }
}

#[tokio::test]
async fn test_bash_hook_missing_command_field() {
    let _xdg_config = setup_isolated_xdg_config();

    let tool_input = serde_json::json!({});
    let err_msg = handle_bash_pretool_hook(&tool_input)
        .await
        .expect_err("Should fail with missing command field")
        .to_string();
    assert!(
        err_msg.contains("Missing 'command' field"),
        "Error should mention missing field, got: {}",
        err_msg
    );
}

#[tokio::test]
async fn test_bash_hook_rule_outcome_variants() {
    enum Expect {
        Allow,
        Ask,
        Deny(&'static str),
        Modify {
            expected_command: &'static str,
            reason_fragments: &'static [&'static str],
        },
    }

    let cases = [
        (
            "deny-rm-rf",
            cfg_bash_rule(
                "deny-rm-rf",
                r"^rm\s+-rf\s+/",
                r#"{ type = "Deny", value = "Dangerous recursive delete" }"#,
            ),
            "rm -rf /",
            Expect::Deny("Dangerous recursive delete"),
        ),
        (
            "modify-dry-run",
            cfg_bash_rule(
                "add-dry-run",
                r"^(docker\s+system\s+prune)",
                r#"{ type = "Modify", value = "$1 --dry-run" }"#,
            ),
            "docker system prune",
            Expect::Modify {
                expected_command: "docker system prune --dry-run",
                reason_fragments: &[
                    "modified by rule 'add-dry-run'",
                    "to: docker system prune --dry-run",
                ],
            },
        ),
        (
            "allow-ls",
            cfg_bash_rule("allow-ls", r"^ls($|\s)", r#"{ type = "Allow" }"#),
            "ls -la",
            Expect::Allow,
        ),
        (
            "no-match-falls-back-to-ask",
            cfg_bash_rule(
                "deny-rm",
                r"^rm\s",
                r#"{ type = "Deny", value = "rm denied" }"#,
            ),
            "ls -la",
            Expect::Ask,
        ),
    ];

    for (label, config, command, expected) in cases {
        let result = run_bash_hook(&config, command).await;
        match expected {
            Expect::Allow => {
                let output = unwrap_pretool_output(&result);
                assert_eq!(
                    output.permission_decision,
                    Some(PermissionDecision::Allow),
                    "case {label}"
                );
                assert_eq!(output.updated_input, None, "case {label}");
            }
            Expect::Ask => {
                let output = unwrap_pretool_output(&result);
                assert_eq!(
                    output.permission_decision,
                    Some(PermissionDecision::Ask),
                    "case {label}"
                );
                assert_eq!(output.permission_decision_reason, None, "case {label}");
                assert_eq!(output.updated_input, None, "case {label}");
            }
            Expect::Deny(reason) => {
                assert_eq!(
                    result.system_message,
                    Some(reason.to_string()),
                    "case {label}: system_message should be populated at top level for deny decisions"
                );
                let output = unwrap_pretool_output(&result);
                assert_eq!(
                    output.permission_decision,
                    Some(PermissionDecision::Deny),
                    "case {label}"
                );
                assert!(
                    output
                        .permission_decision_reason
                        .as_ref()
                        .expect("Deny decision should always carry a reason")
                        .contains(reason),
                    "case {label}"
                );
            }
            Expect::Modify {
                expected_command,
                reason_fragments,
            } => {
                let output = unwrap_pretool_output(&result);
                let reason = output
                    .permission_decision_reason
                    .as_ref()
                    .expect("Should have permission decision reason");
                for fragment in reason_fragments {
                    assert!(
                        reason.contains(fragment),
                        "case {label}: reason should include {fragment:?}: {reason}"
                    );
                }
                assert_pretool_modified_case(&result, expected_command, label);
            }
        }
    }
}

#[tokio::test]
async fn test_bash_hook_first_match_wins_with_multiple_rules() {
    let config = r#"
[[bash_rules]]
name = "specific-deny-rm-rf"
pattern = "^rm\\s+-rf"
action = { type = "Deny", value = "Dangerous rm -rf" }

[[bash_rules]]
name = "generic-allow-rm"
pattern = "^rm"
action = { type = "Allow" }
"#;
    let result = run_bash_hook(config, "rm -rf /").await;
    assert_pretool_deny(&result, "Dangerous rm -rf");

    let result = run_bash_hook(config, "rm file.txt").await;
    assert_pretool_allow(&result);
}

#[tokio::test]
async fn test_argument_filter_variants() {
    enum Expect {
        Ask,
        Deny(&'static str),
        Modified(&'static str),
        ModifiedContains(&'static [&'static str]),
    }

    let cases = [
        (
            "remove-open-flag",
            r#"
[[bash_rules]]
name = "filter-cargo-doc"
pattern = "^cargo doc.*--open"
action = { type = "ArgumentFilter", remove = ["--open"], reason = "Browser flag removed" }

[[bash_rules]]
name = "allow-cargo-doc"
pattern = "^cargo doc"
action = { type = "Allow" }
"#,
            "cargo doc --open --no-deps",
            Expect::Modified("cargo doc --no-deps"),
        ),
        (
            "filtered-command-now-denied",
            r#"
[[bash_rules]]
name = "filter-dangerous-rm"
pattern = "^rm .*-rf"
action = { type = "ArgumentFilter", remove = ["-rf"], reason = "Removed dangerous flag" }

[[bash_rules]]
name = "deny-rm-root"
pattern = "^rm.*/"
action = { type = "Deny", value = "Cannot delete root paths" }
"#,
            "rm -rf /",
            Expect::Deny("Cannot delete root paths"),
        ),
        (
            "filtered-command-falls-back-to-ask",
            r#"
[[bash_rules]]
name = "filter-cargo-doc"
pattern = "^cargo doc\\b"
action = { type = "ArgumentFilter", remove = ["--open"], reason = "Browser flag removed" }

[[bash_rules]]
name = "allow-cargo-build"
pattern = "^cargo build"
action = { type = "Allow" }
"#,
            "cargo doc --open",
            Expect::Ask,
        ),
        (
            "multiple-filters-no-final-allow",
            r#"
[[bash_rules]]
name = "filter-1"
pattern = "^cargo doc\\b"
action = { type = "ArgumentFilter", remove = ["--open"], reason = "First filter" }

[[bash_rules]]
name = "filter-2"
pattern = "^cargo doc\\b"
action = { type = "ArgumentFilter", add = ["--offline"], reason = "Second filter" }
"#,
            "cargo doc --open",
            Expect::Ask,
        ),
        (
            "add-safety-flags",
            r#"
[[bash_rules]]
name = "allow-docker-run-safe"
pattern = "^docker run .* --read-only"
action = { type = "Allow" }

[[bash_rules]]
name = "add-safety-flags"
pattern = "^docker run ubuntu$"
action = { type = "ArgumentFilter", add = ["--read-only", "--security-opt=no-new-privileges"], reason = "Added security flags" }
"#,
            "docker run ubuntu",
            Expect::ModifiedContains(&["--read-only", "--security-opt=no-new-privileges"]),
        ),
        (
            "replace-force-with-interactive",
            r#"
[[bash_rules]]
name = "allow-rm-interactive"
pattern = "^rm .* -i$"
action = { type = "Allow" }

[[bash_rules]]
name = "force-interactive-rm"
pattern = "^rm -f file\\.txt$"
action = { type = "ArgumentFilter", remove = ["-f", "--force"], add = ["-i"], reason = "Replaced force mode with interactive" }
"#,
            "rm -f file.txt",
            Expect::Modified("rm file.txt -i"),
        ),
        (
            "filter-then-modify",
            r#"
[[bash_rules]]
name = "filter-docker-safety"
pattern = "^docker run.*--privileged"
action = { type = "ArgumentFilter", remove = ["--privileged"], reason = "Removed privileged mode" }

[[bash_rules]]
name = "modify-docker-add-limits"
pattern = "^docker run"
action = { type = "Modify", value = "$0 --memory=1g" }
"#,
            "docker run --privileged ubuntu",
            Expect::Modified("docker run --memory=1g"),
        ),
        (
            "filter-then-ask",
            r#"
[[bash_rules]]
name = "filter-sudo-flags"
pattern = "^sudo.*--non-interactive"
action = { type = "ArgumentFilter", remove = ["--non-interactive"], reason = "Removed automation flag" }

[[bash_rules]]
name = "ask-for-sudo"
pattern = "^sudo"
action = { type = "Ask" }
"#,
            "sudo --non-interactive apt install vim",
            Expect::Ask,
        ),
    ];

    for (label, config, command, expected) in cases {
        let result = run_bash_hook(config, command).await;
        match expected {
            Expect::Ask => {
                let output = unwrap_pretool_output(&result);
                assert_eq!(
                    output.permission_decision,
                    Some(PermissionDecision::Ask),
                    "case {label}"
                );
                assert_eq!(output.permission_decision_reason, None, "case {label}");
                assert_eq!(output.updated_input, None, "case {label}");
            }
            Expect::Deny(reason) => {
                let output = unwrap_pretool_output(&result);
                assert_eq!(
                    output.permission_decision,
                    Some(PermissionDecision::Deny),
                    "case {label}"
                );
                assert!(
                    output
                        .permission_decision_reason
                        .as_ref()
                        .expect("deny reason")
                        .contains(reason),
                    "case {label}"
                );
            }
            Expect::Modified(expected_command) => {
                assert_pretool_modified_case(&result, expected_command, label)
            }
            Expect::ModifiedContains(fragments) => {
                assert_pretool_modified_contains_case(&result, fragments, label)
            }
        }
    }
}

// Tests for system_message field population

#[test]
fn test_deny_hook_serialization_format() {
    let output = deny_hook("Check failed");
    let json = serde_json::to_string(&output).expect("Failed to serialize");

    // Verify JSON structure matches Claude Code protocol
    assert!(
        json.contains(r#""decision":"block""#),
        "JSON should contain decision:block, got: {}",
        json
    );
    assert!(
        json.contains(r#""reason":"Check failed""#),
        "JSON should contain reason, got: {}",
        json
    );
    assert!(
        json.contains(r#""systemMessage":"Check failed""#),
        "JSON should contain systemMessage, got: {}",
        json
    );

    // Verify it round-trips correctly
    let parsed: HookOutput = serde_json::from_str(&json).expect("Failed to parse");
    assert_eq!(parsed.decision, Some(HookDecision::Block));
    assert_eq!(parsed.reason, Some("Check failed".to_string()));
    assert_eq!(parsed.system_message, Some("Check failed".to_string()));
}

#[test]
fn test_pretool_deny_hook_serialization_format() {
    let output = pretool_deny_hook("Command blocked".to_string());
    let json = serde_json::to_string(&output).expect("Failed to serialize");

    // Verify JSON structure for PreToolUse hooks
    assert!(
        json.contains(r#""systemMessage":"Command blocked""#),
        "JSON should contain systemMessage at top level, got: {}",
        json
    );
    assert!(
        json.contains(r#""permissionDecisionReason":"Command blocked""#),
        "JSON should contain permissionDecisionReason in hookSpecificOutput, got: {}",
        json
    );
    assert!(
        json.contains(r#""permissionDecision":"deny""#),
        "JSON should contain permissionDecision:deny, got: {}",
        json
    );

    // Verify it round-trips correctly
    let parsed: HookOutput = serde_json::from_str(&json).expect("Failed to parse");
    assert_eq!(parsed.system_message, Some("Command blocked".to_string()));

    let pretool = unwrap_pretool_output(&parsed);
    assert_eq!(pretool.permission_decision, Some(PermissionDecision::Deny));
    assert_eq!(
        pretool.permission_decision_reason,
        Some("Command blocked".to_string())
    );
}

#[test]
fn test_deny_hook_sets_system_message() {
    let reason = "Access denied: insufficient permissions";
    let output = deny_hook(reason);

    // Verify both reason and system_message are set to the same value
    assert_eq!(output.reason, Some(reason.to_string()));
    assert_eq!(output.system_message, Some(reason.to_string()));
    assert_eq!(output.decision, Some(HookDecision::Block));
}

#[test]
fn test_allow_hook_with_message_sets_system_message() {
    let message = "Approved by security rule";
    let output = allow_hook(Some(message.to_string()));

    assert_eq!(output.reason, Some(message.to_string()));
    assert_eq!(output.system_message, Some(message.to_string()));
    assert_eq!(output.decision, Some(HookDecision::Approve));
}

#[test]
fn test_allow_hook_without_message_omits_system_message() {
    let output = allow_hook(None);

    assert_eq!(output.reason, None);
    assert_eq!(output.system_message, None);
    assert_eq!(output.decision, Some(HookDecision::Approve));
}

/// Assert the embedded `PreToolUseOutput` carries the expected decision + optional
/// reason (matching both `permission_decision_reason` and the top-level
/// `system_message`).
fn assert_pretool_decision(
    output: &HookOutput,
    decision: PermissionDecision,
    reason: Option<&str>,
) {
    let pretool = unwrap_pretool_output(output);
    assert_eq!(pretool.permission_decision, Some(decision));
    let expected = reason.map(str::to_string);
    assert_eq!(pretool.permission_decision_reason, expected);
    assert_eq!(output.system_message, expected);
}

#[test]
fn test_pretool_deny_hook_sets_system_message() {
    let reason = "Command blocked by bash rules";
    let output = pretool_deny_hook(reason.to_string());
    // PreToolUse hooks use permission_decision_reason, not the outer `reason`.
    assert_eq!(output.reason, None);
    assert_pretool_decision(&output, PermissionDecision::Deny, Some(reason));
}

#[test]
fn test_pretool_allow_hook_variants() {
    let cases = [
        (
            Some("Allowed by whitelist rule"),
            Some("Allowed by whitelist rule"),
        ),
        (None, None),
    ];

    for (reason, expected_reason) in cases {
        let output = pretool_allow_hook(reason.map(ToString::to_string));
        assert_pretool_decision(&output, PermissionDecision::Allow, expected_reason);
    }
}

#[test]
fn test_pretool_modify_hook_sets_system_message() {
    let reason = "Command modified by security rule";
    let new_input = serde_json::json!({"command": "ls -la"});
    let output = pretool_modify_hook(new_input.clone(), Some(reason.to_string()));

    assert_eq!(output.system_message, Some(reason.to_string()));

    let pretool = unwrap_pretool_output(&output);
    assert_eq!(pretool.permission_decision_reason, Some(reason.to_string()));
    assert_eq!(pretool.updated_input, Some(new_input));
}

#[test]
fn test_pretool_modify_hook_without_reason_omits_system_message() {
    let new_input = serde_json::json!({"command": "ls -la"});
    let output = pretool_modify_hook(new_input.clone(), None);

    assert_eq!(output.system_message, None);

    let pretool = unwrap_pretool_output(&output);
    assert_eq!(pretool.permission_decision_reason, None);
    assert_eq!(pretool.updated_input, Some(new_input));
}

#[test]
fn test_pretool_ask_hook_omits_system_message() {
    let output = pretool_ask_hook();

    // Ask hooks don't provide feedback, so no system_message
    assert_eq!(output.system_message, None);
    assert_eq!(output.reason, None);

    let pretool = unwrap_pretool_output(&output);
    assert_eq!(pretool.permission_decision, Some(PermissionDecision::Ask));
}

// ===== Tool Rules integration tests =====

#[tokio::test]
async fn test_tool_rule_non_bash_outcome_variants() {
    enum Expect {
        Allow,
        Ask,
        Deny(&'static str),
    }

    let cases = [
        (
            "allow-read",
            cfg_tool_rule("allow-read", "Read", r#"{ type = "Allow" }"#),
            "Read",
            serde_json::json!({"file_path": "/tmp/foo.rs"}),
            Expect::Allow,
        ),
        (
            "deny-write",
            cfg_tool_rule(
                "deny-write",
                "Write",
                r#"{ type = "Deny", value = "Writes are blocked" }"#,
            ),
            "Write",
            serde_json::json!({"file_path": "/tmp/foo.rs", "content": "hello"}),
            Expect::Deny("Writes are blocked"),
        ),
        (
            "ask-edit",
            cfg_tool_rule("ask-edit", "Edit", r#"{ type = "Ask" }"#),
            "Edit",
            serde_json::json!({"file_path": "/tmp/foo.rs"}),
            Expect::Ask,
        ),
    ];

    for (label, config, tool, input, expected) in cases {
        let result = run_pretool_hook(&config, tool, &input, "").await;
        match expected {
            Expect::Allow => {
                let output = unwrap_pretool_output(&result);
                assert_eq!(
                    output.permission_decision,
                    Some(PermissionDecision::Allow),
                    "case {label}"
                );
                assert_eq!(output.updated_input, None, "case {label}");
            }
            Expect::Ask => {
                let output = unwrap_pretool_output(&result);
                assert_eq!(
                    output.permission_decision,
                    Some(PermissionDecision::Ask),
                    "case {label}"
                );
                assert_eq!(output.permission_decision_reason, None, "case {label}");
                assert_eq!(output.updated_input, None, "case {label}");
            }
            Expect::Deny(reason) => {
                assert_eq!(
                    result.system_message,
                    Some(reason.to_string()),
                    "case {label}"
                );
                let output = unwrap_pretool_output(&result);
                assert_eq!(
                    output.permission_decision,
                    Some(PermissionDecision::Deny),
                    "case {label}"
                );
                assert!(
                    output
                        .permission_decision_reason
                        .as_ref()
                        .expect("deny reason")
                        .contains(reason),
                    "case {label}"
                );
            }
        }
    }
}

#[tokio::test]
async fn test_tool_rule_with_field_pattern() {
    let config = r#"
[[tool_rules]]
name = "deny-env-write"
tool = "Write"
field = "file_path"
pattern = "\\.env$"
action = { type = "Deny", value = "Cannot write to .env files" }

[[tool_rules]]
name = "allow-write"
tool = "Write"
action = { type = "Allow" }
"#;
    // .env file should be denied
    let result = run_pretool_hook(
        config,
        "Write",
        &serde_json::json!({"file_path": "/home/user/.env", "content": "SECRET=x"}),
        "",
    )
    .await;
    assert_pretool_deny(&result, "Cannot write to .env files");

    // Non-.env file should be allowed
    let result = run_pretool_hook(
        config,
        "Write",
        &serde_json::json!({"file_path": "/home/user/main.rs", "content": "fn main() {}"}),
        "",
    )
    .await;
    assert_pretool_allow(&result);
}

#[tokio::test]
async fn test_tool_rules_checked_before_bash_rules_for_bash() {
    let config = r#"
[[tool_rules]]
name = "tool-rule-allows-bash"
tool = "Bash"
action = { type = "Allow" }

[[bash_rules]]
name = "deny-everything"
pattern = ".*"
action = { type = "Deny", value = "Everything denied" }
"#;
    // tool_rules should match first and Allow, even though bash_rules would deny
    let result = run_pretool_hook(
        config,
        "Bash",
        &serde_json::json!({"command": "rm -rf /"}),
        "",
    )
    .await;
    assert_pretool_allow(&result);
}

#[tokio::test]
async fn test_bash_rules_still_work_when_tool_rules_dont_match() {
    let config = r#"
[[tool_rules]]
name = "allow-read"
tool = "Read"
action = { type = "Allow" }

[[bash_rules]]
name = "allow-ls"
pattern = "^ls($|\\s)"
action = { type = "Allow" }

[[bash_rules]]
name = "deny-rm"
pattern = "^rm"
action = { type = "Deny", value = "rm not allowed" }
"#;
    // Bash tool: tool_rules don't match (only Read matches), falls through to bash_rules
    let result = run_pretool_hook(
        config,
        "Bash",
        &serde_json::json!({"command": "ls -la"}),
        "",
    )
    .await;
    assert_pretool_allow(&result);

    // rm should be denied by bash_rules
    let result = run_pretool_hook(
        config,
        "Bash",
        &serde_json::json!({"command": "rm file.txt"}),
        "",
    )
    .await;
    assert_pretool_deny(&result, "rm not allowed");
}

#[tokio::test]
async fn test_non_bash_tool_no_rules_returns_passthrough() {
    let _xdg_config = setup_isolated_xdg_config();

    let tool_input = serde_json::json!({"file_path": "/tmp/foo"});
    let result = handle_pretool_hook("Read", &tool_input, "")
        .await
        .expect("Should succeed");

    // When no tool rules match a non-Bash tool, all fields should be None
    // (serializes to `{}`, deferring to Claude Code's native permission system)
    assert_eq!(
        result.hook_specific_output, None,
        "Non-Bash tools with no rules should return passthrough (no hook_specific_output)"
    );
    assert_eq!(result.permission_decision, None);
    assert_eq!(result.decision, None);
}

#[tokio::test]
async fn test_non_bash_tool_no_matching_rule_returns_passthrough() {
    let config = cfg_tool_rule("allow-write", "Write", r#"{ type = "Allow" }"#);
    let _xdg_config = setup_user_bash_rules(&config).await;

    // Config exists with rules, but none match Read — should passthrough
    let result = handle_pretool_hook("Read", &serde_json::json!({"file_path": "/tmp/foo"}), "")
        .await
        .expect("Should succeed");

    assert_eq!(result.hook_specific_output, None);
    assert_eq!(result.permission_decision, None);
}

#[tokio::test]
async fn test_tool_rule_wildcard_matching() {
    let config = r#"
[[tool_rules]]
name = "allow-read"
tool = "Read"
action = { type = "Allow" }

[[tool_rules]]
name = "ask-everything-else"
tool = "*"
action = { type = "Ask" }
"#;
    // Read matches specific rule
    let result = run_pretool_hook(config, "Read", &serde_json::json!({}), "").await;
    assert_pretool_allow(&result);

    // Write matches wildcard
    let result = run_pretool_hook(config, "Write", &serde_json::json!({}), "").await;
    assert_pretool_ask(&result);

    // Bash also matches wildcard (before bash_rules run)
    let result = run_pretool_hook(
        config,
        "Bash",
        &serde_json::json!({"command": "echo hi"}),
        "",
    )
    .await;
    assert_pretool_ask(&result);
}

/// Canonical PreToolUse Read event payload used by the passthrough tests.
const PRETOOL_READ_EVENT: &str = r#"{
    "session_id": "test-session",
    "transcript_path": "/tmp/transcript.json",
    "cwd": "/tmp/project",
    "permission_mode": "default",
    "hook_event_name": "PreToolUse",
    "tool_name": "Read",
    "tool_input": {"file_path": "/tmp/foo.rs"}
}"#;

#[tokio::test]
async fn test_exec_hook_impl_non_bash_pretool_produces_output() {
    let _xdg_dir = setup_isolated_xdg_state();
    let _xdg_config = setup_isolated_xdg_config();

    // Non-Bash PreToolUse event should succeed and produce empty JSON output (passthrough).
    run_exec_hook_expect_ok(
        PRETOOL_READ_EVENT,
        "Non-Bash PreToolUse should succeed and produce output",
    )
    .await;
}

#[tokio::test]
async fn test_exec_hook_impl_non_bash_pretool_with_tool_rules() {
    let _xdg_dir = setup_isolated_xdg_state();

    let config = cfg_tool_rule("allow-read", "Read", r#"{ type = "Allow" }"#);
    let _xdg_config = setup_user_bash_rules(&config).await;

    run_exec_hook_expect_ok(
        PRETOOL_READ_EVENT,
        "Non-Bash PreToolUse with tool_rules should succeed",
    )
    .await;
}

#[tokio::test]
async fn test_pretool_hook_invalid_config_defaults_to_ask() {
    let temp_dir = setup_isolated_xdg_config();

    let moriarty_dir = temp_dir.path().join("moriarty");
    tokio::fs::create_dir_all(&moriarty_dir).await.unwrap();
    tokio::fs::write(
        moriarty_dir.join("tool_rules.toml"),
        "this is not valid [[[[ toml",
    )
    .await
    .unwrap();

    let tool_input = serde_json::json!({"file_path": "/tmp/foo.rs"});
    let result = handle_pretool_hook("Read", &tool_input, "")
        .await
        .expect("Should succeed with Ask fallback");

    assert_pretool_ask(&result);
}

// ===== cwd stripping integration tests =====

#[tokio::test]
async fn test_tool_rule_cwd_stripping_matches_relative_pattern() {
    let config = r#"
[[tool_rules]]
name = "allow-flake"
tool = "Read"
field = "path"
pattern = "^flake\\.nix$"
action = { type = "Allow" }
"#;
    // Absolute path with matching cwd should be stripped to "flake.nix" and match
    let result = run_pretool_hook(
        config,
        "Read",
        &serde_json::json!({"path": "/tmp/project/flake.nix"}),
        "/tmp/project",
    )
    .await;
    assert_pretool_allow(&result);
}

#[tokio::test]
async fn test_tool_rule_cwd_stripping_no_match_different_cwd() {
    let config = r#"
[[tool_rules]]
name = "allow-flake"
tool = "Read"
field = "path"
pattern = "^flake\\.nix$"
action = { type = "Allow" }
"#;
    // Different cwd means path is not stripped, so "^flake\.nix$" won't match the absolute path.
    // Since no tool rule matches and this is a non-Bash tool, we get passthrough (no decision).
    let result = run_pretool_hook(
        config,
        "Read",
        &serde_json::json!({"path": "/tmp/project/flake.nix"}),
        "/other/dir",
    )
    .await;

    assert_eq!(
        result.hook_specific_output, None,
        "Should not match when cwd doesn't match path prefix; non-Bash passthrough"
    );
    assert_eq!(result.permission_decision, None);
}

// ===== allow_local integration tests =====

/// Standard `tool_rules` config for `Read` with `allow_local = true` and `Allow` action.
const ALLOW_LOCAL_READ_CONFIG: &str = r#"
[[tool_rules]]
name = "allow-local-read"
tool = "Read"
allow_local = true
action = { type = "Allow" }
"#;

/// Runs a `Read` pretool hook with the standard allow-local config, returning the
/// hook result. Callers are responsible for creating the directory structure that
/// backs `cwd` and `path`.
async fn run_allow_local_read(
    cwd: &std::path::Path,
    path: impl Into<serde_json::Value>,
) -> HookOutput {
    let _xdg_config = setup_user_bash_rules(ALLOW_LOCAL_READ_CONFIG).await;
    handle_pretool_hook(
        "Read",
        &serde_json::json!({ "path": path.into() }),
        cwd.to_str().unwrap(),
    )
    .await
    .expect("Should succeed")
}

#[tokio::test]
async fn test_tool_rule_allow_local_matches_existing_path() {
    let config = cfg_allow_local_rule(
        "Read",
        "file_path",
        r"^src/.*\.rs$",
        r#"{ type = "Allow" }"#,
    );
    let _xdg_config = setup_user_bash_rules(&config).await;

    let temp_dir = TempDir::new().unwrap();
    let cwd = temp_dir.path();
    let src_dir = cwd.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    let existing = src_dir.join("lib.rs");
    std::fs::write(&existing, "fn lib() {}\n").unwrap();

    let result = handle_pretool_hook(
        "Read",
        &serde_json::json!({"file_path": existing}),
        cwd.to_str().unwrap(),
    )
    .await
    .expect("Should succeed");
    assert_pretool_allow(&result);
}

#[tokio::test]
async fn test_tool_rule_allow_local_matches_nonexistent_path() {
    let config = cfg_allow_local_rule(
        "Read",
        "file_path",
        r"^src/.*\.rs$",
        r#"{ type = "Allow" }"#,
    );
    let _xdg_config = setup_user_bash_rules(&config).await;

    let temp_dir = TempDir::new().unwrap();
    let cwd = temp_dir.path();
    std::fs::create_dir_all(cwd.join("src")).unwrap();

    let result = handle_pretool_hook(
        "Read",
        &serde_json::json!({"file_path": "src/generated.rs"}),
        cwd.to_str().unwrap(),
    )
    .await
    .expect("Should succeed");
    assert_pretool_allow(&result);
}

#[tokio::test]
async fn test_tool_rule_allow_local_rejects_path_escape() {
    let config = cfg_allow_local_rule("Read", "path", r"src/lib\.rs$", r#"{ type = "Allow" }"#);
    let _xdg_config = setup_user_bash_rules(&config).await;

    let temp_dir = TempDir::new().unwrap();
    let cwd = temp_dir.path().join("project");
    let sibling = temp_dir.path().join("sibling");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(sibling.join("src")).unwrap();
    std::fs::write(sibling.join("src/lib.rs"), "fn lib() {}\n").unwrap();

    let result = handle_pretool_hook(
        "Read",
        &serde_json::json!({"path": sibling.join("src/lib.rs")}),
        cwd.to_str().unwrap(),
    )
    .await
    .expect("Should succeed");
    assert_eq!(
        result.hook_specific_output, None,
        "Should not match: path is outside cwd, allow_local must reject"
    );
}

#[tokio::test]
async fn test_tool_rule_allow_local_no_match_for_regex_miss() {
    let config = cfg_allow_local_rule("Read", "path", r"^src/.*\.rs$", r#"{ type = "Allow" }"#);
    let _xdg_config = setup_user_bash_rules(&config).await;

    let temp_dir = TempDir::new().unwrap();
    let cwd = temp_dir.path();
    std::fs::create_dir_all(cwd.join("src")).unwrap();
    std::fs::write(cwd.join("Cargo.toml"), "[package]\nname='x'\n").unwrap();

    // This path is local, but the regex still prevents a match.
    let result = handle_pretool_hook(
        "Read",
        &serde_json::json!({"path": cwd.join("Cargo.toml")}),
        cwd.to_str().unwrap(),
    )
    .await
    .expect("Should succeed");
    assert_eq!(
        result.hook_specific_output, None,
        "Should not match: path is local but regex does not match Cargo.toml"
    );
}

/// Runs a Write against a local file under `allow_local = true` for the given
/// `action_toml` fragment and returns the hook result.
async fn run_allow_local_write_with_action(action_toml: &str) -> HookOutput {
    let config = format!(
        r#"
[[tool_rules]]
name = "local-write-action"
tool = "Write"
allow_local = true
{action_toml}
"#,
    );
    let _xdg_config = setup_user_bash_rules(&config).await;

    let temp_dir = TempDir::new().unwrap();
    let cwd = temp_dir.path();
    std::fs::write(cwd.join("local.txt"), "hello\n").unwrap();

    handle_pretool_hook(
        "Write",
        &serde_json::json!({
            "path": cwd.join("local.txt"),
            "content": "updated",
        }),
        cwd.to_str().unwrap(),
    )
    .await
    .expect("Should succeed")
}

#[tokio::test]
async fn test_tool_rule_allow_local_ask_action() {
    let result = run_allow_local_write_with_action("action = { type = \"Ask\" }").await;
    assert_pretool_ask(&result);
}

#[tokio::test]
async fn test_tool_rule_allow_local_deny_action() {
    let result = run_allow_local_write_with_action(
        "action = { type = \"Deny\", value = \"no local writes\" }",
    )
    .await;
    assert_pretool_deny(&result, "no local writes");
}

#[tokio::test]
async fn test_tool_rule_allow_local_with_non_path_field_does_not_match() {
    let config = cfg_allow_local_rule("Read", "command", "^cat", r#"{ type = "Allow" }"#);
    let _xdg_config = setup_user_bash_rules(&config).await;

    let temp_dir = TempDir::new().unwrap();
    let cwd = temp_dir.path();
    std::fs::write(cwd.join("local.txt"), "hello\n").unwrap();

    let result = handle_pretool_hook(
        "Read",
        &serde_json::json!({
            "command": "cat local.txt",
            "path": cwd.join("local.txt"),
        }),
        cwd.to_str().unwrap(),
    )
    .await
    .expect("Should succeed");

    assert_eq!(
        result.hook_specific_output, None,
        "Should not match: field='command' is not a path field, allow_local always fails"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_tool_rule_allow_local_rejects_broken_symlink() {
    use std::os::unix::fs::symlink;

    let temp_dir = TempDir::new().unwrap();
    let cwd = temp_dir.path().join("project");
    let missing = temp_dir.path().join("missing-target");
    std::fs::create_dir_all(&cwd).unwrap();
    symlink(&missing, cwd.join("broken-link")).unwrap();

    let result = run_allow_local_read(&cwd, "broken-link/file.txt").await;

    assert_eq!(
        result.hook_specific_output, None,
        "Should not match: broken symlink cannot be resolved, treated as non-local"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_tool_rule_allow_local_rejects_symlink_escape() {
    use std::os::unix::fs::symlink;

    let temp_dir = TempDir::new().unwrap();
    let cwd = temp_dir.path().join("project");
    let outside = temp_dir.path().join("outside");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "secret\n").unwrap();
    symlink(&outside, cwd.join("linked-outside")).unwrap();

    let result = run_allow_local_read(&cwd, "linked-outside/secret.txt").await;

    assert_eq!(
        result.hook_specific_output, None,
        "Should not match: symlink resolves outside cwd, treated as non-local"
    );
}
