use similar::{ChangeTag, TextDiff};
use std::collections::HashMap;

/// Count total lines changed in an Edit operation
///
/// Returns the sum of lines added and lines removed.
pub fn count_edit_lines(old_string: &str, new_string: &str) -> usize {
    let diff = TextDiff::from_lines(old_string, new_string);

    diff.iter_all_changes()
        .filter(|change| matches!(change.tag(), ChangeTag::Insert | ChangeTag::Delete))
        .count()
}

/// Count lines in a Write operation
///
/// Uses `str::lines()` which treats trailing newlines as line terminators, not additional lines.
/// This matches common editor behavior where "3 lines" means 3 lines of content,
/// even if the file ends with a newline character.
pub fn count_write_lines(content: &str) -> usize {
    if content.is_empty() {
        return 0;
    }
    content.lines().count()
}

/// Extract lines changed from a tool use
/// Returns Some(lines_changed) if it's a code-modifying tool, None otherwise
pub fn extract_lines_from_tool(
    tool_name: &str,
    input: &HashMap<String, serde_json::Value>,
) -> Option<usize> {
    match tool_name {
        "Edit" => {
            let old_string = input.get("old_string")?.as_str()?;
            let new_string = input.get("new_string")?.as_str()?;
            Some(count_edit_lines(old_string, new_string))
        }
        "Write" => {
            let content = input.get("content")?.as_str()?;
            Some(count_write_lines(content))
        }
        "NotebookEdit" => {
            let new_source = input.get("new_source")?.as_str()?;
            Some(count_write_lines(new_source))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_edit_lines_simple_change() {
        let old = "line1\nline2\nline3";
        let new = "line1\nmodified\nline3";
        assert_eq!(count_edit_lines(old, new), 2);
    }

    #[test]
    fn test_count_edit_lines_no_change() {
        let text = "line1\nline2\nline3";
        assert_eq!(count_edit_lines(text, text), 0);
    }

    #[test]
    fn test_count_write_lines() {
        assert_eq!(count_write_lines("line1\nline2\nline3"), 3);
        assert_eq!(count_write_lines("single line"), 1);
        assert_eq!(count_write_lines(""), 0);
        assert_eq!(count_write_lines("line1\nline2\n"), 2);
    }

    #[test]
    fn test_count_write_lines_trailing_newline() {
        // Trailing newline is treated as a line terminator, not an additional line
        assert_eq!(count_write_lines("line1\n"), 1);
        assert_eq!(count_write_lines("line1\nline2\n"), 2);
        assert_eq!(count_write_lines("line1\nline2\nline3\n"), 3);
    }

    #[test]
    fn test_count_write_lines_multiple_trailing_newlines() {
        // Multiple trailing newlines create empty lines
        assert_eq!(count_write_lines("line1\n\n"), 2);
        assert_eq!(count_write_lines("line1\n\n\n"), 3);
    }

    #[test]
    fn test_count_write_lines_only_newlines() {
        assert_eq!(count_write_lines("\n"), 1);
        assert_eq!(count_write_lines("\n\n"), 2);
        assert_eq!(count_write_lines("\n\n\n"), 3);
    }

    #[test]
    fn test_count_write_lines_unicode() {
        assert_eq!(count_write_lines("Hello 👋\nWorld 🌍\n日本語"), 3);
        assert_eq!(count_write_lines("Emoji: 🎉\nUnicode: ñ"), 2);
    }

    #[test]
    fn test_count_edit_lines_only_additions() {
        // Use consistent line endings to test pure additions
        let old = "line1\n";
        let new = "line1\nline2\nline3\nline4\n";
        assert_eq!(count_edit_lines(old, new), 3);
    }

    #[test]
    fn test_count_edit_lines_only_deletions() {
        // Use consistent line endings to test pure deletions
        let old = "line1\nline2\nline3\nline4\n";
        let new = "line1\n";
        assert_eq!(count_edit_lines(old, new), 3);
    }

    #[test]
    fn test_count_edit_lines_empty_to_content() {
        let old = "";
        let new = "line1\nline2\nline3";
        assert_eq!(count_edit_lines(old, new), 3);
    }

    #[test]
    fn test_count_edit_lines_content_to_empty() {
        let old = "line1\nline2\nline3";
        let new = "";
        assert_eq!(count_edit_lines(old, new), 3);
    }

    #[test]
    fn test_count_edit_lines_empty_to_empty() {
        assert_eq!(count_edit_lines("", ""), 0);
    }

    #[test]
    fn test_extract_lines_from_edit_tool() {
        let mut input = HashMap::new();
        input.insert("old_string".to_string(), serde_json::json!("a\nb"));
        input.insert("new_string".to_string(), serde_json::json!("a\nc\nd"));

        let lines = extract_lines_from_tool("Edit", &input);
        assert_eq!(lines, Some(3)); // 1 deleted (b), 2 added (c, d)
    }

    #[test]
    fn test_extract_lines_from_write_tool() {
        let mut input = HashMap::new();
        input.insert(
            "content".to_string(),
            serde_json::json!("line1\nline2\nline3"),
        );

        let lines = extract_lines_from_tool("Write", &input);
        assert_eq!(lines, Some(3));
    }

    #[test]
    fn test_extract_lines_from_notebook_edit_tool() {
        let mut input = HashMap::new();
        input.insert(
            "new_source".to_string(),
            serde_json::json!("print('hello')\nprint('world')\nresult = 42"),
        );

        let lines = extract_lines_from_tool("NotebookEdit", &input);
        assert_eq!(lines, Some(3));
    }

    #[test]
    fn test_extract_lines_from_non_modifying_tool() {
        let input = HashMap::new();
        let lines = extract_lines_from_tool("Read", &input);
        assert_eq!(lines, None);
    }

    #[test]
    fn test_extract_lines_missing_required_fields() {
        let input = HashMap::new();

        // Edit without old_string or new_string
        assert_eq!(extract_lines_from_tool("Edit", &input), None);

        // Write without content
        assert_eq!(extract_lines_from_tool("Write", &input), None);

        // NotebookEdit without new_source
        assert_eq!(extract_lines_from_tool("NotebookEdit", &input), None);
    }

    #[test]
    fn test_extract_lines_invalid_field_types() {
        let mut input = HashMap::new();

        // old_string is a number instead of string
        input.insert("old_string".to_string(), serde_json::json!(123));
        input.insert("new_string".to_string(), serde_json::json!("new"));
        assert_eq!(extract_lines_from_tool("Edit", &input), None);

        input.clear();

        // content is a number instead of string
        input.insert("content".to_string(), serde_json::json!(456));
        assert_eq!(extract_lines_from_tool("Write", &input), None);
    }
}
