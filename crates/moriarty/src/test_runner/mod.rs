//! Test runner for parallel execution of project tools.
//!
//! This module provides functionality to run all configured project tools
//! (lint, test, build, format) in parallel and display comprehensive output.
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

use crate::project_config::runner::verify_and_load_project;

pub async fn exec_test(cmd: crate::TestCommand) -> Result<()> {
    match cmd {
        crate::TestCommand::ProjectTools { project_dir } => run_project_tools(project_dir).await,
    }
}

async fn run_project_tools(project_dir: PathBuf) -> Result<()> {
    // Verify project and load configuration
    let project = verify_and_load_project(project_dir).await?;

    // Display header
    println!(
        "Running project tools for: {}\n",
        project.canonical_dir.display()
    );

    // Get all configured commands
    let all_commands = project.settings.commands.all();
    if all_commands.is_empty() {
        println!("No tools configured in .config/tools.toml");
        return Ok(());
    }

    println!(
        "Found {} configured tool{}: {}\n",
        all_commands.len(),
        if all_commands.len() == 1 { "" } else { "s" },
        all_commands
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    // Run all commands in parallel
    let results = project.run_all_commands().await?;

    // Display results for each tool
    for output in &results {
        println!("{}", "━".repeat(80));
        println!("Tool: {}", output.name);
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

    // Display summary
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
        println!("All tools completed successfully!");
    } else {
        println!(
            "{} tool{} failed!",
            failed_count,
            if failed_count == 1 { "" } else { "s" }
        );
        // Use process::exit instead of returning Err to provide a clean exit code
        // for CI/CD integration. Returning Err would print a debug error message
        // that duplicates the summary we just displayed.
        std::process::exit(1);
    }

    Ok(())
}
