//! Test runner for parallel execution of project tools and checks.
//!
//! This module provides functionality to run all configured project tools
//! (lint, test, build, format) and checks in parallel and display comprehensive output.
//!
//! # Usage
//!
//! ```no_run
//! use moriarty::test_runner;
//! use moriarty::TestCommand;
//! use std::path::PathBuf;
//!
//! # async fn example() -> miette::Result<()> {
//! let cmd = TestCommand::ProjectTools {
//!     project_dir: PathBuf::from("/path/to/project"),
//! };
//! test_runner::exec_test(cmd).await?;
//! # Ok(())
//! # }
//! ```

use std::{
    io::{self, Read, Write},
    path::PathBuf,
};

use miette::{IntoDiagnostic, WrapErr};
use serde_json::json;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::{
    hooks::{
        bash_rules::{
            BashRuleEngine, CommandTrace, EndpointAnalysis, Evaluation, EvaluationContext,
            EvaluationPurpose, FilesystemAnalysis, FilterContinuation, LeafIdentity,
            MatchedCommandRule, MatchedRedirectRule, OriginalContinuation, PolicyAnalysis,
            RedirectCheckAnalysis, RedirectEndpointKind, RedirectEndpointTrace, RewrittenBailTrace,
            RuleMatchExplanation, RuleResult, SubCommandTrace,
        },
        command_split::{AliasBinding, BailReason},
    },
    permission_mode::PermissionMode,
    project_config::runner::{CommandOutput, VerifiedProject, verify_and_load_project},
    user_config::load_user_config_from,
};

pub async fn exec_test(cmd: crate::TestCommand) -> miette::Result<()> {
    match cmd {
        crate::TestCommand::ProjectTools { project_dir } => run_project_tools(project_dir).await,
        crate::TestCommand::Checks { project_dir } => run_checks(project_dir).await,
        crate::TestCommand::BashRules {
            command,
            config,
            json,
            explain,
            cwd,
            mode,
        } => test_bash_rules(command, config, json, explain, cwd, mode)
            .await
            .map(|_| ()),
    }
}

/// Generic function to run items (tools or checks) with common display logic.
///
/// Eliminates duplication between run_project_tools and run_checks by parameterizing
/// the item type name and execution method.
async fn run_items<F, Fut>(
    project_dir: PathBuf,
    item_type_singular: &str,
    item_type_plural: &str,
    get_item_names: impl FnOnce(&VerifiedProject) -> Option<Vec<String>>,
    run_items: F,
) -> miette::Result<()>
where
    F: FnOnce(VerifiedProject) -> Fut,
    Fut: std::future::Future<Output = miette::Result<Vec<CommandOutput>>>,
{
    let project = verify_and_load_project(project_dir).await?;

    println!(
        "Running project {} for: {}\n",
        item_type_plural,
        project.canonical_dir.display()
    );

    let item_names = match get_item_names(&project) {
        Some(names) if !names.is_empty() => names,
        _ => {
            println!("No {} configured in .config/tools.toml", item_type_plural);
            return Ok(());
        }
    };

    println!(
        "Found {} configured {}{}: {}\n",
        item_names.len(),
        item_type_singular,
        if item_names.len() == 1 { "" } else { "s" },
        item_names.join(", ")
    );

    let results = run_items(project).await?;

    for output in &results {
        println!("{}", "━".repeat(80));
        println!("{}: {}", capitalize(item_type_singular), output.name);
        println!("Command: {:?}", output.command);
        println!(
            "Exit Code: {}",
            output
                .exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );
        println!("{}", "━".repeat(80));
        println!();

        if !output.stdout.is_empty() {
            println!("STDOUT:");
            println!("{}", output.stdout);
            println!();
        }

        if !output.stderr.is_empty() {
            println!("STDERR:");
            println!("{}", output.stderr);
            println!();
        }
    }

    println!("{}", "━".repeat(80));
    println!("Summary:");
    println!("{}", "━".repeat(80));

    let mut failed_count = 0;
    for output in &results {
        let success = matches!(output.exit_code, Some(0));
        let symbol = if success { "✓" } else { "✗" };
        let exit_code_str = output
            .exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "unknown".to_string());

        println!("{} {} (exit code: {})", symbol, output.name, exit_code_str);

        if !success {
            failed_count += 1;
        }
    }

    println!();
    if failed_count == 0 {
        println!("All {} completed successfully!", item_type_plural);
    } else {
        println!(
            "{} {}{} failed!",
            failed_count,
            item_type_singular,
            if failed_count == 1 { "" } else { "s" }
        );
        // Use process::exit instead of returning Err to provide a clean exit code
        // for CI/CD integration. Returning Err would print a debug error message
        // that duplicates the summary we just displayed.
        std::process::exit(1);
    }

    Ok(())
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().chain(chars).collect(),
    }
}

