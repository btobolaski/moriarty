//! Parser for Claude Code hooks configuration
//!
//! Hooks enable automated scripts at specific workflow points for validation,
//! approval, and context injection.
//!
//! Configuration is JSON-based and stored in Claude settings files.

use std::collections::HashMap;

#[cfg(test)]
use std::fmt;

use serde::{Deserialize, Serialize};

/// Hook event types that can trigger scripts
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
#[allow(dead_code)]
pub enum HookEvent {
    /// Executes after Claude creates tool parameters but before processing
    PreToolUse,
    /// Runs immediately after successful tool completion
    PostToolUse,
    /// Runs when the user submits a prompt, before Claude processes it
    UserPromptSubmit,
    /// Triggers when Claude sends notifications about permissions or waiting status
    Notification,
    /// Runs when Claude Code starts a new session or resumes an existing session
    SessionStart,
    /// Executes when a session concludes
    SessionEnd,
    /// Prevents main agent from finishing
    Stop,
    /// Prevents subagent task completion
    SubagentStop,
    /// Runs before Claude Code is about to run a compact operation
    PreCompact,
}

/// Session start matcher types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStartMatcher {
    Startup,
    Resume,
    Clear,
    Compact,
}

/// Session end reason types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEndReason {
    Clear,
    Logout,
    PromptInputExit,
    Other,
}

/// Pre-compact matcher types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PreCompactMatcher {
    Manual,
    Auto,
}

/// Hook type (currently only command is supported)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)]
pub enum HookType {
    Command,
}

/// Permission mode levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    Default,
    Plan,
    AcceptEdits,
    BypassPermissions,
}

/// Hook definition with command and execution parameters
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct HookDefinition {
    /// Currently only "command" is supported
    #[serde(rename = "type")]
    pub hook_type: HookType,
    /// Supports $CLAUDE_PROJECT_DIR variable substitution
    pub command: String,
    /// 60-second default balances responsiveness for quick checks while allowing
    /// compilation/analysis tools that may take tens of seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

impl HookDefinition {
    /// Get the timeout value, defaulting to 60 seconds
    #[cfg(test)]
    pub fn timeout_secs(&self) -> u64 {
        self.timeout.unwrap_or(60)
    }
}

/// Hook matcher configuration
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct HookMatcher {
    /// Case-sensitive, supports regex and * for all tools
    pub matcher: String,
    pub hooks: Vec<HookDefinition>,
}

/// Top-level hooks configuration
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct HooksConfig {
    #[serde(flatten)]
    pub hooks: HashMap<HookEvent, Vec<HookMatcher>>,
}

/// Event-specific data for different hook types
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "hook_event_name")]
pub enum HookEventData {
    #[serde(rename = "PreToolUse")]
    PreToolUse {
        tool_name: String,
        tool_input: serde_json::Value,
    },
    #[serde(rename = "PostToolUse")]
    PostToolUse {
        tool_name: String,
        tool_input: serde_json::Value,
        tool_response: serde_json::Value,
    },
    #[serde(rename = "UserPromptSubmit")]
    UserPromptSubmit { user_prompt: String },
    #[serde(rename = "Notification")]
    Notification {
        notification_type: String,
        message: String,
    },
    #[serde(rename = "SessionStart")]
    SessionStart { matcher: SessionStartMatcher },
    #[serde(rename = "SessionEnd")]
    SessionEnd { reason: SessionEndReason },
    #[serde(rename = "Stop")]
    Stop,
    #[serde(rename = "SubagentStop")]
    SubagentStop { subagent_id: String },
    #[serde(rename = "PreCompact")]
    PreCompact { matcher: PreCompactMatcher },
}

/// Input provided to hook scripts via stdin
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookInput {
    pub session_id: String,
    pub transcript_path: String,
    pub cwd: String,
    pub permission_mode: PermissionMode,
    #[serde(flatten)]
    pub event_data: HookEventData,
}

/// Controls whether tool execution should proceed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HookDecision {
    Approve,
    Block,
    Ask,
}

/// Permission decision for tool execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionDecision {
    Allow,
    Deny,
    Ask,
}

/// Hook-specific output for PreToolUse hooks
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreToolUseOutput {
    pub hook_event_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_decision: Option<PermissionDecision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_decision_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_input: Option<serde_json::Value>,
}

/// Hook-specific output for UserPromptSubmit hooks
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserPromptSubmitOutput {
    pub hook_event_name: String,
    pub additional_context: String,
}

