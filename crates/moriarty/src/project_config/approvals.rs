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

#[cfg(test)]
mod tests;

/// Test helper function re-exported for use in integration tests.
///
/// This function is used by other test modules (e.g., `hooks::tests`, `mcp::tool_runner::tests`)
/// to create approved project configurations for testing purposes.
#[cfg(test)]
pub use tests::approve_project_config;
