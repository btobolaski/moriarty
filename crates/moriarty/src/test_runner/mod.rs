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

use std::path::PathBuf;

use miette::Result;

use crate::project_config::runner::{verify_and_load_project, CommandOutput, VerifiedProject};

pub async fn exec_test(cmd: crate::TestCommand) -> Result<()> {
    match cmd {
        crate::TestCommand::ProjectTools { project_dir } => run_project_tools(project_dir).await,
        crate::TestCommand::Checks { project_dir } => run_checks(project_dir).await,
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
