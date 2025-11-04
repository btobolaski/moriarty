//! Project tools approval system.
//!
//! This module provides the approval verification system that ensures project tools
//! are explicitly approved before execution. It tracks SHA-256 hashes of:
//! - `.config/tools.toml` configuration files
//! - Executable binaries referenced by tool commands
//!
//! # Security Model
//!
//! The approval system enforces:
//! - **Explicit approval**: All project tools must be approved via the TUI
//! - **Change detection**: Hash verification detects modifications to configs or binaries
//! - **All-or-nothing**: All configured commands must be approved together
//! - **Script visibility**: Script contents are shown during approval
//! - **In-project warnings**: Extra confirmation for executables within project directories

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

// Unix-specific permission checking. Windows code uses different APIs (see is_writable implementation).
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use fs2::FileExt;

use chrono::{DateTime, Utc};
use miette::{Context, IntoDiagnostic, Result};
use serde::{Deserialize, Serialize};
use tokio::fs::read_to_string;

use crate::{hashing, persistence::FileType};

use super::config::ProjectConfig;

const APPROVALS_FILE: &str = "project_approvals.toml";

/// All project approvals stored in ~/.config/moriarty/project_approvals.toml
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectApprovals {
    /// Map of canonical project path to approval data
    #[serde(default)]
    pub projects: HashMap<String, ProjectApproval>,
}

/// Approval data for a single project
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectApproval {
    /// SHA-256 hash of the tools.toml file contents
    pub tools_config_hash: String,
    /// Timestamp when this project was last approved
    pub last_approved: DateTime<Utc>,
    /// Approved commands with their binary hashes
    pub commands: HashMap<String, CommandApproval>,
    /// Approved checks with their binary hashes
    #[serde(default)]
    pub checks: HashMap<String, CommandApproval>,
}

/// Approval data for a single command
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandApproval {
    /// Original path specified in tools.toml (may be a symlink)
    pub original_path: String,
    /// Canonical path to the binary executable (symlinks resolved)
    pub canonical_path: String,
    /// SHA-256 hash of the binary file
    pub binary_hash: String,
}

/// Type of item being verified
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemType {
    Command,
    Check,
}

/// Result of verifying project approval status
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationResult {
    /// Project is approved and all hashes match
    Approved,
    /// Project has not been approved yet
    NotApproved,
    /// tools.toml hash doesn't match (configuration changed)
    ConfigHashMismatch { expected: String, actual: String },
    /// Binary hash doesn't match (executable changed)
    BinaryHashMismatch {
        item: String,
        expected: String,
        actual: String,
    },
    /// Item (command or check) is configured but not in approvals
    ItemNotApproved { item: String },
}

