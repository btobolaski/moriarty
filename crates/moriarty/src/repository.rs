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
    env,
    path::{Path, PathBuf},
    str,
};

#[cfg(test)]
use std::fs;

use miette::{IntoDiagnostic, WrapErr};
use tracing::{debug, info};
use which::{Error as WhichError, which_in};

use crate::execution;
use process_supervisor::{ProcessReport, ProcessSpec};

/// Workspace identity must not silently substitute for the shared approval key.
/// UTF-8 storage enforces representability without repeat preflights at lookup/save boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRoot(String);

impl WorkspaceRoot {
    fn new(path: PathBuf) -> miette::Result<Self> {
        path.into_os_string().into_string().map(Self).map_err(|_| {
            execution::interrupted(
                "Non-UTF-8 filesystem identity cannot be represented by the approval format",
            )
        })
    }

    /// Borrows the validated working-copy root for path operations. Keep the typed identity at
    /// approval boundaries so a workspace path cannot stand in for the shared repository root.
    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }
}

/// Shared approval identity must not silently substitute for an execution workspace.
/// A String cannot carry an unrepresentable key or require lossy conversion at approval lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryRoot(String);

impl RepositoryRoot {
    fn new(path: PathBuf) -> miette::Result<Self> {
        path.into_os_string().into_string().map(Self).map_err(|_| {
            execution::interrupted(
                "Non-UTF-8 filesystem identity cannot be represented by the approval format",
            )
        })
    }

