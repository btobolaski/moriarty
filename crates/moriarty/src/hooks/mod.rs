//! Hook execution for Claude Code integration
//!
//! This module provides the CLI interface for executing hooks. Currently, this is primarily
//! used for debugging and development - it reads hook input from stdin, parses it, and logs
//! both the parsed data and the environment context.
//!
//! ## Design Decisions
//!
//! - **Parse errors are fatal**: The function returns an error if parsing fails. This ensures
//!   that malformed hook input is visible via exit codes, allowing scripts and CI systems to
//!   detect failures.
//!
//! - **Environment variables are logged**: During development, understanding the complete
//!   environment context helps debug hook execution issues. Sensitive patterns (TOKEN, SECRET,
//!   KEY, PASSWORD, etc.) are automatically redacted.

pub mod parser;
pub mod tracing;

use crate::HooksCommand;
use ::tracing::{error, info};
use miette::Result;
use regex::Regex;
use std::io::Read;
use std::sync::LazyLock;

/// Execute hooks command
pub async fn exec_hooks(cmd: HooksCommand) -> Result<()> {
    match cmd {
        HooksCommand::Exec => exec_hook().await,
    }
}

/// Execute a single hook by reading input from stdin and logging all parsed data
///
/// This is primarily a development/debugging tool. It reads hook input JSON from stdin,
/// parses it, and logs the results along with environment context.
///
/// Returns an error if parsing fails, ensuring malformed input is detectable via exit codes.
async fn exec_hook() -> Result<()> {
    exec_hook_impl(std::io::stdin()).await
}

/// Internal implementation of exec_hook that accepts any Read source for testability
async fn exec_hook_impl<R: Read>(reader: R) -> Result<()> {
    // In tests, the global tracing subscriber may already be initialized by a prior test,
    // causing init failures. This is safe because logging still works via the existing
    // subscriber, and nextest's process isolation prevents cross-contamination.
    let _guard = match tracing::init_tracing().await {
        Ok(guard) => Some(guard),
        Err(_) if cfg!(test) => None,
        Err(e) => return Err(e),
    };

    // Limit input size to 1MB to prevent DoS via memory exhaustion
    const MAX_INPUT_SIZE: usize = 1024 * 1024;
    const LOG_TRUNCATE_SIZE: usize = 500;

    let mut input = String::new();
    let bytes_read = reader
        .take(MAX_INPUT_SIZE as u64)
        .read_to_string(&mut input)
        .map_err(|e| {
            miette::miette!(
                "Failed to read hook input from stdin (this command expects JSON hook data): {}",
                e
            )
        })?;

    if bytes_read == 0 {
        error!("Received empty input from stdin");
        return Err(miette::miette!("No input received from stdin"));
    }

    if bytes_read == MAX_INPUT_SIZE {
        error!(
            bytes_read = bytes_read,
            max_size = MAX_INPUT_SIZE,
            "Input reached maximum size limit, possible truncation"
        );
        return Err(miette::miette!(
            "Hook input reached maximum size of {} bytes and may have been truncated. \
             Reduce input size or increase MAX_INPUT_SIZE constant.",
            MAX_INPUT_SIZE
        ));
    }

    info!(bytes = bytes_read, "Received hook input from stdin");

    let hook_input = parser::parse_hook_input(&input).map_err(|e| {
        // Truncate and sanitize input for logging to prevent log injection and bloat
        let sanitized_input = if input.len() > LOG_TRUNCATE_SIZE {
            // Find the byte index of the Nth character to avoid splitting multi-byte UTF-8
            let safe_truncate = input
                .char_indices()
                .nth(LOG_TRUNCATE_SIZE)
                .map(|(i, _)| i)
                .unwrap_or(input.len());
            format!(
                "{}... [truncated {} bytes]",
                input[..safe_truncate].escape_debug(),
                input.len() - safe_truncate
            )
        } else {
            input.escape_debug().to_string()
        };

        error!(
            error = %e,
            raw_input = %sanitized_input,
            "Failed to parse hook input"
        );

        miette::miette!("Failed to parse hook input: {}", e)
    })?;

    info!(
        session_id = %hook_input.session_id,
        transcript_path = %hook_input.transcript_path,
        cwd = %hook_input.cwd,
        permission_mode = ?hook_input.permission_mode,
        event_data = ?hook_input.event_data,
        "Successfully parsed hook input"
    );

    // Environment variables affect hook execution (PATH, shell config, etc.) and differ
    // between parent and subprocess contexts. Logging them aids debugging execution issues.
    info!("Logging environment variables:");
    for (key, value) in std::env::vars() {
        if is_sensitive_env_var(&key) {
            info!(env_var = %key, value = "[REDACTED]", "Environment variable");
        } else {
            info!(env_var = %key, value = %value, "Environment variable");
        }
    }

    Ok(())
}

