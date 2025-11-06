//! Bash command validation and modification rules.
//!
//! This module provides a rule engine for validating and modifying Bash tool use commands
//! before they are executed by Claude Code. Rules can deny dangerous commands, modify
//! commands to add safety flags, or explicitly allow specific patterns.

use miette::{miette, Result};
use regex::{Regex, RegexSet};
use tracing::debug;

use crate::user_config::{BashRule, BashRuleAction};

/// Runtime representation of a rule with pre-compiled regex for efficient matching.
///
/// Separated from `BashRule` to avoid storing `Regex` (which doesn't implement serde traits)
/// in the TOML-deserializable config struct.
#[derive(Debug)]
struct CompiledRule {
    name: String,
    regex: Regex,
    action: BashRuleAction,
}

/// Includes `rule_name` in all match variants to support logging and debugging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleResult {
    Allowed {
        rule_name: String,
    },
    Denied {
        rule_name: String,
        reason: String,
    },
    Modified {
        rule_name: String,
        new_command: String,
    },
    Asked {
        rule_name: String,
    },
    NoMatch,
}

/// Engine for evaluating bash command rules using RegexSet for O(1) parallel pattern matching.
///
/// Applies first-match-wins semantics: the first regex that matches determines the action.
#[derive(Debug)]
pub struct BashRuleEngine {
    regex_set: RegexSet,
    rules: Vec<CompiledRule>,
}

impl BashRuleEngine {
    /// Compiles regex patterns, skipping invalid ones with error logging.
    ///
    /// Invalid regex patterns are logged and skipped rather than causing failure, allowing
    /// valid rules to continue working even if some rules have errors.
    pub fn from_config(rules: Vec<BashRule>) -> Result<Self> {
        let mut compiled_rules = Vec::new();
        let mut patterns = Vec::new();

        for rule in rules {
            match Regex::new(&rule.pattern) {
                Ok(regex) => {
                    patterns.push(rule.pattern.clone());
                    compiled_rules.push(CompiledRule {
                        name: rule.name,
                        regex,
                        action: rule.action,
                    });
                }
                Err(e) => {
                    tracing::error!(
                        rule_name = %rule.name,
                        pattern = %rule.pattern,
                        error = %e,
                        "Invalid regex pattern in bash rule, skipping rule"
                    );
                }
            }
        }

        let regex_set = RegexSet::new(patterns)
            .map_err(|e| miette!("Failed to build RegexSet from patterns: {}", e))?;

        Ok(Self {
            regex_set,
            rules: compiled_rules,
        })
    }

    pub fn apply_rules(&self, command: &str) -> RuleResult {
        let matches = self.regex_set.matches(command);

        if let Some(first_match_idx) = matches.iter().next() {
            let rule = &self.rules[first_match_idx];

            debug!(
                rule_name = %rule.name,
                command = %command,
                "Bash rule matched"
            );

            return match &rule.action {
                BashRuleAction::Deny(reason) => RuleResult::Denied {
                    rule_name: rule.name.clone(),
                    reason: reason.clone(),
                },
                BashRuleAction::Modify(template) => {
                    let captures = rule
                        .regex
                        .captures(command)
                        .expect("Invariant violation: RegexSet and Regex desynchronized");
                    let new_command = expand_captures(&captures, template);
                    debug!(
                        rule_name = %rule.name,
                        original = %command,
                        modified = %new_command,
                        "Command modified by rule"
                    );
                    RuleResult::Modified {
                        rule_name: rule.name.clone(),
                        new_command,
                    }
                }
                BashRuleAction::Allow => {
                    debug!(rule_name = %rule.name, "Command explicitly allowed");
                    RuleResult::Allowed {
                        rule_name: rule.name.clone(),
                    }
                }
                BashRuleAction::Ask => {
                    debug!(
                        rule_name = %rule.name,
                        command = %command,
                        "Deferring to user for case-by-case authorization decision"
                    );
                    RuleResult::Asked {
                        rule_name: rule.name.clone(),
                    }
                }
            };
        }

        RuleResult::NoMatch
    }
}