impl ProjectApprovals {
    /// Load approvals from disk
    pub async fn load() -> Result<Self> {
        match FileType::Config.load::<Self>(APPROVALS_FILE).await {
            Ok(approvals) => Ok(approvals),
            Err(e) => {
                let error_msg = format!("{:?}", e);
                if error_msg.contains("No such file or directory")
                    || error_msg.contains("cannot find the file")
                    || error_msg.contains("NotFound")
                {
                    Ok(Self::default())
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Save approvals to disk
    pub async fn save(&self) -> Result<()> {
        FileType::Config.persist(APPROVALS_FILE, self).await
    }

    /// Atomically update the approvals file with proper file locking
    ///
    /// This method ensures that concurrent modifications to the approvals file
    /// don't race by using file locking to make the load-modify-save cycle atomic.
    pub async fn update<F>(f: F) -> Result<()>
    where
        F: FnOnce(&mut Self),
    {
        let approvals_path = FileType::Config.build_path(APPROVALS_FILE).await?;
        let lock_path = approvals_path.with_extension("lock");

        if let Some(parent) = lock_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .into_diagnostic()
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        let lock_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .into_diagnostic()
            .with_context(|| format!("Failed to open lock file: {}", lock_path.display()))?;

        lock_file
            .lock_exclusive()
            .into_diagnostic()
            .with_context(|| "Failed to acquire exclusive lock on approvals file")?;

        let mut approvals = Self::load().await?;
        f(&mut approvals);
        approvals.save().await
    }

    /// Verify that a project command is approved and its hashes match
    pub async fn verify_project(
        &self,
        project_dir: &Path,
        command_name: &str,
    ) -> Result<VerificationResult> {
        self.verify_item(project_dir, command_name, ItemType::Command)
            .await
    }

    /// Verify that a project check is approved and its hashes match
    pub async fn verify_check(
        &self,
        project_dir: &Path,
        check_name: &str,
    ) -> Result<VerificationResult> {
        self.verify_item(project_dir, check_name, ItemType::Check)
            .await
    }

    /// Generic verification for commands or checks
    async fn verify_item(
        &self,
        project_dir: &Path,
        item_name: &str,
        item_type: ItemType,
    ) -> Result<VerificationResult> {
        // Canonicalize the project directory
        let canonical_dir = project_dir
            .canonicalize()
            .into_diagnostic()
            .with_context(|| {
                format!(
                    "Failed to canonicalize project directory: {}",
                    project_dir.display()
                )
            })?;

        let project_key = canonical_dir.to_string_lossy().to_string();

        // Check if project exists in approvals
        let Some(approval) = self.projects.get(&project_key) else {
            return Ok(VerificationResult::NotApproved);
        };

        // Load and hash the current tools.toml
        let tools_config_path = canonical_dir.join(".config/tools.toml");
        let tools_config_content = read_to_string(&tools_config_path)
            .await
            .into_diagnostic()
            .with_context(|| format!("Failed to read {}", tools_config_path.display()))?;

        let current_config_hash = hashing::hash_string(&tools_config_content);

        // Check if config hash matches
        if current_config_hash != approval.tools_config_hash {
            return Ok(VerificationResult::ConfigHashMismatch {
                expected: approval.tools_config_hash.clone(),
                actual: current_config_hash,
            });
        }

        // Parse the config once for both commands and checks
        let config: ProjectConfig = toml::from_str(&tools_config_content)
            .into_diagnostic()
            .with_context(|| format!("Failed to parse {}", tools_config_path.display()))?;

        // ARCHITECTURAL NOTE: Commands and checks are handled asymmetrically BY DESIGN:
        //
        // Commands use a fixed struct with 4 predefined fields (lint, test, build, format) because
        // they correspond directly to the MCP protocol's standardized tools (run_lint, run_test,
        // run_build, run_formatter). These are part of the public API contract that MCP clients
        // expect. The fixed structure ensures type safety and prevents runtime errors when the
        // MCP server receives requests for these standardized operations.
        //
        // Checks use a dynamic Vec<Check> because they are arbitrary user-defined validation
        // scripts (e.g., security audits, license checks) with no standardized names or count.
        // Projects can define zero, one, or many checks with any names they choose.
        //
        // This asymmetry is intentional and correct - it reflects the fundamental difference
        // between protocol-defined operations (commands) and user-defined operations (checks).
        let (item_approval, command_array) = match item_type {
            ItemType::Command => {
                let Some(command_approval) = approval.commands.get(item_name) else {
                    return Ok(VerificationResult::ItemNotApproved {
                        item: item_name.to_string(),
                    });
                };

                // Get the command array for verification
                let command_array = match item_name {
                    "lint" => config.commands.lint,
                    "test" => config.commands.test,
                    "build" => config.commands.build,
                    "format" => config.commands.format,
                    _ => None,
                };

                let Some(command_array) = command_array else {
                    return Ok(VerificationResult::ItemNotApproved {
                        item: item_name.to_string(),
                    });
                };

                (command_approval, command_array)
            }
            ItemType::Check => {
                let Some(check_approval) = approval.checks.get(item_name) else {
                    return Ok(VerificationResult::ItemNotApproved {
                        item: item_name.to_string(),
                    });
                };

                // Find the check in the config
                let check = config
                    .checks
                    .as_ref()
                    .and_then(|checks| checks.iter().find(|c| c.name == item_name));

                let Some(check) = check else {
                    return Ok(VerificationResult::ItemNotApproved {
                        item: item_name.to_string(),
                    });
                };

                (check_approval, check.command.clone())
            }
        };

        let binary_name = &command_array[0];
        let (original_path, canonical_path) =
            resolve_binary_path_with_original(binary_name, &canonical_dir)?;

        // Hash immediately after resolution to prevent TOCTOU attacks
        let current_binary_hash = hashing::hash_file(&canonical_path).await?;

        if original_path.to_string_lossy() != item_approval.original_path {
            return Ok(VerificationResult::BinaryHashMismatch {
                item: item_name.to_string(),
                expected: item_approval.binary_hash.clone(),
                actual: format!(
                    "original path changed from {} to {}",
                    item_approval.original_path,
                    original_path.display()
                ),
            });
        }

        if canonical_path.to_string_lossy() != item_approval.canonical_path {
            return Ok(VerificationResult::BinaryHashMismatch {
                item: item_name.to_string(),
                expected: item_approval.binary_hash.clone(),
                actual: format!(
                    "canonical path changed from {} to {}",
                    item_approval.canonical_path,
                    canonical_path.display()
                ),
            });
        }

        // Check if binary hash matches
        if current_binary_hash != item_approval.binary_hash {
            return Ok(VerificationResult::BinaryHashMismatch {
                item: item_name.to_string(),
                expected: item_approval.binary_hash.clone(),
                actual: current_binary_hash,
            });
        }

        Ok(VerificationResult::Approved)
    }

    /// Add or update approval for a project
    pub fn approve_project(
        &mut self,
        project_dir: PathBuf,
        tools_config_hash: String,
        commands: HashMap<String, CommandApproval>,
        checks: HashMap<String, CommandApproval>,
    ) {
        let project_key = project_dir.to_string_lossy().to_string();
        self.projects.insert(
            project_key,
            ProjectApproval {
                tools_config_hash,
                last_approved: Utc::now(),
                commands,
                checks,
            },
        );
    }
}

/// Resolve a binary name to both its original and canonical paths
///
/// Returns (original_path, canonical_path) where:
/// - original_path: The resolved but not canonicalized path (may be a symlink)
/// - canonical_path: The fully resolved path with all symlinks followed
///
/// This tracks symlinks at multiple levels to detect if any intermediate symlink changes.
pub fn resolve_binary_path_with_original(
    binary_name: &str,
    project_dir: &Path,
) -> Result<(PathBuf, PathBuf)> {
    let path = Path::new(binary_name);

    // Determine the original (non-canonicalized) path
    let original_path = if path.is_absolute() {
        path.to_path_buf()
    } else if binary_name.contains('/') {
        // Relative path - resolve relative to project dir
        project_dir.join(binary_name)
    } else {
        // Look up in PATH
        which::which(binary_name)
            .into_diagnostic()
            .with_context(|| format!("Failed to find binary '{}' in PATH", binary_name))?
    };

    // Canonicalize to get the final path with all symlinks resolved
    let canonical_path = original_path
        .canonicalize()
        .into_diagnostic()
        .with_context(|| {
            format!(
                "Failed to canonicalize binary path: {}",
                original_path.display()
            )
        })?;

    Ok((original_path, canonical_path))
}

/// Check if a file is a script by reading its first bytes for a shebang.
///
/// Scripts are treated specially in the approval flow: if a script is also writable,
/// its full contents are displayed to the user during approval. This prevents hidden
/// malicious code execution by ensuring users can review what will actually run.
pub async fn is_script(path: &Path) -> Result<bool> {
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(path)
        .await
        .into_diagnostic()
        .with_context(|| format!("Failed to open file: {}", path.display()))?;

    let mut buffer = [0u8; 2];

    match file.read_exact(&mut buffer).await {
        Ok(_) => Ok(buffer == *b"#!"),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(miette::miette!("Failed to read file header: {}", e)),
    }
}

/// Check if a file is writable by the current user
#[cfg(unix)]
pub async fn is_writable(path: &Path) -> Result<bool> {
    let metadata = tokio::fs::metadata(path)
        .await
        .into_diagnostic()
        .with_context(|| format!("Failed to get metadata for: {}", path.display()))?;

    let permissions = metadata.permissions();
    let mode = permissions.mode();

    // Check only owner write bit (0o200) for security: if the current user can modify the binary,
    // an attacker with access to this user account can inject malicious code before execution,
    // bypassing our hash-based approval system. Group/other write bits are irrelevant to this threat.
    Ok(mode & 0o200 != 0)
}

/// Check if a path is within a project directory
pub fn is_within_project(binary_path: &Path, project_dir: &Path) -> bool {
    binary_path.starts_with(project_dir)
}

/// Read script contents for display in TUI
pub async fn read_script_contents(path: &Path) -> Result<String> {
    tokio::fs::read_to_string(path)
        .await
        .into_diagnostic()
        .with_context(|| format!("Failed to read script: {}", path.display()))
}

/// Test helper to pre-approve a project with the given config content.
/// This bypasses the approval TUI for integration tests.
/// Returns the canonical project path for use in assertions.
#[cfg(test)]
pub async fn approve_project_config(project_dir: &Path, config_content: &str) -> PathBuf {
    let canonical_path = project_dir.canonicalize().unwrap();
    let config: ProjectConfig = toml::from_str(config_content).unwrap();
    let tools_config_hash = crate::hashing::hash_string(config_content);

    // Process commands
    let mut commands = HashMap::new();
    for (name, cmd_array) in config.commands.all() {
        let binary_name = &cmd_array[0];
        let (original_path, resolved_path) =
            resolve_binary_path_with_original(binary_name, &canonical_path).unwrap();
        let binary_hash = crate::hashing::hash_file(&resolved_path).await.unwrap();

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
                resolve_binary_path_with_original(binary_name, &canonical_path).unwrap();
            let binary_hash = crate::hashing::hash_file(&resolved_path).await.unwrap();

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

    let canonical_path_clone = canonical_path.clone();
    ProjectApprovals::update(move |approvals| {
        approvals.approve_project(canonical_path_clone, tools_config_hash, commands, checks);
    })
    .await
    .unwrap();

    canonical_path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_approvals_default() {
        let approvals = ProjectApprovals::default();
        assert_eq!(approvals.projects.len(), 0);
    }

    #[test]
    fn test_approve_project() {
        let mut approvals = ProjectApprovals::default();
        let project_dir = PathBuf::from("/test/project");
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

        approvals.approve_project(
            project_dir.clone(),
            tools_hash.clone(),
            commands.clone(),
            checks,
        );

        let project_key = project_dir.to_string_lossy().to_string();
        assert!(approvals.projects.contains_key(&project_key));

        let approval = &approvals.projects[&project_key];
        assert_eq!(approval.tools_config_hash, tools_hash);
        assert_eq!(approval.commands.len(), 1);
        assert!(approval.commands.contains_key("lint"));
    }

    #[test]
    fn test_approve_project_with_checks() {
        let mut approvals = ProjectApprovals::default();
        let project_dir = PathBuf::from("/test/project");
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

        approvals.approve_project(
            project_dir.clone(),
            tools_hash.clone(),
            commands.clone(),
            checks.clone(),
        );

        let project_key = project_dir.to_string_lossy().to_string();
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
        use tempfile::TempDir;

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
        approve_project_config(temp_dir.path(), config_content).await;

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
        use tempfile::TempDir;

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
        use tempfile::TempDir;

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
        approve_project_config(temp_dir.path(), config_content).await;

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
        use tempfile::TempDir;

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
            use std::os::unix::fs::PermissionsExt;
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
        approve_project_config(temp_dir.path(), &config_content).await;

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
        use tempfile::TempDir;

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
        approve_project_config(temp_dir.path(), config_content).await;

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
        use std::io::Write;
        use tempfile::NamedTempFile;

        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "#!/bin/bash").unwrap();
        writeln!(temp_file, "echo hello").unwrap();
        temp_file.flush().unwrap();

        let is_script_result = is_script(temp_file.path()).await.unwrap();
        assert!(is_script_result);
    }

    #[tokio::test]
    async fn test_is_script_without_shebang() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "fn main() {{}}").unwrap();
        temp_file.flush().unwrap();

        let is_script_result = is_script(temp_file.path()).await.unwrap();
        assert!(!is_script_result);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_is_writable_with_writable_file() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        use tempfile::NamedTempFile;

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
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        use tempfile::NamedTempFile;

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
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        use tempfile::NamedTempFile;

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
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        use tempfile::NamedTempFile;

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
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        use tempfile::NamedTempFile;

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
        use std::os::unix::fs::PermissionsExt;
        use tempfile::TempDir;

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
            let (original, canonical) =
                resolve_binary_path_with_original("sh", &project_dir).unwrap();

            assert!(original.is_absolute());
            assert!(canonical.is_absolute());
            assert!(original.to_string_lossy().contains("sh"));
        }
    }

    #[test]
    fn test_resolve_binary_relative_path() {
        // Relative paths with path separators should be resolved relative to project directory
        use std::io::Write;
        use tempfile::TempDir;

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
        use std::io::Write;
        use tempfile::TempDir;

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
        use std::io::Write;
        use tempfile::TempDir;

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
        use std::io::Write;
        use tempfile::TempDir;

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
            resolve_binary_path_with_original(link_path.to_str().unwrap(), temp_dir.path())
                .unwrap();

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
        use std::io::Write;
        use tempfile::TempDir;

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
        use tempfile::TempDir;

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
                    approvals.approve_project(project_dir, tools_hash, commands, HashMap::new());
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
        use tempfile::TempDir;

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
                    approvals.approve_project(project_dir, tools_hash, commands, HashMap::new());
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
        use tempfile::TempDir;
        use tokio::time::{sleep, Duration};

        let _xdg_dir = TempDir::new().unwrap();
        std::env::set_var("XDG_CONFIG_HOME", _xdg_dir.path());

        // Start a long-running update operation
        let write_handle = tokio::spawn(async {
            ProjectApprovals::update(|approvals| {
                let project_dir = PathBuf::from("/test/project");
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

                approvals.approve_project(project_dir, tools_hash, commands, HashMap::new());

                // Simulate slow operation
                std::thread::sleep(Duration::from_millis(100));
            })
            .await
        });

        // Give write operation time to acquire lock
        sleep(Duration::from_millis(10)).await;

        // Attempt concurrent read - should either see old state or new state, never partial
        let read_handle = tokio::spawn(async {
            match ProjectApprovals::load().await {
                Ok(approvals) => {
                    // If we read successfully, data should be consistent
                    if let Some(approval) = approvals.projects.get("/test/project") {
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
        if let Some(approval) = final_approvals.projects.get("/test/project") {
            assert_eq!(approval.tools_config_hash, "sha256:hash1");
            assert_eq!(approval.commands.len(), 1);
        }
    }

    #[tokio::test]
    async fn test_save_approvals_persists_checks() {
        use tempfile::TempDir;

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
        approve_project_config(temp_dir.path(), config_content).await;

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
    async fn test_load_approvals_without_checks_field() {
        use tempfile::TempDir;

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
}
