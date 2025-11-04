//! Tests for hooks module

use super::*;
use std::io::Cursor;
use tempfile::TempDir;

/// Safe to use std::env::set_var because cargo nextest isolates each test in a separate process.
fn setup_isolated_xdg_state() -> TempDir {
    let temp_dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_STATE_HOME", temp_dir.path());
    temp_dir
}

fn setup_isolated_xdg_config() -> TempDir {
    let temp_dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", temp_dir.path());
    temp_dir
}

async fn setup_user_bash_rules(rules_toml: &str) -> TempDir {
    let temp_dir = setup_isolated_xdg_config();

    let moriarty_dir = temp_dir.path().join("moriarty");
    tokio::fs::create_dir_all(&moriarty_dir).await.unwrap();
    tokio::fs::write(moriarty_dir.join("tool_rules.toml"), rules_toml)
        .await
        .unwrap();

    temp_dir
}

#[tokio::test]
async fn test_exec_hook_empty_input_returns_error() {
    let _xdg_dir = setup_isolated_xdg_state();

    let input = "";
    let cursor = Cursor::new(input);
    let result = exec_hook_impl(cursor).await;

    let err = result.expect_err("Should fail with empty input");
    assert!(err.to_string().contains("No input received"));
}

#[tokio::test]
async fn test_exec_hook_malformed_json_returns_error() {
    let _xdg_dir = setup_isolated_xdg_state();

    let input = "{invalid json";
    let cursor = Cursor::new(input);
    let result = exec_hook_impl(cursor).await;

    let err = result.expect_err("Should fail with malformed JSON");
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
    let result = exec_hook_impl(cursor).await;

    // Should return error without panicking
    let err = result.expect_err("Should fail with large invalid JSON");
    assert!(err.to_string().contains("Failed to parse hook input"));
}

#[tokio::test]
async fn test_exec_hook_all_event_types() {
    let _xdg_dir = setup_isolated_xdg_state();

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
    // Verify that hook output with old decision values fails to parse
    let old_allow = r#"{"decision": "allow"}"#;
    let err = serde_json::from_str::<HookOutput>(old_allow)
        .expect_err("Should reject old 'allow' decision value");
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("unknown variant") || err_msg.contains("allow"),
        "Error should mention unknown variant, got: {}",
        err_msg
    );

    let old_deny = r#"{"decision": "deny"}"#;
    let err = serde_json::from_str::<HookOutput>(old_deny)
        .expect_err("Should reject old 'deny' decision value");
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("unknown variant") || err_msg.contains("deny"),
        "Error should mention unknown variant, got: {}",
        err_msg
    );

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
async fn test_stop_hook_no_config_file() {
    let _xdg_dir = setup_isolated_xdg_state();

    // Create a temp directory without .config/tools.toml
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    std::env::set_var("CLAUDE_PROJECT_DIR", temp_dir.path());

    let result = handle_stop_hook().await.expect("Should succeed");

    assert_eq!(result.decision, Some(HookDecision::Approve));
    assert_eq!(result.reason, None);
}

#[tokio::test]
async fn test_stop_hook_no_checks_defined() {
    let _xdg_dir = setup_isolated_xdg_state();

    // Create project with config but no checks
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path().join(".config");
    std::fs::create_dir(&config_dir).expect("Failed to create .config dir");
    std::fs::write(
        config_dir.join("tools.toml"),
        r#"
[commands]
lint = ["echo", "lint"]
"#,
    )
    .expect("Failed to write tools.toml");

    std::env::set_var("CLAUDE_PROJECT_DIR", temp_dir.path());

    let result = handle_stop_hook().await.expect("Should succeed");

    assert_eq!(result.decision, Some(HookDecision::Approve));
    assert_eq!(result.reason, None);
}

#[tokio::test]
async fn test_stop_hook_empty_checks() {
    let _xdg_dir = setup_isolated_xdg_state();

    // Create project with empty checks array
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path().join(".config");
    std::fs::create_dir(&config_dir).expect("Failed to create .config dir");
    std::fs::write(
        config_dir.join("tools.toml"),
        r#"
[commands]
lint = ["echo", "lint"]

[[checks]]
"#,
    )
    .expect("Failed to write tools.toml");

    std::env::set_var("CLAUDE_PROJECT_DIR", temp_dir.path());

    // TOML parse fails on incomplete table arrays (`[[checks]]` with no fields).
    // This tests fail-open behavior documented in the security model (lines 145-161).
    let result = handle_stop_hook().await.expect("Should succeed");
    assert_eq!(result.decision, Some(HookDecision::Approve));
}

