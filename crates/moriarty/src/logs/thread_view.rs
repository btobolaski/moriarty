use ratatui::{prelude::*, widgets::StatefulWidget};
use tui_scrollview::ScrollViewState;

use super::parser::LogLine;

pub struct ThreadView {
    contents: Vec<LogLine>,
}

impl ThreadView {
    pub fn new(contents: Vec<LogLine>) -> Self {
        Self { contents }
    }
}

impl StatefulWidget for &ThreadView {
    type State = ScrollViewState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        use ratatui::widgets::{Paragraph, Wrap};
        use tui_scrollview::ScrollView;

        // Filter to only User and Assistant messages
        let content_to_render: Vec<_> = self
            .contents
            .iter()
            .filter(|item| matches!(item, LogLine::Assistant(_) | LogLine::User(_)))
            .collect();

        // Calculate content width (reserve space for scrollbar)
        let content_width = area.width.saturating_sub(1);

        // Calculate total height by measuring each item's debug output
        let mut total_height = 0u16;
        let mut rendered_items = Vec::new();

        for item in &content_to_render {
            let debug_text = format!("{:#?}", item);

            // Count lines, accounting for width-based wrapping
            let mut line_count = 0u16;
            for line in debug_text.lines() {
                let line_len = line.len() as u16;
                // Calculate how many visual lines this will take when wrapped
                let wrapped_lines = if line_len == 0 {
                    1
                } else {
                    (line_len + content_width - 1) / content_width
                };
                line_count = line_count.saturating_add(wrapped_lines);
            }

            let height = line_count.max(1);
            rendered_items.push((debug_text, height));
            total_height = total_height.saturating_add(height);
        }

        // Create scroll view with calculated size
        let mut scroll_view = ScrollView::new(ratatui::layout::Size::new(
            content_width,
            total_height.max(1),
        ));

        // Render each item into the scroll buffer
        let mut y_offset = 0u16;
        for (debug_text, height) in rendered_items {
            let paragraph = Paragraph::new(debug_text).wrap(Wrap { trim: false });

            scroll_view.render_widget(paragraph, Rect::new(0, y_offset, content_width, height));

            y_offset = y_offset.saturating_add(height);
        }

        // Render the scroll view to display the visible portion
        scroll_view.render(area, buf, state);
    }
}
