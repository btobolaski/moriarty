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

    /// Create a default `ApprovalState` for tests with the given commands and checks.
    fn create_test_state(
        commands: Vec<CommandInfo>,
        checks: Vec<CommandInfo>,
        section: Section,
        screen: Screen,
    ) -> ApprovalState {
        ApprovalState {
            project_dir: PathBuf::from("/test"),
            tools_config_hash: "hash".to_string(),
            commands,
            checks,
            current_section: section,
            current_item_index: 0,
            screen,
        }
    }

    /// Collects item names so table-driven tests can assert ordering without
    /// comparing the rest of each `CommandInfo` fixture.
    fn item_names(items: &[CommandInfo]) -> Vec<&str> {
        items.iter().map(|item| item.name.as_str()).collect()
    }

    #[test]
    fn test_current_items_returns_section_items() {
        let cases = [
            (
                vec![
                    create_test_command_info("cmd1"),
                    create_test_command_info("cmd2"),
                ],
                vec![create_test_command_info("check1")],
                Section::Commands,
                vec!["cmd1", "cmd2"],
            ),
            (
                vec![create_test_command_info("cmd1")],
                vec![
                    create_test_command_info("check1"),
                    create_test_command_info("check2"),
                ],
                Section::Checks,
                vec!["check1", "check2"],
            ),
        ];

        for (index, (commands, checks, section, expected)) in cases.into_iter().enumerate() {
            let state = create_test_state(commands, checks, section, Screen::CommandReview);
            assert_eq!(item_names(state.current_items()), expected, "case {index}");
        }
    }

    #[test]
    fn test_current_item_returns_correct_item() {
        let mut state = create_test_state(
            vec![
                create_test_command_info("cmd1"),
                create_test_command_info("cmd2"),
            ],
            vec![],
            Section::Commands,
            Screen::CommandReview,
        );
        state.current_item_index = 1;

        let item = state.current_item();
        assert!(item.is_some());
        assert_eq!(item.unwrap().name, "cmd2");
    }

    #[test]
    fn test_current_item_out_of_bounds_returns_none() {
        let mut state = create_test_state(
            vec![create_test_command_info("cmd1")],
            vec![],
            Section::Commands,
            Screen::CommandReview,
        );
        state.current_item_index = 5;

        let item = state.current_item();
        assert!(item.is_none());
    }

    #[test]
    fn test_current_item_with_empty_section_returns_none() {
        let state = create_test_state(
            vec![],
            vec![create_test_command_info("check1")],
            Section::Commands,
            Screen::CommandReview,
        );

        let item = state.current_item();
        assert!(item.is_none());
    }

    #[test]
    fn test_current_item_mut_modifies_correct_item() {
        let mut state = create_test_state(
            vec![
                create_test_command_info("cmd1"),
                create_test_command_info("cmd2"),
            ],
            vec![],
            Section::Commands,
            Screen::CommandReview,
        );

        if let Some(item) = state.current_item_mut() {
            item.approved = true;
        }

        assert!(state.commands[0].approved);
        assert!(!state.commands[1].approved);
    }

    #[test]
    fn test_all_items_collects_commands_then_checks() {
        let cases = [
            (
                vec![
                    create_test_command_info("cmd1"),
                    create_test_command_info("cmd2"),
                ],
                vec![
                    create_test_command_info("check1"),
                    create_test_command_info("check2"),
                ],
                vec!["cmd1", "cmd2", "check1", "check2"],
            ),
            (
                vec![],
                vec![
                    create_test_command_info("check1"),
                    create_test_command_info("check2"),
                ],
                vec!["check1", "check2"],
            ),
            (
                vec![
                    create_test_command_info("cmd1"),
                    create_test_command_info("cmd2"),
                ],
                vec![],
                vec!["cmd1", "cmd2"],
            ),
            (vec![], vec![], vec![]),
        ];

        for (index, (commands, checks, expected)) in cases.into_iter().enumerate() {
            let state =
                create_test_state(commands, checks, Section::Commands, Screen::ProjectOverview);
            let names: Vec<_> = state.all_items().map(|item| item.name.as_str()).collect();
            assert_eq!(names, expected, "case {index}");
        }
    }
}
