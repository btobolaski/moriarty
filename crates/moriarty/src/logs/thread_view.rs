use ratatui::{prelude::*, widgets::StatefulWidget};
use tui_scrollview::ScrollViewState;

use super::parser::LogLine;

pub struct ThreadView {
    // Cache of rendered debug strings (these never change)
    rendered_texts: Vec<String>,
    // Cache of heights (recalculated when width changes)
    heights: Vec<u16>,
    total_height: u16,
    cached_width: u16,
}

impl ThreadView {
    pub fn new(contents: Vec<LogLine>) -> Self {
        // Pre-render the debug text (this is expensive and never changes)
        let rendered_texts: Vec<String> = contents
            .iter()
            .filter(|item| matches!(item, LogLine::Assistant(_) | LogLine::User(_)))
            .map(|item| format!("{:#?}", item))
            .collect();

        // Initial width - will be updated on first render
        let initial_width = 100;
        let (heights, total_height) = Self::calculate_heights(&rendered_texts, initial_width);

        Self {
            rendered_texts,
            heights,
            total_height,
            cached_width: initial_width,
        }
    }

    fn calculate_heights(rendered_texts: &[String], content_width: u16) -> (Vec<u16>, u16) {
        let mut heights = Vec::with_capacity(rendered_texts.len());
        let mut total_height = 0u16;

        for debug_text in rendered_texts {
            // Count lines, accounting for width-based wrapping
            let mut line_count = 0u16;
            for line in debug_text.lines() {
                let line_len = line.len() as u16;
                // Calculate how many visual lines this will take when wrapped
                let wrapped_lines = if line_len == 0 {
                    1
                } else {
                    line_len.div_ceil(content_width)
                };
                line_count = line_count.saturating_add(wrapped_lines);
            }

            let height = line_count.max(1);
            heights.push(height);
            total_height = total_height.saturating_add(height);
        }

        (heights, total_height)
    }

    fn update_heights_if_needed(&mut self, content_width: u16) {
        if content_width != self.cached_width {
            let (heights, total_height) =
                Self::calculate_heights(&self.rendered_texts, content_width);
            self.heights = heights;
            self.total_height = total_height;
            self.cached_width = content_width;
        }
    }
}

impl StatefulWidget for &mut ThreadView {
    type State = ScrollViewState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        use ratatui::widgets::{Paragraph, Wrap};
        use tui_scrollview::ScrollView;

        // Calculate content width (reserve space for scrollbar)
        let content_width = area.width.saturating_sub(1);

        // Update heights cache if width changed (only recalculates heights, not debug text)
        self.update_heights_if_needed(content_width);

        // Create scroll view with cached size
        let mut scroll_view = ScrollView::new(ratatui::layout::Size::new(
            content_width,
            self.total_height.max(1),
        ));

        // Render each item into the scroll buffer using cached data
        let mut y_offset = 0u16;
        for (debug_text, &height) in self.rendered_texts.iter().zip(self.heights.iter()) {
            let paragraph = Paragraph::new(debug_text.as_str()).wrap(Wrap { trim: false });

            scroll_view.render_widget(paragraph, Rect::new(0, y_offset, content_width, height));

            y_offset = y_offset.saturating_add(height);
        }

        // Render the scroll view to display the visible portion
        scroll_view.render(area, buf, state);
    }
}
