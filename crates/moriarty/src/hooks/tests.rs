//! Tests for hooks module

use std::io::Cursor;

use tempfile::TempDir;

use super::*;

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

    // Verify both reason and system_message contain the error
    let message = result.reason.as_ref().expect("reason should be set");
    assert!(message.contains("Check 'test-check' is not approved"));

    assert_eq!(
        result.system_message, result.reason,
        "system_message should match reason for user feedback"
    );
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

    let reason = result.reason.as_ref().expect("reason should be set");
    assert!(reason.contains("Checks failed"), "Reason: {}", reason);
    assert!(
        reason.contains("failing-check"),
        "Should mention check name: {}",
        reason
    );

    // Verify system_message matches reason for user feedback
    assert_eq!(
        result.system_message, result.reason,
        "system_message should match reason for user feedback"
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

    let reason = result.reason.as_ref().expect("reason should be set");
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

    // Verify system_message matches reason for user feedback
    assert_eq!(
        result.system_message, result.reason,
        "system_message should match reason for user feedback"
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

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Ask));
            assert_eq!(output.updated_input, None);
        }
        _ => panic!("Expected PreToolUse hook specific output"),
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
async fn test_bash_hook_no_config_file() {
    let _xdg_config = setup_isolated_xdg_config();

    let tool_input = serde_json::json!({"command": "ls -la"});
    let result = handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Ask));
            assert_eq!(output.updated_input, None);
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }
}

#[tokio::test]
async fn test_bash_hook_no_rules_configured() {
    let _xdg_config = setup_user_bash_rules("# Empty config file\n").await;

    let tool_input = serde_json::json!({"command": "ls -la"});
    let result = handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Ask));
            assert_eq!(output.updated_input, None);
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }
}

#[tokio::test]
async fn test_bash_hook_rule_asks() {
    let config = r#"
[[bash_rules]]
name = "ask-docker"
pattern = "^docker"
action = { type = "Ask" }
"#;
    let _xdg_config = setup_user_bash_rules(config).await;

    let tool_input = serde_json::json!({"command": "docker build ."});
    let result = handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Ask));
            assert_eq!(output.permission_decision_reason, None);
            assert_eq!(output.updated_input, None);
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }
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

    // Verify system_message is populated for user feedback
    assert_eq!(
        result.system_message,
        Some("Dangerous recursive delete".to_string()),
        "system_message should be populated at top level for deny decisions"
    );

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Deny));
            assert!(output
                .permission_decision_reason
                .unwrap()
                .contains("Dangerous recursive delete"));
            assert_eq!(output.updated_input, None);
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }
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

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Allow));
            let reason = output
                .permission_decision_reason
                .expect("Should have permission decision reason");
            assert!(
                reason.contains("modified by rule 'add-dry-run'"),
                "Reason should include rule name: {}",
                reason
            );
            assert!(
                reason.contains("to: docker system prune --dry-run"),
                "Reason should include modified command: {}",
                reason
            );
            let updated = output.updated_input.expect("Should have updated input");
            assert_eq!(
                updated["command"],
                serde_json::Value::String("docker system prune --dry-run".to_string())
            );
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }
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

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Allow));
            assert_eq!(output.updated_input, None);
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }
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

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Ask));
            assert_eq!(output.updated_input, None);
        }
        _ => panic!("Expected PreToolUse hook specific output"),
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
    let _xdg_config = setup_user_bash_rules(config).await;

    let tool_input = serde_json::json!({"command": "rm -rf /"});
    let result = handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Deny));
            assert!(output
                .permission_decision_reason
                .as_ref()
                .unwrap()
                .contains("Dangerous rm -rf"));
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }

    let tool_input = serde_json::json!({"command": "rm file.txt"});
    let result = handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Allow));
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }
}

#[tokio::test]
async fn test_argument_filter_with_allow() {
    let config = r#"
[[bash_rules]]
name = "filter-cargo-doc"
pattern = "^cargo doc.*--open"
action = { type = "ArgumentFilter", remove = ["--open"], reason = "Browser flag removed" }

[[bash_rules]]
name = "allow-cargo-doc"
pattern = "^cargo doc"
action = { type = "Allow" }
"#;
    let _xdg_config = setup_user_bash_rules(config).await;

    let tool_input = serde_json::json!({"command": "cargo doc --open --no-deps"});
    let result = handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Allow));
            // Check that command was filtered
            if let Some(serde_json::Value::Object(modified)) = &output.updated_input {
                assert_eq!(
                    modified.get("command"),
                    Some(&serde_json::Value::String(
                        "cargo doc --no-deps".to_string()
                    ))
                );
            } else {
                panic!("Expected updated_input");
            }
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }
}

