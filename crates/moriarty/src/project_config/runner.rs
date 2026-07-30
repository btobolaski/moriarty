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

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use futures::stream::{self, StreamExt};
use miette::{Context, IntoDiagnostic, Result};
use tokio::process::Command;

use super::{ProjectApprovals, ProjectConfig, VerificationResult, load_project_settings};
use crate::repository::detect_repository_root;

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
///
/// `canonical_dir` is the directory commands execute in (the caller's project directory,
/// canonicalized), while `settings` comes from the shared repository root — the file whose hash
/// verification checked — so a jj secondary workspace or git worktree cannot substitute its own
/// `tools.toml` for the approved one. The resolved-program maps record, per item, the absolute
/// path verification hashed; execution spawns those instead of re-resolving `command[0]` against
/// `canonical_dir`, where an unapproved local file could shadow the approved one.
#[derive(Debug)]
pub struct VerifiedProject {
    pub canonical_dir: PathBuf,
    pub settings: ProjectConfig,
    resolved_commands: HashMap<String, PathBuf>,
    resolved_checks: HashMap<String, PathBuf>,
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
///
/// The configuration is loaded from the repository root rather than `project_dir`: approval
/// verification hashes the repository root's `tools.toml`, so executing a `tools.toml` read from
/// anywhere else (a jj secondary workspace or git worktree working copy) would run command arrays
/// the hash check never covered. `project_dir` still determines where commands execute.
pub async fn verify_and_load_project(project_dir: PathBuf) -> Result<VerifiedProject> {
    let canonical_dir = project_dir
        .canonicalize()
        .into_diagnostic()
        .with_context(|| format!("Failed to canonicalize path: {}", project_dir.display()))?;

    let repository_root = detect_repository_root(&canonical_dir)?;

    let settings = load_project_settings(repository_root.clone()).await?;

    let approvals = ProjectApprovals::load()
        .await
        .context("Failed to load project approvals")?;

    let (resolved_commands, resolved_checks) =
        verify_all_commands(&approvals, &canonical_dir, &repository_root, &settings).await?;

    Ok(VerifiedProject {
        canonical_dir,
        settings,
        resolved_commands,
        resolved_checks,
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
) -> Result<PathBuf> {
    match result {
        VerificationResult::Approved { program } => Ok(program),
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

/// Verifies that all configured commands and checks in the project are approved, returning the
/// per-item program paths verification resolved and hashed (commands map and checks map, in that
/// order) for execution to spawn.
///
/// The `_at_root` verifiers take the already-detected `repository_root` so root detection (which
/// can spawn a `git` subprocess) happens once per load instead of once per item;
/// `canonical_dir` is kept only for the approve-project hint in error messages, which should
/// name the directory the caller actually invoked.
///
/// Fails fast on the first verification failure to prevent partial execution
/// of items where some are approved and others are not.
async fn verify_all_commands(
    approvals: &ProjectApprovals,
    canonical_dir: &Path,
    repository_root: &Path,
    settings: &ProjectConfig,
) -> Result<(HashMap<String, PathBuf>, HashMap<String, PathBuf>)> {
    let all_commands = settings.commands.all();
    let mut resolved_commands = HashMap::with_capacity(all_commands.len());
    for (command_name, _) in &all_commands {
        let verification_result = approvals
            .verify_project_at_root(repository_root, command_name)
            .await
            .with_context(|| format!("Failed to verify command '{}'", command_name))?;

        let program = handle_verification_result(verification_result, "tools", canonical_dir)?;
        resolved_commands.insert(command_name.clone(), program);
    }

    let checks = settings.checks.as_deref().unwrap_or(&[]);
    let mut resolved_checks = HashMap::with_capacity(checks.len());
    for check in checks {
        let verification_result = approvals
            .verify_check_at_root(repository_root, &check.name)
            .await
            .with_context(|| format!("Failed to verify check '{}'", check.name))?;

        let program = handle_verification_result(verification_result, "checks", canonical_dir)?;
        resolved_checks.insert(check.name.clone(), program);
    }

    Ok((resolved_commands, resolved_checks))
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

        let program = Self::resolved_program(&self.resolved_commands, command_name)?;
        self.execute_command(command_name, command, &program).await
    }

    /// Look up the program path verification resolved for `name`. Every configured item is
    /// verified (and thus resolved) during [`verify_and_load_project`], so a miss means the
    /// caller mutated `settings` after construction; erroring beats spawning an unverified
    /// program. Takes the map rather than `&self` because the caller names which of the two
    /// resolved maps applies.
    fn resolved_program(resolved: &HashMap<String, PathBuf>, name: &str) -> Result<PathBuf> {
        resolved.get(name).cloned().ok_or_else(|| {
            miette::miette!(
                "No verified program recorded for '{}'; refusing to execute",
                name
            )
        })
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

        let items = all_commands
            .into_iter()
            .map(|(name, command)| {
                let program = Self::resolved_program(&self.resolved_commands, &name)?;
                Ok((name, command, program))
            })
            .collect::<Result<Vec<_>>>()?;

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

        let items = checks
            .iter()
            .map(|check| {
                let program = Self::resolved_program(&self.resolved_checks, &check.name)?;
                Ok((check.name.clone(), check.command.clone(), program))
            })
            .collect::<Result<Vec<_>>>()?;

        // Sort alphabetically by check name for consistent output
        let sort_fn = |output: &CommandOutput| output.name.clone();

        self.run_items_parallel(items, "check", sort_fn).await
    }

    /// Preserves different sorting strategies: commands use fixed ordering (MCP protocol)
    /// while checks use alphabetical ordering.
    async fn run_items_parallel<K>(
        &self,
        items: Vec<(String, Vec<String>, PathBuf)>,
        item_type: &str,
        sort_fn: impl Fn(&CommandOutput) -> K,
    ) -> Result<Vec<CommandOutput>>
    where
        K: Ord,
    {
        let item_futures = stream::iter(items.into_iter().map(|(name, command, program)| {
            // buffer_unordered requires owned values in closures, cannot share &self across tasks.
            let canonical_dir = self.canonical_dir.clone();
            async move {
                let result =
                    Self::execute_command_static(&canonical_dir, &name, &command, &program).await;
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

    async fn execute_command(
        &self,
        name: &str,
        command: &[String],
        program: &Path,
    ) -> Result<CommandOutput> {
        Self::execute_command_static(&self.canonical_dir, name, command, program).await
    }

    /// Static method to enable calling from async closures without self.
    ///
    /// Required for `run_all_commands()` which needs to spawn multiple async tasks.
    /// Async closures capture `self` by move, but we need to call the same method
    /// from multiple parallel closures. The static method pattern avoids this by
    /// accepting borrowed parameters instead of requiring `self`.
    ///
    /// `program` (the path verification resolved for `command[0]`) is what gets spawned;
    /// `command[0]` itself only appears in the reported [`CommandOutput`]. Spawning the raw
    /// string would re-resolve a relative program against `canonical_dir`, which is not the
    /// repository root verification hashed against when the caller is in a jj secondary
    /// workspace or git worktree.
    async fn execute_command_static(
        canonical_dir: &Path,
        name: &str,
        command: &[String],
        program: &Path,
    ) -> Result<CommandOutput> {
        let (_, args) = command
            .split_first()
            .expect("invariant: verify_all_commands ensures non-empty before execution");

        let output = Command::new(program)
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
        assert_approved_copy_ran_in, setup_isolated_xdg_config,
        setup_jj_main_and_secondary_workspace, setup_project_dir_with_config as setup_test_project,
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
        let program = handle_verification_result(
            VerificationResult::Approved {
                program: PathBuf::from("/test/bin/echo"),
            },
            "tools",
            Path::new("/test/path"),
        )
        .expect("Should succeed for Approved result");
        assert_eq!(program, PathBuf::from("/test/bin/echo"));
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
    async fn resolved_program_missing_entry_is_an_error() {
        // The maps are populated for every configured item at construction, so a miss can only
        // mean `settings` was mutated afterwards; execution must refuse rather than spawn an
        // unverified program.
        let (_tmp, _xdg, project) =
            approved_project("[commands]\nlint = [\"echo\", \"lint\"]\n").await;
        let err = VerifiedProject::resolved_program(&project.resolved_commands, "not-recorded")
            .expect_err("unrecorded item must not execute");
        let err_msg = format!("{:?}", err);
        assert!(
            err_msg.contains("No verified program recorded"),
            "got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn secondary_workspace_cannot_substitute_config_or_programs() {
        // The one claim under test: nothing a divergent secondary workspace carries — its own
        // tools.toml (naming a different check) or its own copy of a relative program — can
        // influence what runs; only the execution cwd is workspace-local. `test` (a command) and
        // `where` (a check) share one script so both resolved maps and all three entry points
        // (`run_command`, `run_all_commands`, `run_all_checks`) are proven in one scenario.
        let _xdg = setup_isolated_xdg_config();
        let config = "[commands]\ntest = [\"./tool.sh\"]\n\n[[checks]]\nname = \"where\"\ncommand = [\"./tool.sh\"]\n";
        let (_base, main, workspace) = setup_jj_main_and_secondary_workspace(config, "tool.sh");
        approvals::approve_project_config(&main, config)
            .await
            .unwrap();
        std::fs::create_dir_all(workspace.join(".config")).unwrap();
        std::fs::write(
            workspace.join(".config/tools.toml"),
            "[commands]\n[[checks]]\nname = \"evil\"\ncommand = [\"./tool.sh\"]\n",
        )
        .unwrap();

        let project = verify_and_load_project(workspace.clone())
            .await
            .expect("workspace should verify against the repository root's approved config");
        assert_eq!(
            project.settings.checks.as_ref().unwrap()[0].name,
            "where",
            "settings must come from the repository root's tools.toml, not the workspace copy"
        );

        let single = project.run_command("test").await.unwrap();
        assert_approved_copy_ran_in(&single.stdout, &main, &workspace);
        let all_commands = project.run_all_commands().await.unwrap();
        assert_approved_copy_ran_in(&all_commands[0].stdout, &main, &workspace);
        let all_checks = project.run_all_checks().await.unwrap();
        assert_approved_copy_ran_in(&all_checks[0].stdout, &main, &workspace);
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
