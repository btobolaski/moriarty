//! State management for the approval TUI.
//!
//! This module defines the state machine that drives the approval workflow.

use std::path::PathBuf;

/// Current screen in the approval flow
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    /// Initial overview showing project and all commands
    ProjectOverview,
    /// Reviewing a specific command
    CommandReview,
    /// Warning screen for in-project executables
    InProjectWarning,
    /// Final summary before approval
    Summary,
    /// Approval completed successfully
    Approved,
    /// User cancelled the approval
    Cancelled,
}

/// State machine driving the entire approval workflow.
///
/// Combines approval data (project path, configuration hash, command details) with
/// UI navigation state (current screen, current command index). This structure is
/// passed through all rendering and event handling functions to maintain consistency
/// between what's displayed and what's approved.
#[derive(Debug)]
pub struct ApprovalState {
    /// Canonical path to the project directory
    pub project_dir: PathBuf,
    /// SHA-256 hash of the tools.toml configuration
    pub tools_config_hash: String,
    /// All commands from the project configuration
    pub commands: Vec<CommandInfo>,
    /// Index of the command currently being reviewed
    pub current_command_index: usize,
    /// Current screen in the flow
    pub screen: Screen,
}

/// Complete metadata for a single command requiring approval.
///
/// Contains all information needed to display the command for user review and make
/// security decisions. Tracks both the command definition (name, args) and security
/// properties (writable, in-project, hash). If the command is a writable script, its
/// contents are included for display to prevent hidden malicious code execution.
#[derive(Debug)]
pub struct CommandInfo {
    /// Command name (lint, test, build, format)
    pub name: String,
    /// Full command array (e.g., ["cargo", "clippy", ...])
    pub command_array: Vec<String>,
    /// Original path from tools.toml (may be a symlink)
    pub original_path: PathBuf,
    /// Resolved canonical path to the binary (all symlinks followed)
    pub canonical_path: PathBuf,
    /// SHA-256 hash of the binary file
    pub binary_hash: String,
    /// Whether the binary is a script (has shebang)
    pub is_script: bool,
    /// Whether the binary is writable by the current user
    pub is_writable: bool,
    /// Whether the binary is within the project directory
    pub is_in_project: bool,
    /// Script contents if it's a writable script
    pub script_contents: Option<String>,
    /// Whether this command has been approved
    pub approved: bool,
}
