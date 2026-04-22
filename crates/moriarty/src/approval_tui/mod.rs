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
#[cfg(test)]
mod tests;

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
    fn render_frame(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
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
        Ok(())
    }

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
        // Canonicalize for config loading and binary resolution
        // (repository root detection will be done during approval save)
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

        // Load command and check information using canonical_dir for binary resolution
        let command_infos = Self::load_items_info(commands, &canonical_dir).await?;

        let check_items: Vec<(String, Vec<String>)> = checks
            .into_iter()
            .map(|check| (check.name, check.command))
            .collect();
        let check_infos = Self::load_items_info(check_items, &canonical_dir).await?;

        let tools_config_hash = crate::hashing::hash_string(&tools_config_content);

        Ok(Self {
            state: ApprovalState {
                // Store canonical_dir - repository root detection will happen in approve_project
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
        self.render_frame(&mut terminal)?;

        while !self.should_quit {
            if let Some(event) = event_stream.next().await {
                let event = event?;
                self.handle_event(event).await?;

                // Re-render after handling event
                self.render_frame(&mut terminal)?;
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

    /// Returns true if `key` was consumed by one of the common navigation bindings
    /// (j/k/Down/Up scroll, optionally PageDown/PageUp if `include_page` is set).
    fn handle_scroll_keys(&mut self, key: KeyEvent, include_page: bool) -> bool {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.scroll_state.scroll_down();
                true
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.scroll_state.scroll_up();
                true
            }
            KeyCode::PageDown if include_page => {
                self.scroll_state.scroll_page_down();
                true
            }
            KeyCode::PageUp if include_page => {
                self.scroll_state.scroll_page_up();
                true
            }
            _ => false,
        }
    }

    /// Cancel+quit on q/Esc, and on `'n'` when `also_n` is true.
    ///
    /// Command-review, warning, and summary screens pass `also_n = true`
    /// because `'n'` is their explicit "no" affirmation. The overview screen
    /// passes `false`: `'n'` has no meaning there and is silently ignored
    /// (it falls through to the `_ =>` arm of `handle_scroll_keys`, which
    /// returns `false` without acting on it).
    ///
    /// Returns true if the key was consumed.
    fn handle_cancel_key(&mut self, key: KeyEvent, also_n: bool) -> bool {
        let is_n = also_n && matches!(key.code, KeyCode::Char('n'));
        if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) || is_n {
            self.state.screen = Screen::Cancelled;
            self.should_quit = true;
            true
        } else {
            false
        }
    }

    fn handle_overview_keys(&mut self, key: KeyEvent) {
        if self.handle_cancel_key(key, false) {
            return;
        }
        match key.code {
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
            _ => {
                self.handle_scroll_keys(key, false);
            }
        }
    }

    fn handle_command_review_keys(&mut self, key: KeyEvent) {
        if self.handle_cancel_key(key, true) {
            return;
        }
        match key.code {
            KeyCode::Enter | KeyCode::Char('y') => {
                if let Some(current_item) = self.state.current_item() {
                    if current_item.is_in_project && current_item.is_writable {
                        self.state.screen = Screen::InProjectWarning;
                    } else {
                        self.approve_current_and_advance();
                    }
                }
            }
            _ => {
                self.handle_scroll_keys(key, true);
            }
        }
    }

    fn handle_warning_keys(&mut self, key: KeyEvent) {
        if self.handle_cancel_key(key, true) {
            return;
        }
        if matches!(key.code, KeyCode::Char('Y')) {
            self.approve_current_and_advance();
        }
    }

    async fn handle_summary_keys(&mut self, key: KeyEvent) {
        if self.handle_cancel_key(key, true) {
            return;
        }
        match key.code {
            KeyCode::Enter | KeyCode::Char('y') => {
                if let Err(e) = self.save_approvals().await {
                    self.error_message = Some(format!("Failed to save approvals: {}", e));
                    self.state.screen = Screen::Cancelled;
                } else {
                    self.state.screen = Screen::Approved;
                }
                self.should_quit = true;
            }
            _ => {
                self.handle_scroll_keys(key, false);
            }
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
            approvals.approve_project(project_dir, tools_config_hash, commands, checks)
        })
        .await
    }
}