    /// Borrows the validated shared root for path operations. Keep the typed identity at approval
    /// boundaries rather than letting a working-copy path become an approval-storage key.
    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }

    pub(crate) fn as_key(&self) -> &str {
        &self.0
    }
}

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
/// # async fn example() -> miette::Result<()> {
/// let root = detect_repository_root(Path::new("/workspace/src/project")).await?;
/// // In a jj workspace: /workspace/src
/// // In a git repo: /workspace
/// // Not in repo: /workspace/src/project (canonicalized)
/// # Ok(())
/// # }
/// ```
pub async fn detect_repository_root(path: &Path) -> miette::Result<RepositoryRoot> {
    info!(path = %path.display(), "Detecting repository root");
    let canonical = path
        .canonicalize()
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to canonicalize path: {}", path.display()));

    // A missing descendant can still identify its jj store; only git needs a canonical cwd.
    if let Some(jj_root) = try_jj_workspace_root(canonical.as_deref().unwrap_or(path)) {
        info!(root = %jj_root.display(), "Detected jj workspace root");
        return RepositoryRoot::new(jj_root);
    }
    let canonical = canonical?;

    if let Some(git_root) = try_git_repository_root(&canonical).await? {
        info!(root = %git_root.display(), "Detected git repository root");
        return RepositoryRoot::new(git_root);
    }

    info!(
        root = %canonical.display(),
        "Not in a repository, using canonicalized path as root"
    );
    RepositoryRoot::new(canonical)
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
pub async fn detect_workspace_root(path: &Path) -> miette::Result<WorkspaceRoot> {
    info!(path = %path.display(), "Detecting workspace root");

    let canonical = path
        .canonicalize()
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to canonicalize path: {}", path.display()))?;

    if let Some(root) = nearest_jj_workspace_dir(&canonical) {
        info!(root = %root.display(), "Detected jj workspace directory");
        return WorkspaceRoot::new(root);
    }

    if let Some(root) = try_git_worktree_root(&canonical).await? {
        info!(root = %root.display(), "Detected git worktree root");
        return WorkspaceRoot::new(root);
    }

    info!(
        root = %canonical.display(),
        "Not in a repository, using canonicalized path as workspace root"
    );
    WorkspaceRoot::new(canonical)
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
async fn try_git_worktree_root(path: &Path) -> miette::Result<Option<PathBuf>> {
    let Some(output) = git_probe(path, "--show-toplevel").await? else {
        return Ok(None);
    };
    let toplevel = git_root_path(&output)?;
    toplevel.canonicalize().map(Some).map_err(|error| {
        execution::interrupted(format!("Failed to resolve Git workspace root: {error}"))
    })
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
async fn try_git_repository_root(path: &Path) -> miette::Result<Option<PathBuf>> {
    let Some(output) = git_probe(path, "--git-common-dir").await? else {
        return Ok(None);
    };
    let common = git_root_path(&output)?;
    let common = if common.is_absolute() {
        common
    } else {
        path.join(common)
    };
    let common = common.canonicalize().map_err(|error| {
        execution::interrupted(format!("Failed to resolve Git common directory: {error}"))
    })?;
    common
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| execution::interrupted("Git common directory has no repository parent"))
        .map(Some)
}

fn git_root_path(output: &ProcessReport) -> miette::Result<PathBuf> {
    let bytes = output.stdout.bytes.strip_suffix(b"\n").ok_or_else(|| {
        execution::interrupted("Git root output is missing its newline terminator")
    })?;
    let text = str::from_utf8(bytes)
        .map_err(|error| execution::interrupted(format!("Non-UTF-8 Git root identity: {error}")))?;
    if text.is_empty() {
        return Err(execution::interrupted(
            "Git returned an empty root identity",
        ));
    }
    Ok(text.into())
}

// The entry point owns the canonical cwd; revalidating here would observe later filesystem state.
// Ordinary absence may fall back; incomplete execution must not invent an approval key.
async fn git_probe(canonical: &Path, flag: &str) -> miette::Result<Option<ProcessReport>> {
    let program = match env::var_os("PATH") {
        Some(paths) => {
            let mut program = None;
            // which checks relative PATH entries against the host cwd; native launch uses the child cwd.
            for directory in env::split_paths(&paths) {
                let candidate = canonical.join(directory).join("git");
                match which_in(candidate, Some(""), canonical) {
                    Ok(found) => {
                        program = Some(found);
                        break;
                    }
                    Err(WhichError::CannotFindBinaryPath) => {}
                    Err(error) => {
                        return Err(execution::interrupted(format!(
                            "Failed to locate Git: {error}"
                        )));
                    }
                }
            }
            let Some(program) = program else {
                return Ok(None);
            };
            program
        }
        // libc owns unset-PATH defaults; guessing them would diverge across platforms/builds.
        None => PathBuf::from("git"),
    };
    // A spawn NotFound can mean the recorded cwd vanished, not that Git is unavailable.
    let output = execution::execute(ProcessSpec {
        program: program.into(),
        args: vec!["rev-parse".into(), flag.into()],
        cwd: Some(canonical.to_path_buf()),
    })
    .await?;
    let status = execution::complete_output(&output)?;
    // Pin both English absence forms; unknown wording/locale still cannot grant another root.
    if status.success() {
        Ok(Some(output))
    } else if status.code() == Some(128)
        && output.stdout.bytes.is_empty()
        && (output.stderr.bytes == b"fatal: not a git repository (or any of the parent directories): .git\n"
            || output.stderr.bytes
                .strip_prefix(b"fatal: not a git repository (or any parent up to mount point ")
                .and_then(|rest| rest.strip_suffix(b")\nStopping at filesystem boundary (GIT_DISCOVERY_ACROSS_FILESYSTEM not set).\n"))
                .is_some_and(|mount| mount.starts_with(b"/")))
    {
        Ok(None)
    } else {
        Err(execution::interrupted(format!(
            "Git root discovery ({flag}) failed with {status}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        ffi::OsString,
        future::ready,
        os::unix::{ffi::OsStringExt, fs::symlink, process::ExitStatusExt},
        process::ExitStatus,
    };

    use rustix::io::Errno;
    use tempfile::TempDir;

    use super::*;
    use crate::{
        project_config::{approvals::ProjectApprovals, config::ProjectConfig},
        test_helpers::{TestEnvVarGuard, run_git_command, setup_git_repo_with_commit, test_report},
    };
    use process_supervisor::{Completion, ProcessError, ProcessOptions, RunningProcess};

    impl WorkspaceRoot {
        pub(crate) fn for_test(path: PathBuf) -> Self {
            Self::new(path).expect("test root must be UTF-8")
        }
    }

    #[tokio::test]
    async fn whitespace_root_identity_survives_nested_discovery() {
        let directory = TempDir::new().unwrap();
        for name in [" leading and trailing ", "newline\n"] {
            let root = directory.path().join(name);
            fs::create_dir(&root).unwrap();
            setup_git_repo_with_commit(&root);
            let nested = root.join("nested");
            fs::create_dir(&nested).unwrap();
            let expected = root.canonicalize().unwrap();
            assert_eq!(
                detect_workspace_root(&nested).await.unwrap().as_path(),
                expected
            );
            assert_eq!(
                detect_repository_root(&nested).await.unwrap().as_path(),
                expected
            );
        }
    }

    #[tokio::test]
    async fn non_utf8_roots_never_share_approval_keys() {
        let directory = TempDir::new().unwrap();
        let first = directory
            .path()
            .join(OsString::from_vec(b"project\xff".to_vec()));
        let second = directory
            .path()
            .join(OsString::from_vec(b"project\xfe".to_vec()));
        assert_ne!(first, second);
        assert_eq!(first.to_string_lossy(), second.to_string_lossy());
        let config: ProjectConfig =
            toml::from_str("[commands]\nlint = [\"echo\", \"ok\"]").unwrap();
        let mut approvals = ProjectApprovals::default();
        // Even legacy replacement-character keys must not be consulted by byte-valued callers.
        approvals
            .projects
            .entry(first.to_string_lossy().into_owned())
            .or_default();
        for root in [&first, &second] {
            match fs::create_dir(root) {
                Ok(()) => {
                    for repository in [false, true] {
                        let error = if repository {
                            detect_repository_root(root).await.map(|_| ())
                        } else {
                            detect_workspace_root(root).await.map(|_| ())
                        }
                        .expect_err("unrepresentable roots must fail discovery");
                        assert!(execution::is_execution_error(&error));
                        assert!(error.to_string().contains("Non-UTF-8"), "{error}");
                    }
                }
                Err(error) => {
                    // Darwin filesystems may reject these names before discovery can run;
                    // the filesystem-independent identity/approval checks still run below.
                    assert!(
                        cfg!(target_os = "macos")
                            && error.raw_os_error() == Some(Errno::ILSEQ.raw_os_error()),
                        "{error}"
                    );
                }
            }
            for (repository, workspace) in [
                (root.as_path(), directory.path()),
                (directory.path(), root.as_path()),
            ] {
                let roots = WorkspaceRoot::new(workspace.into()).and_then(|workspace| {
                    RepositoryRoot::new(repository.into()).map(|repository| (repository, workspace))
                });
                let error = match roots {
                    Ok((repository, workspace)) => {
                        approvals
                            .verify_project_for_workspace(&repository, &workspace, &config, "lint")
                            .await
                    }
                    Err(error) => Err(error),
                }
                .expect_err(
                    "construction must reject either unrepresentable identity before lookup",
                );
                assert!(error.to_string().contains("Non-UTF-8"), "{error}");
                let error = WorkspaceRoot::new(workspace.into())
                    .and_then(|workspace| {
                        RepositoryRoot::new(repository.into()).and_then(|repository| {
                            approvals.approve_project(
                                workspace,
                                repository,
                                Default::default(),
                                Default::default(),
                            )
                        })
                    })
                    .expect_err(
                        "construction must reject either unrepresentable identity before storage",
                    );
                assert!(error.to_string().contains("Non-UTF-8"), "{error}");
                assert_eq!(
                    approvals.projects.len(),
                    1,
                    "rejection must not mutate storage"
                );
            }
        }
        let mut report = test_report(Completion::Exited(ExitStatus::from_raw(0)));
        report.stdout.bytes = first.as_os_str().as_encoded_bytes().to_vec();
        report.stdout.bytes.push(b'\n');
        fs::create_dir(first.to_string_lossy().as_ref()).unwrap();
        let error =
            execution::test_seams::with_result(Ok(report), detect_workspace_root(directory.path()))
                .await
                .expect_err(
                    "a real replacement-character directory cannot substitute for byte output",
                );
        assert!(execution::is_execution_error(&error));
        assert!(
            error.to_string().contains("Non-UTF-8 Git root identity"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn incomplete_probes_never_invent_roots() {
        let directory = TempDir::new().unwrap();
        let mut truncated = test_report(Completion::Exited(ExitStatus::from_raw(0)));
        truncated.stdout.bytes = b"/guessed/root".to_vec();
        truncated.stdout.truncated = true;
        let cancelled = test_report(Completion::Cancelled(ExitStatus::from_raw(0)));
        let startup = Err("Startup interrupted; launch and cleanup unconfirmed");
        for (outcome, repository, expected) in [
            (Ok(truncated), true, "Incomplete process output"),
            (Ok(cancelled), false, "cancelled"),
            (startup, true, "cleanup unconfirmed"),
        ] {
            let discovery = async {
                if repository {
                    detect_repository_root(directory.path()).await.map(|_| ())
                } else {
                    detect_workspace_root(directory.path()).await.map(|_| ())
                }
            };
            let error = execution::test_seams::with_result(outcome, discovery)
                .await
                .unwrap_err();
            assert!(error.to_string().contains(expected), "{expected}: {error}");
        }
    }

    #[tokio::test]
    async fn unexpected_git_failures_never_invent_roots() {
        let directory = TempDir::new().unwrap();
        setup_git_repo_with_commit(directory.path());
        fs::write(directory.path().join(".git/config"), "[broken\n").unwrap();
        let nested = directory.path().join("nested");
        fs::create_dir(&nested).unwrap();
        for repository in [false, true] {
            let error = if repository {
                detect_repository_root(&nested).await.map(|_| ())
            } else {
                detect_workspace_root(&nested).await.map(|_| ())
            }
            .expect_err("corrupt Git config must not be ordinary absence");
            assert!(execution::is_execution_error(&error));
            assert!(error.to_string().contains("Git root discovery"), "{error}");
        }
        for flag in ["--show-toplevel", "--git-common-dir"] {
            let mut report = test_report(Completion::Exited(ExitStatus::from_raw(0)));
            report.stdout.bytes =
                format!("{}\n", directory.path().join("missing-root").display()).into_bytes();
            let error = execution::test_seams::with_result(Ok(report), async {
                if flag == "--show-toplevel" {
                    try_git_worktree_root(directory.path()).await
                } else {
                    try_git_repository_root(directory.path()).await
                }
            })
            .await
            .expect_err("unresolvable roots must fail");
            assert!(execution::is_execution_error(&error));
            assert!(
                error.to_string().contains("Failed to resolve Git"),
                "{error}"
            );
        }
    }

    #[tokio::test]
    async fn vanished_launch_cwd_is_not_missing_git() {
        let directory = TempDir::new().unwrap();
        let handler = |spec: ProcessSpec, _| async move {
            assert!(Path::new(&spec.program).is_file(), "Git is available");
            fs::remove_dir(spec.cwd.as_ref().unwrap()).unwrap();
            let running = RunningProcess::spawn(spec, ProcessOptions::default()).await?;
            running.wait().await
        };
        let error = execution::test_seams::with_operations(
            handler,
            detect_workspace_root(directory.path()),
        )
        .await
        .expect_err("vanished cwd must be an execution failure");
        assert!(error.downcast_ref::<ProcessError>().is_some());
        assert!(execution::is_execution_error(&error));
        assert!(
            error.to_string().contains("process start failed"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn canonical_input_survives_alias_changes_during_probe() {
        // Each entry point owns canonicalization independently; either caller can regress before the shared probe.
        for repository in [false, true] {
            let directory = TempDir::new().unwrap();
            let first = directory.path().join("first");
            let second = directory.path().join("second");
            let alias = directory.path().join("alias");
            fs::create_dir(&first).unwrap();
            fs::create_dir(&second).unwrap();
            symlink(&first, &alias).unwrap();
            let canonical = first.canonicalize().unwrap();
            let expected = canonical.clone();
            let changed_alias = alias.clone();
            let handler = move |spec: ProcessSpec, _| {
                assert_eq!(spec.cwd.as_ref(), Some(&expected));
                fs::remove_file(&changed_alias).unwrap();
                symlink(&second, &changed_alias).unwrap();
                let mut report = test_report(Completion::Exited(ExitStatus::from_raw(128 << 8)));
                report.stderr.bytes =
                    b"fatal: not a git repository (or any of the parent directories): .git\n"
                        .to_vec();
                ready(Ok(report))
            };
            let discovery = async {
                if repository {
                    detect_repository_root(&alias)
                        .await
                        .map(|root| root.as_path().to_path_buf())
                } else {
                    detect_workspace_root(&alias)
                        .await
                        .map(|root| root.as_path().to_path_buf())
                }
            };
            let root = execution::test_seams::with_operations(handler, discovery)
                .await
                .unwrap();
            assert_eq!(
                root, canonical,
                "fallback must use the same canonical input as the probe"
            );
        }
    }

    #[tokio::test]
    async fn missing_git_preserves_canonical_fallback() {
        let directory = TempDir::new().unwrap();
        let _path = TestEnvVarGuard::set("PATH", "");
        let expected = directory.path().canonicalize().unwrap();
        assert_eq!(
            detect_repository_root(directory.path())
                .await
                .unwrap()
                .as_path(),
            expected
        );
        assert_eq!(
            detect_workspace_root(directory.path())
                .await
                .unwrap()
                .as_path(),
            expected
        );
    }

    #[tokio::test]
    async fn test_detect_non_repository() -> miette::Result<()> {
        // Use a temporary directory that's definitely not in a repo
        let temp_dir = env::temp_dir();
        let root = detect_repository_root(&temp_dir).await?;

        // Should return canonicalized temp dir
        assert_eq!(root.as_path(), temp_dir.canonicalize().into_diagnostic()?);
        Ok(())
    }

    #[tokio::test]
    async fn test_detect_git_repository() -> miette::Result<()> {
        // Test with current directory (which should be in git or jj)
        let current_dir = env::current_dir().into_diagnostic()?;
        let root = detect_repository_root(&current_dir).await?;

        // Root should be an absolute path
        // Note: For jj workspaces, root may not be a parent of current_dir
        // since .jj/repo points to the actual repository location
        assert!(
            root.as_path().is_absolute(),
            "Repository root should be absolute path, got: {}",
            root.as_path().display()
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

    #[tokio::test]
    async fn test_git_repository_root_not_in_repo() {
        let temp_dir = env::temp_dir();
        let result = try_git_repository_root(&temp_dir).await.unwrap();

        // Temp dir should not be in a git repository
        // If this fails, the test environment is unusual (temp inside a repo)
        assert!(
            result.is_none(),
            "Temp directory should not be detected as git repository: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_detect_repository_root_nonexistent_directory() {
        let fake_path = Path::new("/this/path/definitely/does/not/exist/moriarty-test-12345");
        let result = detect_repository_root(fake_path).await;

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

    #[tokio::test]
    async fn test_workspace_root_secondary_workspace() {
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
            .await
            .expect("workspace root should resolve in a secondary workspace");
        assert_eq!(
            workspace_root.as_path(),
            workspace_dir.canonicalize().unwrap()
        );

        let repository_root = detect_repository_root(&nested)
            .await
            .expect("repository root should resolve through the pointer");
        assert_eq!(repository_root.as_path(), store_root);
        assert_ne!(workspace_root.as_path(), repository_root.as_path());

        assert_eq!(
            detect_workspace_root(&store_root).await.unwrap().as_path(),
            store_root,
            "main workspace: the two roots should coincide"
        );
    }

    #[tokio::test]
    async fn test_workspace_root_git_worktree() {
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
            .await
            .expect("workspace root should resolve inside a git worktree");
        assert_eq!(workspace_root.as_path(), worktree.canonicalize().unwrap());

        let repository_root = detect_repository_root(&nested)
            .await
            .expect("repository root should resolve to the main repository");
        assert_eq!(repository_root.as_path(), main.canonicalize().unwrap());
        assert_ne!(workspace_root.as_path(), repository_root.as_path());
    }

    #[tokio::test]
    async fn test_git_worktree_root_not_in_repo() {
        let temp_dir = env::temp_dir();
        let result = try_git_worktree_root(&temp_dir).await.unwrap();
        assert!(
            result.is_none(),
            "Temp directory should not be detected as a git worktree: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_workspace_root_not_in_repository() -> miette::Result<()> {
        let temp_dir = env::temp_dir();
        let root = detect_workspace_root(&temp_dir).await?;
        assert_eq!(root.as_path(), temp_dir.canonicalize().into_diagnostic()?);
        Ok(())
    }

    #[tokio::test]
    async fn test_workspace_root_nonexistent_directory() {
        let fake_path = Path::new("/this/path/definitely/does/not/exist/moriarty-test-12345");
        let err_msg = format!("{:?}", detect_workspace_root(fake_path).await.unwrap_err());
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
