use crossterm::event::{KeyCode, KeyEvent};
use futures::StreamExt;
use miette::IntoDiagnostic;
use ratatui::{prelude::*, DefaultTerminal};
use tui_scrollview::ScrollViewState;

use crate::logs::{parser::LogLine, thread_view::ThreadView};

use super::event_bus::{input_stream, Event, UIEvent};

pub struct App {
    thread_view: ThreadView,
    scroll_state: ScrollViewState,
    should_quit: bool,
    current_scroll_y: u16,
    viewport_height: u16,
}

impl App {
    pub fn new(contents: Vec<LogLine>) -> Self {
        Self {
            thread_view: ThreadView::new(contents),
            scroll_state: ScrollViewState::default(),
            should_quit: false,
            current_scroll_y: 0,
            viewport_height: 0,
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
        match key.code {
            // Quit
            KeyCode::Char('q') | KeyCode::Esc => {
                self.should_quit = true;
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
            _ => {
                // Ignore other keys
            }
        }
    }

    fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        self.viewport_height = area.height;
        frame.render_stateful_widget(&mut self.thread_view, area, &mut self.scroll_state);
    }
}
