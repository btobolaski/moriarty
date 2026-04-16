//! Tool call validation rules for any Claude Code tool.
//!
//! This module provides a rule engine for permissioning arbitrary tool calls (Read, Write, Edit,
//! Bash, etc.) before they are executed by Claude Code. Unlike `bash_rules` which operates on
//! command strings, tool rules match on tool name and optionally on a specific field in the
//! tool input using regex patterns.

use std::collections::HashMap;

use regex::Regex;
use tracing::{debug, warn};

use super::bash_rules::{default_fragments, expand_fragments};
use crate::user_config::{ToolRule, ToolRuleAction};

/// Runtime representation of a tool rule with pre-compiled regex for the field pattern.
#[derive(Debug)]
struct CompiledToolRule {
    name: String,
    tool: String,
    field: Option<String>,
    regex: Option<Regex>,
    action: ToolRuleAction,
}

/// Result of evaluating tool rules against a tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolRuleResult {
    Allowed { rule_name: String },
    Denied { rule_name: String, reason: String },
    Asked { rule_name: String },
    NoMatch,
}

/// Engine for evaluating tool rules using first-match-wins semantics.
#[derive(Debug)]
pub struct ToolRuleEngine {
    rules: Vec<CompiledToolRule>,
}

impl ToolRuleEngine {
    /// Compiles tool rules with pattern fragment expansion.
    ///
    /// Rules with incomplete field/pattern pairs (only one present) are logged and skipped.
    /// Invalid regex patterns (after fragment expansion) are logged and skipped.
    pub fn from_config(rules: Vec<ToolRule>, fragments: Option<HashMap<String, String>>) -> Self {
        let mut merged_fragments = default_fragments();
        if let Some(user_frags) = fragments {
            merged_fragments.extend(user_frags);
        }

        let mut compiled = Vec::new();

        for rule in rules {
            // Validate field/pattern pairing
            match (&rule.field, &rule.pattern) {
                (Some(_), None) => {
                    warn!(
                        rule_name = %rule.name,
                        "Tool rule has 'field' without 'pattern', skipping"
                    );
                    continue;
                }
                (None, Some(_)) => {
                    warn!(
                        rule_name = %rule.name,
                        "Tool rule has 'pattern' without 'field', skipping"
                    );
                    continue;
                }
                (Some(field), Some(pattern)) => {
                    // Expand fragments and compile regex
                    let expanded = match expand_fragments(pattern, &merged_fragments) {
                        Ok(p) => p,
                        Err(e) => {
                            warn!(
                                rule_name = %rule.name,
                                pattern = %pattern,
                                error = %e,
                                "Failed to expand pattern fragments in tool rule, skipping"
                            );
                            continue;
                        }
                    };

                    match Regex::new(&expanded) {
                        Ok(regex) => {
                            debug!(
                                rule_name = %rule.name,
                                tool = %rule.tool,
                                field = %field,
                                pattern = %expanded,
                                "Compiled tool rule with field pattern"
                            );
                            compiled.push(CompiledToolRule {
                                name: rule.name,
                                tool: rule.tool,
                                field: Some(field.clone()),
                                regex: Some(regex),
                                action: rule.action,
                            });
                        }
                        Err(e) => {
                            warn!(
                                rule_name = %rule.name,
                                pattern = %expanded,
                                error = %e,
                                "Invalid regex in tool rule, skipping"
                            );
                        }
                    }
                }
                (None, None) => {
                    debug!(
                        rule_name = %rule.name,
                        tool = %rule.tool,
                        "Compiled tool rule (tool-name only)"
                    );
                    compiled.push(CompiledToolRule {
                        name: rule.name,
                        tool: rule.tool,
                        field: None,
                        regex: None,
                        action: rule.action,
                    });
                }
            }
        }

        Self { rules: compiled }
    }

