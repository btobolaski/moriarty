use ratatui::{prelude::*, widgets::StatefulWidget};
use tui_scrollview::ScrollViewState;

use super::parser::LogLine;

#[derive(Debug, Clone)]
enum RenderedMessage {
    User(String),
    Assistant(String),
}

pub struct ThreadView {
    // Cache of rendered messages (these never change)
    rendered_messages: Vec<RenderedMessage>,
    // Cache of heights (recalculated when width changes)
    heights: Vec<u16>,
    total_height: u16,
    cached_width: u16,
}

impl ThreadView {
    pub fn new(contents: Vec<LogLine>) -> Self {
        // Render message field using debug formatting
        let rendered_messages: Vec<RenderedMessage> = contents
            .iter()
            .filter_map(|item| match item {
                LogLine::User(user_msg) => Some(RenderedMessage::User(
                    format!("{:#?}", user_msg.message),
                )),
                LogLine::Assistant(assistant_msg) => Some(RenderedMessage::Assistant(
                    format!("{:#?}", assistant_msg.message),
                )),
                _ => None,
            })
            .collect();

        // Initial width - will be updated on first render
        let initial_width = 100;
        let (heights, total_height) = Self::calculate_heights(&rendered_messages, initial_width);

        Self {
            rendered_messages,
            heights,
            total_height,
            cached_width: initial_width,
        }
    }

    fn calculate_heights(rendered_messages: &[RenderedMessage], content_width: u16) -> (Vec<u16>, u16) {
        let mut heights = Vec::with_capacity(rendered_messages.len());
        let mut total_height = 0u16;

        for message in rendered_messages {
            let text = match message {
                RenderedMessage::User(text) | RenderedMessage::Assistant(text) => text,
            };

            // Count lines, accounting for width-based wrapping
            let mut line_count = 0u16;
            for line in text.lines() {
                let line_len = line.len() as u16;
                // Calculate how many visual lines this will take when wrapped
                let wrapped_lines = if line_len == 0 {
                    1
                } else {
                    line_len.div_ceil(content_width)
                };
                line_count = line_count.saturating_add(wrapped_lines);
            }

            // Add 1 line for the horizontal separator
            let height = line_count.max(1).saturating_add(1);
            heights.push(height);
            total_height = total_height.saturating_add(height);
        }

        (heights, total_height)
    }

    fn update_heights_if_needed(&mut self, content_width: u16) {
        if content_width != self.cached_width {
            let (heights, total_height) =
                Self::calculate_heights(&self.rendered_messages, content_width);
            self.heights = heights;
            self.total_height = total_height;
            self.cached_width = content_width;
        }
    }
}

impl StatefulWidget for &mut ThreadView {
    type State = ScrollViewState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        use ratatui::widgets::{Block, Paragraph, Wrap};
        use tui_scrollview::ScrollView;

        // Calculate content width (reserve space for scrollbar)
        let content_width = area.width.saturating_sub(1);

        // Update heights cache if width changed
        self.update_heights_if_needed(content_width);

        // Create scroll view with cached size
        let mut scroll_view = ScrollView::new(ratatui::layout::Size::new(
            content_width,
            self.total_height.max(1),
        ));

        // Render each message into the scroll buffer
        let mut y_offset = 0u16;
        for (message, &height) in self.rendered_messages.iter().zip(self.heights.iter()) {
            let (text, style) = match message {
                RenderedMessage::User(text) => (text, Style::default().fg(Color::Blue)),
                RenderedMessage::Assistant(text) => (text, Style::default()),
            };

            // Render the message text
            let message_height = height.saturating_sub(1);
            let paragraph = Paragraph::new(text.as_str())
                .wrap(Wrap { trim: false })
                .style(style);

            scroll_view.render_widget(paragraph, Rect::new(0, y_offset, content_width, message_height));

            // Render horizontal separator line
            let separator_y = y_offset.saturating_add(message_height);
            let separator = Block::default()
                .borders(ratatui::widgets::Borders::BOTTOM)
                .border_style(Style::default().fg(Color::DarkGray));

            scroll_view.render_widget(separator, Rect::new(0, separator_y, content_width, 1));

            y_offset = y_offset.saturating_add(height);
        }

        // Render the scroll view to display the visible portion
        scroll_view.render(area, buf, state);
    }
}
