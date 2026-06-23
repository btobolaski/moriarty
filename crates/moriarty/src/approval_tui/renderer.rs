//! Rendering logic for the approval TUI screens.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::*,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use tui_scrollview::{ScrollView, ScrollViewState};

use super::approval_state::{ApprovalState, Screen};

/// Split the area into the standard 3-part vertical layout: Title (3), Content (fill), Help (2).
fn standard_layout(area: Rect) -> [Rect; 3] {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Min(0),    // Content
            Constraint::Length(2), // Help
        ])
        .split(area);
    [chunks[0], chunks[1], chunks[2]]
}

/// Render a bordered title bar with the given text and color.
fn render_title(frame: &mut Frame, area: Rect, text: &str, color: Color) {
    let title = Paragraph::new(text.to_string())
        .block(Block::default().borders(Borders::ALL))
        .style(Style::default().fg(color).bold());
    frame.render_widget(title, area);
}

/// Render a help/status bar at the bottom.
fn render_help(frame: &mut Frame, area: Rect, text: &str, color: Color) {
    let help = Paragraph::new(text.to_string()).style(Style::default().fg(color));
    frame.render_widget(help, area);
}

/// Render a bold help/status bar — used for high-stakes screens (e.g., security warnings)
/// where the help text must be visually prominent.
fn render_help_bold(frame: &mut Frame, area: Rect, text: &str, color: Color) {
    let help = Paragraph::new(text.to_string()).style(Style::default().fg(color).bold());
    frame.render_widget(help, area);
}

/// Render scrollable content in the given area.
fn render_scrollable_content(
    frame: &mut Frame,
    area: Rect,
    scroll_state: &mut ScrollViewState,
    content: &str,
    content_height: u16,
) {
    let width = area.width.saturating_sub(2);
    let height = content_height.max(area.height.saturating_sub(2));
    let mut scroll_view = ScrollView::new(Size::new(width, height));

    scroll_view.render_widget(
        Paragraph::new(content.to_string()).wrap(Wrap { trim: false }),
        Rect::new(0, 0, width, height),
    );

    frame.render_stateful_widget(scroll_view, area, scroll_state);
}

/// Render the defensive "Error: Empty Section" fallback for screens that require a current item.
fn render_empty_section_error(frame: &mut Frame, chunks: &[Rect; 3]) {
    render_title(frame, chunks[0], "Error: Empty Section", Color::Red);

    let content = Paragraph::new(
        "No items in this section. This is a bug in the navigation logic.\n\
        Press q to cancel and report this issue.",
    )
    .wrap(Wrap { trim: false })
    .style(Style::default().fg(Color::Red));
    frame.render_widget(content, chunks[1]);

    render_help(frame, chunks[2], "q to cancel", Color::Red);
}

/// Main render function that dispatches to screen-specific renderers
pub fn render(
    state: &ApprovalState,
    scroll_state: &mut ScrollViewState,
    frame: &mut Frame,
    error_message: &Option<String>,
) {
    match state.screen {
        Screen::ProjectOverview => render_project_overview(state, scroll_state, frame),
        Screen::CommandReview => render_command_review(state, scroll_state, frame),
        Screen::InProjectWarning => render_in_project_warning(state, frame),
        Screen::Summary => render_summary(state, scroll_state, frame),
        Screen::Approved => render_approved(state, frame),
        Screen::Cancelled => render_cancelled(frame, error_message),
    }
}