#[tokio::test]
async fn test_argument_filter_with_deny() {
    let config = r#"
[[bash_rules]]
name = "filter-dangerous-rm"
pattern = "^rm .*-rf"
action = { type = "ArgumentFilter", remove = ["-rf"], reason = "Removed dangerous flag" }

[[bash_rules]]
name = "deny-rm-root"
pattern = "^rm.*/"
action = { type = "Deny", value = "Cannot delete root paths" }
"#;
    let _xdg_config = setup_user_bash_rules(config).await;

    // Command with -rf and / - filter removes -rf, but / still matches deny rule
    let tool_input = serde_json::json!({"command": "rm -rf /"});
    let result = handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Deny));
            assert!(output
                .permission_decision_reason
                .as_ref()
                .unwrap()
                .contains("Cannot delete root paths"));
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }
}

#[tokio::test]
async fn test_argument_filter_with_no_match() {
    let config = r#"
[[bash_rules]]
name = "filter-cargo-doc"
pattern = "^cargo doc\\b"
action = { type = "ArgumentFilter", remove = ["--open"], reason = "Browser flag removed" }

[[bash_rules]]
name = "allow-cargo-build"
pattern = "^cargo build"
action = { type = "Allow" }
"#;
    let _xdg_config = setup_user_bash_rules(config).await;

    // cargo doc --open gets filtered to "cargo doc", but no rule allows "cargo doc"
    let tool_input = serde_json::json!({"command": "cargo doc --open"});
    let result = handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            // Should ask user because filtered command doesn't match any allow rule
            assert_eq!(output.permission_decision, Some(PermissionDecision::Ask));
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }
}

#[tokio::test]
async fn test_argument_filter_loop_prevention() {
    let config = r#"
[[bash_rules]]
name = "filter-1"
pattern = "^cargo doc\\b"
action = { type = "ArgumentFilter", remove = ["--open"], reason = "First filter" }

[[bash_rules]]
name = "filter-2"
pattern = "^cargo doc\\b"
action = { type = "ArgumentFilter", add = ["--offline"], reason = "Second filter" }
"#;
    let _xdg_config = setup_user_bash_rules(config).await;

    // First filter matches and removes --open, but result matches second filter
    // Should ask user to prevent infinite loops
    let tool_input = serde_json::json!({"command": "cargo doc --open"});
    let result = handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            // Should ask user to prevent chained argument filtering
            assert_eq!(output.permission_decision, Some(PermissionDecision::Ask));
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }
}

#[tokio::test]
async fn test_argument_filter_add_arguments() {
    let config = r#"
[[bash_rules]]
name = "allow-docker-run-safe"
pattern = "^docker run .* --read-only"
action = { type = "Allow" }

[[bash_rules]]
name = "add-safety-flags"
pattern = "^docker run ubuntu$"
action = { type = "ArgumentFilter", add = ["--read-only", "--security-opt=no-new-privileges"], reason = "Added security flags" }
"#;
    let _xdg_config = setup_user_bash_rules(config).await;

    let tool_input = serde_json::json!({"command": "docker run ubuntu"});
    let result = handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Allow));
            if let Some(serde_json::Value::Object(modified)) = &output.updated_input {
                let command_str = modified
                    .get("command")
                    .and_then(|v| v.as_str())
                    .expect("Expected command string");
                assert!(command_str.contains("--read-only"));
                assert!(command_str.contains("--security-opt=no-new-privileges"));
            } else {
                panic!("Expected updated_input");
            }
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }
}