async fn run_project_tools(project_dir: PathBuf) -> miette::Result<()> {
    run_items(
        project_dir,
        "tool",
        "tools",
        |project| {
            let commands = project.settings.commands.all();
            if commands.is_empty() {
                None
            } else {
                Some(commands.into_iter().map(|(name, _)| name).collect())
            }
        },
        |project| async move { project.run_all_commands().await },
    )
    .await
}

async fn run_checks(project_dir: PathBuf) -> miette::Result<()> {
    run_items(
        project_dir,
        "check",
        "checks",
        |project| {
            project.settings.checks.as_ref().and_then(|checks| {
                if checks.is_empty() {
                    None
                } else {
                    Some(checks.iter().map(|c| c.name.clone()).collect())
                }
            })
        },
        |project| async move { project.run_all_checks().await },
    )
    .await
}

/// Test a bash command against configured rules.
///
/// Both normal and explain output use the live hook's compound analysis. Explain additionally
/// exposes command matches, redirect resolution/locality and authorization failures, and ordered
/// rule contributors for each leaf.
async fn test_bash_rules(
    command: Option<String>,
    config_path: Option<PathBuf>,
    json: bool,
    explain: bool,
    cwd: Option<PathBuf>,
    mode: Option<PermissionMode>,
) -> miette::Result<RuleResult> {
    // Initialize tracing to stderr for debug output (RUST_LOG env var controls level)
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .try_init();

    // Read command from argument or stdin
    let command = match command {
        Some(cmd) => cmd,
        None => {
            let mut input = String::new();
            std::io::stdin()
                .read_to_string(&mut input)
                .map_err(|e| miette::miette!("Failed to read command from stdin: {}", e))?;
            input.trim().to_string()
        }
    };

    if command.is_empty() {
        return Err(miette::miette!(
            "No command provided. Either pass a command as an argument or provide it via stdin."
        ));
    }

    // Load config from custom path or default
    let config = load_user_config_from(config_path.as_deref()).await?;

    // Extract bash rules
    if config
        .bash_rules
        .as_ref()
        .is_none_or(|rules| rules.is_empty())
    {
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "result": "no_match",
                    "reason": "No bash rules configured"
                })
            );
        } else {
            println!("○ NO MATCH (no bash rules configured)");
            println!(
                "\nConfigure rules in ~/.config/moriarty/tool_rules.toml to test against them."
            );
        }
        return Ok(RuleResult::NoMatch);
    }

    // Create engine
    let engine = BashRuleEngine::from_config(config)?;

    let cwd = resolve_test_cwd(cwd);
    if explain {
        let context = EvaluationContext::new(&cwd, mode);
        let evaluation = engine.evaluate_sync(&command, &context, EvaluationPurpose::Diagnostics);
        let trace = command_trace(&command, &evaluation);
        if json {
            let rendered = serde_json::to_string_pretty(&trace)
                .into_diagnostic()
                .wrap_err("Failed to serialize explain trace")?;
            println!("{rendered}");
        } else {
            output_explain(&trace)?;
        }
        return Ok(trace.final_result);
    }

    let context = EvaluationContext::new(&cwd, mode);
    let result = engine
        .evaluate_sync(&command, &context, EvaluationPurpose::Decision)
        .rule_result();

    // Output result
    if json {
        output_json(&command, &result)?;
    } else {
        output_pretty(&command, &result);
    }

    Ok(result)
}

