//! Interactive TUI for approving project tools.
//!
//! This module provides a terminal user interface for reviewing and approving
//! project tool configurations. The approval flow walks through each command,
//! showing details about the executable and requiring explicit confirmation.
//!
//! Event handlers are async to support atomic file I/O with locking during final approval.

use crossterm::event::{KeyCode, KeyEvent};
use futures::StreamExt;
use miette::{Context, IntoDiagnostic, Result};
use ratatui::DefaultTerminal;
use std::path::{Path, PathBuf};
use tokio::fs::read_to_string;
use tui_scrollview::ScrollViewState;

use crate::{
    project_config::{
        is_script, is_within_project, is_writable, read_script_contents,
        resolve_binary_path_with_original, CommandApproval, ProjectApprovals, ProjectConfig,
    },
    tui::event_bus::{input_stream, Event, UIEvent},
};

mod approval_state;
mod renderer;

use approval_state::{ApprovalState, CommandInfo, Screen, Section};

/// Interactive TUI application for project tools approval.
///
/// Manages the complete approval workflow from loading project configuration
/// through user review to saving approvals. The application maintains all state
/// including scroll position, current screen, and command approval status.
#[derive(Debug)]
pub struct ApprovalApp {
    state: ApprovalState,
    scroll_state: ScrollViewState,
    should_quit: bool,
    error_message: Option<String>,
}

impl ApprovalApp {
    /// Helper function to load command/check information with binary resolution and security analysis
    async fn load_command_info(
        name: String,
        command_array: Vec<String>,
        canonical_dir: &Path,
    ) -> Result<CommandInfo> {
        let binary_name = &command_array[0];
        let (original_path, canonical_path) =
            resolve_binary_path_with_original(binary_name, canonical_dir)?;

        let is_script_file = is_script(&canonical_path).await?;
        let is_writable_file = is_writable(&canonical_path).await?;
        let script_contents = if is_script_file && is_writable_file {
            Some(read_script_contents(&canonical_path).await?)
        } else {
            None
        };

        let binary_hash = crate::hashing::hash_file(&canonical_path).await?;

        Ok(CommandInfo {
            name,
            command_array,
            original_path,
            canonical_path: canonical_path.clone(),
            binary_hash,
            is_script: is_script_file,
            is_writable: is_writable_file,
            is_in_project: is_within_project(&canonical_path, canonical_dir),
            script_contents,
            approved: false,
        })
    }

    /// Load multiple items (commands or checks) into CommandInfo structs.
    ///
    /// Takes a vector of (name, command_array) tuples and loads each one using load_command_info.
    /// Returns a Result containing the vector of loaded CommandInfo structs.
    async fn load_items_info(
        items: Vec<(String, Vec<String>)>,
        canonical_dir: &Path,
    ) -> Result<Vec<CommandInfo>> {
        let mut infos = Vec::new();
        for (name, command_array) in items {
            infos.push(Self::load_command_info(name, command_array, canonical_dir).await?);
        }
        Ok(infos)
    }

    /// Create a new approval app for the given project directory
    pub async fn new(project_dir: PathBuf) -> Result<Self> {
        // Canonicalize the project directory
        let canonical_dir = project_dir
            .canonicalize()
            .into_diagnostic()
            .with_context(|| format!("Failed to canonicalize path: {}", project_dir.display()))?;

        // Load and parse tools.toml
        let tools_config_path = canonical_dir.join(".config/tools.toml");
        let tools_config_content = read_to_string(&tools_config_path)
            .await
            .into_diagnostic()
            .with_context(|| format!("Failed to read {}", tools_config_path.display()))?;

        let config: ProjectConfig = toml::from_str(&tools_config_content)
            .into_diagnostic()
            .with_context(|| format!("Failed to parse {}", tools_config_path.display()))?;

        // Get all configured commands
        let commands = config.commands.all();

        // Get all configured checks
        let checks = config.checks.unwrap_or_default();

        if commands.is_empty() && checks.is_empty() {
            return Err(miette::miette!(
                "No commands or checks configured in {}",
                tools_config_path.display()
            ));
        }

        // Load command and check information
        let command_infos = Self::load_items_info(commands, &canonical_dir).await?;

        let check_items: Vec<(String, Vec<String>)> = checks
            .into_iter()
            .map(|check| (check.name, check.command))
            .collect();
        let check_infos = Self::load_items_info(check_items, &canonical_dir).await?;

        let tools_config_hash = crate::hashing::hash_string(&tools_config_content);

        Ok(Self {
            state: ApprovalState {
                project_dir: canonical_dir,
                tools_config_hash,
                commands: command_infos,
                checks: check_infos,
                current_section: Section::Commands,
                current_item_index: 0,
                screen: Screen::ProjectOverview,
            },
            scroll_state: ScrollViewState::default(),
            should_quit: false,
            error_message: None,
        })
    }

