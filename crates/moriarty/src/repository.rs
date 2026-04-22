//! Repository root detection for jujutsu and git workspaces.
//!
//! This module provides functionality to detect the repository root directory,
//! supporting both jujutsu (jj) workspaces and git repositories. This enables
//! shared approvals across multiple jj workspaces that reference the same
//! repository.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(test)]
use std::fs;

use miette::{IntoDiagnostic, Result, WrapErr};
use tracing::{debug, info};

/// Detects the repository root directory for the given path.
///
/// This function tries multiple strategies in order:
/// 1. Jujutsu: walks up the directory tree looking for `.jj/repo` and reads it
///    to resolve the repository root (no `jj` binary required)
/// 2. Git: runs `git rev-parse --git-common-dir` to find the shared `.git`
///    directory, supporting both regular repos and worktrees
/// 3. Canonicalized current directory (fallback for non-repo projects)
///
/// # Arguments
/// * `path` - The starting directory path (typically the project directory)
///
/// # Returns
/// The repository root path, or the canonicalized path if not in a repository.
///
/// # Example
/// ```no_run
/// use std::path::Path;
/// use moriarty::repository::detect_repository_root;
///
/// let root = detect_repository_root(Path::new("/workspace/src/project"))?;
/// // In a jj workspace: /workspace/src
/// // In a git repo: /workspace
/// // Not in repo: /workspace/src/project (canonicalized)
/// ```
pub fn detect_repository_root(path: &Path) -> Result<PathBuf> {
    info!(path = %path.display(), "Detecting repository root");

    // Try jujutsu first
    if let Some(jj_root) = try_jj_workspace_root(path) {
        info!(root = %jj_root.display(), "Detected jj workspace root");
        return Ok(jj_root);
    }

    // Fall back to git
    if let Some(git_root) = try_git_repository_root(path) {
        info!(root = %git_root.display(), "Detected git repository root");
        return Ok(git_root);
    }

    // Fall back to canonicalized path (not in a repository)
    let canonical = path
        .canonicalize()
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to canonicalize path: {}", path.display()))?;

    info!(
        root = %canonical.display(),
        "Not in a repository, using canonicalized path as root"
    );
    Ok(canonical)
}

/// Attempts to detect jujutsu workspace root.
///
/// Searches for `.jj/repo` file which contains the path to the repository root.
/// Walks up the directory tree until finding `.jj/repo` or reaching the root.
fn try_jj_workspace_root(path: &Path) -> Option<PathBuf> {
    debug!(path = %path.display(), "Attempting jj workspace root detection");

    let mut current = path;
    loop {
        let jj_repo_file = current.join(".jj").join("repo");

        if jj_repo_file.exists() {
            match std::fs::read_to_string(&jj_repo_file) {
                Ok(contents) => {
                    let jj_path_raw = contents.trim();

                    // Empty .jj/repo file is invalid - return None to fall back
                    if jj_path_raw.is_empty() {
                        debug!(
                            file = %jj_repo_file.display(),
                            "Empty .jj/repo file, falling back"
                        );
                        return None;
                    }

                    // The path may be relative or absolute. If relative, resolve it
                    // relative to the workspace directory (current), then canonicalize.
                    let jj_repo_path = if Path::new(jj_path_raw).is_absolute() {
                        PathBuf::from(jj_path_raw)
                    } else {
                        current.join(jj_path_raw)
                    };

                    // Canonicalize to resolve any relative components
                    match jj_repo_path.canonicalize() {
                        Ok(canonical_jj_repo) => {
                            // Get repository root (parent of .jj)
                            match canonical_jj_repo.parent() {
                                Some(jj_dir) => match jj_dir.parent() {
                                    Some(repo_root) => {
                                        debug!(
                                            root = %repo_root.display(),
                                            "Successfully detected jj repository root from .jj/repo"
                                        );
                                        return Some(repo_root.to_path_buf());
                                    }
                                    None => {
                                        debug!(
                                            jj_dir = %jj_dir.display(),
                                            "Failed to get repository root: .jj has no parent"
                                        );
                                        return None;
                                    }
                                },
                                None => {
                                    debug!(
                                        canonical_path = %canonical_jj_repo.display(),
                                        "Failed to get .jj directory: canonical path has no parent"
                                    );
                                    return None;
                                }
                            }
                        }
                        Err(e) => {
                            debug!(
                                error = %e,
                                jj_repo_path = %jj_repo_path.display(),
                                "Failed to canonicalize .jj/repo path"
                            );
                            return None;
                        }
                    }
                }
                Err(e) => {
                    debug!(
                        error = %e,
                        file = %jj_repo_file.display(),
                        "Failed to read .jj/repo file"
                    );
                    return None;
                }
            }
        }

        match current.parent() {
            Some(parent) => current = parent,
            None => {
                debug!("Reached filesystem root without finding .jj/repo");
                return None;
            }
        }
    }
}

