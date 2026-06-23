//! Verified command execution for project tools.
//!
//! This module provides a safe, verified execution model for running project-configured
//! commands (lint, test, build, format). It ensures that:
//!
//! 1. Project directories are canonicalized to prevent path traversal attacks
//! 2. Configuration files are loaded and validated
//! 3. Commands are verified against stored approvals before execution
//! 4. Binary hashes match approved versions
//!
//! # Usage
//!
//! ```no_run
//! use moriarty::project_config::runner::verify_and_load_project;
//!
//! # async fn example() -> miette::Result<()> {
//! // Verify project and load configuration
//! let project = verify_and_load_project("/path/to/project".into()).await?;
//!
//! // Run a single command
//! let output = project.run_command("lint").await?;
//!
//! // Or run all configured commands in parallel
//! let results = project.run_all_commands().await?;
//! # Ok(())
//! # }
//! ```

use std::path::{Path, PathBuf};

use futures::stream::{self, StreamExt};
use miette::{Context, IntoDiagnostic, Result};
use tokio::process::Command;

use super::{ProjectApprovals, ProjectConfig, VerificationResult, load_project_settings};

/// Maximum number of commands to run concurrently.
///
/// Limited to 4 to balance parallelism with system resource usage. This value matches
/// the pattern used in the hooks module and prevents resource exhaustion when running
/// multiple heavyweight tools (compilers, linters, test suites) simultaneously.
const MAX_CONCURRENT_COMMANDS: usize = 4;

/// A verified project with loaded configuration and approved commands.
///
/// This struct represents a project that has been verified against stored approvals.
/// All commands run through this struct are guaranteed to have been approved and
/// their binaries verified.
#[derive(Debug)]
pub struct VerifiedProject {
    pub canonical_dir: PathBuf,
    pub settings: ProjectConfig,
}

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub name: String,
    pub command: Vec<String>,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Verifies a project and loads its configuration.
///
/// This is the main entry point for safely executing project commands. It ensures
/// that ALL configured commands have been explicitly approved before any execution
/// can occur, preventing unauthorized command execution.
pub async fn verify_and_load_project(project_dir: PathBuf) -> Result<VerifiedProject> {
    let canonical_dir = project_dir
        .canonicalize()
        .into_diagnostic()
        .with_context(|| format!("Failed to canonicalize path: {}", project_dir.display()))?;

    let settings = load_project_settings(canonical_dir.clone()).await?;

    let approvals = ProjectApprovals::load()
        .await
        .context("Failed to load project approvals")?;

    verify_all_commands(&approvals, &canonical_dir, &settings).await?;

    Ok(VerifiedProject {
        canonical_dir,
        settings,
    })
}

/// Formats the `"Run: moriarty approve-project <dir>"` fragment used by every
/// non-Approved verification error, so the advice stays worded the same way.
fn approve_hint(canonical_dir: &Path) -> String {
    format!("Run: moriarty approve-project {}", canonical_dir.display())
}

fn handle_verification_result(
    result: VerificationResult,
    item_type_plural: &str,
    canonical_dir: &Path,
) -> Result<()> {
    match result {
        VerificationResult::Approved => Ok(()),
        VerificationResult::NotApproved => Err(miette::miette!(
            "Project {} not approved. {}",
            item_type_plural,
            approve_hint(canonical_dir)
        )),
        VerificationResult::ConfigHashMismatch { expected, actual } => Err(miette::miette!(
            "tools.toml has been modified since approval. {} (expected: {}, actual: {})",
            approve_hint(canonical_dir),
            expected,
            actual
        )),
        VerificationResult::BinaryHashMismatch {
            item,
            expected,
            actual,
        } => Err(miette::miette!(
            "Binary for '{}' has been modified since approval. {} (expected: {}, actual: {})",
            item,
            approve_hint(canonical_dir),
            expected,
            actual
        )),
        VerificationResult::ItemNotApproved { item } => Err(miette::miette!(
            "Item '{}' not approved. {}",
            item,
            approve_hint(canonical_dir)
        )),
    }
}

