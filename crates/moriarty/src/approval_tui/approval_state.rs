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

/// Which section of items is currently being reviewed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Commands,
    Checks,
}

/// State machine driving the entire approval workflow.
///
/// Combines approval data (project path, configuration hash, command/check details) with
/// UI navigation state (current screen, current item index, current section). This structure is
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
    /// All checks from the project configuration
    pub checks: Vec<CommandInfo>,
    /// Which section is currently being reviewed
    pub current_section: Section,
    /// Index of the current item (command or check) being reviewed within the current section.
    pub current_item_index: usize,
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

impl ApprovalState {
    /// Get the current list of items based on the current section
    pub fn current_items(&self) -> &[CommandInfo] {
        match self.current_section {
            Section::Commands => &self.commands,
            Section::Checks => &self.checks,
        }
    }

    /// Get mutable access to the current list of items based on the current section
    pub fn current_items_mut(&mut self) -> &mut [CommandInfo] {
        match self.current_section {
            Section::Commands => &mut self.commands,
            Section::Checks => &mut self.checks,
        }
    }

    /// Get the currently selected item (command or check)
    pub fn current_item(&self) -> Option<&CommandInfo> {
        self.current_items().get(self.current_item_index)
    }

    /// Get the currently selected item mutably (command or check)
    pub fn current_item_mut(&mut self) -> Option<&mut CommandInfo> {
        let index = self.current_item_index;
        self.current_items_mut().get_mut(index)
    }

    /// Get all items (both commands and checks) for final approval
    pub fn all_items(&self) -> impl Iterator<Item = &CommandInfo> {
        self.commands.iter().chain(self.checks.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn create_test_command_info(name: &str) -> CommandInfo {
        CommandInfo {
            name: name.to_string(),
            command_array: vec!["test".to_string()],
            original_path: PathBuf::from("/bin/test"),
            canonical_path: PathBuf::from("/bin/test"),
            binary_hash: "test_hash".to_string(),
            is_script: false,
            is_writable: false,
            is_in_project: false,
            script_contents: None,
            approved: false,
        }
    }

    #[test]
    fn test_current_items_returns_commands_section() {
        let state = ApprovalState {
            project_dir: PathBuf::from("/test"),
            tools_config_hash: "hash".to_string(),
            commands: vec![
                create_test_command_info("cmd1"),
                create_test_command_info("cmd2"),
            ],
            checks: vec![create_test_command_info("check1")],
            current_section: Section::Commands,
            current_item_index: 0,
            screen: Screen::CommandReview,
        };

        let items = state.current_items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "cmd1");
        assert_eq!(items[1].name, "cmd2");
    }

    #[test]
    fn test_current_items_returns_checks_section() {
        let state = ApprovalState {
            project_dir: PathBuf::from("/test"),
            tools_config_hash: "hash".to_string(),
            commands: vec![create_test_command_info("cmd1")],
            checks: vec![
                create_test_command_info("check1"),
                create_test_command_info("check2"),
            ],
            current_section: Section::Checks,
            current_item_index: 0,
            screen: Screen::CommandReview,
        };

        let items = state.current_items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "check1");
        assert_eq!(items[1].name, "check2");
    }

    #[test]
    fn test_current_item_returns_correct_item() {
        let state = ApprovalState {
            project_dir: PathBuf::from("/test"),
            tools_config_hash: "hash".to_string(),
            commands: vec![
                create_test_command_info("cmd1"),
                create_test_command_info("cmd2"),
            ],
            checks: vec![],
            current_section: Section::Commands,
            current_item_index: 1,
            screen: Screen::CommandReview,
        };

        let item = state.current_item();
        assert!(item.is_some());
        assert_eq!(item.unwrap().name, "cmd2");
    }

    #[test]
    fn test_current_item_out_of_bounds_returns_none() {
        let state = ApprovalState {
            project_dir: PathBuf::from("/test"),
            tools_config_hash: "hash".to_string(),
            commands: vec![create_test_command_info("cmd1")],
            checks: vec![],
            current_section: Section::Commands,
            current_item_index: 5, // Out of bounds
            screen: Screen::CommandReview,
        };

        let item = state.current_item();
        assert!(item.is_none());
    }

    #[test]
    fn test_current_item_with_empty_section_returns_none() {
        let state = ApprovalState {
            project_dir: PathBuf::from("/test"),
            tools_config_hash: "hash".to_string(),
            commands: vec![],
            checks: vec![create_test_command_info("check1")],
            current_section: Section::Commands, // Empty section
            current_item_index: 0,
            screen: Screen::CommandReview,
        };

        let item = state.current_item();
        assert!(item.is_none());
    }

    #[test]
    fn test_current_item_mut_modifies_correct_item() {
        let mut state = ApprovalState {
            project_dir: PathBuf::from("/test"),
            tools_config_hash: "hash".to_string(),
            commands: vec![
                create_test_command_info("cmd1"),
                create_test_command_info("cmd2"),
            ],
            checks: vec![],
            current_section: Section::Commands,
            current_item_index: 0,
            screen: Screen::CommandReview,
        };

        if let Some(item) = state.current_item_mut() {
            item.approved = true;
        }

        assert!(state.commands[0].approved);
        assert!(!state.commands[1].approved);
    }

    #[test]
    fn test_all_items_chains_commands_and_checks() {
        let state = ApprovalState {
            project_dir: PathBuf::from("/test"),
            tools_config_hash: "hash".to_string(),
            commands: vec![
                create_test_command_info("cmd1"),
                create_test_command_info("cmd2"),
            ],
            checks: vec![
                create_test_command_info("check1"),
                create_test_command_info("check2"),
            ],
            current_section: Section::Commands,
            current_item_index: 0,
            screen: Screen::CommandReview,
        };

        let all: Vec<&CommandInfo> = state.all_items().collect();
        assert_eq!(all.len(), 4);
        assert_eq!(all[0].name, "cmd1");
        assert_eq!(all[1].name, "cmd2");
        assert_eq!(all[2].name, "check1");
        assert_eq!(all[3].name, "check2");
    }

    #[test]
    fn test_all_items_with_empty_commands() {
        let state = ApprovalState {
            project_dir: PathBuf::from("/test"),
            tools_config_hash: "hash".to_string(),
            commands: vec![],
            checks: vec![
                create_test_command_info("check1"),
                create_test_command_info("check2"),
            ],
            current_section: Section::Checks,
            current_item_index: 0,
            screen: Screen::CommandReview,
        };

        let all: Vec<&CommandInfo> = state.all_items().collect();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "check1");
        assert_eq!(all[1].name, "check2");
    }

    #[test]
    fn test_all_items_with_empty_checks() {
        let state = ApprovalState {
            project_dir: PathBuf::from("/test"),
            tools_config_hash: "hash".to_string(),
            commands: vec![
                create_test_command_info("cmd1"),
                create_test_command_info("cmd2"),
            ],
            checks: vec![],
            current_section: Section::Commands,
            current_item_index: 0,
            screen: Screen::CommandReview,
        };

        let all: Vec<&CommandInfo> = state.all_items().collect();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "cmd1");
        assert_eq!(all[1].name, "cmd2");
    }

    #[test]
    fn test_all_items_with_both_empty() {
        let state = ApprovalState {
            project_dir: PathBuf::from("/test"),
            tools_config_hash: "hash".to_string(),
            commands: vec![],
            checks: vec![],
            current_section: Section::Commands,
            current_item_index: 0,
            screen: Screen::ProjectOverview,
        };

        let all: Vec<&CommandInfo> = state.all_items().collect();
        assert_eq!(all.len(), 0);
    }
}