#[tokio::test]
async fn test_stop_hook_empty_command_array() {
    let _xdg_dir = setup_isolated_xdg_state();

    // Create project with check that has empty command array
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path().join(".config");
    std::fs::create_dir(&config_dir).expect("Failed to create .config dir");
    std::fs::write(
        config_dir.join("tools.toml"),
        r#"
[commands]
lint = ["echo", "lint"]

[[checks]]
name = "empty-command"
command = []
"#,
    )
    .expect("Failed to write tools.toml");

    std::env::set_var("CLAUDE_PROJECT_DIR", temp_dir.path());

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

    // Create project with a check
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_dir = temp_dir.path().join(".config");
    std::fs::create_dir(&config_dir).expect("Failed to create .config dir");
    std::fs::write(
        config_dir.join("tools.toml"),
        r#"
[commands]

[[checks]]
name = "test-check"
command = ["echo", "test"]
"#,
    )
    .expect("Failed to write tools.toml");

    std::env::set_var("CLAUDE_PROJECT_DIR", temp_dir.path());

    // Don't approve the project, so it should be denied
    let result = handle_stop_hook().await.expect("Should succeed");

    assert_eq!(result.decision, Some(HookDecision::Block));
    assert!(result
        .reason
        .unwrap()
        .contains("Check 'test-check' is not approved"));
}

#[tokio::test]
async fn test_stop_hook_check_passes() {
    let _xdg_dir = setup_isolated_xdg_state();
    let _config_dir = setup_isolated_xdg_config();

    #[cfg(unix)]
    let (check_command, check_args) = ("true", vec![]);
    #[cfg(windows)]
    let (check_command, check_args) = ("cmd", vec!["/c", "exit 0"]);

    let temp_dir =
        setup_approved_project_with_checks(vec![("passing-check", check_command, check_args)])
            .await;

    std::env::set_var("CLAUDE_PROJECT_DIR", temp_dir.path());

    let result = handle_stop_hook().await.expect("Should succeed");

    assert_eq!(result.decision, Some(HookDecision::Approve));
    assert_eq!(result.reason, None);
}

#[tokio::test]
async fn test_stop_hook_check_fails() {
    let _xdg_dir = setup_isolated_xdg_state();
    let _config_dir = setup_isolated_xdg_config();

    #[cfg(unix)]
    let (check_command, check_args) = ("false", vec![]);
    #[cfg(windows)]
    let (check_command, check_args) = ("cmd", vec!["/c", "exit 1"]);

    let temp_dir =
        setup_approved_project_with_checks(vec![("failing-check", check_command, check_args)])
            .await;

    std::env::set_var("CLAUDE_PROJECT_DIR", temp_dir.path());

    let result = handle_stop_hook().await.expect("Should succeed");

    assert_eq!(result.decision, Some(HookDecision::Block));
    let reason = result.reason.unwrap();
    assert!(reason.contains("Checks failed"), "Reason: {}", reason);
    assert!(
        reason.contains("failing-check"),
        "Should mention check name: {}",
        reason
    );
}

#[tokio::test]
async fn test_stop_hook_multiple_checks_all_pass() {
    let _xdg_dir = setup_isolated_xdg_state();
    let _config_dir = setup_isolated_xdg_config();

    #[cfg(unix)]
    let (check_command, check_args) = ("true", vec![]);
    #[cfg(windows)]
    let (check_command, check_args) = ("cmd", vec!["/c", "exit 0"]);

    let temp_dir = setup_approved_project_with_checks(vec![
        ("check1", check_command, check_args.clone()),
        ("check2", check_command, check_args),
    ])
    .await;

    std::env::set_var("CLAUDE_PROJECT_DIR", temp_dir.path());

    let result = handle_stop_hook().await.expect("Should succeed");

    assert_eq!(result.decision, Some(HookDecision::Approve));
    assert_eq!(result.reason, None);
}