    /// Evaluate rules against a tool call. Returns the first matching rule's result.
    ///
    /// `tool_input` is `serde_json::Value` rather than a typed struct because Claude Code tool
    /// inputs are heterogeneous — each tool (Read, Write, Edit, Bash, Grep, etc.) has a different
    /// schema, so no single typed struct can represent them all. The upstream `HookEventData`
    /// parser already delivers `tool_input` as `serde_json::Value`.
    ///
    /// Field value extraction: strings use `as_str()`, numbers/bools use `to_string()`,
    /// arrays/objects/null are skipped (field won't match).
    pub fn apply_rules(&self, tool_name: &str, tool_input: &serde_json::Value) -> ToolRuleResult {
        for rule in &self.rules {
            // Check tool name: exact match or wildcard
            if rule.tool != "*" && rule.tool != tool_name {
                continue;
            }

            // If rule has field+regex, check the field value
            if let (Some(field), Some(regex)) = (&rule.field, &rule.regex) {
                let field_value = match tool_input.get(field) {
                    Some(v) => extract_field_value(v),
                    None => {
                        // Field doesn't exist in input, rule doesn't match
                        continue;
                    }
                };

                let Some(value_str) = field_value else {
                    // Field is an array/object/null, can't match
                    continue;
                };

                if !regex.is_match(&value_str) {
                    continue;
                }
            }

            // Rule matched
            debug!(
                rule_name = %rule.name,
                tool_name = %tool_name,
                "Tool rule matched"
            );

            return match &rule.action {
                ToolRuleAction::Allow => ToolRuleResult::Allowed {
                    rule_name: rule.name.clone(),
                },
                ToolRuleAction::Deny { value } => ToolRuleResult::Denied {
                    rule_name: rule.name.clone(),
                    reason: value.clone(),
                },
                ToolRuleAction::Ask => ToolRuleResult::Asked {
                    rule_name: rule.name.clone(),
                },
            };
        }

        ToolRuleResult::NoMatch
    }
}

