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
//! `handle_stop_hook` reads `$CLAUDE_PROJECT_DIR`, returning `Allow` immediately if it is unset.
//! When set, it delegates to [`crate::checks::run_configured_checks`] and maps the returned
//! `CheckRunOutcome` onto allow/deny. That shared routine takes a **fail-open** approach,
//! reporting "no checks to run" (which the handler maps to `Allow`) when:
//! - Project directory doesn't exist or cannot be canonicalized
//! - `.config/tools.toml` cannot be loaded or parsed, except for duplicate check names
//! - No checks are defined in the configuration
//!
//! Duplicate check names fail **closed** because approvals and resolved programs are name-keyed;
//! allowing duplicates could authorize one check's arguments using another check's approval.
//!
//! **Rationale**: This design prioritizes developer experience and avoids breaking workflows
//! when projects don't use the checks feature. Since checks are opt-in security validations,
//! their absence or ordinary load/parse failures should not block execution.
//!
//! **Trade-offs**: An attacker who can manipulate the environment or filesystem to cause
//! config loading failures could bypass checks. However, this requires the same level of
//! access needed to modify approved binaries directly, so it doesn't meaningfully weaken
//! the security model. Once checks are configured and approved, the handler fails **closed**
//! on all verification failures (unapproved checks, argv/binary changes, check failures).
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
pub(crate) mod path_resolution;
pub mod report;
pub mod result;
pub mod tool_rules;
pub mod tracing;

use std::{io::Read, path::PathBuf, result::Result as StdResult, sync::Arc, time::Duration};

use ::tracing::{debug, error, info, warn};
use serde_json::{Map, Value};
use tokio::{task::JoinError, time::error::Elapsed};

use crate::{
    HooksCommand, checks::CheckRunOutcome, permission_mode::PermissionMode,
    user_config::load_user_config,
};
use parser::{
    HookDecision, HookEventData, HookInput, HookOutput, HookSpecificOutput, PermissionDecision,
    PreToolUseOutput,
};
use result::pretool_result;

const TOOL_ARGS_LOG_TRUNCATE_SIZE: usize = 50_000;
const SAFE_LOG_STRING_TRUNCATE_SIZE: usize = 4_096;
const REDACTED_LOG_VALUE: &str = "[redacted]";
// Tokio cannot cancel a running blocking syscall; this bounds hook latency, not pool occupancy.
const FILESYSTEM_EVALUATION_TIMEOUT: Duration = Duration::from_secs(2);

fn fail_closed_blocking<T: Default>(
    result: StdResult<StdResult<T, JoinError>, Elapsed>,
    operation: &'static str,
) -> T {
    match result {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => {
            warn!(%error, operation, "Blocking hook evaluation task failed; failing closed");
            T::default()
        }
        Err(error) => {
            warn!(%error, operation, "Blocking hook evaluation timed out; failing closed");
            T::default()
        }
    }
}

