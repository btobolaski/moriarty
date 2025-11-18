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
//!
//! ## Hook Output Fields: `reason` vs `system_message`
//!
//! Hook outputs populate multiple message fields to support different Claude Code UI modes:
//!
//! - **`reason`** / **`permission_decision_reason`**: Detailed message for logs and verbose mode
//!   (Ctrl+O). May include technical details, command output, or debugging information.
//!
//! - **`system_message`**: User-facing message shown in Claude Code UI without verbose mode.
//!   Should be concise and actionable (e.g., "Check 'semgrep' binary changed. Run: moriarty
//!   approve-project /path").
//!
//! **Why both fields?** The duplication ensures users receive feedback regardless of Claude Code's
//! verbosity settings:
//! - Without verbose mode: Only `system_message` is shown to the user
//! - With verbose mode (Ctrl+O): Both `reason` and `system_message` appear in logs
//!
//! While this duplicates content in the JSON payload, it's required by Claude Code's protocol
//! to provide consistent user feedback. The alternative (showing only verbose output) would
//! require users to enable verbose mode to understand why hooks blocked their commands.

pub mod bash_rules;
pub mod parser;
pub mod tracing;

use crate::project_config::{approvals::ProjectApprovals, config::load_project_settings};
use crate::user_config::load_user_config;
use crate::HooksCommand;
use ::tracing::{debug, error, info, warn};
use futures::stream::StreamExt;
use miette::Result;
use parser::{
    HookDecision, HookEventData, HookOutput, HookSpecificOutput, PermissionDecision,
    PreToolUseOutput,
};
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

    debug!(bytes = bytes_read, "Received hook input from stdin");

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

    // Log hook_input as JSON for better structured logging. The fallback to Debug format
    // is defensive - HookInput derives Serialize and should always serialize successfully.
    if let Ok(json) = serde_json::to_string(&hook_input) {
        info!(hook_input = %json, "Successfully parsed hook input");
    } else {
        info!(
            ?hook_input,
            "Successfully parsed hook input (JSON serialization failed)"
        );
    }

    if let HookEventData::PreToolUse {
        ref tool_name,
        ref tool_input,
    } = hook_input.event_data
    {
        if tool_name == "Bash" {
            let hook_output = handle_bash_pretool_hook(tool_input).await?;

            let json_output = serde_json::to_string(&hook_output)
                .map_err(|e| miette::miette!("Failed to serialize HookOutput: {}", e))?;

            println!("{}", json_output);

            info!(?hook_output, "Bash PreToolUse hook completed");
            return Ok(());
        }
    }

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

/// Allows execution with optional user-facing feedback.
///
/// When `message` is provided, it's sent as both `reason` (for verbose mode/logs)
/// and `system_message` (for immediate user feedback without verbose mode).
///
/// Use `system_message` when you want to provide user-facing feedback even without
/// verbose mode enabled, such as informing users that checks passed or explaining
/// why their command was allowed despite appearing potentially dangerous.
fn allow_hook(message: Option<String>) -> HookOutput {
    HookOutput {
        continue_execution: None,
        stop_reason: None,
        suppress_output: None,
        decision: Some(HookDecision::Approve),
        reason: message.clone(),
        system_message: message,
        permission_decision: None,
        hook_specific_output: None,
    }
}

/// Denies execution with user-facing feedback.
///
/// The message is sent as both `reason` (for verbose mode/logs) and `system_message`
/// (for immediate user feedback without verbose mode).
///
/// Use this when denying commands that users might legitimately attempt, to explain
/// why the command was blocked (e.g., "Check 'semgrep' binary changed").
fn deny_hook(reason: impl Into<String>) -> HookOutput {
    let message = reason.into();
    HookOutput {
        continue_execution: None,
        stop_reason: None,
        suppress_output: None,
        decision: Some(HookDecision::Block),
        reason: Some(message.clone()),
        system_message: Some(message),
        permission_decision: None,
        hook_specific_output: None,
    }
}

/// Allows PreToolUse execution with optional user-facing feedback.
///
/// If a reason is provided, it's sent as both `permission_decision_reason` (for hook-specific output)
/// and `system_message` (for immediate user feedback without verbose mode).
///
/// Use the reason parameter when you want to explain why a command was allowed, especially
/// when a bash rule explicitly permits something that might look suspicious (e.g., "Allowed
/// by whitelist rule 'safe-rm'").
fn pretool_allow_hook(reason: Option<String>) -> HookOutput {
    HookOutput {
        continue_execution: None,
        stop_reason: None,
        suppress_output: None,
        decision: None,
        reason: None,
        system_message: reason.clone(),
        permission_decision: None,
        hook_specific_output: Some(HookSpecificOutput::PreToolUse(PreToolUseOutput {
            hook_event_name: "PreToolUse".to_string(),
            permission_decision: Some(PermissionDecision::Allow),
            permission_decision_reason: reason,
            updated_input: None,
        })),
    }
}