/// Extract a string representation from a JSON value for regex matching.
///
/// Strings use their raw value, numbers and bools use `to_string()`.
/// Arrays, objects, and null return None (cannot be meaningfully matched by regex).
fn extract_field_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rule(
        name: &str,
        tool: &str,
        field: Option<&str>,
        pattern: Option<&str>,
        action: ToolRuleAction,
    ) -> ToolRule {
        ToolRule {
            name: name.to_string(),
            tool: tool.to_string(),
            field: field.map(|s| s.to_string()),
            pattern: pattern.map(|s| s.to_string()),
            action,
        }
    }

    #[test]
    fn test_empty_rules() {
        let engine = ToolRuleEngine::from_config(vec![], None);
        let result = engine.apply_rules("Read", &serde_json::json!({"file_path": "/tmp/foo"}));
        assert_eq!(result, ToolRuleResult::NoMatch);
    }

    #[test]
    fn test_tool_name_only_allow() {
        let rules = vec![make_rule(
            "allow-read",
            "Read",
            None,
            None,
            ToolRuleAction::Allow,
        )];
        let engine = ToolRuleEngine::from_config(rules, None);

        let result = engine.apply_rules("Read", &serde_json::json!({"file_path": "/tmp/foo"}));
        assert_eq!(
            result,
            ToolRuleResult::Allowed {
                rule_name: "allow-read".to_string()
            }
        );

        // Doesn't match other tools
        let result = engine.apply_rules("Write", &serde_json::json!({}));
        assert_eq!(result, ToolRuleResult::NoMatch);
    }

    #[test]
    fn test_tool_name_deny() {
        let rules = vec![make_rule(
            "deny-write",
            "Write",
            None,
            None,
            ToolRuleAction::Deny {
                value: "Writes not allowed".to_string(),
            },
        )];
        let engine = ToolRuleEngine::from_config(rules, None);

        let result = engine.apply_rules("Write", &serde_json::json!({}));
        assert_eq!(
            result,
            ToolRuleResult::Denied {
                rule_name: "deny-write".to_string(),
                reason: "Writes not allowed".to_string(),
            }
        );
    }

    #[test]
    fn test_tool_name_ask() {
        let rules = vec![make_rule(
            "ask-edit",
            "Edit",
            None,
            None,
            ToolRuleAction::Ask,
        )];
        let engine = ToolRuleEngine::from_config(rules, None);

        let result = engine.apply_rules("Edit", &serde_json::json!({}));
        assert_eq!(
            result,
            ToolRuleResult::Asked {
                rule_name: "ask-edit".to_string()
            }
        );
    }

    #[test]
    fn test_wildcard_matches_any_tool() {
        let rules = vec![make_rule("catch-all", "*", None, None, ToolRuleAction::Ask)];
        let engine = ToolRuleEngine::from_config(rules, None);

        assert_eq!(
            engine.apply_rules("Read", &serde_json::json!({})),
            ToolRuleResult::Asked {
                rule_name: "catch-all".to_string()
            }
        );
        assert_eq!(
            engine.apply_rules("Write", &serde_json::json!({})),
            ToolRuleResult::Asked {
                rule_name: "catch-all".to_string()
            }
        );
        assert_eq!(
            engine.apply_rules("Bash", &serde_json::json!({})),
            ToolRuleResult::Asked {
                rule_name: "catch-all".to_string()
            }
        );
    }

    #[test]
    fn test_field_pattern_matching() {
        let rules = vec![make_rule(
            "deny-env-write",
            "Write",
            Some("file_path"),
            Some(r"\.env$"),
            ToolRuleAction::Deny {
                value: "Cannot write .env files".to_string(),
            },
        )];
        let engine = ToolRuleEngine::from_config(rules, None);

        // Matches
        let result = engine.apply_rules(
            "Write",
            &serde_json::json!({"file_path": "/home/user/.env"}),
        );
        assert_eq!(
            result,
            ToolRuleResult::Denied {
                rule_name: "deny-env-write".to_string(),
                reason: "Cannot write .env files".to_string(),
            }
        );

        // Doesn't match (different extension)
        let result = engine.apply_rules(
            "Write",
            &serde_json::json!({"file_path": "/home/user/main.rs"}),
        );
        assert_eq!(result, ToolRuleResult::NoMatch);
    }

    #[test]
    fn test_field_pattern_missing_field_in_input() {
        let rules = vec![make_rule(
            "deny-env",
            "Write",
            Some("file_path"),
            Some(r"\.env$"),
            ToolRuleAction::Deny {
                value: "no".to_string(),
            },
        )];
        let engine = ToolRuleEngine::from_config(rules, None);

        // Field doesn't exist in input
        let result = engine.apply_rules("Write", &serde_json::json!({"content": "hello"}));
        assert_eq!(result, ToolRuleResult::NoMatch);
    }

    #[test]
    fn test_field_value_extraction_types() {
        let rules = vec![make_rule(
            "match-number",
            "Test",
            Some("count"),
            Some("^42$"),
            ToolRuleAction::Allow,
        )];
        let engine = ToolRuleEngine::from_config(rules, None);

        // Number field
        let result = engine.apply_rules("Test", &serde_json::json!({"count": 42}));
        assert_eq!(
            result,
            ToolRuleResult::Allowed {
                rule_name: "match-number".to_string()
            }
        );

        // Bool field (doesn't match "42")
        let result = engine.apply_rules("Test", &serde_json::json!({"count": true}));
        assert_eq!(result, ToolRuleResult::NoMatch);

        // Array/object/null fields can't be matched
        let result = engine.apply_rules("Test", &serde_json::json!({"count": [42]}));
        assert_eq!(result, ToolRuleResult::NoMatch);

        let result = engine.apply_rules("Test", &serde_json::json!({"count": null}));
        assert_eq!(result, ToolRuleResult::NoMatch);

        // Bool positive match (bools are converted to "true"/"false" strings)
        let bool_rules = vec![make_rule(
            "match-bool",
            "Test",
            Some("flag"),
            Some("^true$"),
            ToolRuleAction::Allow,
        )];
        let bool_engine = ToolRuleEngine::from_config(bool_rules, None);

        let result = bool_engine.apply_rules("Test", &serde_json::json!({"flag": true}));
        assert_eq!(
            result,
            ToolRuleResult::Allowed {
                rule_name: "match-bool".to_string()
            }
        );

        let result = bool_engine.apply_rules("Test", &serde_json::json!({"flag": false}));
        assert_eq!(result, ToolRuleResult::NoMatch);
    }

    #[test]
    fn test_first_match_wins() {
        let rules = vec![
            make_rule(
                "allow-specific",
                "Write",
                Some("file_path"),
                Some(r"\.rs$"),
                ToolRuleAction::Allow,
            ),
            make_rule(
                "deny-all-writes",
                "Write",
                None,
                None,
                ToolRuleAction::Deny {
                    value: "Writes denied".to_string(),
                },
            ),
        ];
        let engine = ToolRuleEngine::from_config(rules, None);

        // .rs file matches first rule
        let result = engine.apply_rules("Write", &serde_json::json!({"file_path": "main.rs"}));
        assert_eq!(
            result,
            ToolRuleResult::Allowed {
                rule_name: "allow-specific".to_string()
            }
        );

        // Other file matches second rule
        let result = engine.apply_rules("Write", &serde_json::json!({"file_path": "data.csv"}));
        assert_eq!(
            result,
            ToolRuleResult::Denied {
                rule_name: "deny-all-writes".to_string(),
                reason: "Writes denied".to_string(),
            }
        );
    }

    #[test]
    fn test_incomplete_field_pattern_skipped() {
        let rules = vec![
            // field without pattern - should be skipped
            make_rule(
                "bad-field-only",
                "Write",
                Some("file_path"),
                None,
                ToolRuleAction::Deny {
                    value: "bad".to_string(),
                },
            ),
            // pattern without field - should be skipped
            make_rule(
                "bad-pattern-only",
                "Write",
                None,
                Some(r"\.env$"),
                ToolRuleAction::Deny {
                    value: "bad".to_string(),
                },
            ),
            // Valid fallback
            make_rule("fallback", "Write", None, None, ToolRuleAction::Ask),
        ];
        let engine = ToolRuleEngine::from_config(rules, None);

        // Both bad rules should be skipped, fallback should match
        let result = engine.apply_rules("Write", &serde_json::json!({"file_path": "/home/.env"}));
        assert_eq!(
            result,
            ToolRuleResult::Asked {
                rule_name: "fallback".to_string()
            }
        );
    }

    #[test]
    fn test_invalid_regex_skipped() {
        let rules = vec![
            make_rule(
                "bad-regex",
                "Write",
                Some("file_path"),
                Some("[invalid("),
                ToolRuleAction::Deny {
                    value: "bad".to_string(),
                },
            ),
            make_rule("fallback", "Write", None, None, ToolRuleAction::Allow),
        ];
        let engine = ToolRuleEngine::from_config(rules, None);

        let result = engine.apply_rules("Write", &serde_json::json!({"file_path": "/home/.env"}));
        assert_eq!(
            result,
            ToolRuleResult::Allowed {
                rule_name: "fallback".to_string()
            }
        );
    }

    #[test]
    fn test_fragment_expansion_in_pattern() {
        let mut fragments = HashMap::new();
        fragments.insert("project".to_string(), "/home/user/project".to_string());

        let rules = vec![make_rule(
            "allow-project-read",
            "Read",
            Some("file_path"),
            Some("^{{project}}/"),
            ToolRuleAction::Allow,
        )];
        let engine = ToolRuleEngine::from_config(rules, Some(fragments));

        let result = engine.apply_rules(
            "Read",
            &serde_json::json!({"file_path": "/home/user/project/src/main.rs"}),
        );
        assert_eq!(
            result,
            ToolRuleResult::Allowed {
                rule_name: "allow-project-read".to_string()
            }
        );

        let result = engine.apply_rules("Read", &serde_json::json!({"file_path": "/other/path"}));
        assert_eq!(result, ToolRuleResult::NoMatch);
    }

    #[test]
    fn test_specific_tool_before_wildcard() {
        let rules = vec![
            make_rule("allow-read", "Read", None, None, ToolRuleAction::Allow),
            make_rule("ask-all", "*", None, None, ToolRuleAction::Ask),
        ];
        let engine = ToolRuleEngine::from_config(rules, None);

        // Read matches specific rule
        let result = engine.apply_rules("Read", &serde_json::json!({}));
        assert_eq!(
            result,
            ToolRuleResult::Allowed {
                rule_name: "allow-read".to_string()
            }
        );

        // Write falls through to wildcard
        let result = engine.apply_rules("Write", &serde_json::json!({}));
        assert_eq!(
            result,
            ToolRuleResult::Asked {
                rule_name: "ask-all".to_string()
            }
        );
    }
}
