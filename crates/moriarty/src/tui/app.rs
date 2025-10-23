use crossterm::event::{KeyCode, KeyEvent};
use futures::StreamExt;
use miette::IntoDiagnostic;
use ratatui::{prelude::*, DefaultTerminal};
use tui_scrollview::ScrollViewState;

use crate::logs::{message_modal::MessageModal, parser::LogLine, thread_view::ThreadView};

use super::event_bus::{input_stream, Event, UIEvent};

pub struct App {
    thread_view: ThreadView,
    scroll_state: ScrollViewState,
    should_quit: bool,
    current_scroll_y: u16,
    viewport_height: u16,
    modal_open: bool,
    modal_scroll_state: ScrollViewState,
}

impl App {
    pub fn new(contents: Vec<LogLine>) -> Self {
        Self {
            thread_view: ThreadView::new(contents),
            scroll_state: ScrollViewState::default(),
            should_quit: false,
            current_scroll_y: 0,
            viewport_height: 0,
            modal_open: false,
            modal_scroll_state: ScrollViewState::default(),
        }
    }

    pub async fn run(mut self, mut terminal: DefaultTerminal) -> miette::Result<()> {
        let mut event_stream = input_stream();

        // Initial render
        terminal
            .draw(|frame| self.render(frame))
            .into_diagnostic()?;

        while !self.should_quit {
            if let Some(event) = event_stream.next().await {
                let event = event?;
                self.handle_event(event)?;

                // Re-render after handling event
                terminal
                    .draw(|frame| self.render(frame))
                    .into_diagnostic()?;
            }
        }

        Ok(())
    }

    fn handle_event(&mut self, event: Event) -> miette::Result<()> {
        match event {
            Event::UI(ui_event) => match ui_event {
                UIEvent::Key(key) => self.handle_key(key),
                UIEvent::Render => {
                    // Render is handled in the event loop
                }
                UIEvent::Paste(_) => {
                    // Ignore paste events for now
                }
            },
        }
        Ok(())
    }

