//! Shared helpers for read-only VCS MCP servers (git, jj).
//!
//! Both [`super::git_read_only`] and [`super::jj_read_only`] expose a tiny set of
//! read-only commands that execute an external binary inside a validated project
//! directory. The validation, process spawning, UTF-8 loss-tolerant output handling,
//! and error shaping are identical. This module centralizes those concerns so each
//! server only needs to describe its MCP tool surface.

// standard library imports
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

// 3rd party crates
use rmcp::{model::ErrorCode, ErrorData as McpError, Json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

/// Result of executing a read-only VCS command.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommandResult {
    pub stdout: String,
    pub stderr: String,
    /// Exit code (0 indicates success)
    pub exit_code: i32,
}

/// Canonicalizes `project_dir` and ensures it looks like a valid Moriarty project
/// directory by checking for `.config/tools.toml`.
///
/// Returns `INVALID_PARAMS` errors suitable for direct use in MCP tool responses.
pub fn validate_project_dir(project_dir: &Path) -> Result<PathBuf, McpError> {
    // Canonicalize path to prevent traversal attacks and resolve symlinks
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

    // Verify this is a valid project directory by checking for .config/tools.toml
    let config_path = canonical_dir.join(".config").join("tools.toml");
    if !config_path.exists() {
        return Err(McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: format!(
                "Invalid project directory: {} (missing .config/tools.toml)",
                canonical_dir.display()
            )
            .into(),
            data: None,
        });
    }

    Ok(canonical_dir)
}

/// Runs a read-only command (e.g., `git`, `jj`) inside `project_dir`.
///
/// - Validates `project_dir` via [`validate_project_dir`].
/// - Spawns the command directly (no shell) using `subcommand_args` followed by `args`.
///   `subcommand_args` are the fixed subcommand tokens (e.g., `["status"]` for `git status`,
///   or `["op", "log"]` for `jj op log`); `args` are user-supplied extra flags forwarded
///   verbatim.
/// - Converts stdout/stderr using lossy UTF-8 so invalid encodings never crash the server.
/// - Maps spawn failures to [`ErrorCode::INTERNAL_ERROR`] using `label` in the message.
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

    let mut cmd = Command::new(program);
    cmd.args(subcommand_args);
    cmd.current_dir(canonical_dir);
    cmd.args(args);

    match cmd.output().await {
        Ok(result) => {
            // Use lossy UTF-8 conversion to handle potentially invalid encodings
            // in filenames (e.g., legacy encodings). This prevents server crashes
            // while allowing output to be displayed with replacement characters (�).
            // This is acceptable for a read-only tool where we never modify data.
            let stderr = String::from_utf8_lossy(&result.stderr).to_string();
            let stdout = String::from_utf8_lossy(&result.stdout).to_string();

            Ok(Json(CommandResult {
                exit_code: result.status.code().unwrap_or(-1),
                stderr,
                stdout,
            }))
        }
        Err(error) => Err(McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: format!("{label} failed: {error:?}").into(),
            data: None,
        }),
    }
}

/// Test support shared between `git_read_only` and `jj_read_only`.
///
/// Each backend's unit tests instantiate [`path_safety_tests!`] with a setup
/// function that returns a ready-to-use repo `TempDir` and a `run` closure
/// that executes a typical read-only command through the backend's validated
/// entry point. The macro emits four `#[tokio::test]` cases covering path
/// traversal, symlink resolution, the missing-config rejection branch, and
/// the nonexistent-directory rejection branch.
#[cfg(test)]
pub(crate) mod test_support {
    /// Generates the four path-safety `#[tokio::test]`s against the provided
    /// `$run` closure (async, taking `PathBuf` and `Vec<String>`).
    ///
    /// - `$setup_repo` is an `async` expression (e.g. `setup_git_repo()`) that
    ///   is awaited once per generated test to stand up an initialised repo
    ///   with `.config/tools.toml`.
    /// - `$init_bare_repo` initializes the VCS inside a fresh `TempDir` without
    ///   writing `.config/tools.toml`, used by the missing-config test.
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
            async fn test_rejects_directory_without_config_file() {
                let run = $run;
                let init_bare_repo = $init_bare_repo;
                let temp_dir = ::tempfile::TempDir::new().unwrap();
                init_bare_repo(temp_dir.path());
                let Err(error) = run(temp_dir.path().to_path_buf(), Vec::<String>::new()).await
                else {
                    panic!("Expected error for directory without .config/tools.toml");
                };
                assert_eq!(error.code, ::rmcp::model::ErrorCode::INVALID_PARAMS);
                assert!(error.message.contains("Invalid project directory"));
                assert!(error.message.contains("missing .config/tools.toml"));
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
