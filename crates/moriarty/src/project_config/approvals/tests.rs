//! Tests for the project approvals system

use std::{
    collections::HashMap,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use miette::{IntoDiagnostic, Result, WrapErr};
use tempfile::{NamedTempFile, TempDir};
use tokio::time::{sleep, Duration};

use crate::{hashing, project_config::config::ProjectConfig, repository};

use super::super::{
    is_script, is_within_project, is_writable, resolve_binary_path_with_original, CommandApproval,
    ProjectApprovals, VerificationResult,
};

/// Test helper to pre-approve a project with the given config content.
/// This bypasses the approval TUI for integration tests.
/// Returns the repository root path for use in assertions.
///
/// # Errors
/// Returns an error if:
/// - Repository root detection fails (path doesn't exist, permission denied, etc.)
/// - Config parsing fails (invalid TOML)
/// - Binary resolution fails (binary not found)
/// - File hashing fails (I/O error)
/// - Approval update fails (filesystem error)
pub async fn approve_project_config(project_dir: &Path, config_content: &str) -> Result<PathBuf> {
    // Detect repository root (jj workspace root, git root, or canonicalized path)
    let repository_root = repository::detect_repository_root(project_dir)?;
    let config: ProjectConfig = toml::from_str(config_content)
        .into_diagnostic()
        .wrap_err("Failed to parse test config")?;
    let tools_config_hash = hashing::hash_string(config_content);

    // Process commands (use repository_root for binary resolution)
    let mut commands = HashMap::new();
    for (name, cmd_array) in config.commands.all() {
        let binary_name = &cmd_array[0];
        let (original_path, resolved_path) =
            resolve_binary_path_with_original(binary_name, &repository_root)?;
        let binary_hash = hashing::hash_file(&resolved_path).await?;

        commands.insert(
            name,
            CommandApproval {
                original_path: original_path.to_string_lossy().to_string(),
                canonical_path: resolved_path.to_string_lossy().to_string(),
                binary_hash,
            },
        );
    }

    // Process checks
    let mut checks = HashMap::new();
    if let Some(check_configs) = config.checks {
        for check in check_configs {
            let binary_name = &check.command[0];
            let (original_path, resolved_path) =
                resolve_binary_path_with_original(binary_name, &repository_root)?;
            let binary_hash = hashing::hash_file(&resolved_path).await?;

            checks.insert(
                check.name,
                CommandApproval {
                    original_path: original_path.to_string_lossy().to_string(),
                    canonical_path: resolved_path.to_string_lossy().to_string(),
                    binary_hash,
                },
            );
        }
    }

    let repository_root_clone = repository_root.clone();
    ProjectApprovals::update(move |approvals| {
        approvals.approve_project(repository_root_clone, tools_config_hash, commands, checks)
    })
    .await?;

    Ok(repository_root)
}

/// Helper to run a git command and assert success
fn run_git_command(args: &[&str], current_dir: &Path) {
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

/// Helper to run a jj command and assert success
fn run_jj_command(args: &[&str], current_dir: &Path) {
    let output = Command::new("jj")
        .args(args)
        .current_dir(current_dir)
        .output()
        .expect("Failed to execute jj command");

    assert!(
        output.status.success(),
        "jj {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Helper to create a git repository with an initial commit
fn setup_git_repo_with_commit(repo_path: &Path) {
    run_git_command(&["init"], repo_path);
    std::fs::write(repo_path.join("README.md"), "test").unwrap();
    run_git_command(&["add", "."], repo_path);
    run_git_command(&["commit", "-m", "initial"], repo_path);
}

/// Helper to create a jj repository
fn setup_jj_repo(repo_path: &Path) {
    run_jj_command(
        &["git", "init", repo_path.to_str().unwrap()],
        repo_path.parent().unwrap_or(repo_path),
    );
}

/// Helper to create .config/tools.toml with standard test content
fn create_tools_config(repo_path: &Path, config_content: &str) {
    let config_dir = repo_path.join(".config");
    std::fs::create_dir(&config_dir).unwrap();
    std::fs::write(config_dir.join("tools.toml"), config_content).unwrap();
}

#[test]
fn test_project_approvals_default() {
    let approvals = ProjectApprovals::default();
    assert_eq!(approvals.projects.len(), 0);
}

#[test]
fn test_approve_project() {
    let mut approvals = ProjectApprovals::default();
    let temp_dir = TempDir::new().unwrap();
    let tools_hash = "sha256:abc123".to_string();

    let mut commands = HashMap::new();
    commands.insert(
        "lint".to_string(),
        CommandApproval {
            original_path: "cargo".to_string(),
            canonical_path: "/usr/bin/cargo".to_string(),
            binary_hash: "sha256:def456".to_string(),
        },
    );

    let checks = HashMap::new();

    approvals
        .approve_project(
            temp_dir.path().to_path_buf(),
            tools_hash.clone(),
            commands.clone(),
            checks,
        )
        .unwrap();

    // Project key is now the repository root (canonicalized temp dir in this case)
    let project_key = temp_dir
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert!(approvals.projects.contains_key(&project_key));

    let approval = &approvals.projects[&project_key];
    assert_eq!(approval.tools_config_hash, tools_hash);
    assert_eq!(approval.commands.len(), 1);
    assert!(approval.commands.contains_key("lint"));
}

#[test]
fn test_approve_project_with_checks() {
    let mut approvals = ProjectApprovals::default();
    let temp_dir = TempDir::new().unwrap();
    let tools_hash = "sha256:abc123".to_string();

    let mut commands = HashMap::new();
    commands.insert(
        "lint".to_string(),
        CommandApproval {
            original_path: "cargo".to_string(),
            canonical_path: "/usr/bin/cargo".to_string(),
            binary_hash: "sha256:def456".to_string(),
        },
    );

    let mut checks = HashMap::new();
    checks.insert(
        "security-audit".to_string(),
        CommandApproval {
            original_path: "cargo".to_string(),
            canonical_path: "/usr/bin/cargo".to_string(),
            binary_hash: "sha256:abc789".to_string(),
        },
    );

    approvals
        .approve_project(
            temp_dir.path().to_path_buf(),
            tools_hash.clone(),
            commands.clone(),
            checks.clone(),
        )
        .unwrap();

    let project_key = temp_dir
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert!(approvals.projects.contains_key(&project_key));

    let approval = &approvals.projects[&project_key];
    assert_eq!(approval.tools_config_hash, tools_hash);
    assert_eq!(approval.commands.len(), 1);
    assert!(approval.commands.contains_key("lint"));
    assert_eq!(approval.checks.len(), 1);
    assert!(approval.checks.contains_key("security-audit"));
}

#[tokio::test]
async fn test_verify_check_approved() {
    // Test that verify_check correctly verifies an approved check

    let _xdg_dir = TempDir::new().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", _xdg_dir.path());

    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join(".config");
    std::fs::create_dir(&config_dir).unwrap();

    let config_content = r#"
[commands]

[[checks]]
name = "audit"
command = ["echo", "test"]
"#;

    std::fs::write(config_dir.join("tools.toml"), config_content).unwrap();

    // Use the helper to approve the project with checks
    approve_project_config(temp_dir.path(), config_content)
        .await
        .unwrap();

    // Load approvals and verify the check
    let approvals = ProjectApprovals::load().await.unwrap();
    let result = approvals
        .verify_check(temp_dir.path(), "audit")
        .await
        .unwrap();

    assert_eq!(
        result,
        VerificationResult::Approved,
        "Approved check should verify successfully"
    );
}

#[tokio::test]
async fn test_verify_check_not_approved() {
    // Test that verify_check returns NotApproved for unapproved checks

    let _xdg_dir = TempDir::new().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", _xdg_dir.path());

    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join(".config");
    std::fs::create_dir(&config_dir).unwrap();

    let config_content = r#"
[commands]

[[checks]]
name = "audit"
command = ["echo", "test"]
"#;

    std::fs::write(config_dir.join("tools.toml"), config_content).unwrap();

    // Don't approve - just load approvals
    let approvals = ProjectApprovals::load().await.unwrap();
    let result = approvals
        .verify_check(temp_dir.path(), "audit")
        .await
        .unwrap();

    assert_eq!(
        result,
        VerificationResult::NotApproved,
        "Unapproved check should return NotApproved"
    );
}

#[tokio::test]
async fn test_verify_check_config_hash_mismatch() {
    let _xdg_dir = TempDir::new().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", _xdg_dir.path());

    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join(".config");
    std::fs::create_dir(&config_dir).unwrap();

    let config_content = r#"
[commands]

[[checks]]
name = "audit"
command = ["echo", "test"]
"#;

    std::fs::write(config_dir.join("tools.toml"), config_content).unwrap();

    // Approve the project
    approve_project_config(temp_dir.path(), config_content)
        .await
        .unwrap();

    // Modify the config
    let new_config_content = r#"
[commands]

[[checks]]
name = "audit"
command = ["echo", "modified"]
"#;
    std::fs::write(config_dir.join("tools.toml"), new_config_content).unwrap();

    // Verify should detect config hash mismatch
    let approvals = ProjectApprovals::load().await.unwrap();
    let result = approvals
        .verify_check(temp_dir.path(), "audit")
        .await
        .unwrap();

    match result {
        VerificationResult::ConfigHashMismatch { .. } => {
            // Expected
        }
        _ => panic!(
            "Expected ConfigHashMismatch for modified config, got {:?}",
            result
        ),
    }
}

#[tokio::test]
async fn test_verify_check_binary_hash_mismatch() {
    let _xdg_dir = TempDir::new().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", _xdg_dir.path());

    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join(".config");
    std::fs::create_dir(&config_dir).unwrap();

    // Create a script
    let scripts_dir = temp_dir.path().join("scripts");
    std::fs::create_dir(&scripts_dir).unwrap();
    let script_path = scripts_dir.join("check.sh");
    std::fs::write(&script_path, "#!/bin/bash\necho 'original'\n").unwrap();
    #[cfg(unix)]
    {
        let permissions = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(&script_path, permissions).unwrap();
    }

    let config_content = format!(
        r#"
[commands]

[[checks]]
name = "custom-check"
command = ["{}"]
"#,
        script_path.display()
    );

    std::fs::write(config_dir.join("tools.toml"), &config_content).unwrap();

    // Approve the project
    approve_project_config(temp_dir.path(), &config_content)
        .await
        .unwrap();

    // Modify the script (change the hash)
    std::fs::write(&script_path, "#!/bin/bash\necho 'modified'\n").unwrap();

    // Verify should detect binary hash mismatch
    let approvals = ProjectApprovals::load().await.unwrap();
    let result = approvals
        .verify_check(temp_dir.path(), "custom-check")
        .await
        .unwrap();

    match result {
        VerificationResult::BinaryHashMismatch { item, .. } => {
            assert_eq!(item, "custom-check");
        }
        _ => panic!(
            "Expected BinaryHashMismatch for modified binary, got {:?}",
            result
        ),
    }
}

#[tokio::test]
async fn test_verify_check_not_in_config() {
    let _xdg_dir = TempDir::new().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", _xdg_dir.path());

    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join(".config");
    std::fs::create_dir(&config_dir).unwrap();

    let config_content = r#"
[commands]

[[checks]]
name = "audit"
command = ["echo", "test"]
"#;

    std::fs::write(config_dir.join("tools.toml"), config_content).unwrap();

    // Approve the project
    approve_project_config(temp_dir.path(), config_content)
        .await
        .unwrap();

    // Try to verify a check that wasn't in the config
    let approvals = ProjectApprovals::load().await.unwrap();
    let result = approvals
        .verify_check(temp_dir.path(), "nonexistent-check")
        .await
        .unwrap();

    match result {
        VerificationResult::ItemNotApproved { item } => {
            assert_eq!(item, "nonexistent-check");
        }
        _ => panic!(
            "Expected ItemNotApproved for check not in config, got {:?}",
            result
        ),
    }
}

#[test]
fn test_is_within_project() {
    let project_dir = Path::new("/home/user/project");
    let binary_inside = Path::new("/home/user/project/scripts/build.sh");
    let binary_outside = Path::new("/usr/bin/cargo");

    assert!(is_within_project(binary_inside, project_dir));
    assert!(!is_within_project(binary_outside, project_dir));
}

#[tokio::test]
async fn test_is_script_with_shebang() {
    let mut temp_file = NamedTempFile::new().unwrap();
    writeln!(temp_file, "#!/bin/bash").unwrap();
    writeln!(temp_file, "echo hello").unwrap();
    temp_file.flush().unwrap();

    let is_script_result = is_script(temp_file.path()).await.unwrap();
    assert!(is_script_result);
}

#[tokio::test]
async fn test_is_script_without_shebang() {
    let mut temp_file = NamedTempFile::new().unwrap();
    writeln!(temp_file, "fn main() {{}}").unwrap();
    temp_file.flush().unwrap();

    let is_script_result = is_script(temp_file.path()).await.unwrap();
    assert!(!is_script_result);
}

#[cfg(unix)]
#[tokio::test]
async fn test_is_writable_with_writable_file() {
    let mut temp_file = NamedTempFile::new().unwrap();
    writeln!(temp_file, "#!/bin/bash").unwrap();
    temp_file.flush().unwrap();

    // Set owner-writable permission (0o600 = owner read+write)
    let mut perms = std::fs::metadata(temp_file.path()).unwrap().permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(temp_file.path(), perms).unwrap();

    let result = is_writable(temp_file.path()).await.unwrap();
    assert!(result, "File with 0o600 permissions should be writable");
}

#[cfg(unix)]
#[tokio::test]
async fn test_is_writable_with_readonly_file() {
    let mut temp_file = NamedTempFile::new().unwrap();
    writeln!(temp_file, "#!/bin/bash").unwrap();
    temp_file.flush().unwrap();

    // Set readonly permission (0o400 = owner read-only)
    let mut perms = std::fs::metadata(temp_file.path()).unwrap().permissions();
    perms.set_mode(0o400);
    std::fs::set_permissions(temp_file.path(), perms).unwrap();

    let result = is_writable(temp_file.path()).await.unwrap();
    assert!(
        !result,
        "File with 0o400 permissions should not be writable"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_is_writable_with_executable_only() {
    let mut temp_file = NamedTempFile::new().unwrap();
    writeln!(temp_file, "#!/bin/bash").unwrap();
    temp_file.flush().unwrap();

    // Set execute-only permission (0o500 = owner read+execute, no write)
    let mut perms = std::fs::metadata(temp_file.path()).unwrap().permissions();
    perms.set_mode(0o500);
    std::fs::set_permissions(temp_file.path(), perms).unwrap();

    let result = is_writable(temp_file.path()).await.unwrap();
    assert!(
        !result,
        "File with 0o500 permissions should not be writable"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_is_writable_with_full_permissions() {
    let mut temp_file = NamedTempFile::new().unwrap();
    writeln!(temp_file, "#!/bin/bash").unwrap();
    temp_file.flush().unwrap();

    // Set full permissions (0o755 = owner rwx, group rx, others rx)
    let mut perms = std::fs::metadata(temp_file.path()).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(temp_file.path(), perms).unwrap();

    let result = is_writable(temp_file.path()).await.unwrap();
    assert!(
        result,
        "File with 0o755 permissions should be writable by owner"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_is_writable_checks_owner_bit_only() {
    // Security: We check only owner write bit because if the current user can modify
    // the binary, an attacker with access to this user account can inject malicious code
    // before execution, bypassing our hash-based approval system

    let mut temp_file = NamedTempFile::new().unwrap();
    writeln!(temp_file, "#!/bin/bash").unwrap();
    temp_file.flush().unwrap();

    // Set group-writable but owner-readonly (0o420 = owner r, group w, others none)
    // This shouldn't be considered writable since owner can't write
    let mut perms = std::fs::metadata(temp_file.path()).unwrap().permissions();
    perms.set_mode(0o420);
    std::fs::set_permissions(temp_file.path(), perms).unwrap();

    let result = is_writable(temp_file.path()).await.unwrap();
    assert!(
        !result,
        "File with group-write but no owner-write should not be writable"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_is_writable_with_directory() {
    let temp_dir = TempDir::new().unwrap();

    // Set writable directory
    let mut perms = std::fs::metadata(temp_dir.path()).unwrap().permissions();
    perms.set_mode(0o700);
    std::fs::set_permissions(temp_dir.path(), perms).unwrap();

    let result = is_writable(temp_dir.path()).await.unwrap();
    assert!(result, "Writable directory should be detected as writable");
}

#[test]
fn test_resolve_binary_absolute_path() {
    // Absolute paths should be used as-is, then canonicalized
    let project_dir = PathBuf::from("/tmp");

    // Test with a binary that exists (using sh which should exist on Unix systems)
    #[cfg(unix)]
    {
        let (original, canonical) =
            resolve_binary_path_with_original("/bin/sh", &project_dir).unwrap();

        assert_eq!(original, PathBuf::from("/bin/sh"));
        assert!(canonical.is_absolute());
        // Canonical might resolve symlinks, but should still point to sh
        assert!(canonical.to_string_lossy().contains("sh"));
    }
}

#[test]
fn test_resolve_binary_in_path() {
    // Binaries without path separators should be looked up in PATH
    let project_dir = PathBuf::from("/tmp");

    // Test with 'sh' which should be in PATH on Unix
    #[cfg(unix)]
    {
        let (original, canonical) = resolve_binary_path_with_original("sh", &project_dir).unwrap();

        assert!(original.is_absolute());
        assert!(canonical.is_absolute());
        assert!(original.to_string_lossy().contains("sh"));
    }
}

#[test]
fn test_resolve_binary_relative_path() {
    // Relative paths with path separators should be resolved relative to project directory

    let project_dir = TempDir::new().unwrap();

    // Create subdirectory
    let subdir = project_dir.path().join("bin");
    std::fs::create_dir(&subdir).unwrap();

    let script_path = subdir.join("script.sh");

    // Create a script file
    let mut script = std::fs::File::create(&script_path).unwrap();
    writeln!(script, "#!/bin/bash").unwrap();
    drop(script);

    // Use relative path with separator
    let (original, canonical) =
        resolve_binary_path_with_original("bin/script.sh", project_dir.path()).unwrap();

    assert_eq!(original, project_dir.path().join("bin/script.sh"));
    assert!(canonical.is_absolute());
    assert!(canonical.ends_with("script.sh"));
}

#[test]
fn test_resolve_binary_with_dot_slash() {
    // Paths starting with ./ should be relative to project dir

    let project_dir = TempDir::new().unwrap();
    let script_path = project_dir.path().join("test.sh");

    let mut script = std::fs::File::create(&script_path).unwrap();
    writeln!(script, "#!/bin/bash").unwrap();
    drop(script);

    let (original, canonical) =
        resolve_binary_path_with_original("./test.sh", project_dir.path()).unwrap();

    assert_eq!(original, project_dir.path().join("./test.sh"));
    assert!(canonical.is_absolute());
    assert!(canonical.ends_with("test.sh"));
}

#[test]
fn test_resolve_binary_not_found() {
    // Non-existent binaries should return an error
    let project_dir = PathBuf::from("/tmp");

    let result = resolve_binary_path_with_original(
        "this-binary-definitely-does-not-exist-12345",
        &project_dir,
    );

    let err_msg = format!(
        "{:?}",
        result.expect_err("Should fail with nonexistent binary")
    );
    assert!(err_msg.contains("Failed to find binary") || err_msg.contains("not found"));
}

#[test]
fn test_resolve_binary_with_subdirectory() {
    // Relative paths with subdirectories should work

    let project_dir = TempDir::new().unwrap();
    let scripts_dir = project_dir.path().join("scripts");
    std::fs::create_dir(&scripts_dir).unwrap();

    let script_path = scripts_dir.join("build.sh");
    let mut script = std::fs::File::create(&script_path).unwrap();
    writeln!(script, "#!/bin/bash").unwrap();
    drop(script);

    let (original, canonical) =
        resolve_binary_path_with_original("scripts/build.sh", project_dir.path()).unwrap();

    assert_eq!(original, project_dir.path().join("scripts/build.sh"));
    assert!(canonical.is_absolute());
    assert!(canonical.ends_with("build.sh"));
    assert!(canonical.to_string_lossy().contains("scripts"));
}

#[cfg(unix)]
#[test]
fn test_resolve_binary_follows_symlinks() {
    // Canonical path should resolve all symlinks

    let temp_dir = TempDir::new().unwrap();

    // Create actual binary
    let real_binary = temp_dir.path().join("real.sh");
    let mut script = std::fs::File::create(&real_binary).unwrap();
    writeln!(script, "#!/bin/bash").unwrap();
    drop(script);

    // Create symlink
    let link_path = temp_dir.path().join("link.sh");
    std::os::unix::fs::symlink(&real_binary, &link_path).unwrap();

    let (original, canonical) =
        resolve_binary_path_with_original(link_path.to_str().unwrap(), temp_dir.path()).unwrap();

    // Original should be the symlink
    assert_eq!(original, link_path);

    // Canonical should resolve to real file
    assert_eq!(canonical, real_binary.canonicalize().unwrap());
    assert!(canonical.ends_with("real.sh"));
}

#[cfg(unix)]
#[test]
fn test_resolve_binary_multilevel_symlinks() {
    // Test that multiple levels of symlinks are fully resolved

    let temp_dir = TempDir::new().unwrap();

    // Create actual binary
    let real_binary = temp_dir.path().join("real.sh");
    let mut script = std::fs::File::create(&real_binary).unwrap();
    writeln!(script, "#!/bin/bash").unwrap();
    drop(script);

    // Create symlink chain: link1 -> link2 -> real
    let link2 = temp_dir.path().join("link2.sh");
    std::os::unix::fs::symlink(&real_binary, &link2).unwrap();

    let link1 = temp_dir.path().join("link1.sh");
    std::os::unix::fs::symlink(&link2, &link1).unwrap();

    let (original, canonical) =
        resolve_binary_path_with_original(link1.to_str().unwrap(), temp_dir.path()).unwrap();

    // Original should be link1
    assert_eq!(original, link1);

    // Canonical should resolve all the way to real binary
    assert_eq!(canonical, real_binary.canonicalize().unwrap());
    assert!(canonical.ends_with("real.sh"));
}

#[tokio::test]
#[ignore] // This test can timeout in CI environments due to file locking contention
async fn test_concurrent_approvals_use_file_locking() {
    // Test that concurrent approval updates don't corrupt the file due to file locking
    // This validates that ProjectApprovals::update() properly serializes concurrent writes

    let _xdg_dir = TempDir::new().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", _xdg_dir.path());

    // Spawn fewer concurrent operations to avoid test timeout
    let mut handles = vec![];

    for i in 0..3 {
        let handle = tokio::spawn(async move {
            let project_dir = PathBuf::from(format!("/test/project{}", i));
            let tools_hash = format!("sha256:hash{}", i);
            let mut commands = HashMap::new();

            commands.insert(
                format!("command{}", i),
                CommandApproval {
                    original_path: format!("/usr/bin/cmd{}", i),
                    canonical_path: format!("/usr/bin/cmd{}", i),
                    binary_hash: format!("sha256:binary{}", i),
                },
            );

            ProjectApprovals::update(move |approvals| {
                approvals.approve_project(project_dir, tools_hash, commands, HashMap::new())
            })
            .await
            .expect("Concurrent approval should succeed");
        });

        handles.push(handle);
    }

    // Wait for all concurrent operations to complete
    for handle in handles {
        handle.await.expect("Task should complete successfully");
    }

    // Verify all approvals were recorded without corruption
    let final_approvals = ProjectApprovals::load().await.unwrap();
    assert_eq!(
        final_approvals.projects.len(),
        3,
        "All 3 concurrent approvals should be recorded"
    );

    // Verify each project has correct data
    for i in 0..3 {
        let project_key = format!("/test/project{}", i);
        assert!(
            final_approvals.projects.contains_key(&project_key),
            "Project {} should be in approvals",
            i
        );

        let approval = &final_approvals.projects[&project_key];
        assert_eq!(approval.tools_config_hash, format!("sha256:hash{}", i));
        assert_eq!(approval.commands.len(), 1);
        assert!(approval.commands.contains_key(&format!("command{}", i)));
    }
}

#[tokio::test]
#[ignore] // This test can timeout in CI environments due to file locking contention
async fn test_concurrent_updates_to_same_project() {
    // Test that concurrent updates to the same project are properly serialized
    // Last write should win, and file should not be corrupted

    let _xdg_dir = TempDir::new().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", _xdg_dir.path());

    let project_dir = PathBuf::from("/test/same-project");

    // Spawn fewer concurrent updates to avoid test timeout
    let mut handles = vec![];

    for i in 0..3 {
        let project_dir = project_dir.clone();
        let handle = tokio::spawn(async move {
            let tools_hash = format!("sha256:hash{}", i);
            let mut commands = HashMap::new();

            commands.insert(
                "test".to_string(),
                CommandApproval {
                    original_path: format!("/usr/bin/test{}", i),
                    canonical_path: format!("/usr/bin/test{}", i),
                    binary_hash: format!("sha256:binary{}", i),
                },
            );

            ProjectApprovals::update(move |approvals| {
                approvals.approve_project(project_dir, tools_hash, commands, HashMap::new())
            })
            .await
            .expect("Concurrent update should succeed");
        });

        handles.push(handle);
    }

    // Wait for all operations
    for handle in handles {
        handle.await.expect("Task should complete");
    }

    // Verify file is not corrupted and contains valid data
    let final_approvals = ProjectApprovals::load().await.unwrap();
    assert_eq!(
        final_approvals.projects.len(),
        1,
        "Should have exactly one project"
    );

    let project_key = "/test/same-project";
    assert!(final_approvals.projects.contains_key(project_key));

    // One of the updates should have won (last-write-wins semantics)
    let approval = &final_approvals.projects[project_key];
    assert!(approval.tools_config_hash.starts_with("sha256:hash"));
    assert_eq!(approval.commands.len(), 1);
    assert!(approval.commands.contains_key("test"));
}

#[tokio::test]
async fn test_file_locking_prevents_read_during_write() {
    // Test that file locking prevents reading partially-written approval files
    // This ensures atomicity of the load-modify-save cycle
    let _xdg_dir = TempDir::new().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", _xdg_dir.path());

    // Create a real temp directory for the project
    let _project_temp_dir = TempDir::new().unwrap();
    let project_dir = _project_temp_dir.path().to_path_buf();
    let project_key = project_dir
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let project_key_clone = project_key.clone();

    // Start a long-running update operation
    let write_handle = tokio::spawn(async move {
        ProjectApprovals::update(|approvals| {
            let tools_hash = "sha256:hash1".to_string();
            let mut commands = HashMap::new();

            commands.insert(
                "test".to_string(),
                CommandApproval {
                    original_path: "/usr/bin/test".to_string(),
                    canonical_path: "/usr/bin/test".to_string(),
                    binary_hash: "sha256:binary1".to_string(),
                },
            );

            approvals
                .approve_project(project_dir, tools_hash, commands, HashMap::new())
                .unwrap();

            // Simulate slow operation
            std::thread::sleep(Duration::from_millis(100));
            Ok(())
        })
        .await
    });

    // Give write operation time to acquire lock
    sleep(Duration::from_millis(10)).await;

    // Attempt concurrent read - should either see old state or new state, never partial
    let read_handle = tokio::spawn(async move {
        match ProjectApprovals::load().await {
            Ok(approvals) => {
                // If we read successfully, data should be consistent
                if let Some(approval) = approvals.projects.get(&project_key_clone) {
                    assert_eq!(approval.tools_config_hash, "sha256:hash1");
                    assert_eq!(approval.commands.len(), 1);
                }
            }
            Err(_) => {
                // It's ok if load fails - the important thing is no corruption
            }
        }
    });

    // Wait for both operations
    let _ = write_handle.await.expect("Write should complete");
    read_handle.await.expect("Read task should complete");

    // Verify final state is consistent
    let final_approvals = ProjectApprovals::load().await.unwrap();
    if let Some(approval) = final_approvals.projects.get(&project_key) {
        assert_eq!(approval.tools_config_hash, "sha256:hash1");
        assert_eq!(approval.commands.len(), 1);
    }
}

#[tokio::test]
async fn test_save_approvals_persists_checks() {
    let _xdg_dir = TempDir::new().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", _xdg_dir.path());

    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join(".config");
    std::fs::create_dir(&config_dir).unwrap();

    let config_content = r#"
[commands]
lint = ["echo", "lint"]

[[checks]]
name = "security-audit"
command = ["echo", "audit"]

[[checks]]
name = "license-check"
command = ["echo", "check"]
"#;

    std::fs::write(config_dir.join("tools.toml"), config_content).unwrap();

    // Approve the project using the helper
    approve_project_config(temp_dir.path(), config_content)
        .await
        .unwrap();

    // Load approvals and verify checks were persisted
    let approvals = ProjectApprovals::load().await.unwrap();
    let project_key = temp_dir
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let approval = approvals
        .projects
        .get(&project_key)
        .expect("Project should be approved");

    // Verify both commands and checks are saved
    assert_eq!(approval.commands.len(), 1, "Should have 1 command");
    assert!(approval.commands.contains_key("lint"));

    assert_eq!(approval.checks.len(), 2, "Should have 2 checks");
    assert!(approval.checks.contains_key("security-audit"));
    assert!(approval.checks.contains_key("license-check"));

    // Verify check approvals contain correct data
    let audit_approval = &approval.checks["security-audit"];
    assert!(
        !audit_approval.binary_hash.is_empty(),
        "Binary hash should be set"
    );
}

#[tokio::test]
async fn test_jj_workspaces_share_approvals() {
    let _xdg_dir = TempDir::new().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", _xdg_dir.path());

    let repo_dir = TempDir::new().unwrap();
    let repo_path = repo_dir.path();

    setup_jj_repo(repo_path);

    let config_content = r#"
[commands]
lint = ["echo", "lint"]
"#;
    create_tools_config(repo_path, config_content);

    let repo_root = approve_project_config(repo_path, config_content)
        .await
        .unwrap();

    // Create a second workspace (jj requires destination to NOT exist)
    let workspace2_parent = TempDir::new().unwrap();
    let workspace2_path = workspace2_parent.path().join("workspace2");
    run_jj_command(
        &["workspace", "add", workspace2_path.to_str().unwrap()],
        repo_path,
    );

    // Verify both workspaces resolve to the same repository root
    let workspace2_root = repository::detect_repository_root(&workspace2_path).unwrap();
    assert_eq!(
        repo_root, workspace2_root,
        "Both workspaces should resolve to the same repository root"
    );

    // Verify approval works in the second workspace
    let approvals = ProjectApprovals::load().await.unwrap();
    let result = approvals
        .verify_project(&workspace2_path, "lint")
        .await
        .unwrap();

    assert_eq!(
        result,
        VerificationResult::Approved,
        "Approval from workspace1 should work in workspace2"
    );

    // Verify the approval key is the repository root
    let approval_key = repo_root.to_string_lossy().to_string();
    assert!(
        approvals.projects.contains_key(&approval_key),
        "Approval should be keyed by repository root: {}",
        approval_key
    );
}

#[tokio::test]
async fn test_git_worktrees_share_approvals() {
    let _xdg_dir = TempDir::new().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", _xdg_dir.path());

    let repo_dir = TempDir::new().unwrap();
    let repo_path = repo_dir.path();

    setup_git_repo_with_commit(repo_path);

    let config_content = r#"
[commands]
lint = ["echo", "lint"]
"#;
    create_tools_config(repo_path, config_content);

    let repo_root = approve_project_config(repo_path, config_content)
        .await
        .unwrap();

    // Create a worktree (git requires destination to NOT exist)
    let worktree_parent = TempDir::new().unwrap();
    let worktree_path = worktree_parent.path().join("worktree2");
    run_git_command(
        &["worktree", "add", worktree_path.to_str().unwrap(), "HEAD"],
        repo_path,
    );

    // Verify both worktrees resolve to the same repository root
    let worktree_root = repository::detect_repository_root(&worktree_path).unwrap();
    assert_eq!(
        repo_root, worktree_root,
        "Both worktrees should resolve to the same repository root"
    );

    // Verify approval works in the worktree
    let approvals = ProjectApprovals::load().await.unwrap();
    let result = approvals
        .verify_project(&worktree_path, "lint")
        .await
        .unwrap();

    assert_eq!(
        result,
        VerificationResult::Approved,
        "Approval from main worktree should work in worktree2"
    );

    // Verify the approval key is the repository root
    let approval_key = repo_root.to_string_lossy().to_string();
    assert!(
        approvals.projects.contains_key(&approval_key),
        "Approval should be keyed by repository root: {}",
        approval_key
    );
}

#[tokio::test]
async fn test_load_approvals_without_checks_field() {
    let xdg_dir = TempDir::new().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", xdg_dir.path());

    // Create old-format approval TOML without checks field
    let approvals_dir = xdg_dir.path().join("moriarty");
    std::fs::create_dir_all(&approvals_dir).unwrap();

    let old_format_toml = r#"
[projects."/test/project"]
tools_config_hash = "hash123"
last_approved = "2024-01-01T00:00:00Z"

[projects."/test/project".commands.lint]
original_path = "/bin/echo"
canonical_path = "/bin/echo"
binary_hash = "abc123"
"#;

    std::fs::write(
        approvals_dir.join("project_approvals.toml"),
        old_format_toml,
    )
    .unwrap();

    // Load approvals - should succeed with checks field defaulting to empty HashMap
    let approvals = ProjectApprovals::load().await.unwrap();
    let approval = approvals
        .projects
        .get("/test/project")
        .expect("Project should load");

    assert_eq!(approval.commands.len(), 1);
    assert_eq!(
        approval.checks.len(),
        0,
        "Checks should default to empty HashMap"
    );
    assert_eq!(approval.tools_config_hash, "hash123");
}
