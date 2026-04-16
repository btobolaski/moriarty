//! Bash command validation and modification rules.
//!
//! This module provides a rule engine for validating and modifying Bash tool use commands
//! before they are executed by Claude Code. Rules can deny dangerous commands, modify
//! commands to add safety flags, or explicitly allow specific patterns.

use std::collections::{HashMap, HashSet};

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
    /// Command arguments should be filtered and then re-validated for security.
    ArgumentFiltered {
        rule_name: String,
        new_command: String,
        reason: Option<String>,
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

/// Expands {{fragment_name}} references in a pattern string using iterative substitution.
///
/// Supports nested fragments (fragments referencing other fragments) by performing
/// multiple expansion passes until no more fragment references remain. Detects circular
/// dependencies by tracking which fragments have been expanded - if a fragment appears
/// again after being expanded, a cycle exists (e.g., a → b → a).
///
/// # Arguments
/// * `pattern` - The pattern string potentially containing {{fragment}} references
/// * `fragments` - Map of fragment names to their regex values
///
/// # Errors
/// * Returns error if a referenced fragment doesn't exist in the map
/// * Returns error if circular dependencies are detected (fragments referencing each other)
/// * Returns error if nested expansion exceeds MAX_DEPTH (10 levels)
///
/// # Examples
/// ```
/// # use std::collections::HashMap;
/// # use moriarty::hooks::bash_rules::expand_fragments;
/// let mut fragments = HashMap::new();
/// fragments.insert("safe".to_string(), "[^|&;$]".to_string());
/// fragments.insert("arg".to_string(), "( {{safe}}+)".to_string());
///
/// let pattern = "^ls{{arg}}*$";
/// let expanded = expand_fragments(pattern, &fragments).unwrap();
/// assert_eq!(expanded, "^ls( [^|&;$]+)*$");
/// ```
pub(crate) fn expand_fragments(
    pattern: &str,
    fragments: &HashMap<String, String>,
) -> Result<String> {
    // Maximum nesting depth chosen to allow reasonable fragment composition
    // (e.g., safe_chars -> safe_arg -> safe_pipe) while preventing
    // resource exhaustion from deeply nested or circular references.
    const MAX_DEPTH: usize = 10;

    let fragment_pattern =
        Regex::new(r"\{\{([a-zA-Z_][a-zA-Z0-9_-]*)\}\}").expect("Fragment regex pattern is valid");

    let mut result = pattern.to_string();
    let mut depth = 0;
    let mut all_expanded_fragments = HashSet::new();

    loop {
        let mut changed = false;
        let mut new_result = String::new();
        let mut last_end = 0;
        let mut current_iteration_fragments = HashSet::new();

        for cap in fragment_pattern.captures_iter(&result) {
            let full_match = cap.get(0).unwrap();
            let fragment_name = cap[1].to_string();

            // Collect unique fragments from current iteration for cycle detection against historical set
            current_iteration_fragments.insert(fragment_name.clone());

            // Look up fragment value
            let fragment_value = fragments.get(&fragment_name).ok_or_else(|| {
                miette!(
                    "Undefined pattern fragment '{}' referenced in pattern: {}",
                    fragment_name,
                    pattern
                )
            })?;

            // Build new result with expanded fragment
            new_result.push_str(&result[last_end..full_match.start()]);
            new_result.push_str(fragment_value);
            last_end = full_match.end();

            changed = true;
        }

        if !changed {
            break;
        }

        // Append remaining text
        new_result.push_str(&result[last_end..]);
        result = new_result;

        // Detect circular dependencies by checking if we're re-expanding fragments.
        // Example: If 'a' → '{{b}}' and 'b' → '{{a}}', iterations will be:
        //   Iteration 1: expand 'a' → '{{b}}' (record 'a')
        //   Iteration 2: expand 'b' → '{{a}}' (record 'b')
        //   Iteration 3: expand 'a' → cycle detected ('a' already in set)
        for fragment_name in &current_iteration_fragments {
            if all_expanded_fragments.contains(fragment_name) {
                return Err(miette!(
                    "Circular dependency detected in pattern fragments: '{}' references itself through other fragments",
                    fragment_name
                ));
            }
        }

        // Add current iteration's unique fragments to the all-time set
        all_expanded_fragments.extend(current_iteration_fragments);

        depth += 1;
        if depth > MAX_DEPTH {
            return Err(miette!(
                "Pattern fragment expansion exceeded maximum depth of {}. \
                 This likely indicates overly deep nesting.",
                MAX_DEPTH
            ));
        }
    }

    Ok(result)
}

/// Returns default pattern fragments for common security patterns.
///
/// These fragments are merged with user-defined fragments, with user
/// definitions taking precedence.
pub(crate) fn default_fragments() -> HashMap<String, String> {
    let mut fragments = HashMap::new();

    // Character classes - fundamental building blocks
    fragments.insert("safe_chars".to_string(), "[^|&;$`()<>{}]".to_string());
    fragments.insert(
        "identifier".to_string(),
        "[a-zA-Z_][a-zA-Z0-9_-]*".to_string(),
    );
    fragments.insert("number".to_string(), "[0-9]+".to_string());

    // Argument patterns - common safe argument types
    fragments.insert("safe_arg".to_string(), "( [^|&;$`()<>{}]+)".to_string());
    fragments.insert(
        "safe_flag".to_string(),
        "( -[a-zA-Z_][a-zA-Z0-9_-]*)".to_string(),
    );
    fragments.insert(
        "safe_path".to_string(),
        "( [^|&;$`()<>{}]+/[^|&;$`()<>{}]*)".to_string(),
    );

    // Pipe patterns - safe command piping
    fragments.insert(
        "safe_pipe_cmd".to_string(),
        "(head|tail|grep|wc|sort|uniq)".to_string(),
    );
    fragments.insert(
        "safe_pipe".to_string(),
        "( \\| (head|tail|grep|wc|sort|uniq)( [^|&;$`()<>{}]+)*)".to_string(),
    );

    fragments
}

impl BashRuleEngine {
    /// Compiles rules with pattern fragment expansion.
    ///
    /// Fragment expansion happens before regex compilation, so there's
    /// zero runtime overhead. Invalid regex patterns (after expansion)
    /// are logged and skipped.
    pub fn from_config(
        rules: Vec<BashRule>,
        user_fragments: Option<HashMap<String, String>>,
    ) -> Result<Self> {
        // Merge default fragments with user fragments (user takes precedence)
        let mut fragments = default_fragments();
        if let Some(user_frags) = user_fragments {
            fragments.extend(user_frags);
        }

        let mut compiled_rules = Vec::new();
        let mut patterns = Vec::new();

        for rule in rules {
            // Expand fragments in pattern
            let expanded_pattern = match expand_fragments(&rule.pattern, &fragments) {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!(
                        rule_name = %rule.name,
                        pattern = %rule.pattern,
                        error = %e,
                        "Failed to expand pattern fragments, skipping rule"
                    );
                    continue;
                }
            };

            // Compile expanded pattern to regex
            match Regex::new(&expanded_pattern) {
                Ok(regex) => {
                    tracing::debug!(
                        rule_name = %rule.name,
                        original_pattern = %rule.pattern,
                        expanded_pattern = %expanded_pattern,
                        "Compiled bash rule successfully"
                    );
                    patterns.push(expanded_pattern.clone());
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
                        expanded_pattern = %expanded_pattern,
                        error = %e,
                        "Invalid regex pattern after fragment expansion, skipping rule"
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
                BashRuleAction::Deny { value } => RuleResult::Denied {
                    rule_name: rule.name.clone(),
                    reason: value.clone(),
                },
                BashRuleAction::Modify { value } => {
                    let captures = rule
                        .regex
                        .captures(command)
                        .expect("Invariant violation: RegexSet and Regex desynchronized");
                    let new_command = expand_captures(&captures, value);
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
                BashRuleAction::ArgumentFilter {
                    remove,
                    add,
                    replace,
                    reason,
                } => match filter_arguments(command, remove, add, replace) {
                    Ok(new_command) => {
                        debug!(
                            rule_name = %rule.name,
                            original = %command,
                            filtered = %new_command,
                            "Command arguments filtered"
                        );
                        RuleResult::ArgumentFiltered {
                            rule_name: rule.name.clone(),
                            new_command,
                            reason: reason.clone(),
                        }
                    }
                    Err(e) => {
                        debug!(
                            rule_name = %rule.name,
                            command = %command,
                            error = %e,
                            "Failed to parse command for argument filtering, asking user"
                        );
                        RuleResult::Asked {
                            rule_name: rule.name.clone(),
                        }
                    }
                },
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

/// Parse a bash command into program and arguments using proper shell parsing.
///
/// Uses the `shell-words` crate to correctly handle:
/// - Quoted arguments: `"hello world"` is parsed as a single argument
/// - Escaped characters: `hello\ world` is parsed as a single argument "hello world"
/// - Shell metacharacters: Commands with unmatched quotes or invalid syntax return errors
///
/// This provides security against command injection through malformed arguments
/// that could bypass naive whitespace-based splitting.
///
/// # Returns
/// Result containing tuple of (program, args) where program is the first token
/// and args is the remaining tokens. Returns empty strings/vectors for empty commands.
///
/// # Errors
/// Returns an error if the command contains invalid shell syntax (e.g., unmatched quotes).
fn parse_command(command: &str) -> Result<(String, Vec<String>)> {
    let parts = shell_words::split(command)
        .map_err(|e| miette!("Failed to parse command as shell words: {}", e))?;

    // Empty commands return empty strings/vectors. The security model delegates to the user
    // via the Ask decision when no rules match (NoMatch result in handle_bash_pretool_hook).
    if parts.is_empty() {
        return Ok((String::new(), vec![]));
    }

    let program = parts[0].clone();
    let args = parts[1..].to_vec();

    Ok((program, args))
}

/// Reconstruct a command from program and arguments.
fn build_command(program: &str, args: &[String]) -> String {
    if args.is_empty() {
        return program.to_string();
    }

    let mut result = String::from(program);
    for arg in args {
        result.push(' ');
        result.push_str(arg);
    }
    result
}

/// Apply argument filters to a command, modifying it structurally rather than with regex.
///
/// # Design Rationale
///
/// The operation order (Remove → Replace → Add) establishes clear precedence for conflicting rules:
///
/// **Why Remove before Replace?**
/// Removal rules must take precedence over replacement rules to ensure dangerous arguments
/// are eliminated even if replacement rules target the same argument. Example:
/// - Command: `rm -f file.txt`
/// - Rules: remove `-f`, replace `-f` with `-i`
/// - Correct behavior: `-f` is removed (security wins)
/// - If replaced first: replacement could reintroduce a variant of the dangerous flag
///
/// This ordering guarantees: if an argument matches both remove and replace rules,
/// it will be removed, not transformed.
///
/// **Why Replace before Add?**
/// Security-added arguments must never be subject to user-defined transformation rules.
/// Allowing replacements to modify added arguments would violate the security guarantee
/// that certain flags will be present in the final command. Example:
/// - Command: `docker run ubuntu`
/// - Rules: replace `--read-only` with `--writable`, add `--read-only`
/// - Wrong order: security adds `--read-only`, then user replacement removes it
/// - Correct order: replacements run before add, so added flags are protected
///
/// This ordering guarantees: arguments added by security policies will appear in
/// the final command exactly as specified, without user modifications.
///
/// **Why Add last?**
/// Security flags must be appended after all user-defined transformations to ensure
/// they cannot be removed or modified by any rules, establishing them as the final
/// enforceable security boundary.
///
/// # Prefix Matching for --flag=value
///
/// The removal logic uses prefix matching for `--flag=value` syntax because:
/// - Many commands accept both `--flag value` and `--flag=value` forms
/// - Filtering `--open` should catch both `--open browser` and `--open=browser`
/// - This prevents users from bypassing filters by changing flag syntax
///
/// However, prefix matching is carefully limited:
/// - Only matches on the `=` boundary to avoid false positives
/// - `--col` won't match `--color=always` (no `=` after "col")
/// - `--open` will match `--open=browser` (exact prefix + `=`)
///
/// # Security Considerations
///
/// Uses proper shell parsing (via shell-words crate) to prevent injection attacks
/// through malformed arguments that could bypass naive whitespace-based splitting.
///
/// # Errors
/// Returns an error if the command contains invalid shell syntax.
fn filter_arguments(
    command: &str,
    remove: &Option<Vec<String>>,
    add: &Option<Vec<String>>,
    replace: &Option<HashMap<String, String>>,
) -> Result<String> {
    let (program, mut args) = parse_command(command)?;

    if let Some(remove_list) = remove {
        args.retain(|arg| {
            if remove_list.contains(arg) {
                return false;
            }

            for remove_pattern in remove_list {
                if arg.starts_with(&format!("{}=", remove_pattern)) {
                    return false;
                }
            }

            true
        });
    }

    if let Some(replace_map) = replace {
        for arg in args.iter_mut() {
            if let Some(replacement) = replace_map.get(arg) {
                *arg = replacement.clone();
            }
        }
    }

    if let Some(add_list) = add {
        args.extend(add_list.iter().cloned());
    }

    Ok(build_command(&program, &args))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_rules() {
        let engine = BashRuleEngine::from_config(vec![], None).unwrap();
        let result = engine.apply_rules("ls -la");
        assert_eq!(result, RuleResult::NoMatch);
    }

    #[test]
    fn test_deny_rule() {
        let rules = vec![BashRule {
            name: "deny-rm-rf".to_string(),
            pattern: r"^rm\s+-rf\s+/".to_string(),
            action: BashRuleAction::Deny {
                value: "Dangerous recursive delete".to_string(),
            },
        }];

        let engine = BashRuleEngine::from_config(rules, None).unwrap();
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

        let engine = BashRuleEngine::from_config(rules, None).unwrap();
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

        let engine = BashRuleEngine::from_config(rules, None).unwrap();
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
            action: BashRuleAction::Modify {
                value: "$1 --dry-run".to_string(),
            },
        }];

        let engine = BashRuleEngine::from_config(rules, None).unwrap();
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
            action: BashRuleAction::Modify {
                value: "echo $2 $1".to_string(),
            },
        }];

        let engine = BashRuleEngine::from_config(rules, None).unwrap();
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
                action: BashRuleAction::Deny {
                    value: "All commands denied".to_string(),
                },
            },
        ];

        let engine = BashRuleEngine::from_config(rules, None).unwrap();

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

        let engine = BashRuleEngine::from_config(rules, None).unwrap();

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
                action: BashRuleAction::Deny {
                    value: "Docker denied".to_string(),
                },
            },
        ];

        let engine = BashRuleEngine::from_config(rules, None).unwrap();
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
                action: BashRuleAction::Deny {
                    value: "Docker denied".to_string(),
                },
            },
            BashRule {
                name: "ask-specific".to_string(),
                pattern: r"^docker\s+system\s+prune".to_string(),
                action: BashRuleAction::Ask,
            },
        ];

        let engine = BashRuleEngine::from_config(rules, None).unwrap();
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
                action: BashRuleAction::Modify {
                    value: "$1 --dry-run".to_string(),
                },
            },
        ];

        let engine = BashRuleEngine::from_config(rules, None).unwrap();
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
                action: BashRuleAction::Modify {
                    value: "$1 --dry-run".to_string(),
                },
            },
            BashRule {
                name: "ask-specific".to_string(),
                pattern: r"^docker\s+system\s+prune".to_string(),
                action: BashRuleAction::Ask,
            },
        ];

        let engine = BashRuleEngine::from_config(rules, None).unwrap();
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
            action: BashRuleAction::Deny {
                value: "rm denied".to_string(),
            },
        }];

        let engine = BashRuleEngine::from_config(rules, None).unwrap();
        let result = engine.apply_rules("ls -la");
        assert_eq!(result, RuleResult::NoMatch);
    }

    #[test]
    fn test_invalid_regex() {
        let rules = vec![
            BashRule {
                name: "bad-regex".to_string(),
                pattern: r"[invalid(".to_string(),
                action: BashRuleAction::Deny {
                    value: "test".to_string(),
                },
            },
            BashRule {
                name: "good-rule".to_string(),
                pattern: r"^ls".to_string(),
                action: BashRuleAction::Allow,
            },
        ];

        let engine = BashRuleEngine::from_config(rules, None)
            .expect("Should succeed, skipping invalid rules");

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
            action: BashRuleAction::Deny {
                value: "denied".to_string(),
            },
        }];

        let engine = BashRuleEngine::from_config(rules, None).unwrap();
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
            action: BashRuleAction::Deny {
                value: "whitespace only".to_string(),
            },
        }];

        let engine = BashRuleEngine::from_config(rules, None).unwrap();
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

        let engine = BashRuleEngine::from_config(rules, None).unwrap();
        let result = engine.apply_rules("   ");

        assert_eq!(result, RuleResult::NoMatch);
    }

    #[test]
    fn test_regexset_individual_regex_invariant() {
        let rules = vec![BashRule {
            name: "capture-test".to_string(),
            pattern: r"^(docker\s+\w+)".to_string(),
            action: BashRuleAction::Modify {
                value: "$1 --flag".to_string(),
            },
        }];

        let engine = BashRuleEngine::from_config(rules, None).unwrap();

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
                action: BashRuleAction::Deny {
                    value: "Dangerous rm -rf".to_string(),
                },
            },
            BashRule {
                name: "generic-allow-rm".to_string(),
                pattern: r"^rm".to_string(),
                action: BashRuleAction::Allow,
            },
        ];

        let engine = BashRuleEngine::from_config(rules, None).unwrap();

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
            action: BashRuleAction::Deny {
                value: "Found it".to_string(),
            },
        });

        let engine = BashRuleEngine::from_config(rules, None).unwrap();

        let result = engine.apply_rules("target-command");
        match result {
            RuleResult::Denied { rule_name, .. } => {
                assert_eq!(rule_name, "final-match");
            }
            _ => panic!("Expected to find the matching rule"),
        }
    }

    // Fragment expansion tests

    #[test]
    fn test_expand_fragments_simple() {
        let mut fragments = HashMap::new();
        fragments.insert("safe".to_string(), "[^|&;$]".to_string());

        let pattern = "^ls{{safe}}*$";
        let expanded = expand_fragments(pattern, &fragments).unwrap();
        assert_eq!(expanded, "^ls[^|&;$]*$");
    }

    #[test]
    fn test_expand_fragments_multiple() {
        let mut fragments = HashMap::new();
        fragments.insert("safe".to_string(), "[^|&;$]".to_string());
        fragments.insert("num".to_string(), "[0-9]+".to_string());

        let pattern = "^cmd{{safe}}*{{num}}$";
        let expanded = expand_fragments(pattern, &fragments).unwrap();
        assert_eq!(expanded, "^cmd[^|&;$]*[0-9]+$");
    }

    #[test]
    fn test_expand_fragments_nested() {
        let mut fragments = HashMap::new();
        fragments.insert("safe".to_string(), "[^|&;$]".to_string());
        fragments.insert("arg".to_string(), "( {{safe}}+)".to_string());

        let pattern = "^ls{{arg}}*$";
        let expanded = expand_fragments(pattern, &fragments).unwrap();
        assert_eq!(expanded, "^ls( [^|&;$]+)*$");
    }

    #[test]
    fn test_expand_fragments_deeply_nested() {
        let mut fragments = HashMap::new();
        fragments.insert("a".to_string(), "x".to_string());
        fragments.insert("b".to_string(), "{{a}}y".to_string());
        fragments.insert("c".to_string(), "{{b}}z".to_string());

        let pattern = "{{c}}";
        let expanded = expand_fragments(pattern, &fragments).unwrap();
        assert_eq!(expanded, "xyz");
    }

    #[test]
    fn test_expand_fragments_undefined() {
        let fragments = HashMap::new();

        let pattern = "^ls{{undefined}}*$";
        let result = expand_fragments(pattern, &fragments);

        let error_msg = result
            .expect_err("Should fail on undefined fragment")
            .to_string();
        assert!(error_msg.contains("Undefined pattern fragment"));
        assert!(error_msg.contains("undefined"));
    }

    #[test]
    fn test_expand_fragments_circular_dependency() {
        let mut fragments = HashMap::new();
        fragments.insert("a".to_string(), "{{b}}".to_string());
        fragments.insert("b".to_string(), "{{a}}".to_string());

        let pattern = "{{a}}";
        let result = expand_fragments(pattern, &fragments);

        let error_msg = result
            .expect_err("Should detect circular dependency")
            .to_string();

        // Should specifically detect circular dependency, not hit depth limit
        assert!(
            error_msg.contains("Circular dependency"),
            "Expected circular dependency error, got: {}",
            error_msg
        );
        assert!(
            !error_msg.contains("exceeded maximum depth"),
            "Should detect circular dependency before hitting depth limit"
        );
    }

    #[test]
    fn test_expand_fragments_depth_limit() {
        let mut fragments = HashMap::new();
        // Create a chain: a -> b -> c -> d -> ... (11 levels deep)
        fragments.insert("a".to_string(), "{{b}}".to_string());
        fragments.insert("b".to_string(), "{{c}}".to_string());
        fragments.insert("c".to_string(), "{{d}}".to_string());
        fragments.insert("d".to_string(), "{{e}}".to_string());
        fragments.insert("e".to_string(), "{{f}}".to_string());
        fragments.insert("f".to_string(), "{{g}}".to_string());
        fragments.insert("g".to_string(), "{{h}}".to_string());
        fragments.insert("h".to_string(), "{{i}}".to_string());
        fragments.insert("i".to_string(), "{{j}}".to_string());
        fragments.insert("j".to_string(), "{{k}}".to_string());
        fragments.insert("k".to_string(), "x".to_string());

        let pattern = "{{a}}";
        let result = expand_fragments(pattern, &fragments);

        // Should fail due to depth limit (MAX_DEPTH = 10)
        let error_msg = result
            .expect_err("Should fail due to depth limit")
            .to_string();
        assert!(error_msg.contains("exceeded maximum depth"));
    }

    #[test]
    fn test_expand_fragments_no_fragments() {
        let fragments = HashMap::new();

        let pattern = "^ls [^|&;$]*$";
        let expanded = expand_fragments(pattern, &fragments).unwrap();
        assert_eq!(expanded, "^ls [^|&;$]*$");
    }

    #[test]
    fn test_expand_fragments_empty_pattern() {
        let fragments = HashMap::new();

        let pattern = "";
        let expanded = expand_fragments(pattern, &fragments).unwrap();
        assert_eq!(expanded, "");
    }

    #[test]
    fn test_expand_fragments_with_regex_special_chars() {
        let mut fragments = HashMap::new();
        fragments.insert("paren".to_string(), "()".to_string());
        fragments.insert("bracket".to_string(), "[]".to_string());

        let pattern = "{{paren}}{{bracket}}";
        let expanded = expand_fragments(pattern, &fragments).unwrap();
        assert_eq!(expanded, "()[]");
    }

    #[test]
    fn test_expand_fragments_no_collision_with_capture_groups() {
        let mut fragments = HashMap::new();
        fragments.insert("safe".to_string(), "[^|&;$]".to_string());

        // Pattern contains both fragments and regex capture groups
        let pattern = "^(cargo {{safe}}+) (build|check)$";
        let expanded = expand_fragments(pattern, &fragments).unwrap();
        assert_eq!(expanded, "^(cargo [^|&;$]+) (build|check)$");
    }

    #[test]
    fn test_engine_with_fragments() {
        let mut fragments = HashMap::new();
        fragments.insert("safe".to_string(), "[^|&;$`]".to_string());

        let rules = vec![BashRule {
            name: "allow-ls".to_string(),
            pattern: "^ls{{safe}}*$".to_string(),
            action: BashRuleAction::Allow,
        }];

        let engine = BashRuleEngine::from_config(rules, Some(fragments)).unwrap();

        // Should match after expansion
        let result = engine.apply_rules("ls -la");
        assert!(matches!(result, RuleResult::Allowed { .. }));

        // Should not match (contains pipe)
        let result = engine.apply_rules("ls | grep foo");
        assert!(matches!(result, RuleResult::NoMatch));
    }

    #[test]
    fn test_default_fragments() {
        let defaults = default_fragments();

        // Verify key fragments exist
        assert!(defaults.contains_key("safe_chars"));
        assert!(defaults.contains_key("identifier"));
        assert!(defaults.contains_key("number"));
        assert!(defaults.contains_key("safe_arg"));
        assert!(defaults.contains_key("safe_pipe"));

        // Verify safe_chars blocks injection characters
        let safe_chars = &defaults["safe_chars"];
        assert!(safe_chars.contains('|'));
        assert!(safe_chars.contains('&'));
        assert!(safe_chars.contains('$'));
        assert!(safe_chars.contains('`'));
    }

    #[test]
    fn test_default_fragments_no_circular_deps() {
        let defaults = default_fragments();

        // Try expanding each default fragment
        for (name, _) in &defaults {
            let pattern = format!("{{{{{}}}}}", name);
            let result = expand_fragments(&pattern, &defaults);
            assert!(
                result.is_ok(),
                "Default fragment '{}' has circular dependency",
                name
            );
        }
    }

    #[test]
    fn test_user_fragments_override_defaults() {
        let mut user_fragments = HashMap::new();
        user_fragments.insert("safe_chars".to_string(), "[a-z]".to_string());

        let rules = vec![BashRule {
            name: "test".to_string(),
            pattern: "^{{safe_chars}}+$".to_string(),
            action: BashRuleAction::Allow,
        }];

        let engine = BashRuleEngine::from_config(rules, Some(user_fragments)).unwrap();

        // Should match with user override (lowercase only)
        let result = engine.apply_rules("abc");
        assert!(matches!(result, RuleResult::Allowed { .. }));

        // Should not match uppercase (user override, not default)
        let result = engine.apply_rules("ABC");
        assert!(matches!(result, RuleResult::NoMatch));
    }

    #[test]
    fn test_fragment_expansion_error_logged_and_skipped() {
        let mut fragments = HashMap::new();
        fragments.insert("valid".to_string(), "[a-z]".to_string());

        let rules = vec![
            BashRule {
                name: "bad-fragment".to_string(),
                pattern: "^{{undefined}}$".to_string(),
                action: BashRuleAction::Deny {
                    value: "test".to_string(),
                },
            },
            BashRule {
                name: "good-rule".to_string(),
                pattern: "^{{valid}}+$".to_string(),
                action: BashRuleAction::Allow,
            },
        ];

        // Engine should succeed, skipping the bad rule
        let engine = BashRuleEngine::from_config(rules, Some(fragments)).unwrap();

        // The valid rule should work
        let result = engine.apply_rules("abc");
        assert!(matches!(result, RuleResult::Allowed { .. }));
    }

    #[test]
    fn test_fragment_in_modify_action() {
        let mut fragments = HashMap::new();
        fragments.insert("safe".to_string(), "[^|&;$`]".to_string());

        let rules = vec![BashRule {
            name: "modify-docker".to_string(),
            pattern: "^(docker{{safe}}+)$".to_string(),
            action: BashRuleAction::Modify {
                value: "$1 --dry-run".to_string(),
            },
        }];

        let engine = BashRuleEngine::from_config(rules, Some(fragments)).unwrap();
        let result = engine.apply_rules("docker build");

        match result {
            RuleResult::Modified { new_command, .. } => {
                assert_eq!(new_command, "docker build --dry-run");
            }
            _ => panic!("Expected Modified result"),
        }
    }

    #[test]
    fn test_expand_fragments_same_fragment_multiple_times() {
        let mut fragments = HashMap::new();
        fragments.insert("x".to_string(), "abc".to_string());

        let pattern = "{{x}}-{{x}}";
        let expanded = expand_fragments(pattern, &fragments).unwrap();
        assert_eq!(expanded, "abc-abc");
    }

    #[test]
    fn test_expand_fragments_adjacent_no_separator() {
        let mut fragments = HashMap::new();
        fragments.insert("a".to_string(), "foo".to_string());
        fragments.insert("b".to_string(), "bar".to_string());

        let pattern = "{{a}}{{b}}";
        let expanded = expand_fragments(pattern, &fragments).unwrap();
        assert_eq!(expanded, "foobar");
    }

    #[test]
    fn test_expand_fragments_invalid_name_starting_with_digit() {
        let mut fragments = HashMap::new();
        fragments.insert("123".to_string(), "value".to_string());

        // Fragment names starting with digits don't match the pattern,
        // so they remain unexpanded
        let pattern = "{{123}}";
        let expanded = expand_fragments(pattern, &fragments).unwrap();
        assert_eq!(expanded, "{{123}}");
    }

    #[test]
    fn test_expand_fragments_with_spaces_not_allowed() {
        let mut fragments = HashMap::new();
        fragments.insert("safe".to_string(), "[^|&;$]".to_string());

        // Spaces inside braces don't match the fragment pattern
        let pattern = "{{ safe }}";
        let expanded = expand_fragments(pattern, &fragments).unwrap();
        assert_eq!(expanded, "{{ safe }}");
    }

    #[test]
    fn test_default_fragments_compile_to_valid_regex() {
        let defaults = default_fragments();

        for (name, _) in &defaults {
            // Each default fragment should expand without error
            let test_pattern = format!("{{{{{}}}}}", name);
            let expanded = expand_fragments(&test_pattern, &defaults)
                .unwrap_or_else(|_| panic!("Fragment '{}' should expand without error", name));

            // And should compile to valid regex
            Regex::new(&expanded).unwrap_or_else(|_| {
                panic!(
                    "Fragment '{}' should produce valid regex: {}",
                    name, expanded
                )
            });
        }
    }

    #[test]
    fn test_expand_fragments_circular_with_prefix() {
        let mut fragments = HashMap::new();
        fragments.insert("a".to_string(), "prefix{{b}}".to_string());
        fragments.insert("b".to_string(), "middle{{a}}".to_string());

        let pattern = "{{a}}";
        let result = expand_fragments(pattern, &fragments);

        let error_msg = result
            .expect_err("Should detect circular dependency")
            .to_string();

        assert!(
            error_msg.contains("Circular dependency"),
            "Expected circular dependency error, got: {}",
            error_msg
        );
    }

    // Tests for command parsing and argument filtering functions

    #[test]
    fn test_parse_command_simple() {
        let (prog, args) = parse_command("cargo build").unwrap();
        assert_eq!(prog, "cargo");
        assert_eq!(args, vec!["build"]);
    }

    #[test]
    fn test_parse_command_with_args() {
        let (prog, args) = parse_command("cargo build --release --features foo").unwrap();
        assert_eq!(prog, "cargo");
        assert_eq!(args, vec!["build", "--release", "--features", "foo"]);
    }

    #[test]
    fn test_parse_command_empty() {
        let (prog, args) = parse_command("").unwrap();
        assert_eq!(prog, "");
        assert_eq!(args.len(), 0);
    }

    #[test]
    fn test_build_command() {
        let cmd = build_command("cargo", &["build".to_string(), "--release".to_string()]);
        assert_eq!(cmd, "cargo build --release");
    }

    #[test]
    fn test_build_command_no_args() {
        let cmd = build_command("ls", &[]);
        assert_eq!(cmd, "ls");
    }

    #[test]
    fn test_filter_arguments_remove_simple() {
        let command = "cargo doc --open --no-deps";
        let remove = Some(vec!["--open".to_string()]);
        let filtered = filter_arguments(command, &remove, &None, &None).unwrap();
        assert_eq!(filtered, "cargo doc --no-deps");
    }

    #[test]
    fn test_filter_arguments_remove_position_independent() {
        // --open at the beginning
        let command = "cargo doc --open --no-deps";
        let remove = Some(vec!["--open".to_string()]);
        let filtered = filter_arguments(command, &remove, &None, &None).unwrap();
        assert_eq!(filtered, "cargo doc --no-deps");

        // --open in the middle
        let command = "cargo doc --no-deps --open foo";
        let filtered = filter_arguments(command, &remove, &None, &None).unwrap();
        assert_eq!(filtered, "cargo doc --no-deps foo");

        // --open at the end
        let command = "cargo doc --no-deps --open";
        let filtered = filter_arguments(command, &remove, &None, &None).unwrap();
        assert_eq!(filtered, "cargo doc --no-deps");
    }

    #[test]
    fn test_filter_arguments_remove_with_equals() {
        let command = "cargo build --color=always";
        let remove = Some(vec!["--color".to_string()]);
        let filtered = filter_arguments(command, &remove, &None, &None).unwrap();
        assert_eq!(filtered, "cargo build");
    }

    #[test]
    fn test_filter_arguments_remove_multiple() {
        let command = "cargo doc --open --color=always --no-deps";
        let remove = Some(vec!["--open".to_string(), "--color".to_string()]);
        let filtered = filter_arguments(command, &remove, &None, &None).unwrap();
        assert_eq!(filtered, "cargo doc --no-deps");
    }

    #[test]
    fn test_filter_arguments_add() {
        let command = "docker run ubuntu";
        let add = Some(vec!["--read-only".to_string()]);
        let filtered = filter_arguments(command, &None, &add, &None).unwrap();
        assert_eq!(filtered, "docker run ubuntu --read-only");
    }

    #[test]
    fn test_filter_arguments_add_multiple() {
        let command = "docker run ubuntu";
        let add = Some(vec![
            "--read-only".to_string(),
            "--network=none".to_string(),
        ]);
        let filtered = filter_arguments(command, &None, &add, &None).unwrap();
        assert_eq!(filtered, "docker run ubuntu --read-only --network=none");
    }

    #[test]
    fn test_filter_arguments_replace() {
        let command = "rm -f file.txt";
        let mut replace_map = HashMap::new();
        replace_map.insert("-f".to_string(), "-i".to_string());
        let filtered = filter_arguments(command, &None, &None, &Some(replace_map)).unwrap();
        assert_eq!(filtered, "rm -i file.txt");
    }

    #[test]
    fn test_filter_arguments_replace_multiple() {
        let command = "rm -f file1.txt -rf file2.txt";
        let mut replace_map = HashMap::new();
        replace_map.insert("-f".to_string(), "-i".to_string());
        replace_map.insert("-rf".to_string(), "-ri".to_string());
        let filtered = filter_arguments(command, &None, &None, &Some(replace_map)).unwrap();
        assert_eq!(filtered, "rm -i file1.txt -ri file2.txt");
    }

    #[test]
    fn test_filter_arguments_replace_nonexistent() {
        let command = "cargo build";
        let mut replace_map = HashMap::new();
        replace_map.insert("--open".to_string(), "--offline".to_string());
        let filtered = filter_arguments(command, &None, &None, &Some(replace_map)).unwrap();
        assert_eq!(filtered, "cargo build");
    }

    #[test]
    fn test_filter_arguments_combined() {
        let command = "npm start --open --verbose";
        let remove = Some(vec!["--open".to_string()]);
        let add = Some(vec!["--no-browser".to_string()]);
        let filtered = filter_arguments(command, &remove, &add, &None).unwrap();
        assert_eq!(filtered, "npm start --verbose --no-browser");
    }

    #[test]
    fn test_filter_arguments_no_changes() {
        let command = "cargo build";
        let filtered = filter_arguments(command, &None, &None, &None).unwrap();
        assert_eq!(filtered, "cargo build");
    }

    #[test]
    fn test_filter_arguments_remove_nonexistent() {
        let command = "cargo build";
        let remove = Some(vec!["--open".to_string()]);
        let filtered = filter_arguments(command, &remove, &None, &None).unwrap();
        assert_eq!(filtered, "cargo build");
    }

    #[test]
    fn test_filter_arguments_empty_command() {
        let filtered = filter_arguments("", &None, &None, &None).unwrap();
        assert_eq!(filtered, "");

        let filtered =
            filter_arguments("", &Some(vec!["--flag".to_string()]), &None, &None).unwrap();
        assert_eq!(filtered, "");
    }

    #[test]
    fn test_filter_arguments_whitespace_handling() {
        let command = "cargo  doc    --open   --no-deps";
        let remove = Some(vec!["--open".to_string()]);
        let filtered = filter_arguments(command, &remove, &None, &None).unwrap();
        assert_eq!(filtered, "cargo doc --no-deps");
    }

    #[test]
    fn test_filter_arguments_prefix_match_boundaries() {
        let command = "cargo build --color=always";
        let remove = Some(vec!["--color".to_string()]);
        let filtered = filter_arguments(command, &remove, &None, &None).unwrap();
        assert_eq!(filtered, "cargo build");

        let command = "cargo build --color=always";
        let remove = Some(vec!["--col".to_string()]);
        let filtered = filter_arguments(command, &remove, &None, &None).unwrap();
        assert_eq!(filtered, "cargo build --color=always");

        let command = "cargo build --colours=always";
        let remove = Some(vec!["--color".to_string()]);
        let filtered = filter_arguments(command, &remove, &None, &None).unwrap();
        assert_eq!(filtered, "cargo build --colours=always");
    }

    #[test]
    fn test_filter_arguments_replace_exact_match_only() {
        let mut replace = HashMap::new();
        replace.insert("-f".to_string(), "-i".to_string());

        let filtered =
            filter_arguments("rm -f file", &None, &None, &Some(replace.clone())).unwrap();
        assert_eq!(filtered, "rm -i file");

        let filtered =
            filter_arguments("rm -rf file", &None, &None, &Some(replace.clone())).unwrap();
        assert_eq!(filtered, "rm -rf file");

        let filtered = filter_arguments("rm file-f.txt", &None, &None, &Some(replace)).unwrap();
        assert_eq!(filtered, "rm file-f.txt");
    }

    #[test]
    fn test_filter_arguments_empty_filter_lists() {
        let command = "cargo build";
        let filtered = filter_arguments(command, &Some(vec![]), &None, &None).unwrap();
        assert_eq!(filtered, "cargo build");

        let filtered = filter_arguments(command, &None, &Some(vec![]), &None).unwrap();
        assert_eq!(filtered, "cargo build");
    }

    #[test]
    fn test_parse_command_quoted_arguments() {
        let (prog, args) = parse_command(r#"echo "hello world""#).unwrap();
        assert_eq!(prog, "echo");
        assert_eq!(args, vec!["hello world"]);

        let (prog, args) = parse_command(r#"rm 'file with spaces.txt'"#).unwrap();
        assert_eq!(prog, "rm");
        assert_eq!(args, vec!["file with spaces.txt"]);
    }

    #[test]
    fn test_parse_command_escaped_characters() {
        let (prog, args) = parse_command(r"rm file\ name.txt").unwrap();
        assert_eq!(prog, "rm");
        assert_eq!(args, vec!["file name.txt"]);
    }

    #[test]
    fn test_parse_command_invalid_syntax() {
        let result = parse_command(r#"echo "unmatched quote"#);
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Failed to parse command"));
    }

    #[test]
    fn test_argument_filter_action() {
        let rules = vec![BashRule {
            name: "filter-cargo-doc".to_string(),
            pattern: r"^cargo doc\b".to_string(),
            action: BashRuleAction::ArgumentFilter {
                remove: Some(vec!["--open".to_string()]),
                add: None,
                replace: None,
                reason: Some("Browser flags removed".to_string()),
            },
        }];

        let engine = BashRuleEngine::from_config(rules, None).unwrap();

        let result = engine.apply_rules("cargo doc --open --no-deps");
        match result {
            RuleResult::ArgumentFiltered { new_command, .. } => {
                assert_eq!(new_command, "cargo doc --no-deps");
            }
            _ => panic!("Expected ArgumentFiltered result"),
        }
    }

    #[test]
    fn test_argument_filter_with_revalidation() {
        let rules = vec![
            // First rule: filter --open from cargo doc (only matches when --open is present)
            BashRule {
                name: "filter-cargo-doc".to_string(),
                pattern: r"^cargo doc.*--open".to_string(),
                action: BashRuleAction::ArgumentFilter {
                    remove: Some(vec!["--open".to_string()]),
                    add: None,
                    replace: None,
                    reason: Some("Removed --open".to_string()),
                },
            },
            // Second rule: allow cargo doc (without --open)
            BashRule {
                name: "allow-cargo-doc".to_string(),
                pattern: r"^cargo doc($|\s)".to_string(),
                action: BashRuleAction::Allow,
            },
        ];

        let engine = BashRuleEngine::from_config(rules, None).unwrap();

        // First check: matches filter rule
        let result = engine.apply_rules("cargo doc --open");
        let filtered_cmd = match result {
            RuleResult::ArgumentFiltered { new_command, .. } => new_command,
            _ => panic!("Expected ArgumentFiltered result"),
        };

        // Revalidation: filtered command should match allow rule
        let recheck = engine.apply_rules(&filtered_cmd);
        assert!(matches!(recheck, RuleResult::Allowed { .. }));
    }
}