#[tokio::test]
async fn test_argument_filter_replace_arguments() {
    let config = r#"
[[bash_rules]]
name = "allow-rm-interactive"
pattern = "^rm .* -i$"
action = { type = "Allow" }

[[bash_rules]]
name = "force-interactive-rm"
pattern = "^rm -f file\\.txt$"
action = { type = "ArgumentFilter", remove = ["-f", "--force"], add = ["-i"], reason = "Replaced force mode with interactive" }
"#;
    let _xdg_config = setup_user_bash_rules(config).await;

    let tool_input = serde_json::json!({"command": "rm -f file.txt"});
    let result = handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Allow));
            if let Some(serde_json::Value::Object(modified)) = &output.updated_input {
                assert_eq!(
                    modified.get("command"),
                    Some(&serde_json::Value::String("rm file.txt -i".to_string()))
                );
            } else {
                panic!("Expected updated_input");
            }
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }
}

#[tokio::test]
async fn test_argument_filter_revalidation_to_modify() {
    let config = r#"
[[bash_rules]]
name = "filter-docker-safety"
pattern = "^docker run.*--privileged"
action = { type = "ArgumentFilter", remove = ["--privileged"], reason = "Removed privileged mode" }

[[bash_rules]]
name = "modify-docker-add-limits"
pattern = "^docker run"
action = { type = "Modify", value = "$0 --memory=1g" }
"#;
    let _xdg_config = setup_user_bash_rules(config).await;

    let tool_input = serde_json::json!({"command": "docker run --privileged ubuntu"});
    let result = handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            // After filtering removes --privileged, the command matches Modify rule
            // which transforms it further
            assert_eq!(output.permission_decision, Some(PermissionDecision::Allow));
            if let Some(serde_json::Value::Object(modified)) = &output.updated_input {
                // The Modify rule matches "^docker run" and replaces with "$0 --memory=1g"
                // After ArgumentFilter removes --privileged, we have "docker run ubuntu"
                // But $0 only captures "docker run" (the matched part), so we get "docker run --memory=1g"
                // The "ubuntu" argument is not included because it wasn't part of the regex match
                assert_eq!(
                    modified.get("command"),
                    Some(&serde_json::Value::String(
                        "docker run --memory=1g".to_string()
                    ))
                );
            } else {
                panic!("Expected updated_input");
            }
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }
}

#[tokio::test]
async fn test_argument_filter_revalidation_to_ask() {
    let config = r#"
[[bash_rules]]
name = "filter-sudo-flags"
pattern = "^sudo.*--non-interactive"
action = { type = "ArgumentFilter", remove = ["--non-interactive"], reason = "Removed automation flag" }

[[bash_rules]]
name = "ask-for-sudo"
pattern = "^sudo"
action = { type = "Ask" }
"#;
    let _xdg_config = setup_user_bash_rules(config).await;

    let tool_input = serde_json::json!({"command": "sudo --non-interactive apt install vim"});
    let result = handle_bash_pretool_hook(&tool_input)
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Ask));
        }
        _ => panic!("Expected PreToolUse hook specific output"),
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

    if let Some(HookSpecificOutput::PreToolUse(pretool)) = parsed.hook_specific_output {
        assert_eq!(pretool.permission_decision, Some(PermissionDecision::Deny));
        assert_eq!(
            pretool.permission_decision_reason,
            Some("Command blocked".to_string())
        );
    } else {
        panic!("Expected PreToolUse hook specific output");
    }
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

#[test]
fn test_pretool_deny_hook_sets_system_message() {
    let reason = "Command blocked by bash rules";
    let output = pretool_deny_hook(reason.to_string());

    // PreToolUse hooks use permission_decision_reason, not reason
    assert_eq!(output.reason, None);
    assert_eq!(output.system_message, Some(reason.to_string()));

    if let Some(HookSpecificOutput::PreToolUse(pretool)) = output.hook_specific_output {
        assert_eq!(pretool.permission_decision_reason, Some(reason.to_string()));
        assert_eq!(pretool.permission_decision, Some(PermissionDecision::Deny));
    } else {
        panic!("Expected PreToolUse hook output");
    }
}

#[test]
fn test_pretool_allow_hook_with_reason_sets_system_message() {
    let reason = "Allowed by whitelist rule";
    let output = pretool_allow_hook(Some(reason.to_string()));

    assert_eq!(output.system_message, Some(reason.to_string()));

    if let Some(HookSpecificOutput::PreToolUse(pretool)) = output.hook_specific_output {
        assert_eq!(pretool.permission_decision_reason, Some(reason.to_string()));
        assert_eq!(pretool.permission_decision, Some(PermissionDecision::Allow));
    } else {
        panic!("Expected PreToolUse hook output");
    }
}