/// Hook-specific output that varies by hook type
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HookSpecificOutput {
    // Order matters for untagged enums: UserPromptSubmit has required fields,
    // so it should be checked first to avoid PreToolUse matching everything
    UserPromptSubmit(UserPromptSubmitOutput),
    PreToolUse(PreToolUseOutput),
}

/// Advanced control via JSON output from hook scripts
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookOutput {
    #[serde(rename = "continue", skip_serializing_if = "Option::is_none")]
    pub continue_execution: Option<bool>,
    /// Required if continue_execution is false
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppress_output: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<HookDecision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_decision: Option<PermissionDecision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_specific_output: Option<HookSpecificOutput>,
}

/// Determines how hook script output and errors are handled
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookExitCode {
    /// 0: stdout shown in transcript
    Success,
    /// 2: prevents action, stderr shown to Claude
    BlockingError,
    /// Any other code: non-blocking, stderr shown to user
    NonBlockingError(i32),
}

#[cfg(test)]
impl HookExitCode {
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => Self::Success,
            2 => Self::BlockingError,
            n => Self::NonBlockingError(n),
        }
    }

    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success)
    }

    pub fn is_blocking(&self) -> bool {
        matches!(self, Self::BlockingError)
    }
}

/// Parse hooks configuration from JSON string
#[cfg(test)]
pub fn parse_hooks_config(json: &str) -> Result<HooksConfig, serde_json::Error> {
    serde_json::from_str(json)
}

/// Serialize hooks configuration to JSON string
#[cfg(test)]
pub fn serialize_hooks_config(config: &HooksConfig) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(config)
}

/// Parse hook input from JSON string (stdin to hook script)
pub fn parse_hook_input(json: &str) -> Result<HookInput, serde_json::Error> {
    serde_json::from_str(json)
}

/// Parse hook output from JSON string (stdout from hook script)
#[cfg(test)]
pub fn parse_hook_output(json: &str) -> Result<HookOutput, serde_json::Error> {
    serde_json::from_str(json)
}

#[cfg(test)]
mod tests {
    use serde::{de::DeserializeOwned, Serialize};

    use super::*;

    /// Assert that a Result is Err and the error message contains at least one of the given substrings
    fn assert_err_contains<T: fmt::Debug>(
        result: Result<T, impl std::fmt::Display>,
        expected: &[&str],
    ) {
        let err_msg = result.expect_err("Expected error").to_string();
        assert!(
            expected.iter().any(|s| err_msg.contains(s)),
            "Expected error to contain one of {:?}, got: {}",
            expected,
            err_msg
        );
    }

    fn assert_json_roundtrip<T>(cases: &[(T, &str)])
    where
        T: Copy + DeserializeOwned + PartialEq + Serialize + fmt::Debug,
    {
        for (value, expected_json) in cases {
            let json = serde_json::to_string(value).expect("Failed to serialize");
            assert_eq!(json, *expected_json);

            let parsed: T = serde_json::from_str(&json).expect("Failed to deserialize");
            assert_eq!(parsed, *value);
        }
    }

    fn assert_json_contains<T>(value: &T, present: &[&str], absent: &[&str])
    where
        T: Serialize,
    {
        let json = serde_json::to_string(value).expect("Failed to serialize");
        for fragment in present {
            assert!(
                json.contains(fragment),
                "Missing fragment {fragment:?} in {json}"
            );
        }
        for fragment in absent {
            assert!(
                !json.contains(fragment),
                "Unexpected fragment {fragment:?} in {json}"
            );
        }
    }

    #[test]
    fn test_parse_hooks_config() {
        let json = r#"{
            "PreToolUse": [
                {
                    "matcher": "Bash",
                    "hooks": [
                        {
                            "type": "command",
                            "command": "echo 'Pre-tool validation'",
                            "timeout": 30
                        }
                    ]
                }
            ],
            "SessionStart": [
                {
                    "matcher": "startup",
                    "hooks": [
                        {
                            "type": "command",
                            "command": "echo 'Session started'"
                        }
                    ]
                }
            ]
        }"#;

        let config = parse_hooks_config(json).expect("Failed to parse config");
        assert_eq!(config.hooks.len(), 2);

        let pre_tool = &config.hooks[&HookEvent::PreToolUse][0];
        assert_eq!(pre_tool.matcher, "Bash");
        assert_eq!(pre_tool.hooks[0].timeout, Some(30));