#[tokio::test]
async fn test_stop_hook_check_binary_hash_mismatch() {
    let _xdg_dir = setup_isolated_xdg_state();
    let _config_dir = setup_isolated_xdg_config();

    #[cfg(unix)]
    let (check_command, check_args) = ("true", vec![]);
    #[cfg(windows)]
    let (check_command, check_args) = ("cmd", vec!["/c", "exit 0"]);

    let temp_dir =
        setup_approved_project_with_checks(vec![("test-check", check_command, check_args)]).await;

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

    assert_eq!(result.decision, Some(HookDecision::Block));
    let reason = result.reason.unwrap();
    assert!(
        reason.contains("binary changed"),
        "Expected binary changed error. Reason: {}",
        reason
    );
    assert!(
        reason.contains("test-check"),
        "Expected check name in error. Reason: {}",
        reason
    );
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
async fn test_bash_hook_no_user_config() {
    let _xdg_config = setup_isolated_xdg_config();

    let tool_input = serde_json::json!({"command": "ls -la"});
    let result = handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed");

    assert_eq!(result.decision, Some(HookDecision::Ask));
    assert_eq!(result.updated_input, None);
}

#[tokio::test]
async fn test_bash_hook_missing_command_field() {
    let _xdg_config = setup_isolated_xdg_config();

    let tool_input = serde_json::json!({});
    let result = handle_bash_pretool_hook(&tool_input).await;

    let err = result.expect_err("Should fail with missing command field");
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("Missing 'command' field"),
        "Error should mention missing field, got: {}",
        err_msg
    );
}

#[tokio::test]
async fn test_bash_hook_no_config_file() {
    let _xdg_config = setup_isolated_xdg_config();

    let tool_input = serde_json::json!({"command": "ls -la"});
    let result = handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed");

    assert_eq!(result.decision, Some(HookDecision::Ask));
    assert_eq!(result.updated_input, None);
}

#[tokio::test]
async fn test_bash_hook_no_rules_configured() {
    let _xdg_config = setup_user_bash_rules("# Empty config file\n").await;

    let tool_input = serde_json::json!({"command": "ls -la"});
    let result = handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed");

    assert_eq!(result.decision, Some(HookDecision::Ask));
    assert_eq!(result.updated_input, None);
}

#[tokio::test]
async fn test_bash_hook_rule_denies() {
    let config = r#"
[[bash_rules]]
name = "deny-rm-rf"
pattern = "^rm\\s+-rf\\s+/"
action = { type = "Deny", value = "Dangerous recursive delete" }
"#;
    let _xdg_config = setup_user_bash_rules(config).await;

    let tool_input = serde_json::json!({"command": "rm -rf /"});
    let result = handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed");

    assert_eq!(result.decision, Some(HookDecision::Block));
    assert!(result
        .reason
        .unwrap()
        .contains("Dangerous recursive delete"));
    assert_eq!(result.updated_input, None);
}

#[tokio::test]
async fn test_bash_hook_rule_modifies() {
    let config = r#"
[[bash_rules]]
name = "add-dry-run"
pattern = "^(docker\\s+system\\s+prune)"
action = { type = "Modify", value = "$1 --dry-run" }
"#;
    let _xdg_config = setup_user_bash_rules(config).await;

    let tool_input = serde_json::json!({"command": "docker system prune"});
    let result = handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed");

    assert_eq!(result.decision, Some(HookDecision::Approve));
    assert!(result.reason.unwrap().contains("modified by rule"));
    let updated = result.updated_input.expect("Should have updated input");
    assert_eq!(
        updated["command"],
        serde_json::Value::String("docker system prune --dry-run".to_string())
    );
}

#[tokio::test]
async fn test_bash_hook_rule_allows() {
    let config = r#"
[[bash_rules]]
name = "allow-ls"
pattern = "^ls($|\\s)"
action = { type = "Allow" }
"#;
    let _xdg_config = setup_user_bash_rules(config).await;

    let tool_input = serde_json::json!({"command": "ls -la"});
    let result = handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed");

    assert_eq!(result.decision, Some(HookDecision::Approve));
    assert_eq!(result.updated_input, None);
}

#[tokio::test]
async fn test_bash_hook_no_rules_match() {
    let config = r#"
[[bash_rules]]
name = "deny-rm"
pattern = "^rm\\s"
action = { type = "Deny", value = "rm denied" }
"#;
    let _xdg_config = setup_user_bash_rules(config).await;

    let tool_input = serde_json::json!({"command": "ls -la"});
    let result = handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed");

    assert_eq!(result.decision, Some(HookDecision::Ask));
    assert_eq!(result.updated_input, None);
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
    let _xdg_config = setup_user_bash_rules(config).await;

    let tool_input = serde_json::json!({"command": "rm -rf /"});
    let result = handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed");

    assert_eq!(result.decision, Some(HookDecision::Block));
    assert!(result.reason.as_ref().unwrap().contains("Dangerous rm -rf"));

    let tool_input = serde_json::json!({"command": "rm file.txt"});
    let result = handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed");

    assert_eq!(result.decision, Some(HookDecision::Approve));
}
