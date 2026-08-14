//! Repository and workspace root detection for jujutsu and git workspaces.
//!
//! Two related but distinct roots are detected here:
//! [`detect_repository_root`] resolves to the shared repository (following a jj
//! secondary workspace's store pointer or a git worktree's common dir), which
//! keys approvals so all workspaces of one repository share them;
//! [`detect_workspace_root`] resolves to the root of the working copy
//! containing the path, which is where project checks execute so they validate
//! the files that were actually edited.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(test)]
use std::fs;

use miette::{IntoDiagnostic, WrapErr};
use tracing::{debug, info};

/// Detects the repository root directory for the given path.
///
/// This function tries multiple strategies in order:
/// 1. Jujutsu: walks up the directory tree looking for `.jj/repo` and resolves
///    it to the repository root (no `jj` binary required)
/// 2. Git: runs `git rev-parse --git-common-dir` to find the shared `.git`
///    directory, supporting both regular repos and worktrees
/// 3. Canonicalized current directory (fallback for non-repo projects)
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
pub fn detect_repository_root(path: &Path) -> miette::Result<PathBuf> {
    info!(path = %path.display(), "Detecting repository root");

    if let Some(jj_root) = try_jj_workspace_root(path) {
        info!(root = %jj_root.display(), "Detected jj workspace root");
        return Ok(jj_root);
    }

    if let Some(git_root) = try_git_repository_root(path) {
        info!(root = %git_root.display(), "Detected git repository root");
        return Ok(git_root);
    }

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

/// Detects the root of the workspace containing `path` — the working copy
/// being operated on — as opposed to [`detect_repository_root`], which resolves
/// a jj secondary workspace or git worktree to the shared repository it points
/// at. Approvals are keyed by repository root so all workspaces share them;
/// checks execute in the workspace root so they validate the working copy that
/// was actually edited (and not a subdirectory of it, when the hook is invoked
/// from one).
///
/// Strategies, in order:
/// 1. Jujutsu: walks up to the nearest directory containing `.jj/repo`
///    (the store directory in a main workspace, a pointer file in a secondary
///    one — either way that directory is the workspace root, so no pointer
///    resolution is needed)
/// 2. Git: runs `git rev-parse --show-toplevel`, which returns the current
///    worktree's root rather than the shared repository's
/// 3. Canonicalized input path (fallback for non-repo projects)
///
/// The input is canonicalized before the jj walk so the returned root is
/// canonical too (the walk returns an ancestor of its starting path verbatim).
pub fn detect_workspace_root(path: &Path) -> miette::Result<PathBuf> {
    info!(path = %path.display(), "Detecting workspace root");

    let canonical = path
        .canonicalize()
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to canonicalize path: {}", path.display()))?;

    if let Some(root) = nearest_jj_workspace_dir(&canonical) {
        info!(root = %root.display(), "Detected jj workspace directory");
        return Ok(root);
    }

    if let Some(root) = try_git_worktree_root(&canonical) {
        info!(root = %root.display(), "Detected git worktree root");
        return Ok(root);
    }

    info!(
        root = %canonical.display(),
        "Not in a repository, using canonicalized path as workspace root"
    );
    Ok(canonical)
}

/// Walks up from `path` to the nearest directory containing a `.jj/repo`
/// entry — a workspace root, whether the entry is the store directory (main
/// workspace) or a pointer file (secondary workspace). Shared by
/// [`detect_workspace_root`] (which wants this directory itself) and
/// [`try_jj_workspace_root`] (which resolves the entry to the repository root).
fn nearest_jj_workspace_dir(path: &Path) -> Option<PathBuf> {
    let mut current = path;
    loop {
        if current.join(".jj").join("repo").exists() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
}

/// Attempts to detect the current git worktree's root via
/// `git rev-parse --show-toplevel`, which stays inside the worktree instead of
/// following it back to the shared repository like `--git-common-dir` does.
fn try_git_worktree_root(path: &Path) -> Option<PathBuf> {
    debug!(path = %path.display(), "Attempting git worktree root detection");

    let output = Command::new("git")
        .arg("rev-parse")
        .arg("--show-toplevel")
        .current_dir(path)
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let toplevel_str = String::from_utf8_lossy(&output.stdout);
            let toplevel = Path::new(toplevel_str.trim());

            match toplevel.canonicalize() {
                Ok(root) => {
                    debug!(root = %root.display(), "Successfully detected git worktree root");
                    Some(root)
                }
                Err(e) => {
                    debug!(
                        error = %e,
                        toplevel = %toplevel.display(),
                        "Failed to canonicalize git worktree root"
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
                "git rev-parse --show-toplevel failed"
            );
            None
        }
        Err(e) => {
            debug!(error = %e, "Failed to execute git command (may not be installed)");
            None
        }
    }
}

/// Attempts to detect jujutsu repository root: the nearest workspace's
/// `.jj/repo` entry resolved to the repository it belongs to via
/// [`resolve_jj_repo_root`].
fn try_jj_workspace_root(path: &Path) -> Option<PathBuf> {
    debug!(path = %path.display(), "Attempting jj workspace root detection");

    let Some(workspace_dir) = nearest_jj_workspace_dir(path) else {
        debug!("Reached filesystem root without finding .jj/repo");
        return None;
    };

    let jj_dir = workspace_dir.join(".jj");
    let jj_repo = jj_dir.join("repo");
    resolve_jj_repo_root(&jj_dir, &jj_repo)
}

/// Resolves the jj repository root from a discovered `.jj/repo` entry.
///
/// In the main workspace `.jj/repo` is the repo store directory itself; in a
/// secondary workspace (created by `jj workspace add`) it is a file pointing at
/// the main workspace's store. An absolute pointer is used as-is.
///
/// A relative pointer's base changed across jj versions: jj 0.41+ writes it
/// relative to the `.jj` directory that holds the file, while older jj wrote it
/// relative to the workspace directory. Both bases are tried (modern first) and
/// whichever resolves to an existing store wins — the two bases sit one level
/// apart, so at most one points at a real store. Trying only one base would
/// either break secondary workspaces under jj 0.41+ (the original bug) or drop
/// support for repositories created by older jj.
fn resolve_jj_repo_root(jj_dir: &Path, jj_repo: &Path) -> Option<PathBuf> {
    // Main workspace: the entry is the store directory itself.
    if jj_repo.is_dir() {
        return store_root(jj_repo);
    }

    let contents = match std::fs::read_to_string(jj_repo) {
        Ok(contents) => contents,
        Err(e) => {
            debug!(error = %e, file = %jj_repo.display(), "Failed to read .jj/repo file");
            return None;
        }
    };

    let pointer = contents.trim();
    if pointer.is_empty() {
        debug!(file = %jj_repo.display(), "Empty .jj/repo file, falling back");
        return None;
    }

    let pointer = Path::new(pointer);
    if pointer.is_absolute() {
        return store_root(pointer).or_else(|| {
            debug!(pointer = %pointer.display(), "Absolute .jj/repo pointer does not resolve to a store");
            None
        });
    }

    // `jj_dir` is `<workspace>/.jj`, so its parent is the workspace directory.
    // `.jj` is never the filesystem root, so the parent always exists; the
    // fallback is unreachable but stays correct rather than off-by-one.
    let workspace_dir = jj_dir.parent().unwrap_or_else(|| Path::new("/"));
    store_root(&jj_dir.join(pointer))
        .or_else(|| store_root(&workspace_dir.join(pointer)))
        .or_else(|| {
            debug!(
                pointer = %pointer.display(),
                jj_dir = %jj_dir.display(),
                "Could not resolve relative .jj/repo pointer against the .jj or workspace directory"
            );
            None
        })
}

/// Canonicalizes a candidate `.jj/repo` store path and strips the `.jj/repo`
/// suffix (two components) to yield the repository root.
///
/// Returns `None` when the store does not exist (so callers can try another
/// candidate base) or has no grandparent directory. Canonicalization failures
/// are intentionally not logged here because a failed candidate is expected
/// while probing the alternate relative base.
fn store_root(store: &Path) -> Option<PathBuf> {
    let canonical = store.canonicalize().ok()?;
    match canonical.parent().and_then(Path::parent) {
        Some(repo_root) => {
            debug!(root = %repo_root.display(), "Detected jj repository root from .jj/repo");
            Some(repo_root.to_path_buf())
        }
        None => {
            debug!(
                store = %canonical.display(),
                "Resolved .jj/repo store has no grandparent directory"
            );
            None
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
    use crate::test_helpers::{run_git_command, setup_git_repo_with_commit};

    #[test]
    fn test_detect_non_repository() -> miette::Result<()> {
        // Use a temporary directory that's definitely not in a repo
        let temp_dir = env::temp_dir();
        let root = detect_repository_root(&temp_dir)?;

        // Should return canonicalized temp dir
        assert_eq!(root, temp_dir.canonicalize().into_diagnostic()?);
        Ok(())
    }

    #[test]
    fn test_detect_git_repository() -> miette::Result<()> {
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

    /// Builds a secondary-workspace layout under `base`: a main store directory
    /// at `<base>/<store>/.jj/repo` and a secondary workspace skeleton at
    /// `<base>/<workspace>/.jj/`. Returns the workspace directory, the path of
    /// its (not-yet-written) `.jj/repo` pointer file, and the canonical store
    /// root the pointer should ultimately resolve to. The caller writes the
    /// pointer so each test controls the relative/absolute form under test.
    fn build_jj_store_and_workspace(
        base: &Path,
        store: &str,
        workspace: &str,
    ) -> (PathBuf, PathBuf, PathBuf) {
        let store_root = base.join(store);
        std::fs::create_dir_all(store_root.join(".jj").join("repo")).unwrap();

        let workspace_dir = base.join(workspace);
        let workspace_jj = workspace_dir.join(".jj");
        std::fs::create_dir_all(&workspace_jj).unwrap();

        (
            workspace_dir,
            workspace_jj.join("repo"),
            store_root.canonicalize().unwrap(),
        )
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
    fn test_jj_repo_file_modern_relative_pointer() {
        // jj 0.41+ writes the pointer relative to the `.jj` directory that holds
        // it. The store sits one level higher than the legacy workspace-relative
        // base would reach, so only the `.jj`-relative resolution finds it: the
        // workspace-relative base escapes `temp_base` to a path that does not
        // exist. This guards the jj 0.41 fix.
        let temp_base = TempDir::new().unwrap();
        let (workspace_dir, pointer_file, expected_root) =
            build_jj_store_and_workspace(temp_base.path(), "mainrepo", "workspace");
        std::fs::write(&pointer_file, "../../mainrepo/.jj/repo").unwrap();

        let detected = try_jj_workspace_root(&workspace_dir)
            .expect("modern .jj-relative pointer should resolve to the main store root");
        assert_eq!(detected, expected_root);
    }

    #[test]
    fn test_jj_repo_file_legacy_relative_pointer() {
        // Older jj wrote the pointer relative to the workspace directory. Here
        // store and workspace are siblings, so only the workspace-relative base
        // ("../repo/...") reaches the store; the `.jj`-relative base would look
        // inside `workspace/` and miss. This guards backward compatibility with
        // repositories created by pre-0.41 jj.
        let temp_base = TempDir::new().unwrap();
        let (workspace_dir, pointer_file, expected_root) =
            build_jj_store_and_workspace(temp_base.path(), "repo", "workspace");
        std::fs::write(&pointer_file, "../repo/.jj/repo").unwrap();

        let detected = try_jj_workspace_root(&workspace_dir)
            .expect("legacy workspace-relative pointer should still resolve");
        assert_eq!(detected, expected_root);
    }

    #[test]
    fn test_jj_repo_file_absolute_pointer() {
        // An absolute pointer is used verbatim, independent of either relative base.
        let temp_base = TempDir::new().unwrap();
        let (workspace_dir, pointer_file, expected_root) =
            build_jj_store_and_workspace(temp_base.path(), "mainrepo", "workspace");
        let absolute_store = expected_root.join(".jj").join("repo");
        std::fs::write(&pointer_file, absolute_store.to_str().unwrap()).unwrap();

        let detected = try_jj_workspace_root(&workspace_dir)
            .expect("absolute pointer should resolve to the store's repository root");
        assert_eq!(detected, expected_root);
    }

    #[test]
    fn test_jj_relative_pointer_nonexistent() {
        // A relative pointer that resolves to no existing store under either base
        // must return None so detection falls through to git / canonicalize. A
        // single-component pointer (no `..`) cannot escape the temp dir, so it
        // resolves to a missing path regardless of how deep the temp dir is.
        let temp_dir = TempDir::new().unwrap();
        write_jj_repo_file(&temp_dir, "nonexistent-store/.jj/repo");

        assert!(
            try_jj_workspace_root(temp_dir.path()).is_none(),
            "Relative pointer resolving to no existing store should return None"
        );
    }

    #[test]
    fn test_workspace_root_secondary_workspace() {
        // The two detectors must disagree in a secondary workspace — the repository root follows
        // the pointer to the main store while the workspace root stays at the directory holding
        // the pointer file — and coincide in the main workspace.
        let temp_base = TempDir::new().unwrap();
        let (workspace_dir, pointer_file, store_root) =
            build_jj_store_and_workspace(temp_base.path(), "mainrepo", "workspace");
        let absolute_store = store_root.join(".jj").join("repo");
        std::fs::write(&pointer_file, absolute_store.to_str().unwrap()).unwrap();

        let nested = workspace_dir.join("src").join("inner");
        std::fs::create_dir_all(&nested).unwrap();

        let workspace_root = detect_workspace_root(&nested)
            .expect("workspace root should resolve in a secondary workspace");
        assert_eq!(workspace_root, workspace_dir.canonicalize().unwrap());

        let repository_root = detect_repository_root(&nested)
            .expect("repository root should resolve through the pointer");
        assert_eq!(repository_root, store_root);
        assert_ne!(workspace_root, repository_root);

        assert_eq!(
            detect_workspace_root(&store_root).unwrap(),
            store_root,
            "main workspace: the two roots should coincide"
        );
    }

    #[test]
    fn test_workspace_root_git_worktree() {
        // The git analogue of the secondary-workspace divergence: the
        // repository root follows --git-common-dir back to the main
        // repository, while the workspace root stays at the worktree.
        let temp_base = TempDir::new().unwrap();
        let main = temp_base.path().join("main");
        std::fs::create_dir_all(&main).unwrap();
        setup_git_repo_with_commit(&main);

        let worktree = temp_base.path().join("worktree");
        run_git_command(&["worktree", "add", worktree.to_str().unwrap()], &main);

        let nested = worktree.join("src");
        std::fs::create_dir_all(&nested).unwrap();

        let workspace_root = detect_workspace_root(&nested)
            .expect("workspace root should resolve inside a git worktree");
        assert_eq!(workspace_root, worktree.canonicalize().unwrap());

        let repository_root = detect_repository_root(&nested)
            .expect("repository root should resolve to the main repository");
        assert_eq!(repository_root, main.canonicalize().unwrap());
        assert_ne!(workspace_root, repository_root);
    }

    #[test]
    fn test_git_worktree_root_not_in_repo() {
        let temp_dir = env::temp_dir();
        let result = try_git_worktree_root(&temp_dir);
        assert!(
            result.is_none(),
            "Temp directory should not be detected as a git worktree: {:?}",
            result
        );
    }

    #[test]
    fn test_workspace_root_not_in_repository() -> miette::Result<()> {
        let temp_dir = env::temp_dir();
        let root = detect_workspace_root(&temp_dir)?;
        assert_eq!(root, temp_dir.canonicalize().into_diagnostic()?);
        Ok(())
    }

    #[test]
    fn test_workspace_root_nonexistent_directory() {
        let fake_path = Path::new("/this/path/definitely/does/not/exist/moriarty-test-12345");
        let err_msg = format!("{:?}", detect_workspace_root(fake_path).unwrap_err());
        assert!(
            err_msg.contains("Failed to canonicalize") || err_msg.contains("No such file"),
            "Error should mention canonicalization or file not found failure, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_jj_main_workspace_dir_store() {
        // Main workspace: `.jj/repo` is the store directory itself. Detection
        // must return the workspace root directly (and from nested
        // subdirectories by walking up), not rely on the canonicalize fallback
        // that only happens to work when the input path is already the root.
        let temp_base = TempDir::new().unwrap();
        let repo_dir = temp_base.path().join("repo");
        let store_dir = repo_dir.join(".jj").join("repo");
        std::fs::create_dir_all(&store_dir).unwrap();
        let expected_root = repo_dir.canonicalize().unwrap();

        let from_root = try_jj_workspace_root(&repo_dir)
            .expect("main workspace root should resolve from the .jj/repo store directory");
        assert_eq!(from_root, expected_root);

        let nested = repo_dir.join("src").join("inner");
        std::fs::create_dir_all(&nested).unwrap();
        let from_nested = try_jj_workspace_root(&nested)
            .expect("nested path should walk up to the main workspace root");
        assert_eq!(
            from_nested, expected_root,
            "Walking up from a subdirectory should reach the same repository root"
        );
    }
}