    /// Run the approval app
    pub async fn run(mut self, mut terminal: DefaultTerminal) -> Result<bool> {
        let mut event_stream = input_stream();

        // Initial render
        terminal
            .draw(|frame| {
                renderer::render(
                    &self.state,
                    &mut self.scroll_state,
                    frame,
                    &self.error_message,
                )
            })
            .into_diagnostic()?;

        while !self.should_quit {
            if let Some(event) = event_stream.next().await {
                let event = event?;
                self.handle_event(event).await?;

                // Re-render after handling event
                terminal
                    .draw(|frame| {
                        renderer::render(
                            &self.state,
                            &mut self.scroll_state,
                            frame,
                            &self.error_message,
                        )
                    })
                    .into_diagnostic()?;
            }
        }

        // Return whether the user approved (true) or cancelled (false)
        Ok(self.state.screen == Screen::Approved)
    }

    async fn handle_event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::UI(ui_event) => match ui_event {
                UIEvent::Key(key) => self.handle_key(key).await,
                UIEvent::Render => {}
                UIEvent::Paste(_) => {}
            },
        }
        Ok(())
    }

    async fn handle_key(&mut self, key: KeyEvent) {
        match self.state.screen {
            Screen::ProjectOverview => self.handle_overview_keys(key),
            Screen::CommandReview => self.handle_command_review_keys(key),
            Screen::InProjectWarning => self.handle_warning_keys(key),
            Screen::Summary => self.handle_summary_keys(key).await,
            Screen::Approved | Screen::Cancelled => {
                self.should_quit = true;
            }
        }
    }

    fn handle_overview_keys(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.state.screen = Screen::Cancelled;
                self.should_quit = true;
            }
            KeyCode::Enter | KeyCode::Char('y') => {
                // Start at the first non-empty section
                if !self.state.commands.is_empty() {
                    self.state.current_section = Section::Commands;
                    self.state.current_item_index = 0;
                    self.state.screen = Screen::CommandReview;
                } else if !self.state.checks.is_empty() {
                    self.state.current_section = Section::Checks;
                    self.state.current_item_index = 0;
                    self.state.screen = Screen::CommandReview;
                } else {
                    // Defensive: validation in new() should prevent reaching here
                    self.state.screen = Screen::Summary;
                }
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.scroll_state.scroll_down();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.scroll_state.scroll_up();
            }
            _ => {}
        }
    }

    fn handle_command_review_keys(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.state.screen = Screen::Cancelled;
                self.should_quit = true;
            }
            KeyCode::Enter | KeyCode::Char('y') => {
                if let Some(current_item) = self.state.current_item() {
                    if current_item.is_in_project && current_item.is_writable {
                        self.state.screen = Screen::InProjectWarning;
                    } else {
                        self.approve_current_and_advance();
                    }
                }
            }
            KeyCode::Char('n') => {
                self.state.screen = Screen::Cancelled;
                self.should_quit = true;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.scroll_state.scroll_down();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.scroll_state.scroll_up();
            }
            KeyCode::PageDown => {
                self.scroll_state.scroll_page_down();
            }
            KeyCode::PageUp => {
                self.scroll_state.scroll_page_up();
            }
            _ => {}
        }
    }

    fn handle_warning_keys(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('n') => {
                self.state.screen = Screen::Cancelled;
                self.should_quit = true;
            }
            KeyCode::Char('Y') => {
                self.approve_current_and_advance();
            }
            _ => {}
        }
    }

    async fn handle_summary_keys(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('n') => {
                self.state.screen = Screen::Cancelled;
                self.should_quit = true;
            }
            KeyCode::Enter | KeyCode::Char('y') => {
                if let Err(e) = self.save_approvals().await {
                    self.error_message = Some(format!("Failed to save approvals: {}", e));
                    self.state.screen = Screen::Cancelled;
                } else {
                    self.state.screen = Screen::Approved;
                }
                self.should_quit = true;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.scroll_state.scroll_down();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.scroll_state.scroll_up();
            }
            _ => {}
        }
    }

    fn approve_current_and_advance(&mut self) {
        if let Some(current_item) = self.state.current_item_mut() {
            current_item.approved = true;
        }
        self.scroll_state = ScrollViewState::default();

        let current_items_len = self.state.current_items().len();

        if self.state.current_item_index + 1 < current_items_len {
            self.state.current_item_index += 1;
            self.state.screen = Screen::CommandReview;
        } else {
            // Reached end of current section - transition to next section or Summary
            match self.state.current_section {
                Section::Commands => {
                    // Commands section complete - transition to Checks if any exist, otherwise Summary
                    if !self.state.checks.is_empty() {
                        self.state.current_section = Section::Checks;
                        self.state.current_item_index = 0;
                        self.state.screen = Screen::CommandReview;
                    } else {
                        // No checks to review - all items approved, proceed to final Summary
                        self.state.screen = Screen::Summary;
                    }
                }
                Section::Checks => {
                    // Checks section complete - this is the last section, proceed to Summary
                    self.state.screen = Screen::Summary;
                }
            }
        }
    }

    async fn save_approvals(&self) -> Result<()> {
        // Verify all commands and checks are approved before saving
        let unapproved: Vec<&str> = self
            .state
            .all_items()
            .filter(|item| !item.approved)
            .map(|item| item.name.as_str())
            .collect();

        if !unapproved.is_empty() {
            return Err(miette::miette!(
                "Cannot save approvals: {} item(s) not approved: {}",
                unapproved.len(),
                unapproved.join(", ")
            ));
        }

        let mut commands = std::collections::HashMap::new();
        for cmd_info in &self.state.commands {
            commands.insert(
                cmd_info.name.clone(),
                CommandApproval {
                    original_path: cmd_info.original_path.to_string_lossy().to_string(),
                    canonical_path: cmd_info.canonical_path.to_string_lossy().to_string(),
                    binary_hash: cmd_info.binary_hash.clone(),
                },
            );
        }

        let mut checks = std::collections::HashMap::new();
        for check_info in &self.state.checks {
            checks.insert(
                check_info.name.clone(),
                CommandApproval {
                    original_path: check_info.original_path.to_string_lossy().to_string(),
                    canonical_path: check_info.canonical_path.to_string_lossy().to_string(),
                    binary_hash: check_info.binary_hash.clone(),
                },
            );
        }

        let project_dir = self.state.project_dir.clone();
        let tools_config_hash = self.state.tools_config_hash.clone();

        // Atomically update approvals with file locking
        ProjectApprovals::update(move |approvals| {
            approvals.approve_project(project_dir, tools_config_hash, commands, checks);
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use std::io::Write;
    use tempfile::TempDir;

    fn setup_test_project() -> TempDir {
        let temp_dir = TempDir::new().unwrap();
        let config_dir = temp_dir.path().join(".config");
        std::fs::create_dir(&config_dir).unwrap();

        // Create a simple test script
        let script_path = temp_dir.path().join("test.sh");
        let mut script = std::fs::File::create(&script_path).unwrap();
        writeln!(script, "#!/bin/bash").unwrap();
        writeln!(script, "echo 'test'").unwrap();
        drop(script);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).unwrap();
        }

        let config_content = format!(
            r#"
[commands]
test = ["{}"]
lint = ["echo", "lint"]
"#,
            script_path.display()
        );

        std::fs::write(config_dir.join("tools.toml"), config_content).unwrap();

        temp_dir
    }

    #[tokio::test]
    async fn test_approval_app_initialization() {
        // Test that ApprovalApp correctly loads project configuration
        let temp_dir = setup_test_project();

        let app = ApprovalApp::new(temp_dir.path().to_path_buf())
            .await
            .expect("ApprovalApp initialization should succeed");

        // Verify initial state
        assert_eq!(app.state.current_item_index, 0);
        assert_eq!(app.state.screen, Screen::ProjectOverview);
        assert!(!app.should_quit);
        assert!(app.error_message.is_none());

        // Verify commands were loaded
        assert_eq!(app.state.commands.len(), 2, "Should load 2 commands");
        assert!(
            app.state.commands.iter().any(|c| c.name == "test"),
            "Should have test command"
        );
        assert!(
            app.state.commands.iter().any(|c| c.name == "lint"),
            "Should have lint command"
        );

        // Verify all commands start unapproved
        for cmd in &app.state.commands {
            assert!(!cmd.approved, "Commands should start unapproved");
        }
    }

    #[tokio::test]
    async fn test_approval_app_initialization_with_empty_config() {
        // Test that empty tools.toml returns an error
        let temp_dir = TempDir::new().unwrap();
        let config_dir = temp_dir.path().join(".config");
        std::fs::create_dir(&config_dir).unwrap();

        std::fs::write(config_dir.join("tools.toml"), "[commands]\n").unwrap();

        let result = ApprovalApp::new(temp_dir.path().to_path_buf()).await;

        assert!(result.is_err(), "Empty config should return error");
        let err = result.unwrap_err();
        let err_msg = format!("{:?}", err);
        assert!(
            err_msg.contains("No commands or checks configured"),
            "Error should mention no commands or checks configured"
        );
    }

    #[tokio::test]
    async fn test_approval_app_initialization_with_missing_config() {
        // Test that missing tools.toml returns an error
        let temp_dir = TempDir::new().unwrap();

        let result = ApprovalApp::new(temp_dir.path().to_path_buf()).await;

        assert!(result.is_err(), "Missing config should return error");
    }

    #[tokio::test]
    async fn test_approve_current_and_advance() {
        // Test state transitions when approving commands
        let temp_dir = setup_test_project();

        let mut app = ApprovalApp::new(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        // Start at command review screen
        app.state.screen = Screen::CommandReview;
        app.state.current_item_index = 0;

        // Approve first command
        assert!(!app.state.commands[0].approved);
        app.approve_current_and_advance();
        assert!(
            app.state.commands[0].approved,
            "First command should be approved"
        );
        assert_eq!(
            app.state.current_item_index, 1,
            "Should advance to next command"
        );
        assert_eq!(
            app.state.screen,
            Screen::CommandReview,
            "Should stay in review screen"
        );

        // Approve second command
        assert!(!app.state.commands[1].approved);
        app.approve_current_and_advance();
        assert!(
            app.state.commands[1].approved,
            "Second command should be approved"
        );
        assert_eq!(
            app.state.screen,
            Screen::Summary,
            "Should move to summary screen"
        );
    }

    #[tokio::test]
    async fn test_save_approvals_validation() {
        // Test that save_approvals validates all commands are approved
        let temp_dir = setup_test_project();
        let _xdg_dir = TempDir::new().unwrap();
        std::env::set_var("XDG_CONFIG_HOME", _xdg_dir.path());

        let mut app = ApprovalApp::new(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        // Try to save with unapproved commands
        let result = app.save_approvals().await;
        assert!(result.is_err(), "Should fail with unapproved commands");

        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(
            err_msg.contains("not approved"),
            "Error should mention unapproved commands"
        );

        // Approve all commands
        for cmd in &mut app.state.commands {
            cmd.approved = true;
        }

        // Now save should succeed
        let result = app.save_approvals().await;
        assert!(result.is_ok(), "Should succeed with all commands approved");

        // Verify approvals were saved
        let approvals = ProjectApprovals::load().await.unwrap();
        let project_key = app.state.project_dir.to_string_lossy().to_string();
        assert!(
            approvals.projects.contains_key(&project_key),
            "Project should be in approvals"
        );
    }

    #[tokio::test]
    async fn test_command_info_metadata_loading() {
        // Test that CommandInfo captures all necessary metadata
        let temp_dir = setup_test_project();

        let app = ApprovalApp::new(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        // Find the test command
        let test_cmd = app
            .state
            .commands
            .iter()
            .find(|c| c.name == "test")
            .expect("Should have test command");

        // Verify metadata
        assert!(test_cmd.is_script, "test.sh should be detected as script");
        assert!(test_cmd.is_in_project, "test.sh should be in project");
        assert!(!test_cmd.binary_hash.is_empty(), "Should have binary hash");
        assert!(
            test_cmd.script_contents.is_some(),
            "Should have script contents for writable script"
        );

        // Find the lint command (uses echo binary)
        let lint_cmd = app
            .state
            .commands
            .iter()
            .find(|c| c.name == "lint")
            .expect("Should have lint command");

        assert!(
            !lint_cmd.is_script || lint_cmd.script_contents.is_none(),
            "echo binary should not have script contents"
        );
        assert!(!lint_cmd.is_in_project, "echo should not be in project");
    }

    #[tokio::test]
    async fn test_in_project_warning_flow() {
        // Test that in-project writeable scripts trigger warning screen
        let temp_dir = setup_test_project();

        let app = ApprovalApp::new(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        // Find the test command (in-project script)
        let test_cmd_idx = app
            .state
            .commands
            .iter()
            .position(|c| c.name == "test")
            .expect("Should have test command");

        let test_cmd = &app.state.commands[test_cmd_idx];

        // Verify it would trigger warning
        assert!(
            test_cmd.is_in_project && test_cmd.is_writable,
            "test.sh should trigger in-project warning"
        );
    }

    #[tokio::test]
    async fn test_tools_config_hash_captured() {
        // Test that tools.toml hash is correctly computed and stored
        let temp_dir = setup_test_project();

        let app = ApprovalApp::new(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        assert!(
            app.state.tools_config_hash.starts_with("sha256:"),
            "Config hash should have sha256 prefix"
        );
        assert_eq!(
            app.state.tools_config_hash.len(),
            71,
            "SHA-256 hash should be 7 (prefix) + 64 (hex) = 71 chars"
        );
    }

    #[tokio::test]
    async fn test_canonical_path_resolution() {
        // Test that project directory is properly canonicalized
        let temp_dir = setup_test_project();

        // Create a symlink to the project
        #[cfg(unix)]
        {
            let link_dir = TempDir::new().unwrap();
            let link_path = link_dir.path().join("project_link");
            std::os::unix::fs::symlink(temp_dir.path(), &link_path).unwrap();

            let app = ApprovalApp::new(link_path).await.unwrap();

            // Project dir should be canonicalized (symlink resolved)
            assert!(
                app.state.project_dir.is_absolute(),
                "Project dir should be absolute"
            );
            assert_eq!(
                app.state.project_dir,
                temp_dir.path().canonicalize().unwrap(),
                "Symlink should be resolved to real path"
            );
        }
    }

    #[tokio::test]
    async fn test_approval_app_with_checks() {
        // Test that ApprovalApp correctly loads projects with checks
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
command = ["echo", "license"]
"#;

        std::fs::write(config_dir.join("tools.toml"), config_content).unwrap();

        let app = ApprovalApp::new(temp_dir.path().to_path_buf())
            .await
            .expect("ApprovalApp initialization should succeed");

        // Verify commands were loaded
        assert_eq!(app.state.commands.len(), 1, "Should load 1 command");
        assert!(
            app.state.commands.iter().any(|c| c.name == "lint"),
            "Should have lint command"
        );

        // Verify checks were loaded
        assert_eq!(app.state.checks.len(), 2, "Should load 2 checks");
        assert!(
            app.state.checks.iter().any(|c| c.name == "security-audit"),
            "Should have security-audit check"
        );
        assert!(
            app.state.checks.iter().any(|c| c.name == "license-check"),
            "Should have license-check check"
        );

        // Verify all checks start unapproved
        for check in &app.state.checks {
            assert!(!check.approved, "Checks should start unapproved");
        }

        // Verify initial section is Commands
        assert_eq!(
            app.state.current_section,
            Section::Commands,
            "Should start with Commands section"
        );
    }

    #[tokio::test]
    async fn test_approve_commands_then_checks_section_transition() {
        // Test the section transition from Commands to Checks during approval flow
        let temp_dir = TempDir::new().unwrap();
        let config_dir = temp_dir.path().join(".config");
        std::fs::create_dir(&config_dir).unwrap();

        let config_content = r#"
[commands]
lint = ["echo", "lint"]

[[checks]]
name = "security"
command = ["echo", "security"]
"#;

        std::fs::write(config_dir.join("tools.toml"), config_content).unwrap();

        let mut app = ApprovalApp::new(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        // Start at command review
        app.state.screen = Screen::CommandReview;
        app.state.current_section = Section::Commands;
        app.state.current_item_index = 0;

        // Approve the command
        app.approve_current_and_advance();

        // Should transition to Checks section
        assert_eq!(
            app.state.current_section,
            Section::Checks,
            "Should transition to Checks section after last command"
        );
        assert_eq!(app.state.current_item_index, 0, "Should reset index to 0");
        assert_eq!(
            app.state.screen,
            Screen::CommandReview,
            "Should stay in CommandReview for checks"
        );

        // Approve the check
        app.approve_current_and_advance();

        // Should transition to Summary
        assert_eq!(
            app.state.screen,
            Screen::Summary,
            "Should transition to Summary after last check"
        );
    }

    #[tokio::test]
    async fn test_save_approvals_requires_all_checks_approved() {
        // Test that save_approvals fails when checks are not approved
        let temp_dir = TempDir::new().unwrap();
        let _xdg_dir = TempDir::new().unwrap();
        std::env::set_var("XDG_CONFIG_HOME", _xdg_dir.path());

        let config_dir = temp_dir.path().join(".config");
        std::fs::create_dir(&config_dir).unwrap();

        let config_content = r#"
[commands]
lint = ["echo", "lint"]

[[checks]]
name = "security"
command = ["echo", "security"]
"#;

        std::fs::write(config_dir.join("tools.toml"), config_content).unwrap();

        let mut app = ApprovalApp::new(temp_dir.path().to_path_buf())
            .await
            .unwrap();

        // Approve only commands, not checks
        for cmd in &mut app.state.commands {
            cmd.approved = true;
        }

        // Should fail because checks not approved
        let result = app.save_approvals().await;
        assert!(result.is_err(), "Should fail when checks not approved");
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(
            err_msg.contains("security"),
            "Error should mention unapproved check"
        );

        // Now approve checks too
        for check in &mut app.state.checks {
            check.approved = true;
        }

        // Should succeed
        app.save_approvals()
            .await
            .expect("Should succeed when all items approved");
    }

    #[tokio::test]
    async fn test_approval_app_with_only_checks() {
        // Test that a project with only checks (no commands) loads correctly
        let temp_dir = TempDir::new().unwrap();
        let config_dir = temp_dir.path().join(".config");
        std::fs::create_dir(&config_dir).unwrap();

        let config_content = r#"
[commands]

[[checks]]
name = "security"
command = ["echo", "security"]

[[checks]]
name = "license"
command = ["echo", "license"]
"#;

        std::fs::write(config_dir.join("tools.toml"), config_content).unwrap();

        let app = ApprovalApp::new(temp_dir.path().to_path_buf())
            .await
            .expect("Should load checks-only config");

        assert_eq!(app.state.commands.len(), 0, "Should have no commands");
        assert_eq!(app.state.checks.len(), 2, "Should have 2 checks");
    }

    #[tokio::test]
    async fn test_approve_checks_when_no_commands() {
        use tempfile::TempDir;

        let _xdg_dir = TempDir::new().unwrap();
        std::env::set_var("XDG_CONFIG_HOME", _xdg_dir.path());

        let temp_dir = TempDir::new().unwrap();
        let config_dir = temp_dir.path().join(".config");
        std::fs::create_dir(&config_dir).unwrap();

        let config_content = r#"
[commands]

[[checks]]
name = "security-audit"
command = ["echo", "security"]

[[checks]]
name = "license-check"
command = ["echo", "license"]
"#;

        std::fs::write(config_dir.join("tools.toml"), config_content).unwrap();

        let mut app = ApprovalApp::new(temp_dir.path().to_path_buf())
            .await
            .expect("Should load checks-only config");

        // Verify no commands, only checks
        assert_eq!(app.state.commands.len(), 0);
        assert_eq!(app.state.checks.len(), 2);

        // Verify overview screen
        assert_eq!(app.state.screen, Screen::ProjectOverview);

        // Simulate pressing Enter to start approval (should go directly to Checks section)
        app.handle_overview_keys(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        // Should start at Checks section (not Commands since there are no commands)
        assert_eq!(app.state.current_section, Section::Checks);
        assert_eq!(app.state.screen, Screen::CommandReview);
        assert_eq!(app.state.current_item_index, 0);

        // Approve first check
        app.handle_command_review_keys(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(app.state.checks[0].approved);
        assert_eq!(app.state.current_item_index, 1);

        // Approve second check
        app.handle_command_review_keys(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(app.state.checks[1].approved);

        // Should transition to Summary (no Commands section to go to)
        assert_eq!(app.state.screen, Screen::Summary);

        // All checks should be approved
        assert!(app.state.checks.iter().all(|c| c.approved));
    }

    #[tokio::test]
    async fn test_check_with_relative_script_path() {
        use tempfile::TempDir;

        let _xdg_dir = TempDir::new().unwrap();
        std::env::set_var("XDG_CONFIG_HOME", _xdg_dir.path());

        let temp_dir = TempDir::new().unwrap();
        let config_dir = temp_dir.path().join(".config");
        std::fs::create_dir(&config_dir).unwrap();

        // Create a script in a subdirectory with relative path
        let scripts_dir = temp_dir.path().join("scripts");
        std::fs::create_dir(&scripts_dir).unwrap();
        let script_path = scripts_dir.join("custom-check.sh");
        std::fs::write(&script_path, "#!/bin/bash\necho 'checking'\n").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let permissions = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(&script_path, permissions).unwrap();
        }

        let config_content = r#"
[commands]

[[checks]]
name = "custom-check"
command = ["./scripts/custom-check.sh"]
"#
        .to_string();

        std::fs::write(config_dir.join("tools.toml"), &config_content).unwrap();

        let app = ApprovalApp::new(temp_dir.path().to_path_buf())
            .await
            .expect("Should load config with relative check path");

        assert_eq!(app.state.checks.len(), 1);
        let check = &app.state.checks[0];

        // Verify the check was loaded
        assert_eq!(check.name, "custom-check");

        // Verify paths were resolved relative to project directory
        assert!(
            check.canonical_path.starts_with(temp_dir.path()),
            "Canonical path should be within project directory"
        );
        assert!(
            check.canonical_path.ends_with("scripts/custom-check.sh"),
            "Should resolve to the script path"
        );

        // Verify hash was computed for the resolved file
        assert!(
            !check.binary_hash.is_empty(),
            "Binary hash should be computed"
        );

        // Verify script was detected
        assert!(check.is_script, "Should detect script from shebang");
    }

    #[tokio::test]
    async fn test_check_metadata_captured() {
        use tempfile::TempDir;

        let _xdg_dir = TempDir::new().unwrap();
        std::env::set_var("XDG_CONFIG_HOME", _xdg_dir.path());

        let temp_dir = TempDir::new().unwrap();
        let config_dir = temp_dir.path().join(".config");
        std::fs::create_dir(&config_dir).unwrap();

        // Create a writable script in the project
        let scripts_dir = temp_dir.path().join("scripts");
        std::fs::create_dir(&scripts_dir).unwrap();
        let writable_script = scripts_dir.join("writable-check.sh");
        std::fs::write(&writable_script, "#!/bin/bash\necho 'writable check'\n").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Make it writable and executable
            let permissions = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(&writable_script, permissions).unwrap();
        }

        let config_content = format!(
            r#"
[commands]

[[checks]]
name = "writable-check"
command = ["{}"]
"#,
            writable_script.display()
        );

        std::fs::write(config_dir.join("tools.toml"), &config_content).unwrap();

        let app = ApprovalApp::new(temp_dir.path().to_path_buf())
            .await
            .expect("Should load config with writable check");

        assert_eq!(app.state.checks.len(), 1);
        let check = &app.state.checks[0];

        // Verify metadata is captured correctly for checks
        assert_eq!(check.name, "writable-check");
        assert!(check.is_script, "Should detect as script from shebang");

        #[cfg(unix)]
        {
            assert!(check.is_writable, "Should detect as writable on Unix");
            assert!(
                check.is_in_project,
                "Should detect as in-project since it's in scripts/"
            );

            // For writable scripts, content should be captured
            assert!(
                check.script_contents.is_some(),
                "Writable script contents should be captured"
            );
            let contents = check.script_contents.as_ref().unwrap();
            assert!(
                contents.contains("writable check"),
                "Script contents should include the echo message"
            );
        }
    }
}