/// Denies PreToolUse execution with user-facing feedback.
///
/// The reason is sent as both `permission_decision_reason` (for hook-specific output)
/// and `system_message` (for immediate user feedback without verbose mode).
///
/// Use this when bash rules deny a command, providing clear feedback about why the
/// command was blocked (e.g., "Dangerous recursive delete" or "Command blocked by
/// security rule 'deny-rm-rf'").
fn pretool_deny_hook(reason: String) -> HookOutput {
    HookOutput {
        continue_execution: None,
        stop_reason: None,
        suppress_output: None,
        decision: None,
        reason: None,
        system_message: Some(reason.clone()),
        permission_decision: None,
        hook_specific_output: Some(HookSpecificOutput::PreToolUse(PreToolUseOutput {
            hook_event_name: "PreToolUse".to_string(),
            permission_decision: Some(PermissionDecision::Deny),
            permission_decision_reason: Some(reason),
            updated_input: None,
        })),
    }
}

/// Asks the user for permission to execute a command.
///
/// Use this when bash rules don't have enough information to make a decision
/// (e.g., command doesn't match any allow/deny rules, or an Ask rule matched).
fn pretool_ask_hook() -> HookOutput {
    HookOutput {
        continue_execution: None,
        stop_reason: None,
        suppress_output: None,
        decision: None,
        reason: None,
        system_message: None,
        permission_decision: None,
        hook_specific_output: Some(HookSpecificOutput::PreToolUse(PreToolUseOutput {
            hook_event_name: "PreToolUse".to_string(),
            permission_decision: Some(PermissionDecision::Ask),
            permission_decision_reason: None,
            updated_input: None,
        })),
    }
}

/// Modifies a command and allows execution with optional user-facing feedback.
///
/// If a reason is provided, it's sent as both `permission_decision_reason` (for hook-specific output)
/// and `system_message` (for immediate user feedback without verbose mode).
///
/// Use this when bash rules modify a command to make it safer (e.g., adding safety flags
/// or filtering dangerous arguments). The reason should explain what was changed and why
/// (e.g., "Command modified by rule 'add-dry-run': added --dry-run flag").
fn pretool_modify_hook(new_input: serde_json::Value, reason: Option<String>) -> HookOutput {
    HookOutput {
        continue_execution: None,
        stop_reason: None,
        suppress_output: None,
        decision: None,
        reason: None,
        system_message: reason.clone(),
        permission_decision: None,
        hook_specific_output: Some(HookSpecificOutput::PreToolUse(PreToolUseOutput {
            hook_event_name: "PreToolUse".to_string(),
            permission_decision: Some(PermissionDecision::Allow),
            permission_decision_reason: reason,
            updated_input: Some(new_input),
        })),
    }
}

