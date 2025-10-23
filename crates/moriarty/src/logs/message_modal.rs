use ratatui::{prelude::*, widgets::StatefulWidget};
use tui_scrollview::ScrollViewState;

use super::thread_view::calculate_text_height;

// Modal margins chosen to balance visual separation (prevent feeling cramped)
// with maximizing content area on small terminals. Tested on 80x24 terminals.
const MODAL_HORIZONTAL_MARGIN: u16 = 2; // Leaves 76 cols on 80-col terminal
const MODAL_VERTICAL_MARGIN: u16 = 1; // Leaves 22 rows on 24-row terminal
const MODAL_MIN_WIDTH: u16 = 10; // Minimum for readable text with border
const MODAL_MIN_HEIGHT: u16 = 5; // Room for title, border, 2 text lines

/// A modal view for displaying a single message in detail.
/// Takes up nearly the full screen with a border and is scrollable.
pub struct MessageModal<'a> {
    message_text: &'a str,
}

impl<'a> MessageModal<'a> {
    pub fn new(message_text: &'a str) -> Self {
        Self { message_text }
    }
}

impl<'a> StatefulWidget for MessageModal<'a> {
    type State = ScrollViewState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
        use tui_scrollview::ScrollView;

        let modal_area = Rect {
            x: area.x.saturating_add(MODAL_HORIZONTAL_MARGIN),
            y: area.y.saturating_add(MODAL_VERTICAL_MARGIN),
            width: area
                .width
                .saturating_sub(MODAL_HORIZONTAL_MARGIN * 2)
                .max(MODAL_MIN_WIDTH),
            height: area
                .height
                .saturating_sub(MODAL_VERTICAL_MARGIN * 2)
                .max(MODAL_MIN_HEIGHT),
        };

        Clear.render(modal_area, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" Message Detail (Press Esc to close) ")
            .title_alignment(ratatui::layout::Alignment::Center);

        let inner_area = block.inner(modal_area);
        block.render(modal_area, buf);

        // Reserve 1 column for scrollbar
        let content_width = inner_area.width.saturating_sub(1);
        let content_height = calculate_text_height(self.message_text, content_width);

        let mut scroll_view =
            ScrollView::new(ratatui::layout::Size::new(content_width, content_height));

        let paragraph = Paragraph::new(self.message_text)
            .wrap(Wrap { trim: false })
            .style(Style::default());

        scroll_view.render_widget(paragraph, Rect::new(0, 0, content_width, content_height));

        scroll_view.render(inner_area, buf, state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_message_modal() {
        let modal = MessageModal::new("Test message");
        assert_eq!(modal.message_text, "Test message");
    }

    #[test]
    fn test_calculate_text_height_single_line() {
        let height = calculate_text_height("Short message", 100);
        assert_eq!(height, 1);
    }

    #[test]
    fn test_calculate_text_height_multiline() {
        let height = calculate_text_height("Line 1\nLine 2\nLine 3", 100);
        assert_eq!(height, 3);
    }

    #[test]
    fn test_calculate_text_height_wrapping() {
        let long_line = "a".repeat(200);
        let height = calculate_text_height(&long_line, 100);
        assert_eq!(
            height, 2,
            "200 char line should wrap to 2 lines at width 100"
        );
    }

    #[test]
    fn test_calculate_text_height_empty_string() {
        let height = calculate_text_height("", 100);
        assert_eq!(height, 1, "Empty string should have height of 1");
    }

    #[test]
    fn test_calculate_text_height_empty_lines() {
        let height = calculate_text_height("Line 1\n\nLine 3", 100);
        assert_eq!(height, 3, "Empty lines should count as 1 line each");
    }

    #[test]
    fn test_calculate_text_height_narrow_width() {
        let height = calculate_text_height("Hello", 2);
        assert_eq!(height, 3, "5 char word should wrap to 3 lines at width 2");
    }

    #[test]
    fn test_calculate_text_height_width_one() {
        let height = calculate_text_height("Hi", 1);
        assert_eq!(height, 2, "2 char line should wrap to 2 lines at width 1");
    }

    #[test]
    fn test_calculate_text_height_zero_width() {
        let height = calculate_text_height("Line 1\nLine 2\nLine 3", 0);
        assert_eq!(height, 3, "Zero width should return line count");
    }

    #[test]
    fn test_calculate_text_height_zero_width_empty() {
        let height = calculate_text_height("", 0);
        assert_eq!(height, 1, "Zero width with empty string should return 1");
    }

    #[test]
    fn test_calculate_text_height_overflow() {
        let huge_line = "a".repeat(100000);
        let height = calculate_text_height(&huge_line, 100);
        assert!(
            height > 0,
            "Should not panic and should return a reasonable value"
        );
        assert_eq!(
            height, 656,
            "100000 chars clamped to u16::MAX (65535) wraps to 656 lines at width 100"
        );
    }
}