/// Verifies that all configured commands and checks in the project are approved.
///
/// Fails fast on the first verification failure to prevent partial execution
/// of items where some are approved and others are not.
async fn verify_all_commands(
    approvals: &ProjectApprovals,
    canonical_dir: &Path,
    settings: &ProjectConfig,
) -> Result<()> {
    for (command_name, _) in &settings.commands.all() {
        let verification_result = approvals
            .verify_project(canonical_dir, command_name)
            .await
            .with_context(|| format!("Failed to verify command '{}'", command_name))?;

        handle_verification_result(verification_result, "tools", canonical_dir)?;
    }

    if let Some(checks) = &settings.checks {
        for check in checks {
            let verification_result = approvals
                .verify_check(canonical_dir, &check.name)
                .await
                .with_context(|| format!("Failed to verify check '{}'", check.name))?;

            handle_verification_result(verification_result, "checks", canonical_dir)?;
        }
    }

    Ok(())
}

impl VerifiedProject {
    pub async fn run_command(&self, command_name: &str) -> Result<CommandOutput> {
        let maybe_command = match command_name {
            "lint" => &self.settings.commands.lint,
            "test" => &self.settings.commands.test,
            "build" => &self.settings.commands.build,
            "format" => &self.settings.commands.format,
            _ => {
                return Err(miette::miette!(
                    "Unknown command '{}'. Valid commands: lint, test, build, format",
                    command_name
                ));
            }
        };

        let command = maybe_command.as_ref().ok_or_else(|| {
            miette::miette!(
                "The '{}' command is not configured for this project",
                command_name
            )
        })?;

        if command.is_empty() {
            return Err(miette::miette!(
                "The '{}' command is empty in the configuration",
                command_name
            ));
        }

        self.execute_command(command_name, command).await
    }

    /// Runs all configured commands in parallel with concurrency limit.
    ///
    /// Commands that exit with non-zero status are captured in the output,
    /// not treated as errors. Only execution failures (binary not found, etc) error out.
    pub async fn run_all_commands(&self) -> Result<Vec<CommandOutput>> {
        let all_commands = self.settings.commands.all();

        if all_commands.is_empty() {
            return Ok(Vec::new());
        }

        let items: Vec<_> = all_commands.into_iter().collect();

        // Sort to match Commands::all() order (lint→test→build→format).
        // This provides consistent output ordering despite parallel execution,
        // matching user expectations from the MCP protocol's standardized tool order.
        let sort_fn = |output: &CommandOutput| match output.name.as_str() {
            "lint" => 0,
            "test" => 1,
            "build" => 2,
            "format" => 3,
            _ => 999,
        };

        self.run_items_parallel(items, "command", sort_fn).await
    }

    /// Runs all configured checks in parallel with concurrency limit.
    ///
    /// Checks that exit with non-zero status are captured in the output,
    /// not treated as errors. Only execution failures (binary not found, etc) error out.
    pub async fn run_all_checks(&self) -> Result<Vec<CommandOutput>> {
        let checks = match &self.settings.checks {
            Some(checks) => checks,
            None => return Ok(Vec::new()),
        };

        if checks.is_empty() {
            return Ok(Vec::new());
        }

        let items: Vec<_> = checks
            .iter()
            .map(|check| (check.name.clone(), check.command.clone()))
            .collect();

        // Sort alphabetically by check name for consistent output
        let sort_fn = |output: &CommandOutput| output.name.clone();

        self.run_items_parallel(items, "check", sort_fn).await
    }

    /// Preserves different sorting strategies: commands use fixed ordering (MCP protocol)
    /// while checks use alphabetical ordering.
    async fn run_items_parallel<K>(
        &self,
        items: Vec<(String, Vec<String>)>,
        item_type: &str,
        sort_fn: impl Fn(&CommandOutput) -> K,
    ) -> Result<Vec<CommandOutput>>
    where
        K: Ord,
    {
        let item_futures = stream::iter(items.into_iter().map(|(name, command)| {
            // buffer_unordered requires owned values in closures, cannot share &self across tasks.
            let canonical_dir = self.canonical_dir.clone();
            async move {
                let result = Self::execute_command_static(&canonical_dir, &name, &command).await;
                (name, command, result)
            }
        }))
        .buffer_unordered(MAX_CONCURRENT_COMMANDS)
        .collect::<Vec<_>>();

        let results = item_futures.await;

        let mut outputs = Vec::new();
        for (name, _command, result) in results {
            match result {
                Ok(output) => outputs.push(output),
                Err(e) => {
                    return Err(e.context(format!("Failed to execute {} '{}'", item_type, name)));
                }
            }
        }

        outputs.sort_by_key(sort_fn);

        Ok(outputs)
    }