        let session_start = &config.hooks[&HookEvent::SessionStart][0];
        assert_eq!(session_start.matcher, "startup");
        assert_eq!(session_start.hooks[0].timeout_secs(), 60); // default
    }

    #[test]
    fn test_parse_hook_input() {
        let json = r#"{
            "session_id": "abc-123",
            "transcript_path": "/path/to/transcript",
            "cwd": "/home/user/project",
            "permission_mode": "default",
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": "ls"}
        }"#;

        let input = parse_hook_input(json).expect("Failed to parse input");
        assert_eq!(input.session_id, "abc-123");
        assert_eq!(input.permission_mode, PermissionMode::Default);

        match input.event_data {
            HookEventData::PreToolUse {
                tool_name,
                tool_input,
            } => {
                assert_eq!(tool_name, "Bash");
                assert_eq!(tool_input["command"], "ls");
            }
            _ => panic!("Expected PreToolUse event data"),
        }
    }

    #[test]
    fn test_parse_hook_output() {
        let json = r#"{
            "continue": false,
            "stopReason": "Blocked by security policy",
            "decision": "block",
            "reason": "Path traversal detected"
        }"#;

        let output = parse_hook_output(json).expect("Failed to parse output");
        assert_eq!(output.continue_execution, Some(false));
        assert_eq!(output.decision, Some(HookDecision::Block));
    }

    #[test]
    fn test_exit_code_interpretation() {
        assert!(HookExitCode::from_code(0).is_success());
        assert!(HookExitCode::from_code(2).is_blocking());
        assert!(!HookExitCode::from_code(1).is_blocking());
    }

    #[test]
    fn test_hook_definition_timeout() {
        let with_timeout = HookDefinition {
            hook_type: HookType::Command,
            command: "echo test".to_string(),
            timeout: Some(120),
        };
        assert_eq!(with_timeout.timeout_secs(), 120);

        let without_timeout = HookDefinition {
            hook_type: HookType::Command,
            command: "echo test".to_string(),
            timeout: None,
        };
        assert_eq!(without_timeout.timeout_secs(), 60);
    }

    #[test]
    fn test_serialize_hooks_config_roundtrip() {
        let mut hooks = HashMap::new();
        hooks.insert(
            HookEvent::PreToolUse,
            vec![HookMatcher {
                matcher: "Bash".to_string(),
                hooks: vec![HookDefinition {
                    hook_type: HookType::Command,
                    command: "echo 'test'".to_string(),
                    timeout: Some(30),
                }],
            }],
        );
        hooks.insert(
            HookEvent::SessionStart,
            vec![HookMatcher {
                matcher: "startup".to_string(),
                hooks: vec![HookDefinition {
                    hook_type: HookType::Command,
                    command: "echo 'starting'".to_string(),
                    timeout: None,
                }],
            }],
        );

        let config = HooksConfig { hooks };

        // Serialize to JSON
        let json = serialize_hooks_config(&config).expect("Failed to serialize");

        // Deserialize back
        let parsed = parse_hooks_config(&json).expect("Failed to parse");

        // Should round-trip correctly
        assert_eq!(config, parsed);
        assert_eq!(parsed.hooks.len(), 2);
    }

    #[test]
    fn test_hook_output_field_names() {
        // Ensure the field names match the JSON format from Claude
        let json = r#"{
            "continue": false,
            "stopReason": "test reason"
        }"#;

        let output = parse_hook_output(json).expect("Failed to parse");
        assert_eq!(output.continue_execution, Some(false));
        assert_eq!(output.stop_reason, Some("test reason".to_string()));

        // Test serialization produces correct field names
        let output = HookOutput {
            continue_execution: Some(true),
            stop_reason: Some("reason".to_string()),
            suppress_output: None,
            decision: Some(HookDecision::Approve),
            reason: Some("allowed".to_string()),
            system_message: None,
            permission_decision: None,
            hook_specific_output: None,
        };

        let json = serde_json::to_string(&output).expect("Failed to serialize");
        assert!(json.contains(r#""continue":"#));
        assert!(json.contains(r#""stopReason":"#));
        assert!(json.contains(r#""decision":"#));
    }

    #[test]
    fn test_hook_event_serialization() {
        // Test all HookEvent variants serialize to PascalCase.
        assert_json_roundtrip(&[
            (HookEvent::PreToolUse, r#""PreToolUse""#),
            (HookEvent::PostToolUse, r#""PostToolUse""#),
            (HookEvent::UserPromptSubmit, r#""UserPromptSubmit""#),
            (HookEvent::Notification, r#""Notification""#),
            (HookEvent::SessionStart, r#""SessionStart""#),
            (HookEvent::SessionEnd, r#""SessionEnd""#),
            (HookEvent::Stop, r#""Stop""#),
            (HookEvent::SubagentStop, r#""SubagentStop""#),
            (HookEvent::PreCompact, r#""PreCompact""#),
        ]);
    }

    #[test]
    fn test_session_start_matcher_serialization() {
        assert_json_roundtrip(&[
            (SessionStartMatcher::Startup, r#""startup""#),
            (SessionStartMatcher::Resume, r#""resume""#),
            (SessionStartMatcher::Clear, r#""clear""#),
            (SessionStartMatcher::Compact, r#""compact""#),
        ]);

        // Test case sensitivity - "Startup" should fail.
        assert_err_contains(
            serde_json::from_str::<SessionStartMatcher>(r#""Startup""#),
            &["unknown variant", "Startup"],
        );
    }

    #[test]
    fn test_session_end_reason_serialization() {
        assert_json_roundtrip(&[
            (SessionEndReason::Clear, r#""clear""#),
            (SessionEndReason::Logout, r#""logout""#),
            (SessionEndReason::PromptInputExit, r#""prompt_input_exit""#),
            (SessionEndReason::Other, r#""other""#),
        ]);
    }

    #[test]
    fn test_pre_compact_matcher_serialization() {
        assert_json_roundtrip(&[
            (PreCompactMatcher::Manual, r#""manual""#),
            (PreCompactMatcher::Auto, r#""auto""#),
        ]);
    }

    #[test]
    fn test_hook_type_serialization() {
        assert_json_roundtrip(&[(HookType::Command, r#""command""#)]);
    }

    #[test]
    fn test_permission_mode_serialization() {
        assert_json_roundtrip(&[
            (PermissionMode::Default, r#""default""#),
            (PermissionMode::Plan, r#""plan""#),
            (PermissionMode::AcceptEdits, r#""acceptEdits""#),
            (PermissionMode::BypassPermissions, r#""bypassPermissions""#),
        ]);
    }

    #[test]
    fn test_hook_decision_serialization() {
        assert_json_roundtrip(&[
            (HookDecision::Approve, r#""approve""#),
            (HookDecision::Block, r#""block""#),
            (HookDecision::Ask, r#""ask""#),
        ]);
    }

    #[test]
    fn test_hook_decision_rejects_old_enum_values() {
        // Test that old variant names are no longer accepted
        assert!(
            serde_json::from_str::<HookDecision>(r#""allow""#).is_err(),
            "HookDecision::Allow has been renamed to Approve"
        );
        assert!(
            serde_json::from_str::<HookDecision>(r#""deny""#).is_err(),
            "HookDecision::Deny has been renamed to Block"
        );
    }

    #[test]
    fn test_parse_errors() {
        // Invalid JSON
        assert_err_contains(
            parse_hooks_config("not valid json {"),
            &["expected", "EOF", "invalid"],
        );
        assert_err_contains(
            parse_hook_input("not valid json {"),
            &["expected", "EOF", "invalid"],
        );
        assert_err_contains(
            parse_hook_output("not valid json {"),
            &["expected", "EOF", "invalid"],
        );

        // Invalid enum values
        assert_err_contains(
            serde_json::from_str::<HookDecision>(r#""invalid""#),
            &["unknown variant", "invalid"],
        );
        assert_err_contains(
            serde_json::from_str::<HookType>(r#""invalid""#),
            &["unknown variant", "invalid"],
        );
        assert_err_contains(
            serde_json::from_str::<PermissionMode>(r#""invalid""#),
            &["unknown variant", "invalid"],
        );

        // Missing required fields
        assert_err_contains(
            parse_hook_input(r#"{"session_id": "abc"}"#),
            &["missing field", "event"],
        );
    }

    #[test]
    fn test_hook_output_omits_none_fields() {
        let output = HookOutput {
            continue_execution: Some(true),
            stop_reason: None,
            suppress_output: None,
            decision: Some(HookDecision::Approve),
            reason: None,
            system_message: None,
            permission_decision: None,
            hook_specific_output: None,
        };

        let json = serde_json::to_string(&output).expect("Failed to serialize");
        assert!(json.contains(r#""continue""#));
        assert!(json.contains(r#""decision""#));
        assert!(!json.contains(r#""stopReason""#));
        assert!(!json.contains(r#""suppressOutput""#));
        assert!(!json.contains(r#""reason""#));
        assert!(!json.contains(r#""systemMessage""#));
    }

    #[test]
    fn test_hook_output_default_serializes_to_empty_object() {
        let json =
            serde_json::to_string(&HookOutput::default()).expect("Failed to serialize default");
        assert_eq!(json, "{}");
    }

    #[test]
    fn test_hook_output_serializes_system_message() {
        let output = HookOutput {
            continue_execution: None,
            stop_reason: None,
            suppress_output: None,
            decision: Some(HookDecision::Block),
            reason: Some("Detailed log message".to_string()),
            system_message: Some("User-facing error".to_string()),
            permission_decision: None,
            hook_specific_output: None,
        };

        let json = serde_json::to_string(&output).expect("Failed to serialize");

        // Verify camelCase field name
        assert!(
            json.contains(r#""systemMessage":"User-facing error""#),
            "Expected systemMessage in JSON, got: {}",
            json
        );
        assert!(
            json.contains(r#""reason":"Detailed log message""#),
            "Expected reason in JSON, got: {}",
            json
        );
    }

    #[test]
    fn test_hook_output_omits_none_system_message() {
        let output = HookOutput {
            continue_execution: Some(true),
            stop_reason: None,
            suppress_output: None,
            decision: Some(HookDecision::Approve),
            reason: None,
            system_message: None, // Explicitly None
            permission_decision: None,
            hook_specific_output: None,
        };

        let json = serde_json::to_string(&output).expect("Failed to serialize");
        assert!(
            !json.contains("systemMessage"),
            "Should not include systemMessage when None, got: {}",
            json
        );
    }

    #[test]
    fn test_pretool_output_with_system_message_serialization() {
        let output = HookOutput {
            continue_execution: None,
            stop_reason: None,
            suppress_output: None,
            decision: None,
            reason: None,
            system_message: Some("Modified: added safety flags".to_string()),
            permission_decision: None,
            hook_specific_output: Some(HookSpecificOutput::PreToolUse(PreToolUseOutput {
                hook_event_name: "PreToolUse".to_string(),
                permission_decision: Some(PermissionDecision::Allow),
                permission_decision_reason: Some("Modified: added safety flags".to_string()),
                updated_input: Some(serde_json::json!({"command": "safe-cmd"})),
            })),
        };

        let json = serde_json::to_string(&output).expect("Failed to serialize");

        // Verify system_message appears at top level
        assert!(json.contains(r#""systemMessage":"Modified: added safety flags""#));

        // Verify hook_specific_output also has the reason
        assert!(json.contains(r#""permissionDecisionReason":"Modified: added safety flags""#));
    }

    #[test]
    fn test_system_message_with_special_characters() {
        let message = r#"Error: "quoted" & <tags> and \backslashes"#;
        let output = HookOutput {
            continue_execution: None,
            stop_reason: None,
            suppress_output: None,
            decision: Some(HookDecision::Block),
            reason: Some(message.to_string()),
            system_message: Some(message.to_string()),
            permission_decision: None,
            hook_specific_output: None,
        };

        // Verify JSON escaping works correctly
        let json = serde_json::to_string(&output).expect("Failed to serialize");
        let deserialized: HookOutput = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.system_message, Some(message.to_string()));
    }

    #[test]
    fn test_system_message_empty_string() {
        let output = HookOutput {
            continue_execution: None,
            stop_reason: None,
            suppress_output: None,
            decision: Some(HookDecision::Approve),
            reason: Some(String::new()),
            system_message: Some(String::new()),
            permission_decision: None,
            hook_specific_output: None,
        };

        // Empty string should still be included (not omitted like None)
        let json = serde_json::to_string(&output).expect("Failed to serialize");
        assert!(json.contains(r#""systemMessage":"""#));
    }

    #[test]
    fn test_empty_hooks_config() {
        let config = HooksConfig::default();
        assert!(config.hooks.is_empty());

        let json = serialize_hooks_config(&config).expect("Failed to serialize");
        assert_eq!(json, "{}");

        let parsed = parse_hooks_config(&json).expect("Failed to parse");
        assert_eq!(parsed, config);
    }

    /// Build a hook input JSON envelope for the given event name with optional
    /// extra JSON fields (a comma-separated fragment, no surrounding braces).
    fn hook_envelope(event_name: &str, extras: &str) -> String {
        let sep = if extras.is_empty() { "" } else { "," };
        format!(
            r#"{{
                "session_id": "test",
                "transcript_path": "/path",
                "cwd": "/cwd",
                "permission_mode": "default",
                "hook_event_name": "{event_name}"{sep}{extras}
            }}"#
        )
    }

    /// Row in the hook-event-data table-driven test: `(event_name,
    /// extra_fields_json, variant_check_closure)`. The closure receives the
    /// `event_name` so that assertion failures inside the match arm can name
    /// the failing variant.
    type EventDataCase<'a> = (&'a str, &'a str, &'a dyn Fn(&str, HookEventData));

    #[test]
    fn test_hook_event_data_variants() {
        let cases: &[EventDataCase] = &[
            (
                "PreToolUse",
                r#""tool_name": "Write", "tool_input": {"file_path": "test.txt", "content": "hello"}"#,
                &|name, event| match event {
                    HookEventData::PreToolUse {
                        tool_name,
                        tool_input,
                    } => {
                        assert_eq!(tool_name, "Write", "case {name}: tool_name");
                        assert_eq!(
                            tool_input["file_path"], "test.txt",
                            "case {name}: file_path"
                        );
                        assert_eq!(tool_input["content"], "hello", "case {name}: content");
                    }
                    _ => panic!("case {name}: expected PreToolUse"),
                },
            ),
            (
                "PostToolUse",
                r#""tool_name": "Read", "tool_input": {"file_path": "test.txt"}, "tool_response": {"content": "file contents"}"#,
                &|name, event| match event {
                    HookEventData::PostToolUse {
                        tool_name,
                        tool_input,
                        tool_response,
                    } => {
                        assert_eq!(tool_name, "Read", "case {name}: tool_name");
                        assert_eq!(
                            tool_input["file_path"], "test.txt",
                            "case {name}: file_path"
                        );
                        assert_eq!(
                            tool_response["content"], "file contents",
                            "case {name}: tool_response content"
                        );
                    }
                    _ => panic!("case {name}: expected PostToolUse"),
                },
            ),
            (
                "UserPromptSubmit",
                r#""user_prompt": "Please help me write code""#,
                &|name, event| match event {
                    HookEventData::UserPromptSubmit { user_prompt } => {
                        assert_eq!(
                            user_prompt, "Please help me write code",
                            "case {name}: user_prompt"
                        );
                    }
                    _ => panic!("case {name}: expected UserPromptSubmit"),
                },
            ),
            (
                "SessionStart",
                r#""matcher": "startup""#,
                &|name, event| match event {
                    HookEventData::SessionStart { matcher } => {
                        assert_eq!(
                            matcher,
                            SessionStartMatcher::Startup,
                            "case {name}: matcher"
                        );
                    }
                    _ => panic!("case {name}: expected SessionStart"),
                },
            ),
            (
                "SessionEnd",
                r#""reason": "logout""#,
                &|name, event| match event {
                    HookEventData::SessionEnd { reason } => {
                        assert_eq!(reason, SessionEndReason::Logout, "case {name}: reason");
                    }
                    _ => panic!("case {name}: expected SessionEnd"),
                },
            ),
            ("Stop", "", &|name, event| match event {
                HookEventData::Stop => {}
                _ => panic!("case {name}: expected Stop"),
            }),
            (
                "SubagentStop",
                r#""subagent_id": "agent-123""#,
                &|name, event| match event {
                    HookEventData::SubagentStop { subagent_id } => {
                        assert_eq!(subagent_id, "agent-123", "case {name}: subagent_id");
                    }
                    _ => panic!("case {name}: expected SubagentStop"),
                },
            ),
            (
                "PreCompact",
                r#""matcher": "manual""#,
                &|name, event| match event {
                    HookEventData::PreCompact { matcher } => {
                        assert_eq!(matcher, PreCompactMatcher::Manual, "case {name}: matcher");
                    }
                    _ => panic!("case {name}: expected PreCompact"),
                },
            ),
            (
                "Notification",
                r#""notification_type": "permission_request", "message": "Tool requires approval""#,
                &|name, event| match event {
                    HookEventData::Notification {
                        notification_type,
                        message,
                    } => {
                        assert_eq!(
                            notification_type, "permission_request",
                            "case {name}: notification_type"
                        );
                        assert_eq!(message, "Tool requires approval", "case {name}: message");
                    }
                    _ => panic!("case {name}: expected Notification"),
                },
            ),
        ];

        for (event_name, extras, check) in cases {
            let json = hook_envelope(event_name, extras);
            let input = parse_hook_input(&json)
                .unwrap_or_else(|e| panic!("Failed to parse {event_name}: {e}"));
            check(event_name, input.event_data);
        }
    }

    #[test]
    fn test_permission_decision_serialization() {
        assert_json_roundtrip(&[
            (PermissionDecision::Allow, r#""allow""#),
            (PermissionDecision::Deny, r#""deny""#),
            (PermissionDecision::Ask, r#""ask""#),
        ]);
    }

    #[test]
    fn test_pretool_use_output_serialization_variants() {
        let cases = [
            (
                PreToolUseOutput {
                    hook_event_name: "PreToolUse".to_string(),
                    permission_decision: Some(PermissionDecision::Ask),
                    permission_decision_reason: None,
                    updated_input: None,
                },
                vec![
                    r#""hookEventName":"PreToolUse""#,
                    r#""permissionDecision":"ask""#,
                ],
                vec![r#""permissionDecisionReason""#, r#""updatedInput""#],
            ),
            (
                PreToolUseOutput {
                    hook_event_name: "PreToolUse".to_string(),
                    permission_decision: Some(PermissionDecision::Allow),
                    permission_decision_reason: None,
                    updated_input: None,
                },
                vec![r#""permissionDecision":"allow""#],
                vec![],
            ),
            (
                PreToolUseOutput {
                    hook_event_name: "PreToolUse".to_string(),
                    permission_decision: Some(PermissionDecision::Deny),
                    permission_decision_reason: Some("Dangerous command".to_string()),
                    updated_input: None,
                },
                vec![
                    r#""permissionDecision":"deny""#,
                    r#""permissionDecisionReason":"Dangerous command""#,
                ],
                vec![],
            ),
            (
                PreToolUseOutput {
                    hook_event_name: "PreToolUse".to_string(),
                    permission_decision: Some(PermissionDecision::Allow),
                    permission_decision_reason: Some("Modified".to_string()),
                    updated_input: Some(serde_json::json!({"command": "ls -la"})),
                },
                vec![r#""updatedInput":{"command":"ls -la"}"#],
                vec![],
            ),
            (
                PreToolUseOutput {
                    hook_event_name: "PreToolUse".to_string(),
                    permission_decision: Some(PermissionDecision::Ask),
                    permission_decision_reason: Some("Command not in whitelist".to_string()),
                    updated_input: None,
                },
                vec![
                    r#""permissionDecision":"ask""#,
                    r#""permissionDecisionReason":"Command not in whitelist""#,
                ],
                vec![],
            ),
            (
                PreToolUseOutput {
                    hook_event_name: "PreToolUse".to_string(),
                    permission_decision: Some(PermissionDecision::Deny),
                    permission_decision_reason: Some("Blocked".to_string()),
                    updated_input: Some(serde_json::json!({"command": "ls"})),
                },
                vec![r#""permissionDecision":"deny""#, r#""updatedInput""#],
                vec![],
            ),
        ];

        for (output, present, absent) in cases {
            assert_json_contains(&output, &present, &absent);
        }
    }

    #[test]
    fn test_pretool_use_output_complex_updated_input_roundtrips() {
        let complex_input = serde_json::json!({
            "command": "docker run",
            "env": {
                "KEY1": "value1",
                "KEY2": "value2"
            },
            "volumes": ["/host:/container", "/data:/data"]
        });
        let complex_output = PreToolUseOutput {
            hook_event_name: "PreToolUse".to_string(),
            permission_decision: Some(PermissionDecision::Allow),
            permission_decision_reason: Some("Modified".to_string()),
            updated_input: Some(complex_input.clone()),
        };
        let json = serde_json::to_string(&complex_output).expect("Failed to serialize");
        let parsed: PreToolUseOutput = serde_json::from_str(&json).expect("Failed to deserialize");
        assert_eq!(parsed.updated_input, Some(complex_input));
    }

    #[test]
    fn test_pretool_use_output_roundtrip() {
        // Test round-trip for Deny with reason
        let json = r#"{
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": "Dangerous command"
        }"#;

        let output: PreToolUseOutput = serde_json::from_str(json).expect("Failed to parse");
        assert_eq!(output.hook_event_name, "PreToolUse");
        assert_eq!(output.permission_decision, Some(PermissionDecision::Deny));
        assert_eq!(
            output.permission_decision_reason,
            Some("Dangerous command".to_string())
        );

        let reserialized = serde_json::to_string(&output).expect("Failed to serialize");
        let reparsed: PreToolUseOutput =
            serde_json::from_str(&reserialized).expect("Failed to parse");
        assert_eq!(reparsed.permission_decision, output.permission_decision);
        assert_eq!(
            reparsed.permission_decision_reason,
            output.permission_decision_reason
        );

        // Test round-trip for Allow with modified input
        let json_with_input = r#"{
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "permissionDecisionReason": "Modified command",
            "updatedInput": {"command": "ls -la"}
        }"#;

        let output: PreToolUseOutput =
            serde_json::from_str(json_with_input).expect("Failed to parse");
        assert_eq!(output.permission_decision, Some(PermissionDecision::Allow));
        assert_eq!(output.updated_input.as_ref().unwrap()["command"], "ls -la");

        let reserialized = serde_json::to_string(&output).expect("Failed to serialize");
        let reparsed: PreToolUseOutput =
            serde_json::from_str(&reserialized).expect("Failed to parse");
        assert_eq!(reparsed.updated_input, output.updated_input);
    }

    #[test]
    fn test_user_prompt_submit_output_serialization() {
        let output = UserPromptSubmitOutput {
            hook_event_name: "UserPromptSubmit".to_string(),
            additional_context: "Project uses TypeScript 5.0".to_string(),
        };

        let json = serde_json::to_string(&output).expect("Failed to serialize");
        assert!(json.contains(r#""hookEventName":"UserPromptSubmit""#));
        assert!(json.contains(r#""additionalContext":"Project uses TypeScript 5.0""#));

        // Test deserialization
        let parsed: UserPromptSubmitOutput = serde_json::from_str(&json).expect("Failed to parse");
        assert_eq!(parsed.hook_event_name, output.hook_event_name);
        assert_eq!(parsed.additional_context, output.additional_context);
    }

    #[test]
    fn test_hook_output_with_user_prompt_submit_output() {
        let output = HookOutput {
            continue_execution: None,
            stop_reason: None,
            suppress_output: None,
            decision: None,
            reason: None,
            system_message: None,
            permission_decision: None,
            hook_specific_output: Some(HookSpecificOutput::UserPromptSubmit(
                UserPromptSubmitOutput {
                    hook_event_name: "UserPromptSubmit".to_string(),
                    additional_context: "Test context".to_string(),
                },
            )),
        };

        let json = serde_json::to_string(&output).expect("Failed to serialize");
        assert!(json.contains(r#""hookSpecificOutput""#));
        assert!(json.contains(r#""additionalContext":"Test context""#));

        // Verify round-trip
        let parsed: HookOutput = serde_json::from_str(&json).expect("Failed to parse");
        match parsed.hook_specific_output {
            Some(HookSpecificOutput::UserPromptSubmit(ref submit_output)) => {
                assert_eq!(submit_output.additional_context, "Test context");
            }
            _ => panic!("Expected UserPromptSubmit hook specific output"),
        }
    }

    #[test]
    fn test_hook_output_with_hook_specific_output_serialization() {
        let output = HookOutput {
            continue_execution: None,
            stop_reason: None,
            suppress_output: None,
            decision: None,
            reason: None,
            system_message: None,
            permission_decision: None,
            hook_specific_output: Some(HookSpecificOutput::PreToolUse(PreToolUseOutput {
                hook_event_name: "PreToolUse".to_string(),
                permission_decision: Some(PermissionDecision::Ask),
                permission_decision_reason: None,
                updated_input: None,
            })),
        };

        let json = serde_json::to_string(&output).expect("Failed to serialize");
        assert!(json.contains(r#""hookSpecificOutput""#));
        assert!(json.contains(r#""hookEventName":"PreToolUse""#));
        assert!(json.contains(r#""permissionDecision":"ask""#));

        // Verify round-trip
        let parsed: HookOutput = serde_json::from_str(&json).expect("Failed to parse");
        match parsed.hook_specific_output {
            Some(HookSpecificOutput::PreToolUse(ref pretool_output)) => {
                assert_eq!(
                    pretool_output.permission_decision,
                    Some(PermissionDecision::Ask)
                );
            }
            _ => panic!("Expected PreToolUse hook specific output"),
        }
    }
}