#[test]
fn test_pretool_allow_hook_without_reason_omits_system_message() {
    let output = pretool_allow_hook(None);

    assert_eq!(output.system_message, None);

    if let Some(HookSpecificOutput::PreToolUse(pretool)) = output.hook_specific_output {
        assert_eq!(pretool.permission_decision_reason, None);
        assert_eq!(pretool.permission_decision, Some(PermissionDecision::Allow));
    } else {
        panic!("Expected PreToolUse hook output");
    }
}

#[test]
fn test_pretool_modify_hook_sets_system_message() {
    let reason = "Command modified by security rule";
    let new_input = serde_json::json!({"command": "ls -la"});
    let output = pretool_modify_hook(new_input.clone(), Some(reason.to_string()));

    assert_eq!(output.system_message, Some(reason.to_string()));

    if let Some(HookSpecificOutput::PreToolUse(pretool)) = output.hook_specific_output {
        assert_eq!(pretool.permission_decision_reason, Some(reason.to_string()));
        assert_eq!(pretool.updated_input, Some(new_input));
    } else {
        panic!("Expected PreToolUse hook output");
    }
}

#[test]
fn test_pretool_modify_hook_without_reason_omits_system_message() {
    let new_input = serde_json::json!({"command": "ls -la"});
    let output = pretool_modify_hook(new_input.clone(), None);

    assert_eq!(output.system_message, None);

    if let Some(HookSpecificOutput::PreToolUse(pretool)) = output.hook_specific_output {
        assert_eq!(pretool.permission_decision_reason, None);
        assert_eq!(pretool.updated_input, Some(new_input));
    } else {
        panic!("Expected PreToolUse hook output");
    }
}

#[test]
fn test_pretool_ask_hook_omits_system_message() {
    let output = pretool_ask_hook();

    // Ask hooks don't provide feedback, so no system_message
    assert_eq!(output.system_message, None);
    assert_eq!(output.reason, None);

    if let Some(HookSpecificOutput::PreToolUse(pretool)) = output.hook_specific_output {
        assert_eq!(pretool.permission_decision, Some(PermissionDecision::Ask));
    } else {
        panic!("Expected PreToolUse hook output");
    }
}

// ===== Tool Rules integration tests =====

#[tokio::test]
async fn test_tool_rule_allow_non_bash() {
    let config = r#"
[[tool_rules]]
name = "allow-read"
tool = "Read"
action = { type = "Allow" }
"#;
    let _xdg_config = setup_user_bash_rules(config).await;

    let tool_input = serde_json::json!({"file_path": "/tmp/foo.rs"});
    let result = handle_pretool_hook("Read", &tool_input)
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Allow));
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }
}

#[tokio::test]
async fn test_tool_rule_deny_non_bash() {
    let config = r#"
[[tool_rules]]
name = "deny-write"
tool = "Write"
action = { type = "Deny", value = "Writes are blocked" }
"#;
    let _xdg_config = setup_user_bash_rules(config).await;

    let tool_input = serde_json::json!({"file_path": "/tmp/foo.rs", "content": "hello"});
    let result = handle_pretool_hook("Write", &tool_input)
        .await
        .expect("Should succeed");

    assert_eq!(
        result.system_message,
        Some("Writes are blocked".to_string())
    );

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Deny));
            assert!(output
                .permission_decision_reason
                .unwrap()
                .contains("Writes are blocked"));
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }
}