/// Apply user-level bash_rules from ~/.config/moriarty/tool_rules.toml to validate Bash commands.
///
/// Uses fail-open design: returns Ask when rules are missing or unconfigured, ensuring bash_rules
/// remain opt-in without breaking workflows.
async fn handle_bash_pretool_hook(tool_input: &serde_json::Value) -> Result<HookOutput> {
    use bash_rules::{BashRuleEngine, RuleResult};

    let command = tool_input
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| miette::miette!("Missing 'command' field in Bash tool_input"))?;

    info!(command = %command, "Processing Bash PreToolUse hook");

    let config = match load_user_config().await {
        Ok(cfg) => cfg,
        Err(e) => {
            warn!(error = %e, "Failed to load user config, defaulting to Ask");
            return Ok(pretool_ask_hook());
        }
    };

    let bash_rules = match config.bash_rules {
        Some(rules) if !rules.is_empty() => rules,
        _ => {
            info!("No bash_rules configured, defaulting to Ask");
            return Ok(pretool_ask_hook());
        }
    };

    let engine = BashRuleEngine::from_config(bash_rules, config.pattern_fragments)?;
    let result = engine.apply_rules(command);

    match result {
        RuleResult::Allowed { rule_name } => {
            info!(
                command = %command,
                rule = %rule_name,
                "Bash command allowed by rule"
            );
            Ok(pretool_allow_hook(None))
        }
        RuleResult::Denied { rule_name, reason } => {
            info!(
                command = %command,
                rule = %rule_name,
                reason = %reason,
                "Bash command denied by rule"
            );
            Ok(pretool_deny_hook(reason))
        }
        RuleResult::Modified {
            rule_name,
            new_command,
        } => {
            info!(
                original = %command,
                modified = %new_command,
                rule = %rule_name,
                "Bash command modified by rule"
            );
            let mut updated_tool_input = tool_input.clone();
            let reason = format!(
                "Command modified by rule '{}' to: {}",
                rule_name, new_command
            );
            updated_tool_input["command"] = serde_json::Value::String(new_command);

            Ok(pretool_modify_hook(updated_tool_input, Some(reason)))
        }
        RuleResult::Asked { rule_name } => {
            info!(
                command = %command,
                rule = %rule_name,
                "Bash rule requests user permission"
            );
            Ok(pretool_ask_hook())
        }
        RuleResult::ArgumentFiltered {
            rule_name,
            new_command,
            reason,
        } => {
            info!(
                original = %command,
                filtered = %new_command,
                rule = %rule_name,
                "Command arguments filtered, re-validating"
            );

            // CRITICAL: Re-validate filtered command for security
            let recheck_result = engine.apply_rules(&new_command);

            match recheck_result {
                RuleResult::Allowed {
                    rule_name: allow_rule,
                } => {
                    // Filtered command is explicitly allowed, safe to execute
                    info!(
                        filtered_command = %new_command,
                        allowed_by = %allow_rule,
                        "Filtered command validated and allowed"
                    );

                    let mut updated_tool_input = tool_input.clone();
                    updated_tool_input["command"] = serde_json::Value::String(new_command);

                    let final_reason = reason
                        .unwrap_or_else(|| format!("Arguments filtered by rule '{}'", rule_name));

                    Ok(pretool_modify_hook(updated_tool_input, Some(final_reason)))
                }
                RuleResult::Denied {
                    reason: deny_reason,
                    ..
                } => {
                    // Filtered command matched a deny rule
                    warn!(
                        filtered_command = %new_command,
                        reason = %deny_reason,
                        "Filtered command was denied by rules"
                    );
                    Ok(pretool_deny_hook(deny_reason))
                }
                RuleResult::NoMatch => {
                    // Filtered command doesn't match any rule, ask user
                    info!(
                        filtered_command = %new_command,
                        "Filtered command doesn't match any allow rule, asking user"
                    );
                    Ok(pretool_ask_hook())
                }
                RuleResult::Asked { .. } => {
                    // Filtered command matched an Ask rule
                    info!(
                        filtered_command = %new_command,
                        "Filtered command requires user confirmation"
                    );
                    Ok(pretool_ask_hook())
                }
                RuleResult::Modified {
                    new_command: further_modified,
                    ..
                } => {
                    // Filtered command was modified again (chained modifications)
                    info!(
                        filtered_command = %new_command,
                        further_modified = %further_modified,
                        "Filtered command was modified again by another rule"
                    );

                    let mut updated_tool_input = tool_input.clone();
                    updated_tool_input["command"] = serde_json::Value::String(further_modified);

                    Ok(pretool_modify_hook(updated_tool_input, reason))
                }
                RuleResult::ArgumentFiltered { .. } => {
                    // Prevent infinite loops - don't allow chained argument filtering
                    warn!(
                        filtered_command = %new_command,
                        "Filtered command matched another ArgumentFilter rule, asking user to prevent loops"
                    );
                    Ok(pretool_ask_hook())
                }
            }
        }
        RuleResult::NoMatch => {
            debug!(command = %command, "No bash rules matched, prompting user");
            Ok(pretool_ask_hook())
        }
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
            return Ok(allow_hook(None));
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
            return Ok(allow_hook(None));
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
            return Ok(allow_hook(None));
        }
    };

    // Check if there are any checks defined
    let checks = match config.checks {
        Some(checks) if !checks.is_empty() => checks,
        _ => {
            info!("No checks defined in config, allowing");
            return Ok(allow_hook(None));
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
                        system_message: None,
                        permission_decision: None,
                        hook_specific_output: None,
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
        Ok(allow_hook(None))
    } else {
        error!(failure_count = failures.len(), "Some checks failed");
        Ok(deny_hook(format!(
            "Checks failed:\n\n{}",
            failures.join("\n\n")
        )))
    }
}

#[cfg(test)]
mod tests;