/// Execute hooks command
pub async fn exec_hooks(cmd: HooksCommand) -> miette::Result<()> {
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

fn rules_for_log(rules: &[String]) -> String {
    serde_json::to_string(rules).unwrap_or_else(|_| "[]".to_string())
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
async fn exec_hook() -> miette::Result<()> {
    exec_hook_impl(std::io::stdin()).await
}

async fn exec_hook_impl<R: Read>(reader: R) -> miette::Result<()> {
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
        let outcome = handle_pretool_hook(
            tool_name,
            tool_input,
            &hook_input.cwd,
            Some(hook_input.permission_mode),
        )
        .await?;
        let hook_output = outcome.output;

        let json_output = serde_json::to_string(&hook_output)
            .map_err(|e| miette::miette!("Failed to serialize HookOutput: {}", e))?;

        println!("{}", json_output);

        let tool_args = tool_args_for_log(tool_input);
        let result = pretool_result(&hook_output);
        let permission_mode = hook_input.permission_mode.to_string();
        let rules = rules_for_log(&outcome.contributors);
        info!(
            tool_name = %tool_name,
            tool_args = %tool_args,
            cwd = %hook_input.cwd,
            permission_mode = %permission_mode,
            rules_hash = outcome.rules_hash.as_deref().unwrap_or_default(),
            rule = outcome.rule.as_deref().unwrap_or_default(),
            rules = %rules,
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

/// A decision plus the provenance the completion log records. The hash, deciding rule, and ordered
/// contributors are meaningful only for the matching output; `None` (not an empty string) preserves
/// "no rules involved" until the completion event is serialized.
struct PretoolOutcome {
    output: HookOutput,
    /// Hash of the rule set that produced this decision (see
    /// [`crate::user_config::UserConfig::effective_hash`]); `None` when the config could not be
    /// loaded, so the fallback decision is not attributed to any rules.
    rules_hash: Option<String>,
    /// Name of the rule whose action produced `output`; `None` when no rule decided (passthrough,
    /// unconfigured-Ask, `NoMatch` prompt, or a post-filter re-validation that matched nothing).
    /// For a compound command this is the deciding leaf's rule from canonical evaluation.
    rule: Option<String>,
    /// Ordered rules that contributed to the evaluated decision; distinct same-named rules remain separate.
    contributors: Vec<String>,
}

/// tool_rules are deliberately checked before bash_rules so a tool rule can short-circuit Bash
/// evaluation entirely; reordering this would change which rule set decides and silently alter
/// recorded attributions.
async fn handle_pretool_hook(
    tool_name: &str,
    tool_input: &serde_json::Value,
    cwd: &str,
    mode: Option<PermissionMode>,
) -> miette::Result<PretoolOutcome> {
    let config = match load_config_or_ask().await {
        Ok(c) => c,
        Err(fallback) => {
            return Ok(PretoolOutcome {
                output: fallback,
                rules_hash: None,
                rule: None,
                contributors: Vec::new(),
            });
        }
    };

    let rules_hash = config.effective_hash();
    let outcome = |output, rule: Option<String>, contributors: Vec<String>| PretoolOutcome {
        output,
        rules_hash: Some(rules_hash.clone()),
        rule,
        contributors,
    };

    if let Some(rules) = &config.tool_rules
        && !rules.is_empty()
    {
        let engine = tool_rules::ToolRuleEngine::from_config(
            rules.clone(),
            config.pattern_fragments.clone(),
        );
        let result = engine.apply_rules(tool_name, tool_input, cwd, mode).await;

        match result {
            tool_rules::ToolRuleResult::Allowed { rule_name } => {
                info!(
                    tool_name = %tool_name,
                    rule = %rule_name,
                    "Tool call allowed by tool rule"
                );
                return Ok(outcome(
                    pretool_allow_hook(None),
                    Some(rule_name.clone()),
                    vec![rule_name],
                ));
            }
            tool_rules::ToolRuleResult::Denied { rule_name, reason } => {
                info!(
                    tool_name = %tool_name,
                    rule = %rule_name,
                    reason = %reason,
                    "Tool call denied by tool rule"
                );
                return Ok(outcome(
                    pretool_deny_hook(reason),
                    Some(rule_name.clone()),
                    vec![rule_name],
                ));
            }
            tool_rules::ToolRuleResult::Asked { rule_name } => {
                info!(
                    tool_name = %tool_name,
                    rule = %rule_name,
                    "Tool rule requests user permission"
                );
                return Ok(outcome(
                    pretool_ask_hook(),
                    Some(rule_name.clone()),
                    vec![rule_name],
                ));
            }
            tool_rules::ToolRuleResult::NoMatch => {
                debug!(tool_name = %tool_name, "No tool rules matched, continuing to engine-specific handling");
            }
        }
    }

    if tool_name == "Bash" {
        let (output, rule, contributors) =
            handle_bash_pretool_hook_with_config(tool_input, config, cwd, mode).await?;
        Ok(outcome(output, rule, contributors))
    } else {
        debug!(tool_name = %tool_name, "No tool rules matched for non-Bash tool, deferring to Claude Code");
        Ok(outcome(HookOutput::default(), None, Vec::new()))
    }
}

/// Test-only entry point for bash rule validation.
///
/// Production code routes through `handle_pretool_hook` instead. This wrapper is kept so
/// existing bash-rule tests can call it directly without going through the tool_rules layer.
#[cfg(test)]
async fn handle_bash_pretool_hook(
    tool_input: &serde_json::Value,
    cwd: &str,
) -> miette::Result<HookOutput> {
    let config = match load_config_or_ask().await {
        Ok(c) => c,
        Err(fallback) => return Ok(fallback),
    };
    handle_bash_pretool_hook_with_config(tool_input, config, cwd, None)
        .await
        .map(|(output, _rule, _contributors)| output)
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
    mode: Option<PermissionMode>,
) -> miette::Result<(HookOutput, Option<String>, Vec<String>)> {
    use bash_rules::{BashRuleEngine, PolicyOutcome, RuleResult};

    let command = tool_input
        .get("command")
        .and_then(|value| value.as_str())
        .ok_or_else(|| miette::miette!("Missing 'command' field in Bash tool_input"))?;
    info!(command, "Processing Bash PreToolUse hook");

    if config
        .bash_rules
        .as_ref()
        .is_none_or(|rules| rules.is_empty())
    {
        info!("No bash_rules configured, defaulting to Ask");
        return Ok((pretool_ask_hook(), None, Vec::new()));
    }

    let engine = Arc::new(BashRuleEngine::from_config(config)?);
    let evaluation = match engine.evaluate_live(command, cwd, mode).await {
        Ok(evaluation) => evaluation,
        Err(error) => return Ok(live_evaluation_failure_outcome(&error)),
    };
    let contributors = evaluation.contributors();
    let original_outcome = evaluation.original_outcome();

    match evaluation.rule_result() {
        RuleResult::Allowed { rule_name } => {
            info!(command, rule = %rule_name, "Bash command allowed by rule");
            Ok((pretool_allow_hook(None), Some(rule_name), contributors))
        }
        RuleResult::Denied { rule_name, reason } => {
            info!(command, rule = %rule_name, reason, "Bash command denied by rule");
            Ok((pretool_deny_hook(reason), Some(rule_name), contributors))
        }
        RuleResult::Modified {
            rule_name,
            new_command,
        } => {
            info!(original = command, modified = %new_command, rule = %rule_name, "Bash command modified by rule");
            let reason = match original_outcome {
                PolicyOutcome::ArgumentFilter { reason, .. } => reason.map(str::to_string),
                _ => Some(format!(
                    "Command modified by rule '{}' to: {}",
                    rule_name, new_command
                )),
            };
            let mut updated_tool_input = tool_input.clone();
            updated_tool_input["command"] = serde_json::Value::String(new_command);
            Ok((
                pretool_modify_hook(updated_tool_input, reason),
                Some(rule_name),
                contributors,
            ))
        }
        RuleResult::Asked { rule_name } => {
            info!(command, rule = %rule_name, "Bash rule requests user permission");
            Ok((pretool_ask_hook(), Some(rule_name), contributors))
        }
        RuleResult::ArgumentFiltered {
            rule_name,
            new_command,
            reason,
        } => {
            info!(original = command, filtered = %new_command, rule = %rule_name, "Filtered command validated and allowed");
            let mut updated_tool_input = tool_input.clone();
            updated_tool_input["command"] = serde_json::Value::String(new_command);
            let reason =
                reason.unwrap_or_else(|| format!("Arguments filtered by rule '{}'", rule_name));
            Ok((
                pretool_modify_hook(updated_tool_input, Some(reason)),
                Some(rule_name),
                contributors,
            ))
        }
        RuleResult::NoMatch => {
            debug!(command, "No bash rules matched, prompting user");
            Ok((pretool_ask_hook(), None, contributors))
        }
    }
}

fn live_evaluation_failure_outcome(
    error: &bash_rules::LiveEvaluationFailure,
) -> (HookOutput, Option<String>, Vec<String>) {
    match error {
        bash_rules::LiveEvaluationFailure::Timeout => {
            warn!("Live Bash evaluation timed out; asking without provenance");
        }
        bash_rules::LiveEvaluationFailure::Join(error) => {
            warn!(%error, "Live Bash evaluation task failed; asking without provenance");
        }
    }
    (pretool_ask_hook(), None, Vec::new())
}

/// Reads `$CLAUDE_PROJECT_DIR` (fail-open if unset) and runs the project's configured checks via
/// [`crate::checks::run_configured_checks`], mapping the outcome onto allow/deny. The fail-open /
/// fail-closed policy and resource limits live in that shared routine (see its docs and the
/// module-level "Security Model: Fail-Open Design").
async fn handle_stop_hook() -> miette::Result<HookOutput> {
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

    match crate::checks::run_configured_checks(&project_dir).await? {
        CheckRunOutcome::NoChecks(_) => Ok(allow_hook(None)),
        // deny_hook populates both `reason` and `system_message`. The pre-extraction code built the
        // total-output-cap denial by hand with `system_message: None`; routing every Blocked reason
        // through deny_hook makes that one path consistent with the other denials (and correct for
        // the non-verbose UI, per this module's "reason vs system_message" notes).
        CheckRunOutcome::Blocked(reason) => Ok(deny_hook(reason)),
        CheckRunOutcome::Ran { failures, .. } if failures.is_empty() => Ok(allow_hook(None)),
        CheckRunOutcome::Ran { failures, .. } => Ok(deny_hook(format!(
            "Checks failed:\n\n{}",
            failures.join("\n\n")
        ))),
    }
}

#[cfg(test)]
mod tests;
