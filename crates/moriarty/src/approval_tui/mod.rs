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
use std::path::PathBuf;
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

use approval_state::{ApprovalState, CommandInfo, Screen};

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

        if commands.is_empty() {
            return Err(miette::miette!(
                "No commands configured in {}",
                tools_config_path.display()
            ));
        }

        // Load command information
        let mut command_infos = Vec::new();
        for (name, command_array) in commands {
            let binary_name = &command_array[0];
            let (original_path, canonical_path) =
                resolve_binary_path_with_original(binary_name, &canonical_dir)?;

            let is_script_file = is_script(&canonical_path).await?;
            let is_writable_file = is_writable(&canonical_path).await?;
            let script_contents = if is_script_file && is_writable_file {
                Some(read_script_contents(&canonical_path).await?)
            } else {
                None
            };

            let binary_hash = crate::hashing::hash_file(&canonical_path).await?;

            command_infos.push(CommandInfo {
                name: name.clone(),
                command_array: command_array.clone(),
                original_path: original_path.clone(),
                canonical_path: canonical_path.clone(),
                binary_hash: binary_hash.clone(),
                is_script: is_script_file,
                is_writable: is_writable_file,
                is_in_project: is_within_project(&canonical_path, &canonical_dir),
                script_contents,
                approved: false,
            });
        }

        let tools_config_hash = crate::hashing::hash_string(&tools_config_content);

        Ok(Self {
            state: ApprovalState {
                project_dir: canonical_dir,
                tools_config_hash,
                commands: command_infos,
                current_command_index: 0,
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
                self.state.current_command_index = 0;
                self.state.screen = Screen::CommandReview;
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
                let current_command = &self.state.commands[self.state.current_command_index];
                if current_command.is_in_project && current_command.is_writable {
                    self.state.screen = Screen::InProjectWarning;
                } else {
                    self.approve_current_and_advance();
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
        self.state.commands[self.state.current_command_index].approved = true;
        self.scroll_state = ScrollViewState::default();

        if self.state.current_command_index + 1 < self.state.commands.len() {
            self.state.current_command_index += 1;
            self.state.screen = Screen::CommandReview;
        } else {
            self.state.screen = Screen::Summary;
        }
    }

    async fn save_approvals(&self) -> Result<()> {
        // Verify all commands are approved before saving
        let unapproved: Vec<&str> = self
            .state
            .commands
            .iter()
            .filter(|cmd| !cmd.approved)
            .map(|cmd| cmd.name.as_str())
            .collect();

        if !unapproved.is_empty() {
            return Err(miette::miette!(
                "Cannot save approvals: {} command(s) not approved: {}",
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

        let project_dir = self.state.project_dir.clone();
        let tools_config_hash = self.state.tools_config_hash.clone();

        // Atomically update approvals with file locking
        ProjectApprovals::update(move |approvals| {
            approvals.approve_project(project_dir, tools_config_hash, commands);
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(app.state.current_command_index, 0);
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
            err_msg.contains("No commands configured"),
            "Error should mention no commands configured"
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
        app.state.current_command_index = 0;

        // Approve first command
        assert!(!app.state.commands[0].approved);
        app.approve_current_and_advance();
        assert!(
            app.state.commands[0].approved,
            "First command should be approved"
        );
        assert_eq!(
            app.state.current_command_index, 1,
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
}
