//! Hook execution for Claude Code integration
//!
//! This module provides the CLI interface for executing hooks. Currently, this is primarily
//! used for debugging and development - it reads hook input from stdin, parses it, and logs
//! both the parsed data and the environment context.
//!
//! ## Design Decisions
//!
//! - **Parse errors are fatal**: The function returns an error if parsing fails. This ensures
//!   that malformed hook input is visible via exit codes, allowing scripts and CI systems to
//!   detect failures.
//!
//! - **Environment variables are logged**: During development, understanding the complete
//!   environment context helps debug hook execution issues. Sensitive patterns (TOKEN, SECRET,
//!   KEY, PASSWORD, etc.) are automatically redacted.
//!
//! ## Security Model: Fail-Open Design
//!
//! The Stop hook handler implements a **fail-open** approach, returning `Allow` when:
//! - `$CLAUDE_PROJECT_DIR` environment variable is not set
//! - Project directory doesn't exist or cannot be canonicalized
//! - `.config/tools.toml` cannot be loaded or parsed
//! - No checks are defined in the configuration
//!
//! **Rationale**: This design prioritizes developer experience and avoids breaking workflows
//! when projects don't use the checks feature. Since checks are opt-in security validations,
//! their absence or misconfiguration should not block execution.
//!
//! **Trade-offs**: An attacker who can manipulate the environment or filesystem to cause
//! config loading failures could bypass checks. However, this requires the same level of
//! access needed to modify approved binaries directly, so it doesn't meaningfully weaken
//! the security model. Once checks are configured and approved, the handler fails **closed**
//! on all verification failures (unapproved checks, hash mismatches, check failures).

pub mod parser;
pub mod tracing;

use crate::project_config::{approvals::ProjectApprovals, config::load_project_settings};
use crate::HooksCommand;
use ::tracing::{error, info};
use futures::stream::StreamExt;
use miette::Result;
use parser::{HookDecision, HookEventData, HookOutput};
use std::io::Read;
use std::path::PathBuf;

/// Execute hooks command
pub async fn exec_hooks(cmd: HooksCommand) -> Result<()> {
    match cmd {
        HooksCommand::Exec => exec_hook().await,
    }
}

/// Execute a single hook by reading input from stdin and logging all parsed data
///
/// This is primarily a development/debugging tool. It reads hook input JSON from stdin,
/// parses it, and logs the results along with environment context.
///
/// Returns an error if parsing fails, ensuring malformed input is detectable via exit codes.
async fn exec_hook() -> Result<()> {
    exec_hook_impl(std::io::stdin()).await
}

/// Internal implementation of exec_hook that accepts any Read source for testability
async fn exec_hook_impl<R: Read>(reader: R) -> Result<()> {
    // Global tracing subscriber initialization races are acceptable in tests because nextest's
    // process isolation guarantees no cross-contamination, and failed initialization doesn't
    // affect correctness.
    let _guard = match tracing::init_tracing().await {
        Ok(guard) => Some(guard),
        Err(_) if cfg!(test) => None,
        Err(e) => return Err(e),
    };

    // Limit input size to 1MB to prevent DoS via memory exhaustion
    const MAX_INPUT_SIZE: usize = 1024 * 1024 * 100;
    const LOG_TRUNCATE_SIZE: usize = 50000;

    let mut input = String::new();
    let bytes_read = reader
        .take(MAX_INPUT_SIZE as u64)
        .read_to_string(&mut input)
        .map_err(|e| {
            miette::miette!(
                "Failed to read hook input from stdin (this command expects JSON hook data): {}",
                e
            )
        })?;

    if bytes_read == 0 {
        error!("Received empty input from stdin");
        return Err(miette::miette!("No input received from stdin"));
    }

    if bytes_read == MAX_INPUT_SIZE {
        error!(
            bytes_read = bytes_read,
            max_size = MAX_INPUT_SIZE,
            "Input reached maximum size limit, possible truncation"
        );
        return Err(miette::miette!(
            "Hook input reached maximum size of {} bytes and may have been truncated. \
             Reduce input size or increase MAX_INPUT_SIZE constant.",
            MAX_INPUT_SIZE
        ));
    }

    info!(bytes = bytes_read, "Received hook input from stdin");

    let hook_input = parser::parse_hook_input(&input).map_err(|e| {
        // Truncate and sanitize input for logging to prevent log injection and bloat
        let sanitized_input = if input.len() > LOG_TRUNCATE_SIZE {
            let safe_truncate = input
                .char_indices()
                .nth(LOG_TRUNCATE_SIZE)
                .map(|(i, _)| i)
                .unwrap_or(input.len());
            format!(
                "{}... [truncated {} bytes]",
                input[..safe_truncate].escape_debug(),
                input.len() - safe_truncate
            )
        } else {
            input.escape_debug().to_string()
        };

        error!(
            error = %e,
            raw_input = %sanitized_input,
            "Failed to parse hook input"
        );

        miette::miette!("Failed to parse hook input: {}", e)
    })?;

    info!(?hook_input, "Successfully parsed hook input");

    // Handle Stop hook
    if matches!(hook_input.event_data, HookEventData::Stop) {
        let hook_output = handle_stop_hook().await?;

        // Serialize and output to stdout
        let json_output = serde_json::to_string(&hook_output)
            .map_err(|e| miette::miette!("Failed to serialize HookOutput: {}", e))?;

        println!("{}", json_output);

        info!(?hook_output, "Stop hook completed");
    }

    Ok(())
}

