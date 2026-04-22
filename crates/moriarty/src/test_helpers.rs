//! Shared test helper functions used across multiple test modules.
//!
//! This module is only compiled in test builds.

use std::path::{Path, PathBuf};

use tempfile::TempDir;

/// Create a temporary directory with XDG_CONFIG_HOME set to it.
///
/// Safe to use `std::env::set_var` because cargo nextest isolates each test in
/// a separate process.
///
/// **IMPORTANT**: The returned `TempDir` must be kept alive (bound to a variable)
/// for the duration of the test. `TempDir` deletes its directory when dropped.
pub fn setup_isolated_xdg_config() -> TempDir {
    let temp_dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", temp_dir.path());
    temp_dir
}

/// Create a temporary directory with XDG_STATE_HOME set to it.
pub fn setup_isolated_xdg_state() -> TempDir {
    let temp_dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_STATE_HOME", temp_dir.path());
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
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent).unwrap();
        }
    }
    let contents = format!("#!/bin/bash\n{}\n", body);
    std::fs::write(path, contents).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}