#[tokio::test]
async fn test_tool_rule_ask_non_bash() {
    let config = r#"
[[tool_rules]]
name = "ask-edit"
tool = "Edit"
action = { type = "Ask" }
"#;
    let _xdg_config = setup_user_bash_rules(config).await;

    let tool_input = serde_json::json!({"file_path": "/tmp/foo.rs"});
    let result = handle_pretool_hook("Edit", &tool_input)
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Ask));
        }
        _ => panic!("Expected PreToolUse hook specific output"),
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
    let _xdg_config = setup_user_bash_rules(config).await;

    // .env file should be denied
    let tool_input = serde_json::json!({"file_path": "/home/user/.env", "content": "SECRET=x"});
    let result = handle_pretool_hook("Write", &tool_input)
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Deny));
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }

    // Non-.env file should be allowed
    let tool_input =
        serde_json::json!({"file_path": "/home/user/main.rs", "content": "fn main() {}"});
    let result = handle_pretool_hook("Write", &tool_input)
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Allow));
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }
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
    let _xdg_config = setup_user_bash_rules(config).await;

    // tool_rules should match first and Allow, even though bash_rules would deny
    let tool_input = serde_json::json!({"command": "rm -rf /"});
    let result = handle_pretool_hook("Bash", &tool_input)
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(
                output.permission_decision,
                Some(PermissionDecision::Allow),
                "tool_rules should take precedence over bash_rules"
            );
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }
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
    let _xdg_config = setup_user_bash_rules(config).await;

    // Bash tool: tool_rules don't match (only Read matches), falls through to bash_rules
    let tool_input = serde_json::json!({"command": "ls -la"});
    let result = handle_pretool_hook("Bash", &tool_input)
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Allow));
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }

    // rm should be denied by bash_rules
    let tool_input = serde_json::json!({"command": "rm file.txt"});
    let result = handle_pretool_hook("Bash", &tool_input)
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Deny));
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }
}

#[tokio::test]
async fn test_non_bash_tool_no_rules_defaults_to_ask() {
    let _xdg_config = setup_isolated_xdg_config();

    let tool_input = serde_json::json!({"file_path": "/tmp/foo"});
    let result = handle_pretool_hook("Read", &tool_input)
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(
                output.permission_decision,
                Some(PermissionDecision::Ask),
                "Non-Bash tools with no rules should default to Ask"
            );
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }
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
    let _xdg_config = setup_user_bash_rules(config).await;

    // Read matches specific rule
    let result = handle_pretool_hook("Read", &serde_json::json!({}))
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Allow));
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }

    // Write matches wildcard
    let result = handle_pretool_hook("Write", &serde_json::json!({}))
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(output.permission_decision, Some(PermissionDecision::Ask));
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }

    // Bash also matches wildcard (before bash_rules run)
    let tool_input = serde_json::json!({"command": "echo hi"});
    let result = handle_pretool_hook("Bash", &tool_input)
        .await
        .expect("Should succeed");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(
                output.permission_decision,
                Some(PermissionDecision::Ask),
                "Wildcard should match Bash too"
            );
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }
}

#[tokio::test]
async fn test_exec_hook_impl_non_bash_pretool_produces_output() {
    let _xdg_dir = setup_isolated_xdg_state();
    let _xdg_config = setup_isolated_xdg_config();

    // Non-Bash PreToolUse event should now produce JSON output (Ask by default)
    let input = r#"{
        "session_id": "test-session",
        "transcript_path": "/tmp/transcript.json",
        "cwd": "/tmp/project",
        "permission_mode": "default",
        "hook_event_name": "PreToolUse",
        "tool_name": "Read",
        "tool_input": {"file_path": "/tmp/foo.rs"}
    }"#;

    let cursor = Cursor::new(input);
    let result = exec_hook_impl(cursor).await;
    result.expect("Non-Bash PreToolUse should succeed and produce output");
}

#[tokio::test]
async fn test_exec_hook_impl_non_bash_pretool_with_tool_rules() {
    let _xdg_dir = setup_isolated_xdg_state();

    let config = r#"
[[tool_rules]]
name = "allow-read"
tool = "Read"
action = { type = "Allow" }
"#;
    let _xdg_config = setup_user_bash_rules(config).await;

    let input = r#"{
        "session_id": "test-session",
        "transcript_path": "/tmp/transcript.json",
        "cwd": "/tmp/project",
        "permission_mode": "default",
        "hook_event_name": "PreToolUse",
        "tool_name": "Read",
        "tool_input": {"file_path": "/tmp/foo.rs"}
    }"#;

    let cursor = Cursor::new(input);
    let result = exec_hook_impl(cursor).await;
    result.expect("Non-Bash PreToolUse with tool_rules should succeed");
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
    let result = handle_pretool_hook("Read", &tool_input)
        .await
        .expect("Should succeed with Ask fallback");

    match result.hook_specific_output {
        Some(HookSpecificOutput::PreToolUse(output)) => {
            assert_eq!(
                output.permission_decision,
                Some(PermissionDecision::Ask),
                "Invalid config should fail-open to Ask"
            );
        }
        _ => panic!("Expected PreToolUse hook specific output"),
    }
}