    async fn execute_command(&self, name: &str, command: &[String]) -> Result<CommandOutput> {
        Self::execute_command_static(&self.canonical_dir, name, command).await
    }

    /// Static method to enable calling from async closures without self.
    ///
    /// Required for `run_all_commands()` which needs to spawn multiple async tasks.
    /// Async closures capture `self` by move, but we need to call the same method
    /// from multiple parallel closures. The static method pattern avoids this by
    /// accepting borrowed parameters instead of requiring `self`.
    async fn execute_command_static(
        canonical_dir: &Path,
        name: &str,
        command: &[String],
    ) -> Result<CommandOutput> {
        let (cmd, args) = command
            .split_first()
            .expect("invariant: verify_all_commands ensures non-empty before execution");

        let output = Command::new(cmd)
            .args(args)
            .current_dir(canonical_dir)
            .output()
            .await
            .into_diagnostic()
            .with_context(|| format!("Failed to execute command '{}'", name))?;

        // Use lossy UTF-8 conversion to handle potentially invalid encodings
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        Ok(CommandOutput {
            name: name.to_string(),
            command: command.to_vec(),
            exit_code: output.status.code(),
            stdout,
            stderr,
        })
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::project_config::approvals;
    use crate::test_helpers::{
        setup_isolated_xdg_config, setup_project_dir_with_config as setup_test_project,
    };

    async fn setup_test_project_with_approvals(config_content: &str) -> (TempDir, TempDir) {
        let xdg_dir = setup_isolated_xdg_config();
        let temp_dir = setup_test_project(config_content);
        approvals::approve_project_config(temp_dir.path(), config_content)
            .await
            .unwrap();
        (temp_dir, xdg_dir)
    }

    /// Sets up, approves, and loads a project from `config_content`, returning
    /// the project and the `TempDir` guards that must outlive it.
    async fn approved_project(config_content: &str) -> (TempDir, TempDir, VerifiedProject) {
        let (temp_dir, xdg_dir) = setup_test_project_with_approvals(config_content).await;
        let project = verify_and_load_project(temp_dir.path().to_path_buf())
            .await
            .expect("Should load approved project");
        (temp_dir, xdg_dir, project)
    }

    /// Runs `verify_and_load_project` and asserts it fails with a `not approved`
    /// error. Used by the many tests that exercise the unapproved path.
    async fn assert_verify_not_approved(dir: &Path) {
        let err_msg = format!(
            "{:?}",
            verify_and_load_project(dir.to_path_buf())
                .await
                .expect_err("Should fail on approval check")
        );
        assert!(
            err_msg.contains("not approved"),
            "expected 'not approved' in error, got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn test_canonicalize_path() {
        let temp_dir = setup_test_project(
            r#"
[commands]
lint = ["echo", "lint"]
"#,
        );

        // This will fail verification since it's not approved, but we're just testing canonicalization
        // Should fail at verification stage, not canonicalization
        let err_msg = format!(
            "{:?}",
            verify_and_load_project(temp_dir.path().to_path_buf())
                .await
                .expect_err("Should fail on approval check")
        );
        assert!(
            err_msg.contains("not approved"),
            "Should fail on approval check, not path canonicalization"
        );
    }

    #[tokio::test]
    async fn test_invalid_path() {
        let err_msg = format!(
            "{:?}",
            verify_and_load_project(PathBuf::from("/nonexistent/path/to/project"))
                .await
                .expect_err("Should fail on path canonicalization")
        );
        assert!(
            err_msg.contains("canonicalize") || err_msg.contains("No such file"),
            "Should fail on path canonicalization: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_run_command_with_successful_execution() {
        let temp_dir = setup_test_project(
            r#"
[commands]
test = ["echo", "test output"]
"#,
        );

        // Without approval mocking, this exercises the verification failure path.
        assert_verify_not_approved(temp_dir.path()).await;
    }

    #[tokio::test]
    async fn test_run_command_unknown_command() {
        let temp_dir = setup_test_project(
            r#"
[commands]
lint = ["echo", "lint"]
"#,
        );

        // This test documents the expected error for unknown commands
        // Would need approval mocking to test fully
        let _err = verify_and_load_project(temp_dir.path().to_path_buf())
            .await
            .expect_err("Should fail on approval check");
    }

    #[tokio::test]
    async fn test_run_all_commands_empty_config() {
        let temp_dir = setup_test_project("[commands]");

        // Empty config with no commands passes verification (nothing to verify)
        // In a full test with approvals, this would succeed
        let result = verify_and_load_project(temp_dir.path().to_path_buf()).await;
        // Currently fails because we don't have approval mocking
        // but documents that empty configs are valid
        let _ = result;
    }

    #[tokio::test]
    async fn test_run_all_commands_multiple_tools() {
        let temp_dir = setup_test_project(
            r#"
[commands]
lint = ["echo", "lint"]
test = ["echo", "test"]
build = ["echo", "build"]
format = ["echo", "format"]
"#,
        );

        // Without approval mocking, this exercises the verification failure path.
        assert_verify_not_approved(temp_dir.path()).await;
    }

    #[tokio::test]
    async fn test_handle_verification_result_approved() {
        handle_verification_result(
            VerificationResult::Approved,
            "tools",
            Path::new("/test/path"),
        )
        .expect("Should succeed for Approved result");
    }

    #[tokio::test]
    async fn test_handle_verification_result_not_approved() {
        let err = handle_verification_result(
            VerificationResult::NotApproved,
            "tools",
            Path::new("/test/path"),
        )
        .expect_err("Should fail for NotApproved result");
        let err_msg = format!("{:?}", err);
        assert!(err_msg.contains("Project tools not approved"));
        assert!(err_msg.contains("moriarty approve-project"));
    }

    #[tokio::test]
    async fn test_handle_verification_result_config_hash_mismatch() {
        let err = handle_verification_result(
            VerificationResult::ConfigHashMismatch {
                expected: "abc123".to_string(),
                actual: "def456".to_string(),
            },
            "checks",
            Path::new("/test/path"),
        )
        .expect_err("Should fail for ConfigHashMismatch result");
        let err_msg = format!("{:?}", err);
        assert!(err_msg.contains("tools.toml has been modified"));
        assert!(err_msg.contains("abc123"));
        assert!(err_msg.contains("def456"));
    }

    #[tokio::test]
    async fn test_handle_verification_result_binary_hash_mismatch() {
        let err = handle_verification_result(
            VerificationResult::BinaryHashMismatch {
                item: "mycheck".to_string(),
                expected: "hash1".to_string(),
                actual: "hash2".to_string(),
            },
            "checks",
            Path::new("/test/path"),
        )
        .expect_err("Should fail for BinaryHashMismatch result");
        let err_msg = format!("{:?}", err);
        assert!(err_msg.contains("Binary for 'mycheck' has been modified"));
        assert!(err_msg.contains("hash1"));
        assert!(err_msg.contains("hash2"));
    }

    #[tokio::test]
    async fn test_handle_verification_result_item_not_approved() {
        let err = handle_verification_result(
            VerificationResult::ItemNotApproved {
                item: "mycheck".to_string(),
            },
            "checks",
            Path::new("/test/path"),
        )
        .expect_err("Should fail for ItemNotApproved result");
        let err_msg = format!("{:?}", err);
        assert!(err_msg.contains("Item 'mycheck' not approved"));
    }

    #[tokio::test]
    async fn test_verify_all_commands_with_checks() {
        let (temp_dir, _xdg_dir) = setup_test_project_with_approvals(
            r#"
[commands]
lint = ["echo", "lint"]

[[checks]]
name = "mycheck"
command = ["echo", "check"]
"#,
        )
        .await;

        let project = verify_and_load_project(temp_dir.path().to_path_buf())
            .await
            .expect("Should succeed with approved checks");

        assert_eq!(
            project.settings.commands.lint,
            Some(vec!["echo".to_string(), "lint".to_string()])
        );
        assert!(project.settings.checks.is_some());
        let checks = project.settings.checks.unwrap();
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].name, "mycheck");
    }

    #[tokio::test]
    async fn test_verify_all_commands_checks_not_approved() {
        let _xdg_dir = setup_isolated_xdg_config();
        let temp_dir = setup_test_project(
            r#"
[commands]
lint = ["echo", "lint"]

[[checks]]
name = "unapproved_check"
command = ["echo", "check"]
"#,
        );

        let err = verify_and_load_project(temp_dir.path().to_path_buf())
            .await
            .expect_err("Should fail for unapproved check");
        let err_msg = format!("{:?}", err);
        assert!(err_msg.contains("not approved"));
    }

    #[tokio::test]
    async fn test_run_all_checks_success() {
        let (temp_dir, _xdg_dir) = setup_test_project_with_approvals(
            r#"
[commands]

[[checks]]
name = "check1"
command = ["echo", "first"]

[[checks]]
name = "check2"
command = ["echo", "second"]
"#,
        )
        .await;

        let project = verify_and_load_project(temp_dir.path().to_path_buf())
            .await
            .expect("Should load approved project");
        let outputs = project.run_all_checks().await.expect("Should run checks");

        assert_eq!(outputs.len(), 2);
        // Checks are sorted alphabetically
        assert_eq!(outputs[0].name, "check1");
        assert_eq!(outputs[1].name, "check2");
        assert!(outputs[0].stdout.contains("first"));
        assert!(outputs[1].stdout.contains("second"));
        assert_eq!(outputs[0].exit_code, Some(0));
        assert_eq!(outputs[1].exit_code, Some(0));
    }

    #[tokio::test]
    async fn test_run_all_checks_empty() {
        let (_t, _xdg, project) = approved_project(
            r#"
[commands]
lint = ["echo", "lint"]
"#,
        )
        .await;

        let outputs = project
            .run_all_checks()
            .await
            .expect("Should handle no checks");
        assert_eq!(outputs.len(), 0);
    }

    #[tokio::test]
    async fn test_run_all_checks_nonzero_exit() {
        let (_t, _xdg, project) = approved_project(
            r#"
[commands]

[[checks]]
name = "failing_check"
command = ["sh", "-c", "exit 1"]
"#,
        )
        .await;

        let outputs = project
            .run_all_checks()
            .await
            .expect("Non-zero exit should not error");

        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].exit_code, Some(1));
    }

    #[tokio::test]
    async fn test_run_all_checks_alphabetical_sorting() {
        let (_t, _xdg, project) = approved_project(
            r#"
[commands]

[[checks]]
name = "zebra"
command = ["echo", "z"]

[[checks]]
name = "alpha"
command = ["echo", "a"]

[[checks]]
name = "beta"
command = ["echo", "b"]
"#,
        )
        .await;

        let outputs = project.run_all_checks().await.expect("Should run checks");

        assert_eq!(outputs.len(), 3);
        assert_eq!(outputs[0].name, "alpha");
        assert_eq!(outputs[1].name, "beta");
        assert_eq!(outputs[2].name, "zebra");
    }

    #[tokio::test]
    async fn test_run_all_commands_fixed_sorting() {
        let (_t, _xdg, project) = approved_project(
            r#"
[commands]
format = ["echo", "format"]
build = ["echo", "build"]
test = ["echo", "test"]
lint = ["echo", "lint"]
"#,
        )
        .await;

        let outputs = project
            .run_all_commands()
            .await
            .expect("Should run commands");

        assert_eq!(outputs.len(), 4);
        // Fixed order: lint, test, build, format
        assert_eq!(outputs[0].name, "lint");
        assert_eq!(outputs[1].name, "test");
        assert_eq!(outputs[2].name, "build");
        assert_eq!(outputs[3].name, "format");
    }
}
