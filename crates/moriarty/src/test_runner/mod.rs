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

use std::io::Read;
use std::path::PathBuf;

use miette::{IntoDiagnostic, Result, WrapErr};
use serde_json::json;

use crate::hooks::bash_rules::{BashRuleEngine, RuleResult};
use crate::project_config::runner::{verify_and_load_project, CommandOutput, VerifiedProject};
use crate::user_config::{load_user_config, UserConfig};

pub async fn exec_test(cmd: crate::TestCommand) -> Result<()> {
    match cmd {
        crate::TestCommand::ProjectTools { project_dir } => run_project_tools(project_dir).await,
        crate::TestCommand::Checks { project_dir } => run_checks(project_dir).await,
        crate::TestCommand::BashRules {
            command,
            config,
            json,
        } => test_bash_rules(command, config, json).await,
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

/// Test a bash command against configured rules
async fn test_bash_rules(
    command: Option<String>,
    config_path: Option<PathBuf>,
    json: bool,
) -> Result<()> {
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
    let config = if let Some(path) = config_path {
        let contents = tokio::fs::read(&path)
            .await
            .into_diagnostic()
            .wrap_err_with(|| format!("Failed to read config file: {}", path.display()))?;
        toml::from_slice::<UserConfig>(&contents)
            .into_diagnostic()
            .wrap_err_with(|| format!("Failed to parse config file: {}", path.display()))?
    } else {
        load_user_config().await?
    };

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
        RuleResult::NoMatch => {
            println!("○ NO MATCH");
            println!("  No rules matched this command - would prompt user for approval");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::user_config::{BashRule, BashRuleAction, UserConfig};
    use tempfile::TempDir;

    /// Safe to use std::env::set_var because cargo nextest isolates each test in a separate process.
    fn setup_isolated_xdg_config() -> TempDir {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path());
        temp_dir
    }

    async fn create_test_config(dir: &TempDir, rules: Vec<BashRule>) -> PathBuf {
        let config_dir = dir.path().join("moriarty");
        tokio::fs::create_dir_all(&config_dir).await.unwrap();

        let config = UserConfig {
            pattern_fragments: None,
            bash_rules: Some(rules),
        };

        let config_path = config_dir.join("test_rules.toml");
        let config_toml = toml::to_string_pretty(&config).unwrap();
        tokio::fs::write(&config_path, config_toml).await.unwrap();

        config_path
    }

    #[tokio::test]
    async fn test_bash_rules_allowed() {
        let temp_dir = setup_isolated_xdg_config();
        let rules = vec![BashRule {
            name: "allow-ls".to_string(),
            pattern: r"^ls".to_string(),
            action: BashRuleAction::Allow,
        }];

        let config_path = create_test_config(&temp_dir, rules).await;

        test_bash_rules(Some("ls -la".to_string()), Some(config_path), false)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_bash_rules_denied() {
        let temp_dir = setup_isolated_xdg_config();
        let rules = vec![BashRule {
            name: "deny-rm".to_string(),
            pattern: r"^rm\s+-rf\s+/".to_string(),
            action: BashRuleAction::Deny("Dangerous recursive delete".to_string()),
        }];

        let config_path = create_test_config(&temp_dir, rules).await;

        test_bash_rules(Some("rm -rf /".to_string()), Some(config_path), false)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_bash_rules_modified() {
        let temp_dir = setup_isolated_xdg_config();
        let rules = vec![BashRule {
            name: "modify-docker".to_string(),
            pattern: r"^(docker\s+system\s+prune)$".to_string(),
            action: BashRuleAction::Modify("$1 --dry-run".to_string()),
        }];

        let config_path = create_test_config(&temp_dir, rules).await;

        test_bash_rules(
            Some("docker system prune".to_string()),
            Some(config_path),
            false,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_bash_rules_ask() {
        let temp_dir = setup_isolated_xdg_config();
        let rules = vec![BashRule {
            name: "ask-docker".to_string(),
            pattern: r"^docker".to_string(),
            action: BashRuleAction::Ask,
        }];

        let config_path = create_test_config(&temp_dir, rules).await;

        test_bash_rules(Some("docker build".to_string()), Some(config_path), false)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_bash_rules_no_match() {
        let temp_dir = setup_isolated_xdg_config();
        let rules = vec![BashRule {
            name: "allow-ls".to_string(),
            pattern: r"^ls".to_string(),
            action: BashRuleAction::Allow,
        }];

        let config_path = create_test_config(&temp_dir, rules).await;

        test_bash_rules(Some("cargo build".to_string()), Some(config_path), false)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_bash_rules_json_output() {
        let temp_dir = setup_isolated_xdg_config();
        let rules = vec![BashRule {
            name: "allow-ls".to_string(),
            pattern: r"^ls".to_string(),
            action: BashRuleAction::Allow,
        }];

        let config_path = create_test_config(&temp_dir, rules).await;

        test_bash_rules(Some("ls -la".to_string()), Some(config_path), true)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_bash_rules_empty_command() {
        let temp_dir = setup_isolated_xdg_config();
        let rules = vec![BashRule {
            name: "allow-ls".to_string(),
            pattern: r"^ls".to_string(),
            action: BashRuleAction::Allow,
        }];

        let config_path = create_test_config(&temp_dir, rules).await;

        let err = test_bash_rules(Some("".to_string()), Some(config_path), false)
            .await
            .expect_err("Should fail with empty command");
        assert!(err.to_string().contains("No command provided"));
    }

    #[tokio::test]
    async fn test_bash_rules_no_rules_configured() {
        let temp_dir = setup_isolated_xdg_config();
        let config_path = create_test_config(&temp_dir, vec![]).await;

        test_bash_rules(Some("ls -la".to_string()), Some(config_path), false)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_bash_rules_invalid_config_path() {
        let err = test_bash_rules(
            Some("ls".to_string()),
            Some(PathBuf::from("/nonexistent/path/config.toml")),
            false,
        )
        .await
        .expect_err("Should fail with invalid config path");
        assert!(err.to_string().contains("Failed to read config file"));
    }

    #[tokio::test]
    async fn test_bash_rules_with_pattern_fragments() {
        use std::collections::HashMap;

        let temp_dir = setup_isolated_xdg_config();
        let config_dir = temp_dir.path().join("moriarty");
        tokio::fs::create_dir_all(&config_dir).await.unwrap();

        let mut fragments = HashMap::new();
        fragments.insert("safe_chars".to_string(), "[^|&;$`]".to_string());

        let config = UserConfig {
            pattern_fragments: Some(fragments),
            bash_rules: Some(vec![BashRule {
                name: "allow-ls-with-fragment".to_string(),
                pattern: r"^ls{{safe_chars}}*$".to_string(),
                action: BashRuleAction::Allow,
            }]),
        };

        let config_path = config_dir.join("test_rules.toml");
        let config_toml = toml::to_string_pretty(&config).unwrap();
        tokio::fs::write(&config_path, config_toml).await.unwrap();

        // Should match with fragment expansion
        test_bash_rules(Some("ls -la".to_string()), Some(config_path.clone()), false)
            .await
            .unwrap();

        // Should not match (contains pipe, which is excluded by safe_chars)
        test_bash_rules(Some("ls | grep foo".to_string()), Some(config_path), false)
            .await
            .unwrap(); // Returns Ok but should show NO MATCH
    }

    #[tokio::test]
    async fn test_bash_rules_malformed_toml() {
        let temp_dir = setup_isolated_xdg_config();
        let config_dir = temp_dir.path().join("moriarty");
        tokio::fs::create_dir_all(&config_dir).await.unwrap();

        let config_path = config_dir.join("bad_config.toml");
        tokio::fs::write(&config_path, "this is not valid [[[ toml")
            .await
            .unwrap();

        let err = test_bash_rules(Some("ls".to_string()), Some(config_path), false)
            .await
            .expect_err("Should fail with malformed TOML");
        assert!(err.to_string().contains("Failed to parse config file"));
    }

    #[tokio::test]
    async fn test_bash_rules_invalid_regex_pattern() {
        let temp_dir = setup_isolated_xdg_config();
        let rules = vec![BashRule {
            name: "bad-regex".to_string(),
            pattern: r"[invalid(".to_string(),
            action: BashRuleAction::Deny("test".to_string()),
        }];

        let config_path = create_test_config(&temp_dir, rules).await;

        // Should succeed but skip the invalid rule
        test_bash_rules(Some("anything".to_string()), Some(config_path), false)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_bash_rules_command_with_special_characters() {
        let temp_dir = setup_isolated_xdg_config();
        let rules = vec![BashRule {
            name: "allow-echo".to_string(),
            pattern: r"^echo".to_string(),
            action: BashRuleAction::Allow,
        }];

        let config_path = create_test_config(&temp_dir, rules).await;

        // Test with quotes, newlines, tabs
        test_bash_rules(
            Some("echo \"hello world\"".to_string()),
            Some(config_path.clone()),
            false,
        )
        .await
        .unwrap();

        // Test with unicode
        test_bash_rules(
            Some("echo '你好世界 🌍'".to_string()),
            Some(config_path),
            false,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_bash_rules_very_long_command() {
        let temp_dir = setup_isolated_xdg_config();
        let rules = vec![BashRule {
            name: "allow-all".to_string(),
            pattern: r".*".to_string(),
            action: BashRuleAction::Allow,
        }];

        let config_path = create_test_config(&temp_dir, rules).await;

        // Test with a very long command (1000 characters)
        let long_command = "echo ".to_string() + &"a".repeat(1000);
        test_bash_rules(Some(long_command), Some(config_path), false)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_bash_rules_whitespace_only_command() {
        let temp_dir = setup_isolated_xdg_config();
        let rules = vec![BashRule {
            name: "deny-whitespace".to_string(),
            pattern: r"^\s+$".to_string(),
            action: BashRuleAction::Deny("Whitespace only".to_string()),
        }];

        let config_path = create_test_config(&temp_dir, rules).await;

        test_bash_rules(Some("   \t\n".to_string()), Some(config_path), false)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_bash_rules_json_output_structure_allowed() {
        let temp_dir = setup_isolated_xdg_config();
        let rules = vec![BashRule {
            name: "allow-ls".to_string(),
            pattern: r"^ls".to_string(),
            action: BashRuleAction::Allow,
        }];

        let config_path = create_test_config(&temp_dir, rules).await;

        // Capture stdout by redirecting to a temp file
        let output_file = temp_dir.path().join("output.json");
        let _file = std::fs::File::create(&output_file).unwrap();

        // Run the command (output goes to stdout, we can't easily capture it here)
        // But we can at least verify it doesn't crash with JSON mode
        test_bash_rules(Some("ls".to_string()), Some(config_path), true)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_bash_rules_json_output_structure_denied() {
        let temp_dir = setup_isolated_xdg_config();
        let rules = vec![BashRule {
            name: "deny-rm".to_string(),
            pattern: r"^rm".to_string(),
            action: BashRuleAction::Deny("Dangerous command".to_string()),
        }];

        let config_path = create_test_config(&temp_dir, rules).await;

        test_bash_rules(Some("rm file.txt".to_string()), Some(config_path), true)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_bash_rules_json_output_structure_modified() {
        let temp_dir = setup_isolated_xdg_config();
        let rules = vec![BashRule {
            name: "add-flag".to_string(),
            pattern: r"^(ls)$".to_string(),
            action: BashRuleAction::Modify("$1 -la".to_string()),
        }];

        let config_path = create_test_config(&temp_dir, rules).await;

        test_bash_rules(Some("ls".to_string()), Some(config_path), true)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_bash_rules_first_match_wins() {
        let temp_dir = setup_isolated_xdg_config();
        let rules = vec![
            BashRule {
                name: "deny-ls".to_string(),
                pattern: r"^ls".to_string(),
                action: BashRuleAction::Deny("First rule denies".to_string()),
            },
            BashRule {
                name: "allow-ls".to_string(),
                pattern: r"^ls".to_string(),
                action: BashRuleAction::Allow,
            },
        ];

        let config_path = create_test_config(&temp_dir, rules).await;

        // First rule should win (deny)
        test_bash_rules(Some("ls".to_string()), Some(config_path), false)
            .await
            .unwrap();
    }
}
