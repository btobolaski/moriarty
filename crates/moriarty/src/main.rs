use std::io;

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::Text,
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal, TerminalOptions, Viewport,
};

mod event_bus;

#[derive(Clone, Copy, PartialEq)]
enum AppMode {
    Inline,
    Fullscreen,
}

struct App {
    input: String,
    message_count: usize,
    mode: AppMode,
    input_height: u16,
}

impl Default for App {
    fn default() -> App {
        App {
            input: String::new(),
            message_count: 0,
            mode: AppMode::Inline,
            input_height: 3, // Default minimum height
        }
    }
}

impl App {
    fn on_key(&mut self, key: KeyCode) -> Option<String> {
        match key {
            KeyCode::Enter => {
                if !self.input.is_empty() {
                    self.message_count += 1;
                    let message = format!("[Message {}] {}", self.message_count, self.input);
                    self.input.clear();
                    Some(message)
                } else {
                    None
                }
            }
            KeyCode::Char(c) => {
                self.input.push(c);
                None
            }
            KeyCode::Backspace => {
                self.input.pop();
                None
            }
            _ => None,
        }
    }
}

fn ui_inline(frame: &mut Frame, app: &App) {
    let size = frame.area();

    // Create layout with input area - use the full available space
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1)].as_ref())
        .split(size);

    // Input widget with wrap for longer text when area is taller
    let input = Paragraph::new(Text::from(app.input.as_str()))
        .style(Style::default().fg(Color::Yellow))
        .wrap(ratatui::widgets::Wrap { trim: true })
        .block(Block::default().borders(Borders::ALL).title(format!(
            "Input ({}h) - Enter to send, Tab for About, Alt+/- to resize",
            app.input_height
        )));

    frame.render_widget(input, chunks[0]);
}

fn ui_fullscreen(frame: &mut Frame, _app: &App) {
    let size = frame.area();

    // Create centered about widget
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Percentage(30),
                Constraint::Min(5),
                Constraint::Percentage(30),
            ]
            .as_ref(),
        )
        .split(size);

    let horizontal_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            [
                Constraint::Percentage(20),
                Constraint::Min(20),
                Constraint::Percentage(20),
            ]
            .as_ref(),
        )
        .split(chunks[1]);

    let about = Paragraph::new(Text::from(
        "About\n\nMoriarty Terminal App\n\nA demonstration of ratatui's inline viewport feature.",
    ))
    .style(Style::default().fg(Color::White))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("About (Press Escape to return)"),
    );

    frame.render_widget(about, horizontal_chunks[1]);
}

fn run_inline_mode(app: &mut App) -> Result<bool, Box<dyn std::error::Error>> {
    let mut current_height = app.input_height;

    // Create initial terminal
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(current_height),
        },
    )?;

    loop {
        // Check if we need to recreate terminal with new height
        if current_height != app.input_height {
            terminal.clear()?;
            current_height = app.input_height;

            // Recreate terminal with new height
            let backend = CrosstermBackend::new(io::stdout());
            terminal = Terminal::with_options(
                backend,
                TerminalOptions {
                    viewport: Viewport::Inline(current_height),
                },
            )?;
        }

        terminal.draw(|f| ui_inline(f, app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                        terminal.clear()?;
                        return Ok(false); // Exit app
                    }
                    KeyCode::Tab => {
                        terminal.clear()?;
                        app.mode = AppMode::Fullscreen;
                        return Ok(true); // Switch to fullscreen
                    }
                    KeyCode::Char('+') if key.modifiers.contains(event::KeyModifiers::ALT) => {
                        app.input_height += 1;
                    }
                    KeyCode::Char('-') if key.modifiers.contains(event::KeyModifiers::ALT) => {
                        if app.input_height > 3 {
                            // Minimum height is 3
                            app.input_height -= 1;
                        }
                    }
                    _ => {
                        if let Some(message) = app.on_key(key.code) {
                            terminal.insert_before(1, |buf| {
                                use ratatui::text::Line;
                                let line = Line::from(message);
                                buf.set_line(0, 0, &line, buf.area.width);
                            })?;
                        }
                    }
                }
            }
        }
    }
}

fn run_fullscreen_mode(app: &mut App) -> Result<bool, Box<dyn std::error::Error>> {
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    loop {
        terminal.draw(|f| ui_fullscreen(f, app))?;

        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    KeyCode::Esc => {
                        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
                        app.mode = AppMode::Inline;
                        return Ok(true); // Switch back to inline
                    }
                    KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
                        return Ok(false); // Exit app
                    }
                    _ => {}
                }
            }
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Print some initial content to demonstrate scrolling
    println!("=== Moriarty Terminal App Demo ===");
    println!("This content is in the normal terminal buffer.");
    println!("You can scroll up to see this text even while the app is running.");
    println!("The UI below will stay fixed at the bottom.");
    println!("");

    enable_raw_mode()?;
    let mut app = App::default();

    loop {
        let continue_running = match app.mode {
            AppMode::Inline => run_inline_mode(&mut app)?,
            AppMode::Fullscreen => run_fullscreen_mode(&mut app)?,
        };

        if !continue_running {
            break;
        }
    }

    disable_raw_mode()?;
    println!("App exited. Messages sent: {}", app.message_count);
    Ok(())
}