/// Processes capture groups in reverse order to prevent multi-digit group numbers from being
/// partially replaced (e.g., $10 being treated as $1 followed by "0").
fn expand_captures(captures: &regex::Captures, template: &str) -> String {
    let mut result = template.to_string();

    for i in (0..captures.len()).rev() {
        if let Some(capture) = captures.get(i) {
            let placeholder = format!("${}", i);
            result = result.replace(&placeholder, capture.as_str());
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_rules() {
        let engine = BashRuleEngine::from_config(vec![]).unwrap();
        let result = engine.apply_rules("ls -la");
        assert_eq!(result, RuleResult::NoMatch);
    }

    #[test]
    fn test_deny_rule() {
        let rules = vec![BashRule {
            name: "deny-rm-rf".to_string(),
            pattern: r"^rm\s+-rf\s+/".to_string(),
            action: BashRuleAction::Deny("Dangerous recursive delete".to_string()),
        }];

        let engine = BashRuleEngine::from_config(rules).unwrap();
        let result = engine.apply_rules("rm -rf /");

        match result {
            RuleResult::Denied { rule_name, reason } => {
                assert_eq!(rule_name, "deny-rm-rf");
                assert_eq!(reason, "Dangerous recursive delete");
            }
            _ => panic!("Expected Denied result"),
        }
    }

    #[test]
    fn test_allow_rule() {
        let rules = vec![BashRule {
            name: "allow-ls".to_string(),
            pattern: r"^ls($|\s)".to_string(),
            action: BashRuleAction::Allow,
        }];

        let engine = BashRuleEngine::from_config(rules).unwrap();
        let result = engine.apply_rules("ls -la");
        assert_eq!(
            result,
            RuleResult::Allowed {
                rule_name: "allow-ls".to_string()
            }
        );
    }

    #[test]
    fn test_ask_rule() {
        let rules = vec![BashRule {
            name: "ask-docker".to_string(),
            pattern: r"^docker".to_string(),
            action: BashRuleAction::Ask,
        }];

        let engine = BashRuleEngine::from_config(rules).unwrap();
        let result = engine.apply_rules("docker build");
        assert_eq!(
            result,
            RuleResult::Asked {
                rule_name: "ask-docker".to_string()
            }
        );
    }

    #[test]
    fn test_modify_rule_simple() {
        let rules = vec![BashRule {
            name: "add-dry-run".to_string(),
            pattern: r"^(docker\s+system\s+prune)$".to_string(),
            action: BashRuleAction::Modify("$1 --dry-run".to_string()),
        }];

        let engine = BashRuleEngine::from_config(rules).unwrap();
        let result = engine.apply_rules("docker system prune");

        match result {
            RuleResult::Modified {
                rule_name,
                new_command,
            } => {
                assert_eq!(rule_name, "add-dry-run");
                assert_eq!(new_command, "docker system prune --dry-run");
            }
            _ => panic!("Expected Modified result"),
        }
    }

    #[test]
    fn test_modify_rule_multiple_groups() {
        let rules = vec![BashRule {
            name: "swap-args".to_string(),
            pattern: r"^echo\s+(\w+)\s+(\w+)$".to_string(),
            action: BashRuleAction::Modify("echo $2 $1".to_string()),
        }];

        let engine = BashRuleEngine::from_config(rules).unwrap();
        let result = engine.apply_rules("echo hello world");

        match result {
            RuleResult::Modified {
                rule_name,
                new_command,
            } => {
                assert_eq!(rule_name, "swap-args");
                assert_eq!(new_command, "echo world hello");
            }
            _ => panic!("Expected Modified result"),
        }
    }

    #[test]
    fn test_first_match_wins() {
        let rules = vec![
            BashRule {
                name: "allow-ls".to_string(),
                pattern: r"^ls".to_string(),
                action: BashRuleAction::Allow,
            },
            BashRule {
                name: "deny-all".to_string(),
                pattern: r".*".to_string(),
                action: BashRuleAction::Deny("All commands denied".to_string()),
            },
        ];

        let engine = BashRuleEngine::from_config(rules).unwrap();

        // ls matches first rule (allow)
        let result = engine.apply_rules("ls -la");
        assert_eq!(
            result,
            RuleResult::Allowed {
                rule_name: "allow-ls".to_string()
            }
        );

        // rm matches second rule (deny)
        let result = engine.apply_rules("rm file.txt");
        match result {
            RuleResult::Denied { rule_name, .. } => {
                assert_eq!(rule_name, "deny-all");
            }
            _ => panic!("Expected Denied result"),
        }
    }

    #[test]
    fn test_ask_overrides_allow_with_ordering() {
        let rules = vec![
            BashRule {
                name: "ask-specific-docker".to_string(),
                pattern: r"^docker\s+system\s+prune".to_string(),
                action: BashRuleAction::Ask,
            },
            BashRule {
                name: "allow-all-docker".to_string(),
                pattern: r"^docker".to_string(),
                action: BashRuleAction::Allow,
            },
        ];

        let engine = BashRuleEngine::from_config(rules).unwrap();

        // Specific pattern matches first (ask)
        let result = engine.apply_rules("docker system prune");
        assert_eq!(
            result,
            RuleResult::Asked {
                rule_name: "ask-specific-docker".to_string()
            }
        );

        // Generic pattern matches second (allow)
        let result = engine.apply_rules("docker build");
        assert_eq!(
            result,
            RuleResult::Allowed {
                rule_name: "allow-all-docker".to_string()
            }
        );
    }

    #[test]
    fn test_ask_vs_deny_ordering() {
        // Test 1: Ask before Deny - Ask wins
        let rules = vec![
            BashRule {
                name: "ask-specific".to_string(),
                pattern: r"^docker\s+system\s+prune".to_string(),
                action: BashRuleAction::Ask,
            },
            BashRule {
                name: "deny-all-docker".to_string(),
                pattern: r"^docker".to_string(),
                action: BashRuleAction::Deny("Docker denied".to_string()),
            },
        ];

        let engine = BashRuleEngine::from_config(rules).unwrap();
        let result = engine.apply_rules("docker system prune");
        assert_eq!(
            result,
            RuleResult::Asked {
                rule_name: "ask-specific".to_string()
            }
        );

        // Test 2: Deny before Ask - Deny wins
        let rules = vec![
            BashRule {
                name: "deny-all-docker".to_string(),
                pattern: r"^docker".to_string(),
                action: BashRuleAction::Deny("Docker denied".to_string()),
            },
            BashRule {
                name: "ask-specific".to_string(),
                pattern: r"^docker\s+system\s+prune".to_string(),
                action: BashRuleAction::Ask,
            },
        ];

        let engine = BashRuleEngine::from_config(rules).unwrap();
        let result = engine.apply_rules("docker system prune");
        match result {
            RuleResult::Denied { rule_name, reason } => {
                assert_eq!(rule_name, "deny-all-docker");
                assert_eq!(reason, "Docker denied");
            }
            _ => panic!("Expected Denied result"),
        }
    }

    #[test]
    fn test_ask_vs_modify_ordering() {
        // Test 1: Ask before Modify - Ask wins
        let rules = vec![
            BashRule {
                name: "ask-specific".to_string(),
                pattern: r"^docker\s+system\s+prune".to_string(),
                action: BashRuleAction::Ask,
            },
            BashRule {
                name: "modify-all-docker".to_string(),
                pattern: r"^(docker\s+.*)".to_string(),
                action: BashRuleAction::Modify("$1 --dry-run".to_string()),
            },
        ];

        let engine = BashRuleEngine::from_config(rules).unwrap();
        let result = engine.apply_rules("docker system prune");
        assert_eq!(
            result,
            RuleResult::Asked {
                rule_name: "ask-specific".to_string()
            }
        );

        // Test 2: Modify before Ask - Modify wins
        let rules = vec![
            BashRule {
                name: "modify-all-docker".to_string(),
                pattern: r"^(docker\s+.*)".to_string(),
                action: BashRuleAction::Modify("$1 --dry-run".to_string()),
            },
            BashRule {
                name: "ask-specific".to_string(),
                pattern: r"^docker\s+system\s+prune".to_string(),
                action: BashRuleAction::Ask,
            },
        ];

        let engine = BashRuleEngine::from_config(rules).unwrap();
        let result = engine.apply_rules("docker system prune");
        match result {
            RuleResult::Modified {
                rule_name,
                new_command,
            } => {
                assert_eq!(rule_name, "modify-all-docker");
                assert_eq!(new_command, "docker system prune --dry-run");
            }
            _ => panic!("Expected Modified result"),
        }
    }

    #[test]
    fn test_no_match() {
        let rules = vec![BashRule {
            name: "deny-rm".to_string(),
            pattern: r"^rm\s".to_string(),
            action: BashRuleAction::Deny("rm denied".to_string()),
        }];

        let engine = BashRuleEngine::from_config(rules).unwrap();
        let result = engine.apply_rules("ls -la");
        assert_eq!(result, RuleResult::NoMatch);
    }

    #[test]
    fn test_invalid_regex() {
        let rules = vec![
            BashRule {
                name: "bad-regex".to_string(),
                pattern: r"[invalid(".to_string(),
                action: BashRuleAction::Deny("test".to_string()),
            },
            BashRule {
                name: "good-rule".to_string(),
                pattern: r"^ls".to_string(),
                action: BashRuleAction::Allow,
            },
        ];

        let engine =
            BashRuleEngine::from_config(rules).expect("Should succeed, skipping invalid rules");

        // The invalid rule should be skipped, but the valid rule should work
        let result = engine.apply_rules("ls -la");
        assert_eq!(
            result,
            RuleResult::Allowed {
                rule_name: "good-rule".to_string()
            }
        );

        // Command not matching any valid rule should return NoMatch
        let result = engine.apply_rules("rm file.txt");
        assert_eq!(result, RuleResult::NoMatch);
    }

    #[test]
    fn test_expand_captures_full_match() {
        let re = Regex::new(r"^(echo)\s+(\w+)$").unwrap();
        let caps = re.captures("echo hello").unwrap();
        let result = expand_captures(&caps, "$0");
        assert_eq!(result, "echo hello");
    }

    #[test]
    fn test_expand_captures_groups() {
        let re = Regex::new(r"^(\w+)\s+(\w+)$").unwrap();
        let caps = re.captures("hello world").unwrap();
        let result = expand_captures(&caps, "$2 $1");
        assert_eq!(result, "world hello");
    }

    #[test]
    fn test_expand_captures_no_placeholder() {
        let re = Regex::new(r"^test$").unwrap();
        let caps = re.captures("test").unwrap();
        let result = expand_captures(&caps, "replacement");
        assert_eq!(result, "replacement");
    }

    #[test]
    fn test_expand_captures_double_digit_groups() {
        let re = Regex::new(r"^(\w+) (\w+) (\w+) (\w+) (\w+) (\w+) (\w+) (\w+) (\w+) (\w+) (\w+)$")
            .unwrap();
        let caps = re.captures("a1 a2 a3 a4 a5 a6 a7 a8 a9 a10 a11").unwrap();
        let result = expand_captures(&caps, "$10 then $1");
        assert_eq!(result, "a10 then a1");
    }

    #[test]
    fn test_expand_captures_adjacent_groups() {
        let re = Regex::new(r"^(\w+) (\w+)$").unwrap();
        let caps = re.captures("hello world").unwrap();
        let result = expand_captures(&caps, "$1$2");
        assert_eq!(result, "helloworld");
    }

    #[test]
    fn test_expand_captures_nonexistent_group() {
        let re = Regex::new(r"^(\w+) (\w+)$").unwrap();
        let caps = re.captures("hello world").unwrap();
        let result = expand_captures(&caps, "$1 $999");
        assert_eq!(result, "hello $999");
    }

    #[test]
    fn test_apply_rules_empty_command() {
        let rules = vec![BashRule {
            name: "deny-all".to_string(),
            pattern: r".*".to_string(),
            action: BashRuleAction::Deny("denied".to_string()),
        }];

        let engine = BashRuleEngine::from_config(rules).unwrap();
        let result = engine.apply_rules("");

        match result {
            RuleResult::Denied { .. } => {}
            _ => panic!("Expected empty command to match '.*' pattern"),
        }
    }

    #[test]
    fn test_apply_rules_whitespace_only() {
        let rules = vec![BashRule {
            name: "deny-whitespace".to_string(),
            pattern: r"^\s+$".to_string(),
            action: BashRuleAction::Deny("whitespace only".to_string()),
        }];

        let engine = BashRuleEngine::from_config(rules).unwrap();
        let result = engine.apply_rules("   \t\n");

        match result {
            RuleResult::Denied { reason, .. } => {
                assert_eq!(reason, "whitespace only");
            }
            _ => panic!("Expected whitespace command to be denied"),
        }
    }

    #[test]
    fn test_apply_rules_no_match_on_whitespace() {
        let rules = vec![BashRule {
            name: "match-non-whitespace".to_string(),
            pattern: r"^\S+$".to_string(),
            action: BashRuleAction::Allow,
        }];

        let engine = BashRuleEngine::from_config(rules).unwrap();
        let result = engine.apply_rules("   ");

        assert_eq!(result, RuleResult::NoMatch);
    }

    #[test]
    fn test_regexset_individual_regex_invariant() {
        let rules = vec![BashRule {
            name: "capture-test".to_string(),
            pattern: r"^(docker\s+\w+)".to_string(),
            action: BashRuleAction::Modify("$1 --flag".to_string()),
        }];

        let engine = BashRuleEngine::from_config(rules).unwrap();

        let result = engine.apply_rules("docker build");

        match result {
            RuleResult::Modified { new_command, .. } => {
                assert_eq!(new_command, "docker build --flag");
            }
            _ => panic!("Expected Modified result"),
        }
    }

    #[test]
    fn test_multiple_patterns_match_first_wins() {
        let rules = vec![
            BashRule {
                name: "specific-deny".to_string(),
                pattern: r"^rm\s+-rf".to_string(),
                action: BashRuleAction::Deny("Dangerous rm -rf".to_string()),
            },
            BashRule {
                name: "generic-allow-rm".to_string(),
                pattern: r"^rm".to_string(),
                action: BashRuleAction::Allow,
            },
        ];

        let engine = BashRuleEngine::from_config(rules).unwrap();

        let result = engine.apply_rules("rm -rf /");

        match result {
            RuleResult::Denied { rule_name, reason } => {
                assert_eq!(rule_name, "specific-deny");
                assert_eq!(reason, "Dangerous rm -rf");
            }
            _ => panic!("Expected first rule (deny) to win, got: {:?}", result),
        }
    }

    #[test]
    fn test_large_rule_set_still_matches_correctly() {
        let mut rules = vec![];
        for i in 0..100 {
            rules.push(BashRule {
                name: format!("rule-{}", i),
                pattern: format!(r"^command-{}($|\s)", i),
                action: BashRuleAction::Allow,
            });
        }

        rules.push(BashRule {
            name: "final-match".to_string(),
            pattern: r"^target-command".to_string(),
            action: BashRuleAction::Deny("Found it".to_string()),
        });

        let engine = BashRuleEngine::from_config(rules).unwrap();

        let result = engine.apply_rules("target-command");
        match result {
            RuleResult::Denied { rule_name, .. } => {
                assert_eq!(rule_name, "final-match");
            }
            _ => panic!("Expected to find the matching rule"),
        }
    }
}
