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
pub(crate) mod command_split;
pub mod parser;
pub mod report;
pub mod result;
pub mod tool_rules;
pub mod tracing;

use std::{io::Read, path::PathBuf};

use ::tracing::{debug, error, info, warn};
use futures::stream::StreamExt;
use miette::Result;
use serde_json::{Map, Value};

use crate::project_config::{approvals::ProjectApprovals, config::load_project_settings};
use crate::user_config::load_user_config;
use crate::HooksCommand;
use parser::{
    HookDecision, HookEventData, HookInput, HookOutput, HookSpecificOutput, PermissionDecision,
    PreToolUseOutput,
};
use result::pretool_result;

const TOOL_ARGS_LOG_TRUNCATE_SIZE: usize = 50_000;
const SAFE_LOG_STRING_TRUNCATE_SIZE: usize = 4_096;
const REDACTED_LOG_VALUE: &str = "[redacted]";

/// Execute hooks command
pub async fn exec_hooks(cmd: HooksCommand) -> Result<()> {
    match cmd {
        HooksCommand::Exec => exec_hook().await,
        HooksCommand::Report(args) => {
            let timezone = crate::cost_report::parse_timezone(&args.timezone)?;
            report::run(
                args.dir,
                args.start_time,
                args.end_time,
                args.tool,
                args.result,
                timezone,
            )
            .await
        }
    }
}

fn hook_input_for_log(hook_input: &HookInput) -> String {
    match serde_json::to_value(hook_input) {
        Ok(value) => json_value_for_log(&value),
        Err(_) => "[hook input unavailable]".to_string(),
    }
}

fn tool_args_for_log(tool_input: &Value) -> String {
    truncate_log_field(&tool_input.to_string(), TOOL_ARGS_LOG_TRUNCATE_SIZE)
}

fn json_value_for_log(value: &Value) -> String {
    let sanitized_input = sanitize_log_value(None, value);
    let serialized =
        serde_json::to_string(&sanitized_input).unwrap_or_else(|_| sanitized_input.to_string());

    truncate_log_field(&serialized, TOOL_ARGS_LOG_TRUNCATE_SIZE)
}

fn sanitize_log_value(key: Option<&str>, value: &Value) -> Value {
    if key.is_some_and(is_sensitive_log_key) {
        return Value::String(REDACTED_LOG_VALUE.to_string());
    }

    match value {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| sanitize_log_value(None, item))
                .collect(),
        ),
        Value::Object(map) => Value::Object(sanitize_log_object(map)),
        Value::String(text) if key.is_some_and(is_safe_log_string_key) => {
            Value::String(truncate_log_field(text, SAFE_LOG_STRING_TRUNCATE_SIZE))
        }
        Value::String(text) => Value::String(format!("[string {} bytes]", text.len())),
        _ => value.clone(),
    }
}

fn sanitize_log_object(map: &Map<String, Value>) -> Map<String, Value> {
    map.iter()
        .map(|(key, value)| (key.clone(), sanitize_log_value(Some(key), value)))
        .collect()
}

fn is_sensitive_log_key(key: &str) -> bool {
    let uppercase_key = key.to_ascii_uppercase();
    ["TOKEN", "SECRET", "KEY", "PASSWORD"]
        .iter()
        .any(|pattern| uppercase_key.contains(pattern))
}

fn is_safe_log_string_key(key: &str) -> bool {
    matches!(
        key,
        "cwd" | "file_path" | "hook_event_name" | "permission_mode" | "session_id" | "tool_name"
    ) || key.ends_with("_path")
        || key == "path"
}

fn truncate_log_field(field: &str, max_size: usize) -> String {
    if field.len() <= max_size {
        return field.to_string();
    }

    let safe_truncate = field
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= max_size)
        .last()
        .unwrap_or(0);

    format!(
        "{}... [truncated {} bytes]",
        &field[..safe_truncate],
        field.len() - safe_truncate
    )
}