/// Attempts to detect git repository root.
///
/// Uses `git rev-parse --git-common-dir` to find the common git directory,
/// which works correctly for both regular repos and worktrees. The common dir
/// is the `.git` directory of the main repository, and its parent is the repo root.
fn try_git_repository_root(path: &Path) -> Option<PathBuf> {
    debug!(path = %path.display(), "Attempting git repository root detection");

    let output = Command::new("git")
        .arg("rev-parse")
        .arg("--git-common-dir")
        .current_dir(path)
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let git_common_dir_str = String::from_utf8_lossy(&output.stdout);
            let git_common_dir = PathBuf::from(git_common_dir_str.trim());

            // --git-common-dir can return a relative or absolute path
            // Resolve relative paths relative to the current directory
            let git_common_dir = if git_common_dir.is_absolute() {
                git_common_dir
            } else {
                path.join(git_common_dir)
            };

            // Canonicalize and get parent (repository root)
            match git_common_dir.canonicalize() {
                Ok(canonical_git_dir) => match canonical_git_dir.parent() {
                    Some(root) => {
                        debug!(root = %root.display(), "Successfully detected git repository root");
                        Some(root.to_path_buf())
                    }
                    None => {
                        debug!(
                            git_dir = %canonical_git_dir.display(),
                            "Git common dir has no parent"
                        );
                        None
                    }
                },
                Err(e) => {
                    debug!(
                        error = %e,
                        git_common_dir = %git_common_dir.display(),
                        "Failed to canonicalize git common dir"
                    );
                    None
                }
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            debug!(
                exit_code = output.status.code(),
                stderr = %stderr,
                "git rev-parse command failed"
            );
            None
        }
        Err(e) => {
            debug!(error = %e, "Failed to execute git command (may not be installed)");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::env;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn test_detect_non_repository() -> Result<()> {
        // Use a temporary directory that's definitely not in a repo
        let temp_dir = env::temp_dir();
        let root = detect_repository_root(&temp_dir)?;

        // Should return canonicalized temp dir
        assert_eq!(root, temp_dir.canonicalize().into_diagnostic()?);
        Ok(())
    }

    #[test]
    fn test_detect_git_repository() -> Result<()> {
        // Test with current directory (which should be in git or jj)
        let current_dir = env::current_dir().into_diagnostic()?;
        let root = detect_repository_root(&current_dir)?;

        // Root should be an absolute path
        // Note: For jj workspaces, root may not be a parent of current_dir
        // since .jj/repo points to the actual repository location
        assert!(
            root.is_absolute(),
            "Repository root should be absolute path, got: {}",
            root.display()
        );
        Ok(())
    }

    #[test]
    fn test_jj_workspace_root_not_in_workspace() {
        let temp_dir = env::temp_dir();
        let result = try_jj_workspace_root(&temp_dir);

        // Temp dir should not be in a jj workspace
        // If this fails, the test environment is unusual (temp inside a workspace)
        assert!(
            result.is_none(),
            "Temp directory should not be detected as jj workspace: {:?}",
            result
        );
    }

    #[test]
    fn test_git_repository_root_not_in_repo() {
        let temp_dir = env::temp_dir();
        let result = try_git_repository_root(&temp_dir);

        // Temp dir should not be in a git repository
        // If this fails, the test environment is unusual (temp inside a repo)
        assert!(
            result.is_none(),
            "Temp directory should not be detected as git repository: {:?}",
            result
        );
    }

    #[test]
    fn test_detect_repository_root_nonexistent_directory() {
        let fake_path = Path::new("/this/path/definitely/does/not/exist/moriarty-test-12345");
        let result = detect_repository_root(fake_path);

        assert!(
            result.is_err(),
            "Should fail for non-existent directory, got: {:?}",
            result
        );

        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(
            err_msg.contains("Failed to canonicalize") || err_msg.contains("No such file"),
            "Error should mention canonicalization or file not found failure, got: {}",
            err_msg
        );
    }

    /// Writes a `.jj/repo` file inside `temp_dir` so tests can exercise
    /// malformed or empty workspace metadata without invoking `jj`.
    fn write_jj_repo_file(temp_dir: &TempDir, contents: &str) {
        let jj_dir = temp_dir.path().join(".jj");
        fs::create_dir(&jj_dir).unwrap();
        fs::write(jj_dir.join("repo"), contents).unwrap();
    }

    #[test]
    fn test_jj_repo_file_corrupted() {
        let temp_dir = TempDir::new().unwrap();
        write_jj_repo_file(&temp_dir, "/nonexistent/invalid/path/.jj/repo");

        let result = try_jj_workspace_root(temp_dir.path());
        assert!(
            result.is_none(),
            "Corrupted .jj/repo file should cause detection to return None (fallback gracefully)"
        );
    }

    #[test]
    fn test_jj_repo_file_empty() {
        let temp_dir = TempDir::new().unwrap();
        write_jj_repo_file(&temp_dir, "");

        let result = try_jj_workspace_root(temp_dir.path());
        assert!(
            result.is_none(),
            "Empty .jj/repo file should cause graceful fallback"
        );
    }

    #[test]
    fn test_jj_repo_file_with_relative_path() {
        let temp_base = TempDir::new().unwrap();

        // Create repository directory with .jj
        let repo_dir = temp_base.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();
        let repo_jj_dir = repo_dir.join(".jj");
        std::fs::create_dir_all(&repo_jj_dir).unwrap();
        let repo_jj_repo_dir = repo_jj_dir.join("repo");
        std::fs::create_dir_all(&repo_jj_repo_dir).unwrap();

        // Create workspace directory
        let workspace_dir = temp_base.path().join("workspace");
        std::fs::create_dir_all(&workspace_dir).unwrap();
        let workspace_jj_dir = workspace_dir.join(".jj");
        std::fs::create_dir_all(&workspace_jj_dir).unwrap();

        // Create .jj/repo with relative path pointing to repo
        let jj_repo_file = workspace_jj_dir.join("repo");
        std::fs::write(&jj_repo_file, "../repo/.jj/repo").unwrap();

        // Detection should resolve the relative path correctly
        let result = try_jj_workspace_root(&workspace_dir);

        let detected_root = match result {
            Some(root) => root,
            None => {
                panic!("Should successfully detect repository root from relative path in .jj/repo")
            }
        };

        let expected_root = repo_dir.canonicalize().unwrap();
        assert_eq!(
            detected_root, expected_root,
            "Repository root should match the canonicalized repo directory"
        );
    }
}
