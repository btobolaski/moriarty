//! Classification of a PreToolUse hook's output into a single result label.
//!
//! The label is moriarty-internal — a one-word summary of what the hook decided — rather than
//! part of the Claude Code wire protocol, so it lives next to the hook logic instead of in
//! `parser`. The completion log records it as a clean field and `hooks report` aggregates by it.

// 3rd party crates
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

// local / workspace deps
use super::parser::{HookOutput, HookSpecificOutput, PermissionDecision};

/// The outcome of a PreToolUse hook evaluation. The serde and `ValueEnum` spellings agree
/// (`"allow"`…`"passthrough"`) so the log field, the `--result` CLI filter, and the report
/// output all use one vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum PreToolResult {
    Allow,
    Deny,
    Ask,
    Modify,
    Passthrough,
}

impl PreToolResult {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Ask => "ask",
            Self::Modify => "modify",
            Self::Passthrough => "passthrough",
        }
    }
}

/// `updated_input` is checked before the decision because a modify rewrites the command while
/// still reporting an `Allow` decision; reading the decision first would hide that the command
/// was changed. Any output without a PreToolUse-specific decision means no rule matched, so the
/// hook defers to Claude Code's native permission system (`passthrough`).
pub fn pretool_result(output: &HookOutput) -> PreToolResult {
    let Some(HookSpecificOutput::PreToolUse(pre)) = &output.hook_specific_output else {
        return PreToolResult::Passthrough;
    };

    if pre.updated_input.is_some() {
        return PreToolResult::Modify;
    }

    match pre.permission_decision {
        Some(PermissionDecision::Allow) => PreToolResult::Allow,
        Some(PermissionDecision::Deny) => PreToolResult::Deny,
        Some(PermissionDecision::Ask) => PreToolResult::Ask,
        None => PreToolResult::Passthrough,
    }
}

#[cfg(test)]
mod tests {
    use super::{PreToolResult, pretool_result};
    use crate::hooks::parser::{
        HookOutput, HookSpecificOutput, PermissionDecision, PreToolUseOutput,
    };
    use crate::hooks::{
        pretool_allow_hook, pretool_ask_hook, pretool_deny_hook, pretool_modify_hook,
    };

    #[test]
    fn allow_output_maps_to_allow() {
        assert_eq!(
            pretool_result(&pretool_allow_hook(None)),
            PreToolResult::Allow
        );
    }

    #[test]
    fn deny_output_maps_to_deny() {
        assert_eq!(
            pretool_result(&pretool_deny_hook("blocked".to_string())),
            PreToolResult::Deny
        );
    }

    #[test]
    fn ask_output_maps_to_ask() {
        assert_eq!(pretool_result(&pretool_ask_hook()), PreToolResult::Ask);
    }

    #[test]
    fn modify_output_maps_to_modify() {
        let new_input = serde_json::json!({ "command": "ls" });
        assert_eq!(
            pretool_result(&pretool_modify_hook(new_input, None)),
            PreToolResult::Modify
        );
    }

    #[test]
    fn modify_takes_priority_over_the_decision() {
        // A modify reports an updated input alongside a decision; the updated input must win so the
        // rewrite is never misreported as a plain allow/deny. Deny is used here to make the
        // precedence unambiguous.
        let output = HookOutput {
            hook_specific_output: Some(HookSpecificOutput::PreToolUse(PreToolUseOutput {
                hook_event_name: "PreToolUse".to_string(),
                permission_decision: Some(PermissionDecision::Deny),
                permission_decision_reason: None,
                updated_input: Some(serde_json::json!({ "command": "ls" })),
            })),
            ..HookOutput::default()
        };
        assert_eq!(pretool_result(&output), PreToolResult::Modify);
    }

    #[test]
    fn default_output_maps_to_passthrough() {
        assert_eq!(
            pretool_result(&HookOutput::default()),
            PreToolResult::Passthrough
        );
    }

    #[test]
    fn serde_spelling_matches_as_str() {
        for result in [
            PreToolResult::Allow,
            PreToolResult::Deny,
            PreToolResult::Ask,
            PreToolResult::Modify,
            PreToolResult::Passthrough,
        ] {
            let serialized = serde_json::to_string(&result).expect("result should serialize");
            assert_eq!(serialized, format!("\"{}\"", result.as_str()));
        }
    }
}
