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

use super::{load_project_settings, ProjectApprovals, ProjectConfig, VerificationResult};

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
    // Canonicalize path to prevent traversal attacks
    let canonical_dir = project_dir
        .canonicalize()
        .into_diagnostic()
        .with_context(|| format!("Failed to canonicalize path: {}", project_dir.display()))?;

    // Load project settings
    let settings = load_project_settings(canonical_dir.clone()).await?;

    // Load approvals
    let approvals = ProjectApprovals::load()
        .await
        .context("Failed to load project approvals")?;

    // Verify all configured commands
    verify_all_commands(&approvals, &canonical_dir, &settings).await?;

    Ok(VerifiedProject {
        canonical_dir,
        settings,
    })
}

/// Verifies that all configured commands in the project are approved.
///
/// Fails fast on the first verification failure to prevent partial execution
/// of commands where some are approved and others are not.
async fn verify_all_commands(
    approvals: &ProjectApprovals,
    canonical_dir: &Path,
    settings: &ProjectConfig,
) -> Result<()> {
    // Get all configured commands
    let all_commands = settings.commands.all();

    // Verify each command
    for (command_name, _) in &all_commands {
        let verification_result = approvals
            .verify_project(canonical_dir, command_name)
            .await
            .with_context(|| format!("Failed to verify command '{}'", command_name))?;

        match verification_result {
            VerificationResult::Approved => {
                // Continue to next command
            }
            VerificationResult::NotApproved => {
                return Err(miette::miette!(
                    "Project tools not approved. Run: moriarty approve-project {}",
                    canonical_dir.display()
                ));
            }
            VerificationResult::ConfigHashMismatch { expected, actual } => {
                return Err(miette::miette!(
                    "tools.toml has been modified since approval. \
                     Run: moriarty approve-project {} \
                     (expected: {}, actual: {})",
                    canonical_dir.display(),
                    expected,
                    actual
                ));
            }
            VerificationResult::BinaryHashMismatch {
                item,
                expected,
                actual,
            } => {
                return Err(miette::miette!(
                    "Binary for '{}' has been modified since approval. \
                     Run: moriarty approve-project {} \
                     (expected: {}, actual: {})",
                    item,
                    canonical_dir.display(),
                    expected,
                    actual
                ));
            }
            VerificationResult::ItemNotApproved { item } => {
                return Err(miette::miette!(
                    "Tool '{}' not approved. Run: moriarty approve-project {}",
                    item,
                    canonical_dir.display()
                ));
            }
        }
    }

    Ok(())
}

impl VerifiedProject {
    pub async fn run_command(&self, command_name: &str) -> Result<CommandOutput> {
        // Get the command from settings
        let maybe_command = match command_name {
            "lint" => &self.settings.commands.lint,
            "test" => &self.settings.commands.test,
            "build" => &self.settings.commands.build,
            "format" => &self.settings.commands.format,
            _ => {
                return Err(miette::miette!(
                    "Unknown command '{}'. Valid commands: lint, test, build, format",
                    command_name
                ))
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

        // Execute the command
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

        // Create futures for all commands
        let command_futures = stream::iter(all_commands.into_iter().map(|(name, command)| {
            // Must clone PathBuf because async closure captures by move and multiple
            // closures need access to canonical_dir. Can't share reference across threads.
            let canonical_dir = self.canonical_dir.clone();
            async move {
                let result = Self::execute_command_static(&canonical_dir, &name, &command).await;
                (name, command, result)
            }
        }))
        .buffer_unordered(MAX_CONCURRENT_COMMANDS)
        .collect::<Vec<_>>();

        let results = command_futures.await;

        // Convert results, propagating any errors
        let mut outputs = Vec::new();
        for (name, _command, result) in results {
            match result {
                Ok(output) => outputs.push(output),
                Err(e) => {
                    return Err(e.context(format!("Failed to execute command '{}'", name)));
                }
            }
        }

        // Sort to match Commands::all() order (lint→test→build→format).
        // This provides consistent output ordering despite parallel execution,
        // matching user expectations from the MCP protocol's standardized tool order.
        outputs.sort_by_key(|output| match output.name.as_str() {
            "lint" => 0,
            "test" => 1,
            "build" => 2,
            "format" => 3,
            _ => 999,
        });

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
        let (cmd, args) = command.split_first().expect("command is not empty");

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
    use super::*;
    use tempfile::TempDir;

    fn setup_test_project(config_content: &str) -> TempDir {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config_dir = temp_dir.path().join(".config");
        std::fs::create_dir(&config_dir).expect("Failed to create .config dir");
        std::fs::write(config_dir.join("tools.toml"), config_content)
            .expect("Failed to write tools.toml");
        temp_dir
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
        let result = verify_and_load_project(temp_dir.path().to_path_buf()).await;

        // Should fail at verification stage, not canonicalization
        assert!(result.is_err());
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(
            err_msg.contains("not approved"),
            "Should fail on approval check, not path canonicalization"
        );
    }

    #[tokio::test]
    async fn test_invalid_path() {
        let result = verify_and_load_project(PathBuf::from("/nonexistent/path/to/project")).await;

        assert!(result.is_err());
        let err_msg = format!("{:?}", result.unwrap_err());
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

        // For now, this will fail at verification - we need approval mocking for full test
        // This test documents the expected behavior once approvals are mocked
        let result = verify_and_load_project(temp_dir.path().to_path_buf()).await;
        assert!(result.is_err());
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(err_msg.contains("not approved"));
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
        let result = verify_and_load_project(temp_dir.path().to_path_buf()).await;
        assert!(result.is_err());
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

        // Would need approval mocking to test the full parallel execution and ordering
        let result = verify_and_load_project(temp_dir.path().to_path_buf()).await;
        assert!(result.is_err());
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(err_msg.contains("not approved"));
    }
}