fn render_project_overview(
    state: &ApprovalState,
    scroll_state: &mut ScrollViewState,
    frame: &mut Frame,
) {
    let chunks = standard_layout(frame.area());

    render_title(frame, chunks[0], "Project Tools Approval", Color::Cyan);

    let total_items = state.commands.len() + state.checks.len();
    let content_height = total_items as u16 * 2 + 15;

    let mut content = format!("Project: {}\n\n", state.project_dir.display());

    if !state.commands.is_empty() {
        content.push_str(&format!(
            "Commands ({}):\n\n{}\n\n",
            state.commands.len(),
            state
                .commands
                .iter()
                .map(|cmd| format!("  • {} → {}", cmd.name, cmd.command_array.join(" ")))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    if !state.checks.is_empty() {
        content.push_str(&format!(
            "Checks ({}):\n\n{}\n\n",
            state.checks.len(),
            state
                .checks
                .iter()
                .map(|check| format!("  • {} → {}", check.name, check.command_array.join(" ")))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    content.push_str(
        "These tools will have access to:\n\
        - Read/write access to the project directory\n\
        - Full filesystem access with your user permissions\n\
        - Network access\n\
        \n\
        Press Enter to review each item, or q to cancel.",
    );

    render_scrollable_content(frame, chunks[1], scroll_state, &content, content_height);
    render_help(
        frame,
        chunks[2],
        "↑/k up | ↓/j down | Enter approve | q cancel",
        Color::DarkGray,
    );
}

fn render_command_review(
    state: &ApprovalState,
    scroll_state: &mut ScrollViewState,
    frame: &mut Frame,
) {
    let chunks = standard_layout(frame.area());

    let Some(current) = state.current_item() else {
        render_empty_section_error(frame, &chunks);
        return;
    };

    let (section_label, current_num, total_num) = match state.current_section {
        super::approval_state::Section::Commands => (
            "Command",
            state.current_item_index + 1,
            state.commands.len(),
        ),
        super::approval_state::Section::Checks => {
            ("Check", state.current_item_index + 1, state.checks.len())
        }
    };

    render_title(
        frame,
        chunks[0],
        &format!(
            "{} Review ({}/{}): {}",
            section_label, current_num, total_num, current.name
        ),
        Color::Cyan,
    );

    let mut content_lines: u16 = 15;
    if current.is_script
        && let Some(contents) = &current.script_contents
    {
        content_lines += contents.lines().count() as u16 + 3;
    }

    let mut content = format!(
        "Command: {}\n\
        \n\
        Binary: {}\n\
        Hash: {}\n\
        \n",
        current.command_array.join(" "),
        current.canonical_path.display(),
        current.binary_hash,
    );

    let mut warnings = Vec::new();
    if current.is_in_project {
        warnings.push("⚠ WARNING: Executable is INSIDE the project directory");
    }
    if current.is_writable {
        warnings.push("⚠ WARNING: Executable is WRITABLE by you");
    }
    if current.is_script {
        warnings.push("ℹ INFO: This is a script (has shebang)");
    }

    if !warnings.is_empty() {
        content.push('\n');
        for warning in warnings {
            content.push_str(&format!("{}\n", warning));
        }
        content.push('\n');
    }

    if let Some(script_contents) = &current.script_contents {
        content.push_str("Script contents:\n");
        content.push_str("────────────────────────────────────────\n");
        content.push_str(script_contents);
        content.push_str("\n────────────────────────────────────────\n");
    }

    content.push_str("\nDo you approve this command?\n");

    render_scrollable_content(frame, chunks[1], scroll_state, &content, content_lines);
    render_help(
        frame,
        chunks[2],
        "↑/k up | ↓/j down | PgUp/PgDn page | y approve | n/q cancel",
        Color::DarkGray,
    );
}

fn render_in_project_warning(state: &ApprovalState, frame: &mut Frame) {
    let chunks = standard_layout(frame.area());

    let Some(current) = state.current_item() else {
        render_empty_section_error(frame, &chunks);
        return;
    };

    render_title(frame, chunks[0], "⚠⚠⚠ SECURITY WARNING ⚠⚠⚠", Color::Red);

    let content = format!(
        "This executable is INSIDE the project directory AND writable by you:\n\
        \n\
        {}\n\
        \n\
        This means:\n\
        • The executable can be modified by anyone with write access to this project\n\
        • Git clones, file syncs, or malicious changes could replace this file\n\
        • Approving this is a SIGNIFICANT SECURITY RISK\n\
        \n\
        Only approve if:\n\
        • You trust everyone with write access to this project directory\n\
        • You understand the executable contents and have reviewed them\n\
        • This is a legitimate project-specific tool\n\
        \n\
        This requires CAPITAL Y to confirm you understand the risks.\n\
        Press n to cancel (recommended).",
        current.canonical_path.display()
    );

    let paragraph = Paragraph::new(content)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::Red));
    frame.render_widget(paragraph, chunks[1]);

    render_help_bold(
        frame,
        chunks[2],
        "CAPITAL Y (Shift+Y) to confirm | n/q/Esc to cancel",
        Color::Red,
    );
}

fn render_summary(state: &ApprovalState, scroll_state: &mut ScrollViewState, frame: &mut Frame) {
    let chunks = standard_layout(frame.area());

    render_title(frame, chunks[0], "Approval Summary", Color::Green);

    let total_items = state.commands.len() + state.checks.len();
    let content_height = total_items as u16 * 4 + 15;

    let mut content = format!(
        "You have reviewed and approved all items for:\n\
        {}\n\n",
        state.project_dir.display()
    );

    if !state.commands.is_empty() {
        let command_list = state
            .commands
            .iter()
            .map(|cmd| {
                format!(
                    "✓ {} → {}\n  Binary: {}\n  Hash: {}",
                    cmd.name,
                    cmd.command_array.join(" "),
                    cmd.canonical_path.display(),
                    cmd.binary_hash
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        content.push_str(&format!("Approved commands:\n{}\n\n", command_list));
    }

    if !state.checks.is_empty() {
        let check_list = state
            .checks
            .iter()
            .map(|check| {
                format!(
                    "✓ {} → {}\n  Binary: {}\n  Hash: {}",
                    check.name,
                    check.command_array.join(" "),
                    check.canonical_path.display(),
                    check.binary_hash
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        content.push_str(&format!("Approved checks:\n{}\n\n", check_list));
    }

    content.push_str(
        "These approvals will be saved to:\n\
        ~/.config/moriarty/project_approvals.toml\n\
        \n\
        Press Enter to save and complete approval, or q to cancel.",
    );

    render_scrollable_content(frame, chunks[1], scroll_state, &content, content_height);
    render_help(
        frame,
        chunks[2],
        "↑/k up | ↓/j down | Enter save & approve | q cancel",
        Color::DarkGray,
    );
}

fn render_approved(state: &ApprovalState, frame: &mut Frame) {
    let chunks = standard_layout(frame.area());

    render_title(frame, chunks[0], "✓ Approval Complete", Color::Green);

    let total_items = state.commands.len() + state.checks.len();
    let content = format!(
        "Successfully approved project tools for:\n\
        {}\n\
        \n\
        {} item(s) have been approved and saved ({} commands, {} checks).\n\
        \n\
        The MCP server will now execute these tools when requested by Claude.",
        state.project_dir.display(),
        total_items,
        state.commands.len(),
        state.checks.len()
    );

    let paragraph = Paragraph::new(content)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::Green));
    frame.render_widget(paragraph, chunks[1]);

    render_help(frame, chunks[2], "Press any key to exit", Color::DarkGray);
}

fn render_cancelled(frame: &mut Frame, error_message: &Option<String>) {
    let chunks = standard_layout(frame.area());

    render_title(frame, chunks[0], "✗ Approval Cancelled", Color::Yellow);

    let mut content = String::from("Approval process cancelled.\n\n");

    if let Some(error) = error_message {
        content.push_str(&format!("Error: {}\n\n", error));
    }

    content.push_str("No changes have been made to your approvals.\n\n");
    content.push_str("The MCP server will not execute project tools until they are approved.");

    let paragraph = Paragraph::new(content)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::Yellow));
    frame.render_widget(paragraph, chunks[1]);

    render_help(frame, chunks[2], "Press any key to exit", Color::DarkGray);
}
