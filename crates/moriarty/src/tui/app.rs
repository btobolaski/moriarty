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
}

impl App {
    pub fn new(contents: Vec<LogLine>) -> Self {
        Self {
            thread_view: ThreadView::new(contents),
            scroll_state: ScrollViewState::default(),
            should_quit: false,
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

    fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            // Quit
            KeyCode::Char('q') | KeyCode::Esc => {
                self.should_quit = true;
            }
            // Scroll down
            KeyCode::Char('j') | KeyCode::Down => {
                self.scroll_state.scroll_down();
            }
            // Scroll up
            KeyCode::Char('k') | KeyCode::Up => {
                self.scroll_state.scroll_up();
            }
            // Page down
            KeyCode::Char('f') | KeyCode::PageDown => {
                self.scroll_state.scroll_page_down();
            }
            // Page up
            KeyCode::Char('b') | KeyCode::PageUp => {
                self.scroll_state.scroll_page_up();
            }
            // Scroll to top
            KeyCode::Char('g') | KeyCode::Home => {
                self.scroll_state.scroll_to_top();
            }
            // Scroll to bottom (Shift+G)
            KeyCode::Char('G') => {
                self.scroll_state.scroll_to_bottom();
            }
            KeyCode::End => {
                self.scroll_state.scroll_to_bottom();
            }
            _ => {
                // Ignore other keys
            }
        }
    }

    fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        frame.render_stateful_widget(&self.thread_view, area, &mut self.scroll_state);
    }
}
