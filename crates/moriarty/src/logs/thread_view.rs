use ratatui::{prelude::*, widgets::StatefulWidget};
use tui_scrollview::ScrollViewState;

use super::{formatter::format_log_line, parser::LogLine};

/// Calculate the number of lines required to display text at the given width.
/// Accounts for line wrapping based on character count.
pub(super) fn calculate_text_height(text: &str, content_width: u16) -> u16 {
    if content_width == 0 {
        let line_count = text.lines().count();
        return u16::try_from(line_count).unwrap_or(u16::MAX).max(1);
    }

    let mut line_count = 0u16;
    for line in text.lines() {
        let line_len = u16::try_from(line.len()).unwrap_or(u16::MAX);
        let wrapped_lines = if line_len == 0 {
            1
        } else {
            line_len.div_ceil(content_width)
        };
        line_count = line_count.saturating_add(wrapped_lines);
    }
    line_count.max(1)
}

#[derive(Debug, Clone)]
struct RenderedMessage {
    formatted: String,
    original: LogLine,
}

pub struct ThreadView {
    rendered_messages: Vec<RenderedMessage>,
    heights: Vec<u16>,
    total_height: u16,
    cached_width: u16,
    selected_index: usize,
}

impl ThreadView {
    pub fn new(contents: Vec<LogLine>) -> Self {
        // Render all log lines using the formatter
        let rendered_messages: Vec<RenderedMessage> = contents
            .into_iter()
            .map(|log_line| {
                let formatted = format_log_line(&log_line);
                RenderedMessage {
                    formatted,
                    original: log_line,
                }
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
            selected_index: 0,
        }
    }

    fn calculate_heights(
        rendered_messages: &[RenderedMessage],
        content_width: u16,
    ) -> (Vec<u16>, u16) {
        let mut heights = Vec::with_capacity(rendered_messages.len());
        let mut total_height = 0u16;

        for message in rendered_messages {
            let line_count = calculate_text_height(&message.formatted, content_width);

            let height = line_count.saturating_add(1);
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

    /// Move selection to the next message. Returns true if selection moved.
    pub fn select_next(&mut self) -> bool {
        if self.selected_index + 1 < self.rendered_messages.len() {
            self.selected_index += 1;
            true
        } else {
            false
        }
    }

    /// Move selection to the previous message. Returns true if selection moved.
    pub fn select_previous(&mut self) -> bool {
        if self.selected_index > 0 {
            self.selected_index -= 1;
            true
        } else {
            false
        }
    }

    /// Move selection to the first message.
    pub fn select_first(&mut self) {
        if !self.rendered_messages.is_empty() {
            self.selected_index = 0;
        }
    }

    /// Move selection to the last message.
    pub fn select_last(&mut self) {
        if !self.rendered_messages.is_empty() {
            self.selected_index = self.rendered_messages.len() - 1;
        }
    }

    /// Get the Y offset where the selected message starts.
    pub fn get_selected_y_offset(&self) -> u16 {
        self.heights
            .iter()
            .take(self.selected_index)
            .copied()
            .fold(0u16, |acc, h| acc.saturating_add(h))
    }

    /// Get the total number of messages.
    pub fn get_message_count(&self) -> usize {
        self.rendered_messages.len()
    }

    /// Get the currently selected index.
    pub fn get_selected_index(&self) -> usize {
        self.selected_index
    }

    /// Get the height of the selected message.
    pub fn get_selected_height(&self) -> u16 {
        self.heights.get(self.selected_index).copied().unwrap_or(0)
    }

    pub fn get_selected_message(&self) -> Option<&str> {
        self.rendered_messages
            .get(self.selected_index)
            .map(|msg| msg.formatted.as_str())
    }

    /// Get the original LogLine for the selected message (used by modal for debug view)
    pub fn get_selected_log_line(&self) -> Option<&LogLine> {
        self.rendered_messages
            .get(self.selected_index)
            .map(|msg| &msg.original)
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
        for (index, (message, &height)) in self
            .rendered_messages
            .iter()
            .zip(self.heights.iter())
            .enumerate()
        {
            // Use blue for user messages, default for everything else
            let base_style = match &message.original {
                LogLine::User(_) => Style::default().fg(Color::Blue),
                _ => Style::default(),
            };

            // Apply selection highlight if this is the selected message
            let style = if index == self.selected_index {
                base_style.add_modifier(ratatui::style::Modifier::REVERSED)
            } else {
                base_style
            };

            // Render the message text
            let message_height = height.saturating_sub(1);
            let paragraph = Paragraph::new(message.formatted.as_str())
                .wrap(Wrap { trim: false })
                .style(style);

            scroll_view.render_widget(
                paragraph,
                Rect::new(0, y_offset, content_width, message_height),
            );

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logs::parser::{
        AssistantCacheCreation, AssistantLogLine, AssistantLogMessage, AssistantUsage, LogMessage,
        LogMessageContent, Summary, UserLogLine,
    };
    use chrono::Utc;
    use uuid::Uuid;

    /// Helper to create a minimal UserLogLine for testing
    fn create_test_user_message(text: &str) -> LogLine {
        LogLine::User(UserLogLine {
            parent_uuid: None,
            is_sidechain: false,
            agent_id: None,
            user_type: "test".to_string(),
            cwd: "/test".to_string(),
            session_id: Uuid::new_v4(),
            version: "1.0.0".to_string(),
            git_branch: "main".to_string(),
            slug: None,
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
            todos: None,
            source_tool_assistant_uuid: None,
        })
    }

    /// Helper to create a minimal AssistantLogLine for testing
    fn create_test_assistant_message(text: &str) -> LogLine {
        LogLine::Assistant(AssistantLogLine {
            parent_uuid: None,
            is_sidechain: false,
            agent_id: None,
            user_type: "test".to_string(),
            cwd: "/test".to_string(),
            session_id: "test-session".to_string(),
            version: "1.0.0".to_string(),
            git_branch: "main".to_string(),
            slug: None,
            message: AssistantLogMessage {
                id: "msg_test".to_string(),
                r#type: "message".to_string(),
                role: "assistant".to_string(),
                model: "claude-3".to_string(),
                container: None,
                content: LogMessageContent::String(text.to_string()),
                stop_reason: None,
                stop_sequence: None,
                usage: AssistantUsage {
                    input_tokens: 0,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                    cache_creation: AssistantCacheCreation {
                        ephemeral_5m_input_tokens: 0,
                        ephemeral_1h_input_tokens: 0,
                    },
                    output_tokens: 0,
                    service_tier: None,
                    server_tool_use: None,
                    inference_geo: None,
                },
                context_management: None,
            },
            request_id: None,
            uuid: Uuid::new_v4(),
            timestamp: Utc::now(),
            is_api_error_message: None,
            error: None,
        })
    }

    #[test]
    fn test_new_thread_view_starts_at_first_message() {
        let messages = vec![
            create_test_user_message("Hello"),
            create_test_assistant_message("Hi there"),
        ];
        let view = ThreadView::new(messages);

        assert_eq!(view.get_selected_index(), 0);
        assert_eq!(view.get_message_count(), 2);
    }

    #[test]
    fn test_new_thread_view_empty_messages() {
        let view = ThreadView::new(vec![]);

        assert_eq!(view.get_selected_index(), 0);
        assert_eq!(view.get_message_count(), 0);
    }

    #[test]
    fn test_select_next_moves_forward() {
        let messages = vec![
            create_test_user_message("First"),
            create_test_assistant_message("Second"),
            create_test_user_message("Third"),
        ];
        let mut view = ThreadView::new(messages);

        assert_eq!(view.get_selected_index(), 0);

        let moved = view.select_next();
        assert!(moved);
        assert_eq!(view.get_selected_index(), 1);

        let moved = view.select_next();
        assert!(moved);
        assert_eq!(view.get_selected_index(), 2);
    }

    #[test]
    fn test_select_next_stops_at_last_message() {
        let messages = vec![
            create_test_user_message("First"),
            create_test_assistant_message("Second"),
        ];
        let mut view = ThreadView::new(messages);

        view.select_next(); // Move to index 1
        let moved = view.select_next(); // Try to move past end

        assert!(!moved);
        assert_eq!(view.get_selected_index(), 1);
    }

    #[test]
    fn test_select_next_empty_list() {
        let mut view = ThreadView::new(vec![]);

        let moved = view.select_next();
        assert!(!moved);
        assert_eq!(view.get_selected_index(), 0);
    }

    #[test]
    fn test_select_previous_moves_backward() {
        let messages = vec![
            create_test_user_message("First"),
            create_test_assistant_message("Second"),
            create_test_user_message("Third"),
        ];
        let mut view = ThreadView::new(messages);

        view.select_next();
        view.select_next();
        assert_eq!(view.get_selected_index(), 2);

        let moved = view.select_previous();
        assert!(moved);
        assert_eq!(view.get_selected_index(), 1);

        let moved = view.select_previous();
        assert!(moved);
        assert_eq!(view.get_selected_index(), 0);
    }

    #[test]
    fn test_select_previous_stops_at_first_message() {
        let messages = vec![
            create_test_user_message("First"),
            create_test_assistant_message("Second"),
        ];
        let mut view = ThreadView::new(messages);

        let moved = view.select_previous(); // Try to move before start

        assert!(!moved);
        assert_eq!(view.get_selected_index(), 0);
    }

    #[test]
    fn test_select_first_from_middle() {
        let messages = vec![
            create_test_user_message("First"),
            create_test_assistant_message("Second"),
            create_test_user_message("Third"),
        ];
        let mut view = ThreadView::new(messages);

        view.select_next();
        view.select_next();
        assert_eq!(view.get_selected_index(), 2);

        view.select_first();
        assert_eq!(view.get_selected_index(), 0);
    }

    #[test]
    fn test_select_first_when_already_first() {
        let messages = vec![
            create_test_user_message("First"),
            create_test_assistant_message("Second"),
        ];
        let mut view = ThreadView::new(messages);

        view.select_first();
        assert_eq!(view.get_selected_index(), 0);
    }

    #[test]
    fn test_select_first_empty_list() {
        let mut view = ThreadView::new(vec![]);

        view.select_first();
        assert_eq!(view.get_selected_index(), 0);
    }

    #[test]
    fn test_select_last_from_middle() {
        let messages = vec![
            create_test_user_message("First"),
            create_test_assistant_message("Second"),
            create_test_user_message("Third"),
        ];
        let mut view = ThreadView::new(messages);

        assert_eq!(view.get_selected_index(), 0);

        view.select_last();
        assert_eq!(view.get_selected_index(), 2);
    }

    #[test]
    fn test_select_last_when_already_last() {
        let messages = vec![
            create_test_user_message("First"),
            create_test_assistant_message("Second"),
        ];
        let mut view = ThreadView::new(messages);

        view.select_last();
        assert_eq!(view.get_selected_index(), 1);

        view.select_last();
        assert_eq!(view.get_selected_index(), 1);
    }

    #[test]
    fn test_select_last_single_message() {
        let messages = vec![create_test_user_message("Only")];
        let mut view = ThreadView::new(messages);

        view.select_last();
        assert_eq!(view.get_selected_index(), 0);
    }

    #[test]
    fn test_select_last_empty_list() {
        let mut view = ThreadView::new(vec![]);

        view.select_last();
        assert_eq!(view.get_selected_index(), 0);
    }

    #[test]
    fn test_get_selected_y_offset_first_message() {
        let messages = vec![
            create_test_user_message("First"),
            create_test_assistant_message("Second"),
        ];
        let view = ThreadView::new(messages);

        assert_eq!(view.get_selected_y_offset(), 0);
    }

    #[test]
    fn test_get_selected_y_offset_cumulative() {
        let messages = vec![
            create_test_user_message("First"),
            create_test_assistant_message("Second"),
            create_test_user_message("Third"),
        ];
        let mut view = ThreadView::new(messages);

        let first_offset = view.get_selected_y_offset();
        assert_eq!(first_offset, 0);

        view.select_next();
        let second_offset = view.get_selected_y_offset();
        assert!(
            second_offset > 0,
            "Second message should have non-zero offset"
        );

        let first_height = view.heights[0];
        assert_eq!(second_offset, first_height);

        view.select_next();
        let third_offset = view.get_selected_y_offset();
        let expected = first_height.saturating_add(view.heights[1]);
        assert_eq!(third_offset, expected);
    }

    #[test]
    fn test_get_selected_height() {
        let messages = vec![
            create_test_user_message("Short"),
            create_test_assistant_message("This is a much longer message that will wrap"),
        ];
        let view = ThreadView::new(messages);

        let height = view.get_selected_height();
        assert!(height > 0);
    }

    #[test]
    fn test_get_selected_height_empty_list() {
        let view = ThreadView::new(vec![]);

        let height = view.get_selected_height();
        assert_eq!(height, 0);
    }

    #[test]
    fn test_message_filtering() {
        // ThreadView now displays all log types (User, Assistant, Summary, etc.)
        let messages = vec![
            create_test_user_message("User 1"),
            LogLine::Summary(Summary {
                summary: "Summary message".to_string(),
                leaf_uuid: Uuid::new_v4(),
            }),
            create_test_assistant_message("Assistant 1"),
        ];
        let view = ThreadView::new(messages);

        // All 3 messages should be rendered
        assert_eq!(view.get_message_count(), 3);
    }

    #[test]
    fn test_selection_navigation_with_single_message() {
        let messages = vec![create_test_user_message("Only message")];
        let mut view = ThreadView::new(messages);

        assert_eq!(view.get_selected_index(), 0);
        assert!(!view.select_next()); // Can't go forward
        assert!(!view.select_previous()); // Can't go backward
        assert_eq!(view.get_selected_index(), 0);

        view.select_first();
        assert_eq!(view.get_selected_index(), 0);

        view.select_last();
        assert_eq!(view.get_selected_index(), 0);
    }

    #[test]
    fn test_get_selected_message_returns_text() {
        let messages = vec![
            create_test_user_message("User text"),
            create_test_assistant_message("Assistant text"),
        ];
        let view = ThreadView::new(messages);

        let message = view.get_selected_message();
        assert!(message.is_some());
        assert!(message.unwrap().contains("User text"));
    }

    #[test]
    fn test_get_selected_message_empty_list() {
        let view = ThreadView::new(vec![]);

        let message = view.get_selected_message();
        assert!(message.is_none());
    }

    #[test]
    fn test_get_selected_message_after_navigation() {
        let messages = vec![
            create_test_user_message("First"),
            create_test_assistant_message("Second"),
        ];
        let mut view = ThreadView::new(messages);

        view.select_next();
        let message = view.get_selected_message();
        assert!(message.is_some());
        assert!(message.unwrap().contains("Second"));
    }

    #[test]
    fn test_get_selected_message_user_vs_assistant() {
        let messages = vec![
            create_test_user_message("User"),
            create_test_assistant_message("Assistant"),
        ];
        let mut view = ThreadView::new(messages);

        let user_msg = view.get_selected_message();
        assert!(user_msg.is_some());

        view.select_next();
        let assistant_msg = view.get_selected_message();
        assert!(assistant_msg.is_some());
    }
}