fn resolve_test_cwd(cwd: Option<PathBuf>) -> String {
    match cwd {
        Some(path) => path.to_string_lossy().into_owned(),
        None => std::env::current_dir()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

pub(crate) fn command_trace(command: &str, evaluation: &Evaluation) -> CommandTrace {
    let (bindings, sub_commands, bail) = policy_trace_parts(evaluation.original_analysis());
    let mut rewritten_sub_commands = Vec::new();
    let mut rewritten_bail = None;
    match evaluation.continuation() {
        OriginalContinuation::None => {}
        OriginalContinuation::ModifyRedirectCheck(check) => {
            (rewritten_sub_commands, rewritten_bail) = redirect_trace_parts(check);
        }
        OriginalContinuation::ArgumentFilterRecheck {
            recheck,
            continuation,
        } => {
            let (_, traces, _) = policy_trace_parts(recheck);
            rewritten_sub_commands = traces;
            if let FilterContinuation::ModifyRedirectCheck(check) = continuation {
                let (mut traces, bail) = redirect_trace_parts(check);
                rewritten_sub_commands.append(&mut traces);
                rewritten_bail = bail;
            }
        }
    }
    CommandTrace {
        original: command.to_string(),
        bindings,
        sub_commands,
        rewritten_sub_commands,
        rewritten_bail,
        bail,
        final_result: evaluation.rule_result(),
        contributors: evaluation.contributors(),
    }
}

fn policy_trace_parts(
    analysis: &PolicyAnalysis,
) -> (Vec<AliasBinding>, Vec<SubCommandTrace>, Option<BailReason>) {
    match analysis {
        PolicyAnalysis::Bail { reason, .. } => (Vec::new(), Vec::new(), Some(*reason)),
        PolicyAnalysis::Leaves(leaves) => {
            let mut bindings = Vec::new();
            let sub_commands = leaves
                .iter()
                .map(|leaf| {
                    let identity = leaf.identity();
                    for binding in identity.bindings() {
                        if !bindings.contains(binding) {
                            bindings.push(binding.clone());
                        }
                    }
                    sub_command_trace(
                        identity,
                        leaf.endpoints().analyzed().unwrap_or_default(),
                        leaf.command().matched_rule().map(command_rule_explanation),
                    )
                })
                .collect();
            (bindings, sub_commands, None)
        }
    }
}

fn redirect_trace_parts(
    analysis: &RedirectCheckAnalysis,
) -> (Vec<SubCommandTrace>, Option<RewrittenBailTrace>) {
    match analysis {
        RedirectCheckAnalysis::Bail { command, reason } => (
            Vec::new(),
            Some(RewrittenBailTrace {
                command: command.clone(),
                reason: *reason,
            }),
        ),
        RedirectCheckAnalysis::Leaves(leaves) => (
            leaves
                .iter()
                .map(|leaf| sub_command_trace(leaf.identity(), leaf.endpoints(), None))
                .collect(),
            None,
        ),
    }
}

fn sub_command_trace(
    identity: &LeafIdentity,
    endpoints: &[EndpointAnalysis],
    matched: Option<RuleMatchExplanation>,
) -> SubCommandTrace {
    SubCommandTrace {
        original: identity.original().to_string(),
        alias_expanded: identity.alias_expanded().map(str::to_string),
        normalized: identity.normalized().to_string(),
        bindings: identity.bindings().to_vec(),
        requires_confirmation: identity.requires_confirmation().map(str::to_string),
        output_redirects: endpoints.iter().map(endpoint_trace).collect(),
        matched,
    }
}

fn unmatched_redirect_failure(matched_rule: Option<&MatchedRedirectRule>) -> Option<String> {
    matched_rule
        .is_none()
        .then(|| "no eligible AllowRedirect rule matched".to_string())
}

fn endpoint_trace(endpoint: &EndpointAnalysis) -> RedirectEndpointTrace {
    match endpoint {
        EndpointAnalysis::Descriptor {
            original_target,
            match_text,
            matched_rule,
        } => RedirectEndpointTrace {
            original_target: original_target.clone(),
            kind: RedirectEndpointKind::Descriptor.label(),
            match_text: Some(match_text.clone()),
            is_local: Some(false),
            matched: matched_rule.as_ref().map(redirect_rule_explanation),
            failure: unmatched_redirect_failure(matched_rule.as_ref()),
        },
        EndpointAnalysis::Filesystem {
            original_target,
            resolution:
                FilesystemAnalysis::Resolved {
                    target,
                    matched_rule,
                },
        } => RedirectEndpointTrace {
            original_target: original_target.clone(),
            kind: target.kind().label(),
            match_text: Some(target.match_text().to_string()),
            is_local: Some(target.is_local()),
            matched: matched_rule.as_ref().map(redirect_rule_explanation),
            failure: unmatched_redirect_failure(matched_rule.as_ref()),
        },
        EndpointAnalysis::Filesystem {
            original_target,
            resolution: FilesystemAnalysis::Failed { reason, .. },
        } => RedirectEndpointTrace {
            original_target: original_target.clone(),
            kind: RedirectEndpointKind::Filesystem.label(),
            match_text: None,
            is_local: None,
            matched: None,
            failure: Some(reason.clone()),
        },
    }
}

fn command_rule_explanation(rule: &MatchedCommandRule) -> RuleMatchExplanation {
    RuleMatchExplanation {
        rule_name: rule.rule_name().to_string(),
        expanded_pattern: rule.expanded_pattern().to_string(),
        action_summary: rule.action_summary().to_string(),
    }
}

fn redirect_rule_explanation(rule: &MatchedRedirectRule) -> RuleMatchExplanation {
    RuleMatchExplanation {
        rule_name: rule.rule_name().to_string(),
        expanded_pattern: rule.expanded_pattern().to_string(),
        action_summary: rule.action_summary().to_string(),
    }
}

fn output_explain(trace: &CommandTrace) -> miette::Result<()> {
    let stdout = io::stdout();
    write_explanation(&mut stdout.lock(), trace)
        .into_diagnostic()
        .wrap_err("Failed to render bash-rule explanation")
}

fn write_redirect_explanation(
    writer: &mut impl Write,
    endpoint: &RedirectEndpointTrace,
) -> io::Result<()> {
    writeln!(
        writer,
        "    {} redirect target: {}",
        endpoint.kind, endpoint.original_target
    )?;
    if let Some(match_text) = &endpoint.match_text {
        writeln!(writer, "      resolved for matching: {match_text}")?;
    }
    if let Some(is_local) = endpoint.is_local {
        writeln!(writer, "      project-local: {is_local}")?;
    }
    if let Some(matched) = &endpoint.matched {
        writeln!(
            writer,
            "      allowed by redirect rule '{}'  [{}]",
            matched.rule_name, matched.action_summary
        )?;
        writeln!(writer, "        pattern: {}", matched.expanded_pattern)?;
    }
    if let Some(failure) = &endpoint.failure {
        writeln!(writer, "      not authorized: {failure}")?;
    }
    Ok(())
}

fn write_sub_command_explanation(
    writer: &mut impl Write,
    label: &str,
    index: usize,
    sub: &SubCommandTrace,
) -> io::Result<()> {
    writeln!(writer, "  {label} {}: {}", index + 1, sub.original)?;
    if let Some(expanded) = &sub.alias_expanded {
        writeln!(writer, "    alias-expanded: {expanded}")?;
    }
    if sub.original != sub.normalized {
        writeln!(writer, "    analyzed for matching: {}", sub.normalized)?;
    }
    for binding in &sub.bindings {
        writeln!(
            writer,
            "    consumed binding: {}={}",
            binding.name, binding.value
        )?;
    }
    for endpoint in &sub.output_redirects {
        write_redirect_explanation(writer, endpoint)?;
    }
    if let Some(reason) = &sub.requires_confirmation {
        writeln!(writer, "    requires confirmation → {reason}")?;
    }
    match &sub.matched {
        Some(explanation) => {
            writeln!(
                writer,
                "    matched rule '{}'  [{}]",
                explanation.rule_name, explanation.action_summary
            )?;
            writeln!(writer, "      pattern: {}", explanation.expanded_pattern)?;
        }
        None => writeln!(writer, "    no rule matched")?,
    }
    Ok(())
}

fn write_explanation(writer: &mut impl Write, trace: &CommandTrace) -> io::Result<()> {
    writeln!(writer, "Command: {}", trace.original)?;
    for binding in &trace.bindings {
        writeln!(
            writer,
            "  Binding: {}={} (analysis metadata only; grants no permission)",
            binding.name, binding.value
        )?;
    }

    if let Some(reason) = &trace.bail {
        writeln!(
            writer,
            "  Could not analyze ({reason:?}); only an explicit Deny on the whole command is honored."
        )?;
    }
    for (index, sub) in trace.sub_commands.iter().enumerate() {
        write_sub_command_explanation(writer, "Leaf", index, sub)?;
    }
    if let Some(rewritten) = &trace.rewritten_bail {
        writeln!(writer, "  Rewritten command: {}", rewritten.command)?;
        writeln!(
            writer,
            "    could not analyze rewrite ({:?})",
            rewritten.reason
        )?;
    }
    for (index, sub) in trace.rewritten_sub_commands.iter().enumerate() {
        write_sub_command_explanation(writer, "Rewritten leaf", index, sub)?;
    }

    writeln!(writer)?;
    write!(writer, "Final decision: ")?;
    write_result_line(writer, &trace.final_result)?;
    if !trace.contributors.is_empty() {
        writeln!(
            writer,
            "Contributing rules: {}",
            trace.contributors.join(", ")
        )?;
    }
    Ok(())
}

fn write_result_line(writer: &mut impl Write, result: &RuleResult) -> io::Result<()> {
    match result {
        RuleResult::Allowed { rule_name } => writeln!(writer, "✓ ALLOWED by rule: {rule_name}"),
        RuleResult::Denied { rule_name, reason } => {
            writeln!(writer, "✗ DENIED by rule: {rule_name} ({reason})")
        }
        RuleResult::Modified {
            rule_name,
            new_command,
        } => writeln!(writer, "→ MODIFIED by rule: {rule_name} → {new_command}"),
        RuleResult::Asked { rule_name } => writeln!(writer, "? ASK by rule: {rule_name}"),
        RuleResult::ArgumentFiltered {
            rule_name,
            new_command,
            ..
        } => writeln!(
            writer,
            "⚙ ARGUMENT FILTERED by rule: {rule_name} → {new_command}"
        ),
        RuleResult::NoMatch => writeln!(writer, "○ NO MATCH — would prompt the user"),
    }
}

fn output_json(command: &str, result: &RuleResult) -> miette::Result<()> {
    let output = match result {
        RuleResult::Allowed { rule_name } => json!({
            "command": command,
            "result": "allowed",
            "rule_name": rule_name,
        }),
        RuleResult::Denied { rule_name, reason } => json!({
            "command": command,
            "result": "denied",
            "rule_name": rule_name,
            "reason": reason,
        }),
        RuleResult::Modified {
            rule_name,
            new_command,
        } => json!({
            "command": command,
            "result": "modified",
            "rule_name": rule_name,
            "new_command": new_command,
        }),
        RuleResult::Asked { rule_name } => json!({
            "command": command,
            "result": "ask",
            "rule_name": rule_name,
        }),
        RuleResult::ArgumentFiltered {
            rule_name,
            new_command,
            reason,
        } => json!({
            "command": command,
            "result": "argument_filtered",
            "rule_name": rule_name,
            "new_command": new_command,
            "reason": reason,
        }),
        RuleResult::NoMatch => json!({
            "command": command,
            "result": "no_match",
        }),
    };

    let json_string = serde_json::to_string_pretty(&output)
        .into_diagnostic()
        .wrap_err("Failed to serialize JSON output")?;
    println!("{}", json_string);
    Ok(())
}

fn output_pretty(command: &str, result: &RuleResult) {
    match result {
        RuleResult::Allowed { rule_name } => {
            println!("✓ ALLOWED by rule: {}", rule_name);
        }
        RuleResult::Denied { rule_name, reason } => {
            println!("✗ DENIED by rule: {}", rule_name);
            println!("  Reason: {}", reason);
        }
        RuleResult::Modified {
            rule_name,
            new_command,
        } => {
            println!("→ MODIFIED by rule: {}", rule_name);
            println!("  Original: {}", command);
            println!("  Modified: {}", new_command);
        }
        RuleResult::Asked { rule_name } => {
            println!("? ASK by rule: {}", rule_name);
            println!("  This command requires user approval");
        }
        RuleResult::ArgumentFiltered {
            rule_name,
            new_command,
            reason,
        } => {
            println!("⚙ ARGUMENT FILTERED by rule: {}", rule_name);
            println!("  Original: {}", command);
            println!("  Filtered: {}", new_command);
            if let Some(r) = reason {
                println!("  Reason: {}", r);
            }
        }
        RuleResult::NoMatch => {
            println!("○ NO MATCH");
            println!("  No rules matched this command - would prompt user for approval");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};

    use tempfile::TempDir;

    use super::*;
    use crate::test_helpers::{
        PATH_ALIAS_COMMAND, PATH_ALIAS_READ_RULES, setup_isolated_xdg_config,
    };
    use crate::user_config::{BashRule, BashRuleAction, UserConfig};

    async fn create_test_config(dir: &TempDir, rules: Vec<BashRule>) -> PathBuf {
        write_user_config(
            dir,
            UserConfig {
                pattern_fragments: None,
                bash_path_aliases: BTreeSet::new(),
                bash_rules: Some(rules),
                tool_rules: None,
            },
        )
        .await
    }

    async fn write_user_config(dir: &TempDir, config: UserConfig) -> PathBuf {
        let config_dir = dir.path().join("moriarty");
        tokio::fs::create_dir_all(&config_dir).await.unwrap();
        let config_path = config_dir.join("test_rules.toml");
        let toml = toml::to_string_pretty(&config).unwrap();
        tokio::fs::write(&config_path, toml).await.unwrap();
        config_path
    }

    /// Run `test_bash_rules` with a single rule and return its `Result`.
    ///
    /// Callers are expected to `.unwrap_or_else(|e| panic!("case {label:?}: {e}"))`
    /// so table-driven tests identify the failing case in panic output.
    async fn run_once(rule: BashRule, command: &str, json: bool) -> miette::Result<RuleResult> {
        let dir = setup_isolated_xdg_config();
        let cfg = create_test_config(&dir, vec![rule]).await;
        test_bash_rules(
            Some(command.to_string()),
            Some(cfg),
            json,
            false,
            None,
            None,
        )
        .await
    }

    fn explain(
        engine: &BashRuleEngine,
        command: &str,
        cwd: &str,
        mode: Option<PermissionMode>,
    ) -> CommandTrace {
        let context = EvaluationContext::new(cwd, mode);
        let evaluation = engine.evaluate_sync(command, &context, EvaluationPurpose::Diagnostics);
        command_trace(command, &evaluation)
    }

    fn allow(name: &str, pattern: &str) -> BashRule {
        BashRule {
            name: name.to_string(),
            pattern: pattern.to_string(),
            modes: None,
            action: BashRuleAction::Allow,
        }
    }
    fn deny(name: &str, pattern: &str, value: &str) -> BashRule {
        BashRule {
            name: name.to_string(),
            pattern: pattern.to_string(),
            modes: None,
            action: BashRuleAction::Deny {
                value: value.to_string(),
            },
        }
    }
    fn modify(name: &str, pattern: &str, value: &str) -> BashRule {
        BashRule {
            name: name.to_string(),
            pattern: pattern.to_string(),
            modes: None,
            action: BashRuleAction::Modify {
                value: value.to_string(),
            },
        }
    }
    fn ask(name: &str, pattern: &str) -> BashRule {
        BashRule {
            name: name.to_string(),
            pattern: pattern.to_string(),
            modes: None,
            action: BashRuleAction::Ask,
        }
    }

    /// Table-driven coverage for the happy-path bash-rule flows.
    ///
    /// Each case just asserts `test_bash_rules(...)` returns Ok; the earlier
    /// individual tests only did the same smoke-check.
    #[tokio::test]
    async fn test_bash_rule_matrix() {
        struct Case {
            label: &'static str,
            rule: BashRule,
            command: &'static str,
            json: bool,
        }
        let cases = [
            Case {
                label: "allowed",
                rule: allow("allow-ls", r"^ls"),
                command: "ls -la",
                json: false,
            },
            Case {
                label: "denied",
                rule: deny("deny-rm", r"^rm\s+-rf\s+/", "Dangerous recursive delete"),
                command: "rm -rf /",
                json: false,
            },
            Case {
                label: "modified",
                rule: modify(
                    "modify-docker",
                    r"^(docker\s+system\s+prune)$",
                    "$1 --dry-run",
                ),
                command: "docker system prune",
                json: false,
            },
            Case {
                label: "ask",
                rule: ask("ask-docker", r"^docker"),
                command: "docker build",
                json: false,
            },
            Case {
                label: "no match",
                rule: allow("allow-ls", r"^ls"),
                command: "cargo build",
                json: false,
            },
            Case {
                label: "json output",
                rule: allow("allow-ls", r"^ls"),
                command: "ls -la",
                json: true,
            },
            Case {
                label: "json structure allowed",
                rule: allow("allow-ls", r"^ls"),
                command: "ls",
                json: true,
            },
            Case {
                label: "json structure denied",
                rule: deny("deny-rm", r"^rm", "Dangerous command"),
                command: "rm file.txt",
                json: true,
            },
            Case {
                label: "json structure modified",
                rule: modify("add-flag", r"^(ls)$", "$1 -la"),
                command: "ls",
                json: true,
            },
            Case {
                label: "invalid regex skipped",
                rule: deny("bad-regex", r"[invalid(", "test"),
                command: "anything",
                json: false,
            },
            Case {
                label: "special chars quotes",
                rule: allow("allow-echo", r"^echo"),
                command: "echo \"hello world\"",
                json: false,
            },
            Case {
                label: "special chars unicode",
                rule: allow("allow-echo", r"^echo"),
                command: "echo '\u{4f60}\u{597d}\u{4e16}\u{754c} \u{1f30d}'",
                json: false,
            },
            Case {
                label: "whitespace only",
                rule: deny("deny-whitespace", r"^\s+$", "Whitespace only"),
                command: "   \t\n",
                json: false,
            },
        ];

        for c in cases {
            run_once(c.rule.clone(), c.command, c.json)
                .await
                .unwrap_or_else(|e| panic!("bash_rule_matrix case {:?}: {e}", c.label));
        }
    }

    #[tokio::test]
    async fn test_bash_rules_very_long_command() {
        let long = "echo ".to_string() + &"a".repeat(1000);
        run_once(allow("allow-all", r".*"), &long, false)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_bash_rules_first_match_wins() {
        // Two-rule config; first deny rule should win over later allow.
        let dir = setup_isolated_xdg_config();
        let cfg = create_test_config(
            &dir,
            vec![
                deny("deny-ls", r"^ls", "First rule denies"),
                allow("allow-ls", r"^ls"),
            ],
        )
        .await;
        test_bash_rules(Some("ls".to_string()), Some(cfg), false, false, None, None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_bash_rules_no_rules_configured() {
        let dir = setup_isolated_xdg_config();
        let cfg = create_test_config(&dir, vec![]).await;
        test_bash_rules(
            Some("ls -la".to_string()),
            Some(cfg),
            false,
            false,
            None,
            None,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_bash_rules_with_pattern_fragments() {
        let dir = setup_isolated_xdg_config();
        let mut fragments = HashMap::new();
        fragments.insert("safe_chars".to_string(), "[^|&;$`]".to_string());
        let cfg = write_user_config(
            &dir,
            UserConfig {
                pattern_fragments: Some(fragments),
                bash_path_aliases: BTreeSet::new(),
                bash_rules: Some(vec![allow(
                    "allow-ls-with-fragment",
                    r"^ls{{safe_chars}}*$",
                )]),
                tool_rules: None,
            },
        )
        .await;

        // Matches: every char in " -la" is permitted by safe_chars ([^|&;$`]).
        test_bash_rules(
            Some("ls -la".to_string()),
            Some(cfg.clone()),
            false,
            false,
            None,
            None,
        )
        .await
        .unwrap_or_else(|e| panic!("pattern_fragments ls -la: {e}"));

        // Must NOT match: the pipe char is excluded by safe_chars, so the
        // expanded pattern `^ls[^|&;$`]*$` rejects "ls | grep foo".
        // test_bash_rules still returns Ok (no rule error); the hook simply
        // emits NO MATCH output, which is the behaviour under test.
        test_bash_rules(
            Some("ls | grep foo".to_string()),
            Some(cfg),
            false,
            false,
            None,
            None,
        )
        .await
        .unwrap_or_else(|e| panic!("pattern_fragments ls | grep foo: {e}"));
    }

    #[tokio::test]
    async fn test_bash_rules_empty_command() {
        let dir = setup_isolated_xdg_config();
        let cfg = create_test_config(&dir, vec![allow("allow-ls", r"^ls")]).await;
        let err = test_bash_rules(Some(String::new()), Some(cfg), false, false, None, None)
            .await
            .expect_err("Should fail with empty command");
        assert!(err.to_string().contains("No command provided"));
    }

    #[tokio::test]
    async fn test_bash_rules_invalid_config_path() {
        let err = test_bash_rules(
            Some("ls".to_string()),
            Some(PathBuf::from("/nonexistent/path/config.toml")),
            false,
            false,
            None,
            None,
        )
        .await
        .expect_err("Should fail with invalid config path");
        assert!(err.to_string().contains("Failed to read config file"));
    }

    #[tokio::test]
    async fn test_bash_rules_malformed_toml() {
        let dir = setup_isolated_xdg_config();
        let config_dir = dir.path().join("moriarty");
        tokio::fs::create_dir_all(&config_dir).await.unwrap();
        let cfg = config_dir.join("bad_config.toml");
        tokio::fs::write(&cfg, "this is not valid [[[ toml")
            .await
            .unwrap();
        let err = test_bash_rules(Some("ls".to_string()), Some(cfg), false, false, None, None)
            .await
            .expect_err("Should fail with malformed TOML");
        assert!(err.to_string().contains("Failed to parse config file"));
    }

    #[tokio::test]
    async fn permission_mode_selects_rules_in_normal_and_explain_execution() {
        let dir = setup_isolated_xdg_config();
        let mut plan_deny = deny("plan-deny", r"^ls", "plan denied");
        plan_deny.modes = Some(BTreeSet::from([PermissionMode::Plan]));
        let cfg =
            create_test_config(&dir, vec![plan_deny, allow("unrestricted-allow", r"^ls")]).await;

        for explain in [false, true] {
            let plan = test_bash_rules(
                Some("ls".to_string()),
                Some(cfg.clone()),
                false,
                explain,
                None,
                Some(PermissionMode::Plan),
            )
            .await
            .unwrap();
            assert!(matches!(
                plan,
                RuleResult::Denied { ref rule_name, .. } if rule_name == "plan-deny"
            ));

            assert!(matches!(
                test_bash_rules(
                    Some("ls".to_string()),
                    Some(cfg.clone()),
                    false,
                    explain,
                    None,
                    None,
                )
                .await
                .unwrap(),
                RuleResult::Allowed { ref rule_name } if rule_name == "unrestricted-allow"
            ));
        }
    }

    #[tokio::test]
    async fn normal_and_json_explain_modes_share_alias_aware_compound_analysis() {
        let dir = setup_isolated_xdg_config();
        let config = toml::from_str(&format!(
            "bash_path_aliases = [\"P\"]\n{PATH_ALIAS_READ_RULES}"
        ))
        .unwrap();
        let cfg = write_user_config(&dir, config).await;
        let command = PATH_ALIAS_COMMAND;

        let mut results = Vec::new();
        for explain in [false, true] {
            results.push(
                test_bash_rules(
                    Some(command.to_string()),
                    Some(cfg.clone()),
                    explain,
                    explain,
                    Some(PathBuf::from("/work/project")),
                    None,
                )
                .await
                .unwrap(),
            );
        }

        assert_eq!(results[0], results[1]);
        assert!(matches!(results[0], RuleResult::Allowed { .. }));
    }

    #[test]
    fn text_explain_renders_rewritten_leaf_metadata() {
        let trace = CommandTrace {
            original: "filtered".to_string(),
            bindings: Vec::new(),
            sub_commands: Vec::new(),
            rewritten_sub_commands: vec![SubCommandTrace {
                original: "cat $P/file".to_string(),
                alias_expanded: Some("cat /work/project/file".to_string()),
                normalized: "cat file".to_string(),
                bindings: vec![AliasBinding {
                    name: "P".to_string(),
                    value: "/work/project".to_string(),
                }],
                requires_confirmation: None,
                output_redirects: Vec::new(),
                matched: Some(RuleMatchExplanation {
                    rule_name: "allow-cat".to_string(),
                    expanded_pattern: "^cat ".to_string(),
                    action_summary: "Allow".to_string(),
                }),
            }],
            rewritten_bail: None,
            bail: None,
            final_result: RuleResult::NoMatch,
            contributors: vec!["allow-cat".to_string()],
        };
        let mut output = Vec::new();
        write_explanation(&mut output, &trace).unwrap();
        let output = String::from_utf8(output).unwrap();

        for expected in [
            "Rewritten leaf 1: cat $P/file",
            "alias-expanded: cat /work/project/file",
            "consumed binding: P=/work/project",
            "matched rule 'allow-cat'  [Allow]",
            "pattern: ^cat ",
        ] {
            assert!(
                output.contains(expected),
                "missing {expected:?} in {output:?}"
            );
        }
    }

    #[test]
    fn explain_renderers_include_redirect_policy_and_provenance() {
        let cwd = tempfile::tempdir().unwrap();
        let cwd = cwd.path().to_str().unwrap();
        let engine = BashRuleEngine::from_config(UserConfig {
            pattern_fragments: None,
            bash_path_aliases: BTreeSet::new(),
            bash_rules: Some(vec![
                modify("redirecting-echo", r"^rewrite$", "echo > report.txt"),
                allow("allow-echo", r"^echo($|\s)"),
                deny("deny-rm", r"^rm", "no removal"),
                BashRule {
                    name: "allow-local".to_string(),
                    pattern: r"^reports/".to_string(),
                    modes: None,
                    action: BashRuleAction::AllowRedirect { allow_local: true },
                },
            ]),
            tool_rules: None,
        })
        .unwrap();
        let allowed = explain(&engine, "echo hi > reports/status.txt", cwd, None);
        let unresolved = explain(&engine, "echo hi > $OUT", cwd, None);
        let denied = explain(&engine, "rm > reports/denied.txt", cwd, None);
        let rewritten = explain(&engine, "rewrite", cwd, None);
        assert_eq!(
            denied.sub_commands[0].output_redirects[0]
                .matched
                .as_ref()
                .unwrap()
                .rule_name,
            "allow-local"
        );

        let mut output = Vec::new();
        write_explanation(&mut output, &allowed).unwrap();
        write_explanation(&mut output, &unresolved).unwrap();
        write_explanation(&mut output, &denied).unwrap();
        write_explanation(&mut output, &rewritten).unwrap();
        let output = String::from_utf8(output).unwrap();
        for expected in [
            "resolved for matching: reports/status.txt",
            "project-local: true",
            "allowed by redirect rule 'allow-local'",
            "Contributing rules: allow-echo, allow-local",
            "DENIED by rule: deny-rm",
            "Rewritten leaf 1: echo > report.txt",
        ] {
            assert!(
                output.contains(expected),
                "missing {expected:?} in {output:?}"
            );
        }

        assert_eq!(output.matches("not authorized:").count(), 2);
        let unresolved_endpoint = &unresolved.sub_commands[0].output_redirects[0];
        assert!(unresolved_endpoint.failure.is_some());
        assert!(unresolved_endpoint.matched.is_none());
        let rewritten_endpoint = &rewritten.rewritten_sub_commands[0].output_redirects[0];
        assert!(rewritten_endpoint.failure.is_some());
        assert!(rewritten_endpoint.matched.is_none());

        let json = serde_json::to_value(&allowed).unwrap();
        let endpoint = &json["sub_commands"][0]["output_redirects"][0];
        assert_eq!(endpoint["match_text"], "reports/status.txt");
        assert_eq!(endpoint["is_local"], true);
        assert_eq!(endpoint["matched"]["rule_name"], "allow-local");
        assert_eq!(json["contributors"], json!(["allow-echo", "allow-local"]));
    }

    #[test]
    fn explanation_renders_bail_bindings_and_remaining_results() {
        let mut config: UserConfig = toml::from_str(&format!(
            "bash_path_aliases = [\"P\"]\n{PATH_ALIAS_READ_RULES}"
        ))
        .unwrap();
        config.bash_rules.get_or_insert_default().push(modify(
            "dynamic-rewrite",
            r"^rewrite$",
            "echo $(date)",
        ));
        let engine = BashRuleEngine::from_config(config).unwrap();
        let traces = [
            explain(&engine, PATH_ALIAS_COMMAND, "/work/project", None),
            explain(&engine, "echo $(date)", "", None),
            explain(&engine, "rewrite", "", None),
        ];
        let mut output = Vec::new();
        for trace in &traces {
            write_explanation(&mut output, trace).unwrap();
        }
        write_result_line(
            &mut output,
            &RuleResult::Modified {
                rule_name: "modify".to_string(),
                new_command: "echo hi".to_string(),
            },
        )
        .unwrap();
        write_result_line(
            &mut output,
            &RuleResult::ArgumentFiltered {
                rule_name: "filter".to_string(),
                new_command: "cargo doc".to_string(),
                reason: None,
            },
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        for expected in [
            "Binding: P=/work/project/node_modules/pkg",
            "Could not analyze",
            "Rewritten command: echo $(date)",
            "could not analyze rewrite (CommandSubstitution)",
            "NO MATCH",
            "MODIFIED by rule: modify",
            "ARGUMENT FILTERED by rule: filter",
        ] {
            assert!(
                output.contains(expected),
                "missing {expected:?} in {output:?}"
            );
        }
    }
}
