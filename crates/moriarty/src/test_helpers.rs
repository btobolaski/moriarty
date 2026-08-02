//! Shared test helper functions used across multiple test modules.
//!
//! This module is only compiled in test builds.

use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::Command,
};

use tempfile::TempDir;
#[cfg(unix)]
use which::which;

// ---------------------------------------------------------------------------
// Centralized unsafe environment mutation — one unsafe block for the crate
// ---------------------------------------------------------------------------

fn apply_test_env_var(key: &OsStr, value: Option<&OsStr>) {
    // SAFETY: This crate's tests must be run with `cargo nextest`, not
    // `cargo test`. nextest executes each test in a separate process, so these
    // process-global environment mutations cannot race with other tests in the
    // same process. See CLAUDE.md and README.md for the project test contract.
    unsafe {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}

/// Set a process-global environment variable during a test.
///
/// Only safe under `cargo nextest`, which isolates each test in its own process.
pub fn set_test_env_var<K, V>(key: K, value: V)
where
    K: AsRef<OsStr>,
    V: AsRef<OsStr>,
{
    apply_test_env_var(key.as_ref(), Some(value.as_ref()));
}

/// Remove a process-global environment variable during a test.
///
/// Only safe under `cargo nextest`, which isolates each test in its own process.
pub fn remove_test_env_var<K>(key: K)
where
    K: AsRef<OsStr>,
{
    apply_test_env_var(key.as_ref(), None);
}

/// A guard that sets an environment variable and restores the previous value on drop.
///
/// Use this for variables such as `RUST_LOG` or `HOME` where the test should
/// restore the developer's original value rather than leaving the variable set
/// or removed.
pub struct TestEnvVarGuard {
    key: OsString,
    original: Option<OsString>,
}

impl TestEnvVarGuard {
    /// Set `key` to `value` and save the previous value for later restoration.
    pub fn set<K, V>(key: K, value: V) -> Self
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        let key = key.as_ref().to_os_string();
        let original = std::env::var_os(&key);
        set_test_env_var(&key, value);
        Self { key, original }
    }

    /// Remove `key` and save the previous value for later restoration.
    pub fn unset<K>(key: K) -> Self
    where
        K: AsRef<OsStr>,
    {
        let key = key.as_ref().to_os_string();
        let original = std::env::var_os(&key);
        remove_test_env_var(&key);
        Self { key, original }
    }
}

impl Drop for TestEnvVarGuard {
    fn drop(&mut self) {
        match self.original.as_ref() {
            Some(value) => set_test_env_var(&self.key, value),
            None => remove_test_env_var(&self.key),
        }
    }
}

// ---------------------------------------------------------------------------
// Semantic helpers that use the centralized primitives
// ---------------------------------------------------------------------------

/// Create a temporary directory with XDG_CONFIG_HOME set to it.
///
/// The returned `TempDir` must be kept alive for the test's duration.
pub fn setup_isolated_xdg_config() -> TempDir {
    let temp_dir = tempfile::tempdir().unwrap();
    set_test_env_var("XDG_CONFIG_HOME", temp_dir.path());
    temp_dir
}

/// Create a temporary directory with XDG_STATE_HOME set to it.
///
/// The returned `TempDir` must be kept alive for the test's duration.
pub fn setup_isolated_xdg_state() -> TempDir {
    let temp_dir = tempfile::tempdir().unwrap();
    set_test_env_var("XDG_STATE_HOME", temp_dir.path());
    temp_dir
}

/// Create a temporary project directory with a `.config/tools.toml` file.
pub fn setup_project_dir_with_config(config_content: &str) -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    write_tools_config(temp_dir.path(), config_content);
    temp_dir
}

/// Writes `contents` to `<project_dir>/.config/tools.toml`, creating `.config` if needed.
///
/// Returns the path of the written `tools.toml` file.
pub fn write_tools_config(project_dir: &Path, contents: &str) -> PathBuf {
    let config_dir = project_dir.join(".config");
    if !config_dir.exists() {
        std::fs::create_dir(&config_dir).unwrap();
    }
    let path = config_dir.join("tools.toml");
    std::fs::write(&path, contents).unwrap();
    path
}

/// Fixture bodies must remain POSIX `sh` compatible so they behave consistently when `sh` is not
/// Bash. On Unix, the interpreter is resolved from `PATH` because Nix environments need not expose
/// FHS paths such as `/bin/bash`.
pub fn create_executable_script(path: &Path, body: &str) {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent).unwrap();
    }
    #[cfg(unix)]
    let shebang = which("sh")
        .expect("test environment must provide sh in PATH")
        .to_string_lossy()
        .into_owned();
    #[cfg(not(unix))]
    let shebang = "/bin/bash";
    let contents = format!("#!{shebang}\n{body}\n");
    std::fs::write(path, contents).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// Lay out a jj main workspace plus a secondary workspace whose `.jj/repo` pointer file targets
