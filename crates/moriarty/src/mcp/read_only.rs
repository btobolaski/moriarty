//! Shared helpers for read-only VCS MCP servers (git, jj).
//!
//! Both [`super::git_read_only`] and [`super::jj_read_only`] expose a tiny set of
//! read-only commands that execute an external binary inside a validated directory.
//! The validation, process spawning, UTF-8 loss-tolerant output handling, and
//! error shaping are identical. This module centralizes those concerns so each
//! server only needs to describe its MCP tool surface. Truncated captures fail explicitly: a partial
//! diff/log/file response must never look complete. Request ownership supplies cancellation; these
//! commands retain the supervisor's default startup/cleanup budgets and no execution deadline.

// standard library imports
use std::{
    ffi::OsStr,
    path::{Component, Path, PathBuf},
};

// 3rd party crates
use rmcp::{ErrorData as McpError, Json, model::ErrorCode};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::execution;
use process_supervisor::ProcessSpec;

/// Result of executing a read-only VCS command.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommandResult {
    pub stdout: String,
    pub stderr: String,
    /// Exit code (0 indicates success)
    pub exit_code: i32,
}

/// Rejects syntactic parent traversal and canonicalizes `project_dir` so the
/// VCS process cannot be pointed outside the caller's requested directory by an
/// indirect path that later resolves elsewhere.
///
/// Returns `INVALID_PARAMS` errors suitable for direct use in MCP tool responses.
pub fn validate_project_dir(project_dir: &Path) -> Result<PathBuf, McpError> {
    // Reject `..` before canonicalization so the caller cannot spell a valid
    // escape hatch that later resolves outside the requested directory.
    if project_dir
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: format!(
                "Invalid project directory: {} (parent traversal is not allowed)",
                project_dir.display()
            )
            .into(),
            data: None,
        });
    }

    let canonical_dir = project_dir.canonicalize().map_err(|e| McpError {
        code: ErrorCode::INVALID_PARAMS,
        message: format!(
            "Invalid project directory: {} ({})",
            project_dir.display(),
            e
        )
        .into(),
        data: None,
    })?;

    if !canonical_dir.is_dir() {
        return Err(McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: format!(
                "Invalid project directory: {} (not a directory)",
                canonical_dir.display()
            )
            .into(),
            data: None,
        });
    }

    Ok(canonical_dir)
}

/// Runs a fixed VCS subcommand in a validated directory without shell
/// interpretation.
///
/// `subcommand_args` are trusted tokens chosen by the server wrapper, while
/// `args` are the already-validated user arguments that should appear after the
/// fixed subcommand.
pub async fn run_read_only_command<S1, S2, I1, I2>(
    program: &str,
    label: &str,
    project_dir: PathBuf,
    subcommand_args: I1,
    args: I2,
) -> Result<Json<CommandResult>, McpError>
where
    S1: AsRef<OsStr>,
    S2: AsRef<OsStr>,
    I1: IntoIterator<Item = S1>,
    I2: IntoIterator<Item = S2>,
{
    let canonical_dir = validate_project_dir(&project_dir)?;

    let spec = ProcessSpec {
        program: program.into(),
        args: subcommand_args
            .into_iter()
            .map(|arg| arg.as_ref().to_owned())
            .chain(args.into_iter().map(|arg| arg.as_ref().to_owned()))
            .collect(),
        cwd: Some(canonical_dir),
    };
    let result = execution::execute(spec).await.and_then(|output| {
        let status = execution::complete_output(&output)?;
        Ok(Json(CommandResult {
            exit_code: status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout.bytes).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr.bytes).into_owned(),
        }))
    });
    result.map_err(|error| McpError {
        code: ErrorCode::INTERNAL_ERROR,
        message: format!("{label} failed: {error}").into(),
        data: None,
    })
}

/// Test support shared between `git_read_only` and `jj_read_only`.
///
/// Each backend's unit tests instantiate [`path_safety_tests!`] with a setup
/// function that returns a ready-to-use repo `TempDir` and a `run` closure
/// that executes a typical read-only command through the backend's validated
/// entry point. The macro emits five `#[tokio::test]` cases covering parent
/// traversal rejection, symlink resolution, acceptance of directories without
/// project config, regular-file rejection, and the nonexistent-directory
/// rejection branch.
#[cfg(test)]
pub(crate) mod test_support {
    use std::{os::unix::process::ExitStatusExt, process::ExitStatus};

    use tempfile::TempDir;