    /// Ensures the selected message is visible by auto-scrolling the viewport.
    ///
    /// The auto-scroll behavior is asymmetric to provide a better UX:
    /// - When scrolling up: Only the top of the message needs to be visible, providing
    ///   context while keeping the view stable.
    /// - When scrolling down: The entire message is brought into view because the user
    ///   is moving forward through content and expects to see the complete message.
    ///
    /// Note: This method tracks scroll position independently from ScrollViewState
    /// because the library doesn't expose its internal position. The tracking is
    /// approximate for page up/down operations, which may cause minor synchronization
    /// drift but is corrected during single-line navigation.
    fn ensure_selection_visible(&mut self) {
        if self.viewport_height == 0 {
            return; // Not rendered yet
        }

        let selected_y = self.thread_view.get_selected_y_offset();
        let selected_height = self.thread_view.get_selected_height();
        let selected_bottom = selected_y.saturating_add(selected_height);
        let viewport_bottom = self.current_scroll_y.saturating_add(self.viewport_height);

        // Selection is above viewport: scroll up to show the top
        if selected_y < self.current_scroll_y {
            let scroll_amount = self.current_scroll_y.saturating_sub(selected_y);
            // ScrollViewState doesn't support multi-line scrolling in a single call,
            // so we must scroll one line at a time
            for _ in 0..scroll_amount {
                self.scroll_state.scroll_up();
            }
            self.current_scroll_y = selected_y;
        }
        // Selection is below viewport: scroll down to show the entire message
        else if selected_bottom > viewport_bottom {
            let scroll_amount = selected_bottom.saturating_sub(viewport_bottom);
            // ScrollViewState doesn't support multi-line scrolling in a single call,
            // so we must scroll one line at a time
            for _ in 0..scroll_amount {
                self.scroll_state.scroll_down();
            }
            self.current_scroll_y = self.current_scroll_y.saturating_add(scroll_amount);
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if self.modal_open {
            self.handle_modal_keys(key);
        } else {
            self.handle_main_view_keys(key);
        }
    }

    fn handle_modal_keys(&mut self, key: KeyEvent) {
        match key.code {
            // Close modal
            KeyCode::Esc | KeyCode::Char('q') => {
                self.modal_open = false;
                self.modal_scroll_state = ScrollViewState::default();
            }
            // Scroll modal down
            KeyCode::Char('j') | KeyCode::Down => {
                self.modal_scroll_state.scroll_down();
            }
            // Scroll modal up
            KeyCode::Char('k') | KeyCode::Up => {
                self.modal_scroll_state.scroll_up();
            }
            // Page down in modal
            KeyCode::Char('f') | KeyCode::PageDown => {
                self.modal_scroll_state.scroll_page_down();
            }
            // Page up in modal
            KeyCode::Char('b') | KeyCode::PageUp => {
                self.modal_scroll_state.scroll_page_up();
            }
            // Jump to top of modal
            KeyCode::Char('g') | KeyCode::Home => {
                self.modal_scroll_state.scroll_to_top();
            }
            // Jump to bottom of modal
            KeyCode::Char('G') | KeyCode::End => {
                self.modal_scroll_state.scroll_to_bottom();
            }
            _ => {}
        }
    }

    fn handle_main_view_keys(&mut self, key: KeyEvent) {
        match key.code {
            // Quit
            KeyCode::Char('q') | KeyCode::Esc => {
                self.should_quit = true;
            }
            // Open modal for selected message
            KeyCode::Enter => {
                if self.thread_view.get_message_count() > 0 {
                    self.modal_open = true;
                }
            }
            // Move selection down
            KeyCode::Char('j') | KeyCode::Down => {
                self.thread_view.select_next();
                self.ensure_selection_visible();
            }
            // Move selection up
            KeyCode::Char('k') | KeyCode::Up => {
                self.thread_view.select_previous();
                self.ensure_selection_visible();
            }
            // Page down
            KeyCode::Char('f') | KeyCode::PageDown => {
                self.scroll_state.scroll_page_down();
                // Update tracked scroll position (approximate)
                self.current_scroll_y = self.current_scroll_y.saturating_add(self.viewport_height);
            }
            // Page up
            KeyCode::Char('b') | KeyCode::PageUp => {
                self.scroll_state.scroll_page_up();
                // Update tracked scroll position (approximate)
                self.current_scroll_y = self.current_scroll_y.saturating_sub(self.viewport_height);
            }
            // Jump to first message
            KeyCode::Char('g') | KeyCode::Home => {
                self.thread_view.select_first();
                self.scroll_state.scroll_to_top();
                self.current_scroll_y = 0;
            }
            // Jump to last message (Shift+G)
            KeyCode::Char('G') | KeyCode::End => {
                self.thread_view.select_last();
                self.scroll_state.scroll_to_bottom();
                // Update scroll position to be at bottom (will be corrected on next render)
                self.current_scroll_y = u16::MAX;
            }
            _ => {}
        }
    }

    fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        self.viewport_height = area.height;

        // Always render the thread view
        frame.render_stateful_widget(&mut self.thread_view, area, &mut self.scroll_state);

        // If modal is open, render it on top
        if self.modal_open {
            if let Some(message_text) = self.thread_view.get_selected_message() {
                let modal = MessageModal::new(message_text);
                frame.render_stateful_widget(modal, area, &mut self.modal_scroll_state);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logs::parser::{LogMessage, LogMessageContent, UserLogLine};
    use chrono::Utc;
    use uuid::Uuid;

    fn create_test_user_message(text: &str) -> LogLine {
        LogLine::User(UserLogLine {
            parent_uuid: None,
            is_sidechain: false,
            user_type: "test".to_string(),
            cwd: "/test".to_string(),
            session_id: Uuid::new_v4(),
            version: "1.0.0".to_string(),
            git_branch: "main".to_string(),
            message: LogMessage {
                role: "user".to_string(),
                content: LogMessageContent::String(text.to_string()),
            },
            is_meta: None,
            uuid: Uuid::new_v4(),
            timestamp: Utc::now(),
            tool_use_result: None,
            thinking_metadata: None,
            is_visible_in_transcript_only: None,
            is_compact_summary: None,
        })
    }

    #[test]
    fn test_new_app_modal_closed() {
        let messages = vec![create_test_user_message("Test")];
        let app = App::new(messages);

        assert!(!app.modal_open, "Modal should be closed initially");
    }

    #[test]
    fn test_handle_key_enter_opens_modal() {
        let messages = vec![create_test_user_message("Test")];
        let mut app = App::new(messages);

        app.handle_key(KeyEvent::from(KeyCode::Enter));

        assert!(
            app.modal_open,
            "Enter key should open modal when messages exist"
        );
    }

    #[test]
    fn test_handle_key_enter_no_messages() {
        let mut app = App::new(vec![]);

        app.handle_key(KeyEvent::from(KeyCode::Enter));

        assert!(
            !app.modal_open,
            "Enter key should not open modal when no messages"
        );
    }

    #[test]
    fn test_handle_key_esc_closes_modal() {
        let messages = vec![create_test_user_message("Test")];
        let mut app = App::new(messages);

        app.modal_open = true;

        app.handle_key(KeyEvent::from(KeyCode::Esc));

        assert!(!app.modal_open, "Esc should close modal");
    }

    #[test]
    fn test_handle_key_q_closes_modal() {
        let messages = vec![create_test_user_message("Test")];
        let mut app = App::new(messages);

        app.modal_open = true;

        app.handle_key(KeyEvent::from(KeyCode::Char('q')));

        assert!(!app.modal_open, "'q' should close modal when modal is open");
    }

    #[test]
    fn test_handle_key_q_quits_when_modal_closed() {
        let messages = vec![create_test_user_message("Test")];
        let mut app = App::new(messages);

        app.handle_key(KeyEvent::from(KeyCode::Char('q')));

        assert!(app.should_quit, "'q' should quit app when modal is closed");
    }

    #[test]
    fn test_modal_blocks_main_navigation() {
        let messages = vec![
            create_test_user_message("First"),
            create_test_user_message("Second"),
        ];
        let mut app = App::new(messages);

        let initial_index = app.thread_view.get_selected_index();

        app.modal_open = true;

        app.handle_key(KeyEvent::from(KeyCode::Char('j')));

        assert_eq!(
            app.thread_view.get_selected_index(),
            initial_index,
            "Main view selection should not change when modal is open"
        );
    }
}
