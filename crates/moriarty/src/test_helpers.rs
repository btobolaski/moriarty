//! Shared test helper functions used across multiple test modules.
//!
//! This module is only compiled in test builds.

use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

use tempfile::TempDir;

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

/// Create an executable shell script at `path` containing `body`.
///
/// The file begins with a `#!/bin/bash` shebang and, on Unix, is marked as
/// executable (mode 0o755). Parent directories are created as needed.
pub fn create_executable_script(path: &Path, body: &str) {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent).unwrap();
    }
    let contents = format!("#!/bin/bash\n{}\n", body);
    std::fs::write(path, contents).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
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