/// Helper to create a HookOutput that allows execution with an optional reason
fn allow_hook() -> HookOutput {
    HookOutput {
        continue_execution: None,
        stop_reason: None,
        suppress_output: None,
        decision: Some(HookDecision::Approve),
        reason: None,
    }
}

/// Helper to create a HookOutput that denies execution with a reason
fn deny_hook(reason: impl Into<String>) -> HookOutput {
    HookOutput {
        continue_execution: None,
        stop_reason: None,
        suppress_output: None,
        decision: Some(HookDecision::Block),
        reason: Some(reason.into()),
    }
}

/// Handle Stop hook by running project checks if configured
///
/// This function:
/// 1. Checks for $CLAUDE_PROJECT_DIR environment variable
/// 2. Loads and validates the project's .config/tools.toml
/// 3. Verifies all checks are approved
/// 4. Runs all checks in parallel
/// 5. Returns HookOutput with decision based on check results
///
/// ## Security Model: Fail-Open Design
///
/// This handler implements a **fail-open** approach, returning `Allow` when:
/// - `$CLAUDE_PROJECT_DIR` environment variable is not set
/// - Project directory doesn't exist or cannot be canonicalized
/// - `.config/tools.toml` cannot be loaded or parsed
/// - No checks are defined in the configuration
///
/// **Rationale**: This design prioritizes developer experience and avoids breaking workflows
/// when projects don't use the checks feature. Since checks are opt-in security validations,
/// their absence or misconfiguration should not block execution.
///
/// **Trade-offs**: An attacker who can manipulate the environment or filesystem to cause
/// config loading failures could bypass checks. However, this requires the same level of
/// access needed to modify approved binaries directly, so it doesn't meaningfully weaken
/// the security model. Once checks are configured and approved, the handler fails **closed**
/// on all verification failures (unapproved checks, hash mismatches, check failures).
async fn handle_stop_hook() -> Result<HookOutput> {
    // Check for project directory environment variable
    let project_dir = match std::env::var("CLAUDE_PROJECT_DIR") {
        Ok(dir) => {
            info!(project_dir = %dir, "Found CLAUDE_PROJECT_DIR");
            PathBuf::from(dir)
        }
        Err(_) => {
            info!("No CLAUDE_PROJECT_DIR set, allowing without checks");
            return Ok(allow_hook());
        }
    };

    // Canonicalize the project directory path
    let canonical_dir = match project_dir.canonicalize() {
        Ok(dir) => dir,
        Err(e) => {
            error!(
                project_dir = %project_dir.display(),
                error = %e,
                "Failed to canonicalize project directory"
            );
            return Ok(allow_hook());
        }
    };

    // Try to load project config
    let config = match load_project_settings(canonical_dir.clone()).await {
        Ok(config) => config,
        Err(e) => {
            info!(
                error = %e,
                "No .config/tools.toml found, allowing without checks"
            );
            return Ok(allow_hook());
        }
    };

    // Check if there are any checks defined
    let checks = match config.checks {
        Some(checks) if !checks.is_empty() => checks,
        _ => {
            info!("No checks defined in config, allowing");
            return Ok(allow_hook());
        }
    };

    info!(check_count = checks.len(), "Found checks to run");

    // Validate all checks have non-empty commands
    for check in &checks {
        if check.command.is_empty() {
            error!(check_name = %check.name, "Check has empty command");
            return Ok(deny_hook(format!(
                "Check '{}' has empty command array in {}/.config/tools.toml\n\
                 Expected format: command = [\"binary\", \"arg1\", \"arg2\"]",
                check.name,
                canonical_dir.display()
            )));
        }
    }

    // Load project approvals
    let approvals = ProjectApprovals::load().await?;

    // Verify all checks are approved
    for check in &checks {
        let verification = approvals.verify_check(&canonical_dir, &check.name).await?;

        use crate::project_config::approvals::VerificationResult;
        match verification {
            VerificationResult::Approved => {
                info!(check_name = %check.name, "Check is approved");
            }
            VerificationResult::NotApproved => {
                error!(check_name = %check.name, "Check not approved");
                return Ok(deny_hook(format!(
                    "Check '{}' is not approved. Run: moriarty approve-project {}",
                    check.name,
                    canonical_dir.display()
                )));
            }
            VerificationResult::ConfigHashMismatch { expected, actual } => {
                error!(
                    check_name = %check.name,
                    expected = %expected,
                    actual = %actual,
                    "Config hash mismatch"
                );
                return Ok(deny_hook(format!(
                    "Project configuration changed. Run: moriarty approve-project {}",
                    canonical_dir.display()
                )));
            }
            VerificationResult::BinaryHashMismatch {
                item,
                expected,
                actual,
            } => {
                error!(
                    check_name = %check.name,
                    item = %item,
                    expected = %expected,
                    actual = %actual,
                    "Binary hash mismatch"
                );
                return Ok(deny_hook(format!(
                    "Check '{}' binary changed. Run: moriarty approve-project {}",
                    check.name,
                    canonical_dir.display()
                )));
            }
            VerificationResult::ItemNotApproved { item } => {
                error!(check_name = %check.name, item = %item, "Item not approved");
                return Ok(deny_hook(format!(
                    "Check '{}' not in approvals. Run: moriarty approve-project {}",
                    item,
                    canonical_dir.display()
                )));
            }
        }
    }

    // Run all checks with concurrency limits and timeout protection
    //
    // ## Resource Limits Rationale:
    //
    // CHECK_TIMEOUT_SECS (5 minutes): Balances allowing slow checks (e.g., linting large
    // codebases) while preventing indefinitely hanging processes that could DoS the system.
    // Most CI checks complete in seconds; 5 minutes provides generous headroom.
    //
    // MAX_CONCURRENT_CHECKS (4): Limits resource consumption when many checks are configured.
    // Prevents fork bombing or exhausting file descriptors if a malicious config defines hundreds
    // of checks. Value chosen to match typical CPU core count while still providing parallelism.
    //
    // MAX_OUTPUT_SIZE (1MB per check): Prevents individual checks from consuming excessive memory
    // via stdout/stderr. Typical check output is <10KB; 1MB allows detailed error messages and
    // verbose tooling while preventing abuse.
    //
    // MAX_TOTAL_OUTPUT (10MB total): Prevents aggregate memory exhaustion across all checks.
    // With 4 concurrent checks, this allows each to use its full 1MB quota with headroom.
    const CHECK_TIMEOUT_SECS: u64 = 300;
    const MAX_CONCURRENT_CHECKS: usize = 4;
    const MAX_OUTPUT_SIZE: usize = 1024 * 1024;
    const MAX_TOTAL_OUTPUT: usize = 10 * 1024 * 1024;

    // Execute checks with concurrency limits
    let timeout_duration = std::time::Duration::from_secs(CHECK_TIMEOUT_SECS);
    let canonical_dir_clone = canonical_dir.clone();

    let check_futures = futures::stream::iter(checks.into_iter().map(move |check| {
        let canonical_dir = canonical_dir_clone.clone();
        async move {
            // Split command into executable and arguments
            // Defensive handling despite line 240 validation because async timing allows config
            // modification between validation and execution, and graceful degradation is safer
            // than panicking in production.
            let Some((cmd, args)) = check.command.split_first() else {
                return (
                    check.name,
                    check.command,
                    Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "Check command array is empty",
                    )),
                );
            };

            let output = tokio::process::Command::new(cmd)
                .args(args)
                .current_dir(&canonical_dir)
                .output()
                .await;

            (check.name, check.command, output)
        }
    }))
    .buffer_unordered(MAX_CONCURRENT_CHECKS)
    .collect::<Vec<_>>();

    // Wait for all checks to complete with timeout
    let results = match tokio::time::timeout(timeout_duration, check_futures).await {
        Ok(results) => results,
        Err(_) => {
            error!(timeout_secs = CHECK_TIMEOUT_SECS, "Checks timed out");
            return Ok(deny_hook(format!(
                "Checks timed out after {} seconds",
                CHECK_TIMEOUT_SECS
            )));
        }
    };

    // Process results
    let mut failures = Vec::new();
    let mut all_output = Vec::new();
    let mut total_output_size = 0;

    for (check_name, command, output_result) in results {
        match output_result {
            Ok(output) => {
                let exit_code = output.status.code().unwrap_or(-1);

                // Truncate output to prevent excessive memory usage
                let truncate_output = |s: &str| -> String {
                    if s.len() > MAX_OUTPUT_SIZE {
                        format!(
                            "{}... [truncated {} bytes]",
                            &s[..MAX_OUTPUT_SIZE],
                            s.len() - MAX_OUTPUT_SIZE
                        )
                    } else {
                        s.to_string()
                    }
                };

                let stdout = truncate_output(&String::from_utf8_lossy(&output.stdout));
                let stderr = truncate_output(&String::from_utf8_lossy(&output.stderr));

                let combined_output = if stdout.is_empty() && stderr.is_empty() {
                    "<no output>".to_string()
                } else if stderr.is_empty() {
                    stdout.clone()
                } else if stdout.is_empty() {
                    stderr.clone()
                } else {
                    format!("stdout:\n{}\nstderr:\n{}", stdout, stderr)
                };

                total_output_size += combined_output.len();

                // Limit total output to prevent unbounded memory growth
                if total_output_size > MAX_TOTAL_OUTPUT {
                    error!(
                        total_size = total_output_size,
                        max_total = MAX_TOTAL_OUTPUT,
                        "Total check output exceeded limit"
                    );
                    return Ok(HookOutput {
                        continue_execution: None,
                        stop_reason: None,
                        suppress_output: None,
                        decision: Some(HookDecision::Block),
                        reason: Some(format!(
                            "Total check output exceeded {} MB limit. Checks produced too much output.",
                            MAX_TOTAL_OUTPUT / (1024 * 1024)
                        )),
                    });
                }

                info!(
                    check_name = %check_name,
                    exit_code = exit_code,
                    output_size = combined_output.len(),
                    "Check completed"
                );

                let output_entry = format!(
                    "Check '{}' [exit code: {}]:\n{}",
                    check_name, exit_code, combined_output
                );
                all_output.push(output_entry);

                if exit_code != 0 {
                    failures.push(format!(
                        "Check '{}' failed with exit code {}\nCommand: {:?}\n{}",
                        check_name, exit_code, command, combined_output
                    ));
                }
            }
            Err(e) => {
                error!(
                    check_name = %check_name,
                    error = %e,
                    "Failed to execute check"
                );
                failures.push(format!(
                    "Check '{}' failed to execute: {}\nCommand: {:?}",
                    check_name, e, command
                ));
            }
        }
    }

    // Return result based on failures
    info!(
        total_output_size = total_output_size,
        "Finished processing all check results"
    );

    if failures.is_empty() {
        info!("All checks passed");
        Ok(allow_hook())
    } else {
        error!(failure_count = failures.len(), "Some checks failed");
        Ok(deny_hook(format!(
            "Checks failed:\n\n{}",
            failures.join("\n\n")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tempfile::TempDir;

    /// Safe to use std::env::set_var because cargo nextest isolates each test in a separate process.
    fn setup_isolated_xdg_state() -> TempDir {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_STATE_HOME", temp_dir.path());
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
            r#"{"session_id":"s","transcript_path":"/t","cwd":"/c","permission_mode":"default","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{}}"#,
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

        let old_ask = r#"{"decision": "ask"}"#;
        let err = serde_json::from_str::<HookOutput>(old_ask)
            .expect_err("Should reject old 'ask' decision value");
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("unknown variant") || err_msg.contains("ask"),
            "Error should mention unknown variant, got: {}",
            err_msg
        );
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
            setup_approved_project_with_checks(vec![("test-check", check_command, check_args)])
                .await;

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

    /// Helper to set up isolated XDG_CONFIG_HOME
    fn setup_isolated_xdg_config() -> TempDir {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path());
        temp_dir
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
        use crate::project_config::approvals::{
            CommandApproval, ProjectApproval, ProjectApprovals,
        };
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
}