/// Regex pattern for detecting sensitive environment variable names
static SENSITIVE_ENV_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(TOKEN|SECRET|PASSWORD|PASS|PASSWD|CREDENTIAL|AUTH|APIKEY|_KEY$|^.*_KEY_|PRIVATE|WEBHOOK|SESSION)")
        .expect("Invalid regex pattern")
});

/// Check if an environment variable name suggests it contains sensitive data
fn is_sensitive_env_var(key: &str) -> bool {
    SENSITIVE_ENV_PATTERN.is_match(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tempfile::TempDir;

    /// Safe to use std::env::set_var because cargo nextest isolates each test in a separate process.
    fn setup_isolated_xdg_state() -> TempDir {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_STATE_HOME", temp_dir.path());
        temp_dir
    }

    #[tokio::test]
    async fn test_exec_hook_empty_input_returns_error() {
        let _xdg_dir = setup_isolated_xdg_state();

        let input = "";
        let cursor = Cursor::new(input);
        let result = exec_hook_impl(cursor).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("No input received"));
    }

    #[tokio::test]
    async fn test_exec_hook_malformed_json_returns_error() {
        let _xdg_dir = setup_isolated_xdg_state();

        let input = "{invalid json";
        let cursor = Cursor::new(input);
        let result = exec_hook_impl(cursor).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Failed to parse hook input"));
    }

    #[tokio::test]
    async fn test_exec_hook_valid_input_succeeds() {
        let _xdg_dir = setup_isolated_xdg_state();

        let input = r#"{
            "session_id": "test-session",
            "transcript_path": "/tmp/transcript.json",
            "cwd": "/tmp/project",
            "permission_mode": "default",
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": "echo test"}
        }"#;

        let cursor = Cursor::new(input);
        let result = exec_hook_impl(cursor).await;

        result.unwrap(); // Panics with full error details if it fails
    }

    #[tokio::test]
    async fn test_exec_hook_large_input_within_limit() {
        let _xdg_dir = setup_isolated_xdg_state();

        // Create valid input close to but under 1MB limit to verify we don't crash
        let large_command = "x".repeat(1024 * 900); // 900KB of padding
        let input = format!(
            r#"{{"session_id":"test","transcript_path":"/t","cwd":"/c","permission_mode":"default","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{{"command":"{}"}}}}"#,
            large_command
        );

        let cursor = Cursor::new(input);
        let result = exec_hook_impl(cursor).await;

        // Should succeed - large but valid input
        result.unwrap();
    }

    #[tokio::test]
    async fn test_exec_hook_truncates_long_invalid_input() {
        let _xdg_dir = setup_isolated_xdg_state();

        // Create 1000-byte invalid JSON
        let input = format!("{{\"invalid\": \"{}", "x".repeat(1000));

        let cursor = Cursor::new(input);
        let result = exec_hook_impl(cursor).await;

        // Should return error without panicking
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Failed to parse hook input"));
    }

    #[tokio::test]
    async fn test_exec_hook_all_event_types() {
        let _xdg_dir = setup_isolated_xdg_state();

        let test_cases = vec![
            r#"{"session_id":"s","transcript_path":"/t","cwd":"/c","permission_mode":"default","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{}}"#,
            r#"{"session_id":"s","transcript_path":"/t","cwd":"/c","permission_mode":"default","hook_event_name":"PostToolUse","tool_name":"Bash","tool_input":{},"tool_output":""}"#,
            r#"{"session_id":"s","transcript_path":"/t","cwd":"/c","permission_mode":"default","hook_event_name":"SessionStart","matcher":"startup"}"#,
            r#"{"session_id":"s","transcript_path":"/t","cwd":"/c","permission_mode":"default","hook_event_name":"SessionEnd","reason":"logout"}"#,
            r#"{"session_id":"s","transcript_path":"/t","cwd":"/c","permission_mode":"default","hook_event_name":"Stop"}"#,
        ];

        for json in test_cases {
            let cursor = Cursor::new(json);
            let result = exec_hook_impl(cursor).await;
            result.unwrap_or_else(|e| panic!("Failed for input {}: {}", json, e));
        }
    }

    #[tokio::test]
    async fn test_exec_hook_redacts_sensitive_env_vars() {
        let _xdg_dir = setup_isolated_xdg_state();

        // Set test environment variables
        std::env::set_var("TEST_API_TOKEN", "secret123");
        std::env::set_var("TEST_SAFE_VAR", "public_value");

        let input = r#"{
            "session_id": "test-session",
            "transcript_path": "/tmp/transcript.json",
            "cwd": "/tmp/project",
            "permission_mode": "default",
            "hook_event_name": "Stop"
        }"#;

        let cursor = Cursor::new(input);
        let result = exec_hook_impl(cursor).await;
        result.unwrap();

        // Force flush by dropping the guard (it's in result's scope which ended)
        // Give async file writes time to complete
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Read the log file and verify redaction
        let log_path = tracing::get_current_log_path().await.unwrap();
        let log_dir = log_path.parent().unwrap();
        let entries: Vec<_> = std::fs::read_dir(log_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("hooks.log"))
            .collect();

        assert!(!entries.is_empty(), "Should have at least one log file");

        let log_file = &entries[0];
        let content = std::fs::read_to_string(log_file.path()).unwrap();

        // Verify sensitive var is redacted
        assert!(
            content.contains("TEST_API_TOKEN"),
            "Should log the variable name"
        );
        assert!(content.contains("[REDACTED]"), "Should redact the value");
        assert!(
            !content.contains("secret123"),
            "Should NOT log the secret value. Log content:\n{}",
            content
        );

        // Verify safe var is logged
        assert!(
            content.contains("TEST_SAFE_VAR"),
            "Should log safe variable name"
        );
        assert!(
            content.contains("public_value"),
            "Should log safe variable value"
        );

        // Clean up
        std::env::remove_var("TEST_API_TOKEN");
        std::env::remove_var("TEST_SAFE_VAR");
    }

    #[tokio::test]
    async fn test_exec_hook_exactly_at_size_limit() {
        let _xdg_dir = setup_isolated_xdg_state();

        // Create input of exactly MAX_INPUT_SIZE (1MB)
        // Account for JSON structure overhead
        const MAX_INPUT_SIZE: usize = 1024 * 1024;
        let json_overhead = r#"{"session_id":"test","transcript_path":"/t","cwd":"/c","permission_mode":"default","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":""}}"#.len();
        let padding_size = MAX_INPUT_SIZE - json_overhead;
        let padding = "x".repeat(padding_size);
        let input = format!(
            r#"{{"session_id":"test","transcript_path":"/t","cwd":"/c","permission_mode":"default","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{{"command":"{}"}}}}"#,
            padding
        );

        assert_eq!(input.len(), MAX_INPUT_SIZE, "Input should be exactly 1MB");

        let cursor = Cursor::new(input);
        let result = exec_hook_impl(cursor).await;

        // Should return error because we hit the size limit
        assert!(result.is_err(), "Should error at exactly MAX_INPUT_SIZE");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("maximum size"),
            "Error should mention size limit: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_exec_hook_exceeds_size_limit() {
        let _xdg_dir = setup_isolated_xdg_state();

        // Create input larger than MAX_INPUT_SIZE
        const MAX_INPUT_SIZE: usize = 1024 * 1024;
        let oversized_data = "x".repeat(MAX_INPUT_SIZE + 1000);
        let input = format!(
            r#"{{"session_id":"test","transcript_path":"/t","cwd":"/c","permission_mode":"default","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{{"command":"{}"}}}}"#,
            oversized_data
        );

        let cursor = Cursor::new(input);
        let result = exec_hook_impl(cursor).await;

        // Should error due to size limit (input gets truncated, then parse fails or size check fails)
        assert!(result.is_err(), "Oversized input should fail");
        let err = result.unwrap_err();
        // Error could be about size limit or parse failure due to truncation
        assert!(
            err.to_string().contains("maximum size") || err.to_string().contains("parse"),
            "Error should mention size or parsing: {}",
            err
        );
    }

    #[test]
    fn test_is_sensitive_env_var_matches_sensitive_patterns() {
        // Should match - these contain sensitive data
        assert!(is_sensitive_env_var("API_TOKEN"));
        assert!(is_sensitive_env_var("SECRET_KEY"));
        assert!(is_sensitive_env_var("PASSWORD"));
        assert!(is_sensitive_env_var("DB_PASSWORD"));
        assert!(is_sensitive_env_var("WEBHOOK_URL"));
        assert!(is_sensitive_env_var("AWS_ACCESS_KEY"));
        assert!(is_sensitive_env_var("STRIPE_SECRET_KEY"));
        assert!(is_sensitive_env_var("MY_API_KEY"));
        assert!(is_sensitive_env_var("GITHUB_TOKEN"));
        assert!(is_sensitive_env_var("AUTH_TOKEN"));
        assert!(is_sensitive_env_var("APIKEY"));
        assert!(is_sensitive_env_var("PRIVATE_KEY"));
        assert!(is_sensitive_env_var("SESSION_SECRET"));
        assert!(is_sensitive_env_var("DB_PASS"));
        assert!(is_sensitive_env_var("PASSWD"));
        assert!(is_sensitive_env_var("CREDENTIAL"));
    }

    #[test]
    fn test_is_sensitive_env_var_case_insensitive() {
        // Case insensitive matching
        assert!(is_sensitive_env_var("api_token"));
        assert!(is_sensitive_env_var("SeCrEt"));
        assert!(is_sensitive_env_var("PaSsWoRd"));
        assert!(is_sensitive_env_var("AUTH"));
    }

    #[test]
    fn test_is_sensitive_env_var_does_not_match_safe_vars() {
        // Should NOT match - these are safe to log
        assert!(!is_sensitive_env_var("PATH"));
        assert!(!is_sensitive_env_var("HOME"));
        assert!(!is_sensitive_env_var("RUST_LOG"));
        assert!(!is_sensitive_env_var("CARGO_HOME"));
        assert!(!is_sensitive_env_var("USER"));
        assert!(!is_sensitive_env_var("SHELL"));
        assert!(!is_sensitive_env_var("TERM"));
        assert!(!is_sensitive_env_var("LANG"));

        // KEYBOARD should not match even though it contains "key"
        assert!(!is_sensitive_env_var("KEYBOARD"));
        assert!(!is_sensitive_env_var("MONKEY_MODE"));
        assert!(!is_sensitive_env_var("DEPLOYMENT"));
    }

    #[test]
    fn test_is_sensitive_env_var_edge_cases() {
        // Empty string
        assert!(!is_sensitive_env_var(""));

        // Patterns at boundaries
        assert!(is_sensitive_env_var("_KEY"));
        assert!(is_sensitive_env_var("MY_KEY"));
        assert!(is_sensitive_env_var("SOMETHING_KEY_ELSE"));
    }
}