/// Parse failures must surface as a nonzero exit code so Claude Code can distinguish a hook
/// crash from a deliberate decision.
async fn exec_hook() -> Result<()> {
    exec_hook_impl(std::io::stdin()).await
}

async fn exec_hook_impl<R: Read>(reader: R) -> Result<()> {
    // Global tracing subscriber initialization races are acceptable in tests because nextest's
    // process isolation guarantees no cross-contamination, and failed initialization doesn't
    // affect correctness.
    let _guard = match tracing::init_tracing().await {
        Ok(guard) => Some(guard),
        Err(_) if cfg!(test) => None,
        Err(e) => return Err(e),
    };

    // Cap stdin to prevent DoS via memory exhaustion.
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

    let hook_input_log = hook_input_for_log(&hook_input);
    info!(hook_input = %hook_input_log, "Successfully parsed hook input");

    if let HookEventData::PreToolUse {
        ref tool_name,
        ref tool_input,
    } = hook_input.event_data
    {
        let outcome = handle_pretool_hook(tool_name, tool_input, &hook_input.cwd).await?;
        let hook_output = outcome.output;

        let json_output = serde_json::to_string(&hook_output)
            .map_err(|e| miette::miette!("Failed to serialize HookOutput: {}", e))?;

        println!("{}", json_output);

        let tool_args = tool_args_for_log(tool_input);
        let result = pretool_result(&hook_output);
        info!(
            tool_name = %tool_name,
            tool_args = %tool_args,
            cwd = %hook_input.cwd,
            rules_hash = outcome.rules_hash.as_deref().unwrap_or_default(),
            rule = outcome.rule.as_deref().unwrap_or_default(),
            result = result.as_str(),
            ?hook_output,
            "PreToolUse hook completed"
        );
        return Ok(());
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

// The reason/system_message duplication in these constructors is deliberate; see the module-level
// "Hook Output Fields: reason vs system_message" section.
fn allow_hook(message: Option<String>) -> HookOutput {
    HookOutput {
        decision: Some(HookDecision::Approve),
        reason: message.clone(),
        system_message: message,
        ..HookOutput::default()
    }
}

fn deny_hook(reason: impl Into<String>) -> HookOutput {
    let message = reason.into();
    HookOutput {
        decision: Some(HookDecision::Block),
        reason: Some(message.clone()),
        system_message: Some(message),
        ..HookOutput::default()
    }
}

fn pretool_hook(
    decision: PermissionDecision,
    reason: Option<String>,
    updated_input: Option<serde_json::Value>,
) -> HookOutput {
    HookOutput {
        system_message: reason.clone(),
        hook_specific_output: Some(HookSpecificOutput::PreToolUse(PreToolUseOutput {
            hook_event_name: "PreToolUse".to_string(),
            permission_decision: Some(decision),
            permission_decision_reason: reason,
            updated_input,
        })),
        ..HookOutput::default()
    }
}

fn pretool_allow_hook(reason: Option<String>) -> HookOutput {
    pretool_hook(PermissionDecision::Allow, reason, None)
}

fn pretool_deny_hook(reason: String) -> HookOutput {
    pretool_hook(PermissionDecision::Deny, Some(reason), None)
}

fn pretool_ask_hook() -> HookOutput {
    pretool_hook(PermissionDecision::Ask, None, None)
}

fn pretool_modify_hook(new_input: serde_json::Value, reason: Option<String>) -> HookOutput {
    pretool_hook(PermissionDecision::Allow, reason, Some(new_input))
}

/// `Err` carries the ready-to-emit Ask fallback rather than an error type so call sites can
/// `return Ok(fallback)` directly — a load failure is a decision, not a hook failure.
async fn load_config_or_ask() -> std::result::Result<crate::user_config::UserConfig, HookOutput> {
    match load_user_config().await {
        Ok(cfg) => Ok(cfg),
        Err(e) => {
            warn!(error = %e, "Failed to load user config, defaulting to Ask");
            Err(pretool_ask_hook())
        }
    }
}

/// A decision plus the provenance the completion log records. Grouped because the three values are
/// produced together and only meaningful as a unit: the log line must attribute the decision to the
/// rule set and rule that made it, and `None` (not an empty string) marks "no rules involved" so
/// the report layer can distinguish legacy lines from genuinely unattributed decisions.
struct PretoolOutcome {
    output: HookOutput,
    /// Hash of the rule set that produced this decision (see
    /// [`crate::user_config::UserConfig::effective_hash`]); `None` when the config could not be
    /// loaded, so the fallback decision is not attributed to any rules.
    rules_hash: Option<String>,
    /// Name of the rule whose action produced `output`; `None` when no rule decided (passthrough,
    /// unconfigured-Ask, `NoMatch` prompt, or a post-filter re-validation that matched nothing).
    /// For a compound command this is the deciding leaf's rule, mirroring `merge_results`.
    rule: Option<String>,
}

/// tool_rules are deliberately checked before bash_rules so a tool rule can short-circuit Bash
/// evaluation entirely; reordering this would change which rule set decides and silently alter
/// recorded attributions.
async fn handle_pretool_hook(
    tool_name: &str,
    tool_input: &serde_json::Value,
    cwd: &str,
) -> Result<PretoolOutcome> {
    let config = match load_config_or_ask().await {
        Ok(c) => c,
        Err(fallback) => {
            return Ok(PretoolOutcome {
                output: fallback,
                rules_hash: None,
                rule: None,
            })
        }
    };

    let rules_hash = config.effective_hash();
    let outcome = |output, rule: Option<String>| PretoolOutcome {
        output,
        rules_hash: Some(rules_hash.clone()),
        rule,
    };

    if let Some(rules) = &config.tool_rules {
        if !rules.is_empty() {
            let engine = tool_rules::ToolRuleEngine::from_config(
                rules.clone(),
                config.pattern_fragments.clone(),
            );
            let result = engine.apply_rules(tool_name, tool_input, cwd).await;

            match result {
                tool_rules::ToolRuleResult::Allowed { rule_name } => {
                    info!(
                        tool_name = %tool_name,
                        rule = %rule_name,
                        "Tool call allowed by tool rule"
                    );
                    return Ok(outcome(pretool_allow_hook(None), Some(rule_name)));
                }
                tool_rules::ToolRuleResult::Denied { rule_name, reason } => {
                    info!(
                        tool_name = %tool_name,
                        rule = %rule_name,
                        reason = %reason,
                        "Tool call denied by tool rule"
                    );
                    return Ok(outcome(pretool_deny_hook(reason), Some(rule_name)));
                }
                tool_rules::ToolRuleResult::Asked { rule_name } => {
                    info!(
                        tool_name = %tool_name,
                        rule = %rule_name,
                        "Tool rule requests user permission"
                    );
                    return Ok(outcome(pretool_ask_hook(), Some(rule_name)));
                }
                tool_rules::ToolRuleResult::NoMatch => {
                    debug!(tool_name = %tool_name, "No tool rules matched, continuing to engine-specific handling");
                }
            }
        }
    }

    if tool_name == "Bash" {
        let (output, rule) = handle_bash_pretool_hook_with_config(tool_input, config, cwd).await?;
        Ok(outcome(output, rule))
    } else {
        debug!(tool_name = %tool_name, "No tool rules matched for non-Bash tool, deferring to Claude Code");
        Ok(outcome(HookOutput::default(), None))
    }
}

/// Test-only entry point for bash rule validation.
///
/// Production code routes through `handle_pretool_hook` instead. This wrapper is kept so
/// existing bash-rule tests can call it directly without going through the tool_rules layer.
#[cfg(test)]
async fn handle_bash_pretool_hook(tool_input: &serde_json::Value, cwd: &str) -> Result<HookOutput> {
    let config = match load_config_or_ask().await {
        Ok(c) => c,
        Err(fallback) => return Ok(fallback),
    };
    handle_bash_pretool_hook_with_config(tool_input, config, cwd)
        .await
        .map(|(output, _rule)| output)
}

/// Apply bash_rules from a pre-loaded config to validate Bash commands.
///
/// `cwd` must be the verbatim value from the hook input — not canonicalized — because rule
/// normalization strips it as a literal string prefix, and the recorded `cwd` must round-trip
/// through `rules replay` to reproduce the same normalization.
///
/// The returned rule name is the one whose *action* produced the decision (`None` when no rule
/// decided), which the completion log records as attribution.
async fn handle_bash_pretool_hook_with_config(
    tool_input: &serde_json::Value,
    config: crate::user_config::UserConfig,
    cwd: &str,
) -> Result<(HookOutput, Option<String>)> {
    use bash_rules::{BashRuleEngine, RuleResult};

    let command = tool_input
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| miette::miette!("Missing 'command' field in Bash tool_input"))?;

    info!(command = %command, "Processing Bash PreToolUse hook");

    let bash_rules = match config.bash_rules {
        Some(rules) if !rules.is_empty() => rules,
        _ => {
            info!("No bash_rules configured, defaulting to Ask");
            return Ok((pretool_ask_hook(), None));
        }
    };

    let engine = BashRuleEngine::from_config(bash_rules, config.pattern_fragments)?;
    let result = engine.apply_rules_compound(command, cwd);

    match result {
        RuleResult::Allowed { rule_name } => {
            info!(
                command = %command,
                rule = %rule_name,
                "Bash command allowed by rule"
            );
            Ok((pretool_allow_hook(None), Some(rule_name)))
        }
        RuleResult::Denied { rule_name, reason } => {
            info!(
                command = %command,
                rule = %rule_name,
                reason = %reason,
                "Bash command denied by rule"
            );
            Ok((pretool_deny_hook(reason), Some(rule_name)))
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

            Ok((
                pretool_modify_hook(updated_tool_input, Some(reason)),
                Some(rule_name),
            ))
        }
        RuleResult::Asked { rule_name } => {
            info!(
                command = %command,
                rule = %rule_name,
                "Bash rule requests user permission"
            );
            Ok((pretool_ask_hook(), Some(rule_name)))
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

            let recheck_result = engine.apply_rules_compound(&new_command, cwd);

            match recheck_result {
                RuleResult::Allowed {
                    rule_name: allow_rule,
                } => {
                    info!(
                        filtered_command = %new_command,
                        allowed_by = %allow_rule,
                        "Filtered command validated and allowed"
                    );

                    let mut updated_tool_input = tool_input.clone();
                    updated_tool_input["command"] = serde_json::Value::String(new_command);

                    let final_reason = reason
                        .unwrap_or_else(|| format!("Arguments filtered by rule '{}'", rule_name));

                    // The visible effect (the rewritten command) is the filter rule's action, so
                    // attribution names it rather than the allow rule that merely re-validated.
                    Ok((
                        pretool_modify_hook(updated_tool_input, Some(final_reason)),
                        Some(rule_name),
                    ))
                }
                RuleResult::Denied {
                    rule_name: deny_rule,
                    reason: deny_reason,
                } => {
                    warn!(
                        filtered_command = %new_command,
                        reason = %deny_reason,
                        "Filtered command was denied by rules"
                    );
                    Ok((pretool_deny_hook(deny_reason), Some(deny_rule)))
                }
                RuleResult::NoMatch => {
                    info!(
                        filtered_command = %new_command,
                        "Filtered command doesn't match any allow rule, asking user"
                    );
                    Ok((pretool_ask_hook(), None))
                }
                RuleResult::Asked {
                    rule_name: ask_rule,
                } => {
                    info!(
                        filtered_command = %new_command,
                        "Filtered command requires user confirmation"
                    );
                    Ok((pretool_ask_hook(), Some(ask_rule)))
                }
                RuleResult::Modified {
                    rule_name: modify_rule,
                    new_command: further_modified,
                } => {
                    info!(
                        filtered_command = %new_command,
                        further_modified = %further_modified,
                        "Filtered command was modified again by another rule"
                    );

                    let mut updated_tool_input = tool_input.clone();
                    updated_tool_input["command"] = serde_json::Value::String(further_modified);

                    Ok((
                        pretool_modify_hook(updated_tool_input, reason),
                        Some(modify_rule),
                    ))
                }
                RuleResult::ArgumentFiltered {
                    rule_name: chained_rule,
                    ..
                } => {
                    // Prevent infinite loops - don't allow chained argument filtering
                    warn!(
                        filtered_command = %new_command,
                        "Filtered command matched another ArgumentFilter rule, asking user to prevent loops"
                    );
                    Ok((pretool_ask_hook(), Some(chained_rule)))
                }
            }
        }
        RuleResult::NoMatch => {
            debug!(command = %command, "No bash rules matched, prompting user");
            Ok((pretool_ask_hook(), None))
        }
    }
}

/// Every early `allow_hook(None)` return below is the fail-open path described in the
/// module-level "Security Model: Fail-Open Design" section; once checks exist and are
/// approved, any verification failure fails closed.
async fn handle_stop_hook() -> Result<HookOutput> {
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

    let repository_root = match crate::repository::detect_repository_root(&project_dir) {
        Ok(root) => {
            info!(
                project_dir = %project_dir.display(),
                repository_root = %root.display(),
                "Detected repository root"
            );
            root
        }
        Err(e) => {
            error!(
                project_dir = %project_dir.display(),
                error = %e,
                "Failed to detect repository root"
            );
            return Ok(allow_hook(None));
        }
    };

    let config = match load_project_settings(repository_root.clone()).await {
        Ok(config) => config,
        Err(e) => {
            info!(
                error = %e,
                "No .config/tools.toml found, allowing without checks"
            );
            return Ok(allow_hook(None));
        }
    };

    let checks = match config.checks {
        Some(checks) if !checks.is_empty() => checks,
        _ => {
            info!("No checks defined in config, allowing");
            return Ok(allow_hook(None));
        }
    };

    info!(check_count = checks.len(), "Found checks to run");

    for check in &checks {
        if check.command.is_empty() {
            error!(check_name = %check.name, "Check has empty command");
            return Ok(deny_hook(format!(
                "Check '{}' has empty command array in {}/.config/tools.toml\n\
                 Expected format: command = [\"binary\", \"arg1\", \"arg2\"]",
                check.name,
                repository_root.display()
            )));
        }
    }

    let approvals = ProjectApprovals::load().await?;

    for check in &checks {
        let verification = approvals
            .verify_check(&repository_root, &check.name)
            .await?;

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
                    repository_root.display()
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
                    repository_root.display()
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
                    repository_root.display()
                )));
            }
            VerificationResult::ItemNotApproved { item } => {
                error!(check_name = %check.name, item = %item, "Item not approved");
                return Ok(deny_hook(format!(
                    "Check '{}' not in approvals. Run: moriarty approve-project {}",
                    item,
                    repository_root.display()
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

    let timeout_duration = std::time::Duration::from_secs(CHECK_TIMEOUT_SECS);
    let repository_root_clone = repository_root.clone();

    let check_futures = futures::stream::iter(checks.into_iter().map(move |check| {
        let repository_root = repository_root_clone.clone();
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
                .current_dir(&repository_root)
                .output()
                .await;

            (check.name, check.command, output)
        }
    }))
    .buffer_unordered(MAX_CONCURRENT_CHECKS)
    .collect::<Vec<_>>();

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
