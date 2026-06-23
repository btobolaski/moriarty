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

use std::{io::Read, path::PathBuf};

use miette::{IntoDiagnostic, Result, WrapErr};
use serde_json::json;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::{
    hooks::bash_rules::{BashRuleEngine, CommandTrace, RuleResult},
    project_config::runner::{CommandOutput, VerifiedProject, verify_and_load_project},
    user_config::load_user_config_from,
};

pub async fn exec_test(cmd: crate::TestCommand) -> Result<()> {
    match cmd {
        crate::TestCommand::ProjectTools { project_dir } => run_project_tools(project_dir).await,
        crate::TestCommand::Checks { project_dir } => run_checks(project_dir).await,
        crate::TestCommand::BashRules {
            command,
            config,
            json,
            explain,
            cwd,
        } => test_bash_rules(command, config, json, explain, cwd).await,
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
) -> Result<()>
where
    F: FnOnce(VerifiedProject) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<CommandOutput>>>,
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

async fn run_project_tools(project_dir: PathBuf) -> Result<()> {
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

async fn run_checks(project_dir: PathBuf) -> Result<()> {
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
/// Without `explain`, output is the single-command rule result (unchanged historical behavior).
/// With `explain`, it shows the compound split, each leaf's normalized text and matching rule, and
/// the merged decision the hook would actually make for `cwd`.
async fn test_bash_rules(
    command: Option<String>,
    config_path: Option<PathBuf>,
    json: bool,
    explain: bool,
    cwd: Option<PathBuf>,
) -> Result<()> {
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
    let bash_rules = match config.bash_rules {
        Some(rules) if !rules.is_empty() => rules,
        _ => {
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
            return Ok(());
        }
    };

    // Create engine
    let engine = BashRuleEngine::from_config(bash_rules, config.pattern_fragments)?;

    if explain {
        let cwd = resolve_explain_cwd(cwd);
        let trace = engine.explain(&command, &cwd);
        if json {
            let rendered = serde_json::to_string_pretty(&trace)
                .into_diagnostic()
                .wrap_err("Failed to serialize explain trace")?;
            println!("{rendered}");
        } else {
            output_explain(&trace);
        }
        return Ok(());
    }

    // Apply rules
    let result = engine.apply_rules(&command);

    // Output result
    if json {
        output_json(&command, &result)?;
    } else {
        output_pretty(&command, &result);
    }

    Ok(())
}

/// Resolves the simulated hook cwd for `--explain`: the explicit `--cwd`, else the process working
/// directory, else empty (which disables path normalization).
fn resolve_explain_cwd(cwd: Option<PathBuf>) -> String {
    match cwd {
        Some(path) => path.to_string_lossy().into_owned(),
        None => std::env::current_dir()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

fn output_explain(trace: &CommandTrace) {
    println!("Command: {}", trace.original);

    if let Some(reason) = &trace.bail {
        println!(
            "  Could not analyze ({reason:?}); only an explicit Deny on the whole command is honored."
        );
    } else {
        for (index, sub) in trace.sub_commands.iter().enumerate() {
            println!("  Leaf {}: {}", index + 1, sub.normalized);
            if sub.original != sub.normalized {
                println!("    (before cwd normalization: {})", sub.original);
            }
            if sub.real_file_write {
                println!("    writes a real file → any Allow is capped at Ask");
            }
            match &sub.matched {
                Some(explanation) => {
                    println!(
                        "    matched rule '{}'  [{}]",
                        explanation.rule_name, explanation.action_summary
                    );
                    println!("      pattern: {}", explanation.expanded_pattern);
                }
                None => println!("    no rule matched"),
            }
        }
    }

    println!();
    print!("Final decision: ");
    print_result_line(&trace.final_result);
}

/// One-line rendering of a final decision for the `--explain` footer.
fn print_result_line(result: &RuleResult) {
    match result {
        RuleResult::Allowed { rule_name } => println!("✓ ALLOWED by rule: {rule_name}"),
        RuleResult::Denied { rule_name, reason } => {
            println!("✗ DENIED by rule: {rule_name} ({reason})")
        }
        RuleResult::Modified {
            rule_name,
            new_command,
        } => println!("→ MODIFIED by rule: {rule_name} → {new_command}"),
        RuleResult::Asked { rule_name } => println!("? ASK by rule: {rule_name}"),
        RuleResult::ArgumentFiltered {
            rule_name,
            new_command,
            ..
        } => println!("⚙ ARGUMENT FILTERED by rule: {rule_name} → {new_command}"),
        RuleResult::NoMatch => println!("○ NO MATCH — would prompt the user"),
    }
}

fn output_json(command: &str, result: &RuleResult) -> Result<()> {
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
    use std::collections::HashMap;

    use tempfile::TempDir;

    use super::*;
    use crate::test_helpers::setup_isolated_xdg_config;
    use crate::user_config::{BashRule, BashRuleAction, UserConfig};

    async fn create_test_config(dir: &TempDir, rules: Vec<BashRule>) -> PathBuf {
        write_user_config(
            dir,
            UserConfig {
                pattern_fragments: None,
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
    async fn run_once(rule: BashRule, command: &str, json: bool) -> miette::Result<()> {
        let dir = setup_isolated_xdg_config();
        let cfg = create_test_config(&dir, vec![rule]).await;
        test_bash_rules(Some(command.to_string()), Some(cfg), json, false, None).await
    }

    fn allow(name: &str, pattern: &str) -> BashRule {
        BashRule {
            name: name.to_string(),
            pattern: pattern.to_string(),
            action: BashRuleAction::Allow,
        }
    }
    fn deny(name: &str, pattern: &str, value: &str) -> BashRule {
        BashRule {
            name: name.to_string(),
            pattern: pattern.to_string(),
            action: BashRuleAction::Deny {
                value: value.to_string(),
            },
        }
    }
    fn modify(name: &str, pattern: &str, value: &str) -> BashRule {
        BashRule {
            name: name.to_string(),
            pattern: pattern.to_string(),
            action: BashRuleAction::Modify {
                value: value.to_string(),
            },
        }
    }
    fn ask(name: &str, pattern: &str) -> BashRule {
        BashRule {
            name: name.to_string(),
            pattern: pattern.to_string(),
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
        test_bash_rules(Some("ls".to_string()), Some(cfg), false, false, None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_bash_rules_no_rules_configured() {
        let dir = setup_isolated_xdg_config();
        let cfg = create_test_config(&dir, vec![]).await;
        test_bash_rules(Some("ls -la".to_string()), Some(cfg), false, false, None)
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
        )
        .await
        .unwrap_or_else(|e| panic!("pattern_fragments ls | grep foo: {e}"));
    }

    #[tokio::test]
    async fn test_bash_rules_empty_command() {
        let dir = setup_isolated_xdg_config();
        let cfg = create_test_config(&dir, vec![allow("allow-ls", r"^ls")]).await;
        let err = test_bash_rules(Some(String::new()), Some(cfg), false, false, None)
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
        let err = test_bash_rules(Some("ls".to_string()), Some(cfg), false, false, None)
            .await
            .expect_err("Should fail with malformed TOML");
        assert!(err.to_string().contains("Failed to parse config file"));
    }

    #[tokio::test]
    async fn test_bash_rules_explain_mode_succeeds() {
        let dir = setup_isolated_xdg_config();
        let cfg = create_test_config(&dir, vec![allow("allow-ls", r"^ls")]).await;
        // Render both pretty and JSON explain, over a compound command (per-leaf branch) and a
        // command-substitution command (bail branch), so both rendering paths are exercised.
        // The trace content itself is asserted by BashRuleEngine::explain's own tests.
        for command in ["ls && echo hi", "ls $(curl x)"] {
            for json in [false, true] {
                test_bash_rules(
                    Some(command.to_string()),
                    Some(cfg.clone()),
                    json,
                    true,
                    None,
                )
                .await
                .unwrap_or_else(|e| panic!("explain {command:?} json={json}: {e}"));
            }
        }
    }
}