/// the main store, with `config` written to the main workspace's `.config/tools.toml` and a
/// deliberately divergent `./<script>` in each working copy: the main (approvable) copy prints
/// the markers [`assert_approved_copy_ran_in`] checks for — "approved-copy" plus its physical
/// cwd — while the workspace copy prints "unapproved-workspace-copy".
///
/// Returns `(base_guard, main_dir, workspace_dir)`; the guard must outlive the test. Approval is
/// left to the caller and must come after any additional files, since it hashes the referenced
/// binaries.
pub fn setup_jj_main_and_secondary_workspace(
    config: &str,
    script: &str,
) -> (TempDir, PathBuf, PathBuf) {
    let base = TempDir::new().unwrap();

    let main = base.path().join("main");
    std::fs::create_dir_all(main.join(".jj/repo")).unwrap();
    std::fs::create_dir_all(main.join(".config")).unwrap();
    std::fs::write(main.join(".config/tools.toml"), config).unwrap();
    create_executable_script(&main.join(script), "echo approved-copy\npwd -P");

    let workspace = base.path().join("workspace");
    std::fs::create_dir_all(workspace.join(".jj")).unwrap();
    std::fs::write(
        workspace.join(".jj/repo"),
        main.join(".jj/repo").to_str().unwrap(),
    )
    .unwrap();
    create_executable_script(&workspace.join(script), "echo unapproved-workspace-copy");

    (base, main, workspace)
}

/// Assert `output` proves the approved main-workspace script from
/// [`setup_jj_main_and_secondary_workspace`] ran — not the workspace's shadow copy — with the
/// secondary workspace as its physical working directory.
pub fn assert_approved_copy_ran_in(output: &str, main: &Path, workspace: &Path) {
    let workspace_root = workspace.canonicalize().unwrap();
    let main_root = main.canonicalize().unwrap();
    assert!(
        output.contains("approved-copy"),
        "the hashed repository copy must run, got: {output}"
    );
    assert!(
        !output.contains("unapproved-workspace-copy"),
        "the workspace copy must not shadow the approved one, got: {output}"
    );
    assert!(
        output.contains(workspace_root.to_str().unwrap()),
        "cwd must be the secondary workspace, got: {output}"
    );
    assert!(
        !output.contains(main_root.to_str().unwrap()),
        "cwd must not be the main workspace, got: {output}"
    );
}

/// Run a git command in `current_dir`, panicking with git's stderr on failure.
pub fn run_git_command(args: &[&str], current_dir: &Path) {
    let output = Command::new("git")
        .args(args)
        .current_dir(current_dir)
        .output()
        .expect("Failed to execute git command");

    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Create a git repository at `repo_path` with one initial commit (committing
/// any files already present there).
///
/// Configures a local user identity rather than relying on global config,
/// which may be absent in CI environments or isolated test setups.
pub fn setup_git_repo_with_commit(repo_path: &Path) {
    run_git_command(&["init"], repo_path);
    run_git_command(&["config", "user.email", "test@example.com"], repo_path);
    run_git_command(&["config", "user.name", "Test User"], repo_path);
    std::fs::write(repo_path.join("README.md"), "test").unwrap();
    run_git_command(&["add", "."], repo_path);
    run_git_command(&["commit", "-m", "initial"], repo_path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_var_guard_set_restores_existing_value() {
        let key = "MORIARTY_TEST_ENV_GUARD_SET_RESTORE";
        set_test_env_var(key, "original");

        {
            let _guard = TestEnvVarGuard::set(key, "temporary");
            assert_eq!(std::env::var_os(key), Some(OsString::from("temporary")));
        }

        assert_eq!(std::env::var_os(key), Some(OsString::from("original")));
        remove_test_env_var(key);
    }

    #[test]
    fn test_env_var_guard_set_removes_when_originally_absent() {
        let key = "MORIARTY_TEST_ENV_GUARD_SET_ABSENT";
        remove_test_env_var(key);

        {
            let _guard = TestEnvVarGuard::set(key, "temporary");
            assert_eq!(std::env::var_os(key), Some(OsString::from("temporary")));
        }

        assert_eq!(std::env::var_os(key), None);
    }

    #[test]
    fn test_env_var_guard_unset_restores_existing_value() {
        let key = "MORIARTY_TEST_ENV_GUARD_UNSET_RESTORE";
        set_test_env_var(key, "original");

        {
            let _guard = TestEnvVarGuard::unset(key);
            assert_eq!(std::env::var_os(key), None);
        }

        assert_eq!(std::env::var_os(key), Some(OsString::from("original")));
        remove_test_env_var(key);
    }

    #[test]
    fn test_env_var_guard_unset_preserves_absence() {
        let key = "MORIARTY_TEST_ENV_GUARD_UNSET_ABSENT";
        remove_test_env_var(key);

        {
            let _guard = TestEnvVarGuard::unset(key);
            assert_eq!(std::env::var_os(key), None);
        }

        assert_eq!(std::env::var_os(key), None);
    }
}