    use super::*;
    use crate::test_helpers::test_report;
    use process_supervisor::Completion;

    #[tokio::test]
    async fn incomplete_and_interrupted_vcs_results_are_errors_not_partial_success() {
        let directory = TempDir::new().unwrap();
        // Truncation pins this caller's complete-output gate, not just the shared exit mapping.
        for (expected, completion, truncated) in [
            (
                "Incomplete",
                Completion::Exited(ExitStatus::from_raw(0)),
                true,
            ),
            (
                "cancelled",
                Completion::Cancelled(ExitStatus::from_raw(0)),
                false,
            ),
        ] {
            let mut report = test_report(completion);
            report.stdout.bytes = b"private child output".to_vec();
            report.stdout.truncated = truncated;
            let error = execution::test_seams::with_result(
                Ok(report),
                run_read_only_command(
                    "git",
                    "diff",
                    directory.path().to_path_buf(),
                    ["diff"],
                    Vec::<String>::new(),
                ),
            )
            .await
            .err()
            .expect("incomplete VCS output must be rejected");
            assert_eq!(error.code, ErrorCode::INTERNAL_ERROR);
            assert!(!error.message.contains("private child output"));
            assert!(error.message.contains(expected), "{error}");
        }
    }

    /// Generates the five path-safety `#[tokio::test]`s against the provided
    /// `$run` closure (async, taking `PathBuf` and `Vec<String>`).
    ///
    /// - `$setup_repo` is an `async` expression (e.g. `setup_git_repo()`) that
    ///   is awaited once per generated test to stand up an initialised repo.
    /// - `$init_bare_repo` initializes the VCS inside a fresh `TempDir`, used by
    ///   the no-project-config test.
    macro_rules! path_safety_tests {
        ($run:expr, $setup_repo:expr, $init_bare_repo:expr $(,)?) => {
            #[tokio::test]
            async fn test_rejects_path_with_parent_traversal() {
                let run = $run;
                let temp_dir = $setup_repo.await;
                let malicious_path = temp_dir.path().join("..").join("..").join("tmp");
                let Err(error) = run(malicious_path, Vec::<String>::new()).await else {
                    panic!("Expected error for path traversal attempt");
                };
                assert_eq!(error.code, ::rmcp::model::ErrorCode::INVALID_PARAMS);
                assert!(error.message.contains("Invalid project directory"));
            }

            #[tokio::test]
            async fn test_resolves_symlinks_safely() {
                let run = $run;
                let temp_dir = $setup_repo.await;
                let link_dir = ::tempfile::TempDir::new().unwrap();
                let link_path = link_dir.path().join("project_link");
                #[cfg(unix)]
                std::os::unix::fs::symlink(temp_dir.path(), &link_path).unwrap();
                #[cfg(windows)]
                std::os::windows::fs::symlink_dir(temp_dir.path(), &link_path).unwrap();
                let cmd_result = run(link_path, Vec::<String>::new()).await.unwrap();
                assert_eq!(cmd_result.0.exit_code, 0);
            }

            #[tokio::test]
            async fn test_accepts_directory_without_config_file() {
                let run = $run;
                let init_bare_repo = $init_bare_repo;
                let temp_dir = ::tempfile::TempDir::new().unwrap();
                init_bare_repo(temp_dir.path());
                let cmd_result = run(temp_dir.path().to_path_buf(), Vec::<String>::new())
                    .await
                    .unwrap();
                assert_eq!(cmd_result.0.exit_code, 0);
            }

            #[tokio::test]
            async fn test_rejects_regular_file() {
                let run = $run;
                let temp_dir = ::tempfile::TempDir::new().unwrap();
                let file_path = temp_dir.path().join("not-a-directory");
                std::fs::write(&file_path, "content").unwrap();
                let Err(error) = run(file_path, Vec::<String>::new()).await else {
                    panic!("Expected error for regular file path");
                };
                assert_eq!(error.code, ::rmcp::model::ErrorCode::INVALID_PARAMS);
                assert!(error.message.contains("not a directory"));
            }

            #[tokio::test]
            async fn test_rejects_nonexistent_directory() {
                let run = $run;
                let Err(error) = run(
                    std::path::PathBuf::from("/nonexistent/directory"),
                    Vec::<String>::new(),
                )
                .await
                else {
                    panic!("Expected error for nonexistent directory");
                };
                assert_eq!(error.code, ::rmcp::model::ErrorCode::INVALID_PARAMS);
            }
        };
    }

    pub(crate) use path_safety_tests;
}
