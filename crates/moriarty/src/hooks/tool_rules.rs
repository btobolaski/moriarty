//! Tool call validation rules for any Claude Code tool.
//!
//! This module provides a rule engine for permissioning arbitrary tool calls (Read, Write, Edit,
//! Bash, etc.) before they are executed by Claude Code. Unlike `bash_rules` which operates on
//! command strings, tool rules match on tool name and optionally on a specific field in the
//! tool input using regex patterns. Rules may also set `allow_local = true` to require that
//! `path` or `file_path` resolves to a canonical path within the canonicalized hook `cwd`, with
//! safe handling of non-existent targets. Field values that start with the hook input's `cwd`
//! have that prefix stripped before matching, so rules can use relative paths.

use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    fs, io,
    path::{Component, Path, PathBuf},
};

use regex::Regex;
use tokio::task::spawn_blocking;
use tracing::{debug, warn};

use super::bash_rules::{default_fragments, expand_fragments};
use crate::user_config::{ToolRule, ToolRuleAction};

/// Runtime representation of a tool rule with pre-compiled regex for the field pattern.
#[derive(Debug)]
struct CompiledToolRule {
    name: String,
    tool: String,
    allow_local: bool,
    field: Option<String>,
    regex: Option<Regex>,
    action: ToolRuleAction,
}

/// Result of resolving a single candidate path (`path` or `file_path`) for an `allow_local`
/// check. `None` at the call site means the field was absent or non-string in the tool input.
/// When present, `is_local` indicates whether the fully-resolved path falls under `canonical_cwd`.
/// Broken symlinks and unresolvable paths are represented as `None` (not `is_local = false`),
/// so they can never satisfy a locality check.
#[derive(Debug, Clone)]
struct CandidatePathEvaluation {
    /// Whether the resolved path starts with the canonicalized `cwd`.
    is_local: bool,
    /// The fully canonicalized path (existing portions) with any non-existent suffix safely
    /// appended via [`rebuild_missing_suffix`].
    resolved_path: PathBuf,
}

/// Aggregated locality evaluation for both `path` and `file_path` fields of a tool input.
/// Produced once per `apply_rules` call (potentially on the blocking thread pool) and then
/// shared across all `allow_local` rules during first-match-wins evaluation.
#[derive(Debug, Clone)]
struct LocalPathEvaluation {
    /// The canonicalized hook working directory — the trust boundary for locality checks.
    canonical_cwd: PathBuf,
    /// Evaluation of the `path` field, if present and resolvable.
    path: Option<CandidatePathEvaluation>,
    /// Evaluation of the `file_path` field, if present and resolvable.
    file_path: Option<CandidatePathEvaluation>,
}

impl LocalPathEvaluation {
    fn any_local(&self) -> bool {
        self.path
            .as_ref()
            .is_some_and(|evaluation| evaluation.is_local)
            || self
                .file_path
                .as_ref()
                .is_some_and(|evaluation| evaluation.is_local)
    }

    fn resolved_local_path(&self, field: &str) -> Option<&Path> {
        match field {
            "path" => self
                .path
                .as_ref()
                .filter(|evaluation| evaluation.is_local)
                .map(|evaluation| evaluation.resolved_path.as_path()),
            "file_path" => self
                .file_path
                .as_ref()
                .filter(|evaluation| evaluation.is_local)
                .map(|evaluation| evaluation.resolved_path.as_path()),
            _ => None,
        }
    }
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
    allow_local_tools: HashSet<String>,
    has_wildcard_allow_local: bool,
}

fn record_allow_local_tool(
    allow_local_tools: &mut HashSet<String>,
    has_wildcard_allow_local: &mut bool,
    tool: &str,
) {
    if tool == "*" {
        *has_wildcard_allow_local = true;
    } else {
        allow_local_tools.insert(tool.to_string());
    }
}

/// Extracts only the `path` and `file_path` fields from the tool input so that only those
/// two small strings need to be moved into the `spawn_blocking` closure, avoiding a full
/// clone of a potentially-large input (e.g., a Write tool call's `content` field).
fn locality_input(tool_input: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "path": tool_input.get("path"),
        "file_path": tool_input.get("file_path"),
    })
}

fn is_missing_path_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
    )
        // Older Windows/Rust combinations may surface ERROR_DIRECTORY (267) as ErrorKind::Other.
        || cfg!(windows) && error.raw_os_error() == Some(267)
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
        let mut allow_local_tools = HashSet::new();
        let mut has_wildcard_allow_local = false;

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
                            if rule.allow_local {
                                record_allow_local_tool(
                                    &mut allow_local_tools,
                                    &mut has_wildcard_allow_local,
                                    &rule.tool,
                                );
                            }

                            compiled.push(CompiledToolRule {
                                name: rule.name,
                                tool: rule.tool,
                                allow_local: rule.allow_local,
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
                    if rule.allow_local {
                        record_allow_local_tool(
                            &mut allow_local_tools,
                            &mut has_wildcard_allow_local,
                            &rule.tool,
                        );
                    }

                    compiled.push(CompiledToolRule {
                        name: rule.name,
                        tool: rule.tool,
                        allow_local: rule.allow_local,
                        field: None,
                        regex: None,
                        action: rule.action,
                    });
                }
            }
        }

        Self {
            rules: compiled,
            allow_local_tools,
            has_wildcard_allow_local,
        }
    }

    fn has_matching_allow_local_rule(&self, tool_name: &str) -> bool {
        self.has_wildcard_allow_local || self.allow_local_tools.contains(tool_name)
    }

    fn apply_rules_core(
        &self,
        tool_name: &str,
        tool_input: &serde_json::Value,
        cwd: &str,
        local_evaluation: Option<&LocalPathEvaluation>,
    ) -> ToolRuleResult {
        for rule in &self.rules {
            if rule.tool != "*" && rule.tool != tool_name {
                continue;
            }

            if rule.allow_local && !rule_matches_allow_local(rule, local_evaluation) {
                continue;
            }

            let local_evaluation_for_regex = if rule.allow_local {
                local_evaluation
            } else {
                None
            };

            if !rule_matches_regex(rule, tool_input, cwd, local_evaluation_for_regex) {
                continue;
            }

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

    /// Evaluate rules against a tool call. Returns the first matching rule's result.
    ///
    /// `tool_input` is `serde_json::Value` rather than a typed struct because Claude Code tool
    /// inputs are heterogeneous — each tool (Read, Write, Edit, Bash, Grep, etc.) has a different
    /// schema, so no single typed struct can represent them all. The upstream `HookEventData`
    /// parser already delivers `tool_input` as `serde_json::Value`.
    ///
    /// When a field value starts with `cwd/`, the prefix is stripped before regex matching so
    /// that rules can be written with relative paths (e.g., `^src/` instead of
    /// `^/home/user/project/src/`). Rules with `allow_local = true` additionally require that
    /// either the `path` or `file_path` field resolves to a canonical path within the canonicalized
    /// hook cwd. The filesystem work for that locality check runs on the blocking thread pool.
    pub async fn apply_rules(
        &self,
        tool_name: &str,
        tool_input: &serde_json::Value,
        cwd: &str,
    ) -> ToolRuleResult {
        let local_evaluation = if self.has_matching_allow_local_rule(tool_name) {
            let locality_value = locality_input(tool_input);
            let cwd_owned = cwd.to_string();
            match spawn_blocking(move || {
                evaluate_local_paths(&locality_value, Path::new(&cwd_owned))
            })
            .await
            {
                Ok(evaluation) => evaluation,
                Err(error) => {
                    // Treat locality evaluation failures as a non-match so the hook never
                    // panics. All allow_local rules are skipped in this case, so evaluation falls
                    // through to any later non-allow_local rules or NoMatch.
                    warn!(error = %error, "allow_local path evaluation task failed");
                    None
                }
            }
        } else {
            None
        };

        self.apply_rules_core(tool_name, tool_input, cwd, local_evaluation.as_ref())
    }

    #[cfg(test)]
    fn apply_rules_sync(
        &self,
        tool_name: &str,
        tool_input: &serde_json::Value,
        cwd: &str,
    ) -> ToolRuleResult {
        let local_evaluation = if self.has_matching_allow_local_rule(tool_name) {
            let locality_value = locality_input(tool_input);
            evaluate_local_paths(&locality_value, Path::new(cwd))
        } else {
            None
        };

        self.apply_rules_core(tool_name, tool_input, cwd, local_evaluation.as_ref())
    }
}

fn rule_matches_allow_local(
    rule: &CompiledToolRule,
    local_evaluation: Option<&LocalPathEvaluation>,
) -> bool {
    let Some(local_evaluation) = local_evaluation else {
        return false;
    };

    match rule.field.as_deref() {
        Some("path") => local_evaluation
            .path
            .as_ref()
            .is_some_and(|evaluation| evaluation.is_local),
        Some("file_path") => local_evaluation
            .file_path
            .as_ref()
            .is_some_and(|evaluation| evaluation.is_local),
        Some(_) => false,
        None => local_evaluation.any_local(),
    }
}

fn rule_matches_regex(
    rule: &CompiledToolRule,
    tool_input: &serde_json::Value,
    cwd: &str,
    local_evaluation: Option<&LocalPathEvaluation>,
) -> bool {
    if let (Some(field), Some(regex)) = (&rule.field, &rule.regex) {
        let value_for_matching = match local_evaluation {
            Some(local_evaluation) => {
                let Some(resolved_path) = local_evaluation.resolved_local_path(field) else {
                    return false;
                };
                strip_cwd_prefix(
                    &resolved_path.to_string_lossy(),
                    &local_evaluation.canonical_cwd.to_string_lossy(),
                )
                .to_string()
            }
            None => {
                let field_value = match tool_input.get(field) {
                    Some(v) => extract_field_value(v),
                    None => return false,
                };

                let Some(value_str) = field_value else {
                    return false;
                };

                strip_cwd_prefix(&value_str, cwd).to_string()
            }
        };

        regex.is_match(&value_for_matching)
    } else {
        true
    }
}

fn evaluate_local_paths(tool_input: &serde_json::Value, cwd: &Path) -> Option<LocalPathEvaluation> {
    let canonical_cwd = match fs::canonicalize(cwd) {
        Ok(path) => path,
        Err(error) => {
            warn!(cwd = %cwd.display(), error = %error, "Failed to canonicalize hook cwd for allow_local check");
            return None;
        }
    };

    Some(LocalPathEvaluation {
        canonical_cwd: canonical_cwd.clone(),
        path: evaluate_candidate_path(tool_input, "path", &canonical_cwd),
        file_path: evaluate_candidate_path(tool_input, "file_path", &canonical_cwd),
    })
}

fn evaluate_candidate_path(
    tool_input: &serde_json::Value,
    field: &str,
    canonical_cwd: &Path,
) -> Option<CandidatePathEvaluation> {
    let candidate = tool_input.get(field).and_then(|value| value.as_str())?;
    let candidate = PathBuf::from(candidate);
    let resolved = if candidate.is_absolute() {
        candidate
    } else {
        canonical_cwd.join(candidate)
    };

    match canonicalize_allow_missing(&resolved) {
        Ok(path) => Some(CandidatePathEvaluation {
            is_local: path.starts_with(canonical_cwd),
            resolved_path: path,
        }),
        Err(error) => {
            debug!(
                field,
                candidate = %resolved.display(),
                cwd = %canonical_cwd.display(),
                error = %error,
                "Failed to resolve candidate path for allow_local check"
            );
            None
        }
    }
}

fn canonicalize_allow_missing(path: &Path) -> io::Result<PathBuf> {
    let mut current = path.to_path_buf();
    let mut removed_components = Vec::new();

    loop {
        match fs::canonicalize(&current) {
            Ok(canonical) => {
                return rebuild_missing_suffix(canonical, removed_components.into_iter().rev())
            }
            Err(error) if is_missing_path_error(&error) => {
                // TOCTOU note: between `canonicalize` failing and this `symlink_metadata`
                // call, the entry at `current` can change. All possible races are fail-safe:
                // we either correctly detect a broken symlink, or conservatively reject a
                // path that has been concurrently replaced. We never incorrectly admit an
                // escaping path.
                if fs::symlink_metadata(&current)
                    .is_ok_and(|metadata| metadata.file_type().is_symlink())
                {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "broken symlink in path; cannot determine locality",
                    ));
                }

                let Some(component) = current.components().next_back() else {
                    return Err(error);
                };

                match component {
                    Component::Prefix(_) | Component::RootDir => return Err(error),
                    Component::CurDir => removed_components.push(MissingPathComponent::CurDir),
                    Component::ParentDir => {
                        removed_components.push(MissingPathComponent::ParentDir)
                    }
                    Component::Normal(name) => {
                        removed_components.push(MissingPathComponent::Normal(name.to_os_string()))
                    }
                }

                if !current.pop() {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
}

fn rebuild_missing_suffix(
    mut base: PathBuf,
    components: impl IntoIterator<Item = MissingPathComponent>,
) -> io::Result<PathBuf> {
    // `floor` is the component-depth of the canonicalized ancestor — the security boundary.
    // Any `..` that would push depth below this level means the non-existent suffix is trying
    // to climb above the verified canonical root, which must be rejected to prevent path
    // traversal attacks (e.g., `cwd/missing/../../etc/passwd`).
    let floor = base.components().count();
    let mut depth = floor;

    for component in components {
        match component {
            MissingPathComponent::CurDir => {}
            MissingPathComponent::Normal(name) => {
                base.push(name);
                depth += 1;
            }
            MissingPathComponent::ParentDir => {
                if depth == floor {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "path escapes canonicalized ancestor",
                    ));
                }
                base.pop();
                depth -= 1;
            }
        }
    }

    Ok(base)
}

#[derive(Debug)]
enum MissingPathComponent {
    CurDir,
    ParentDir,
    Normal(OsString),
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

/// Strip the cwd prefix from a value to produce a relative path for matching.
///
/// Guards against partial directory name matches (e.g., cwd `/foo` does not strip from
/// `/foobar/baz`) by requiring a `/` boundary or exact equality after the prefix.
fn strip_cwd_prefix<'a>(value: &'a str, cwd: &str) -> &'a str {
    let cwd = cwd.strip_suffix('/').unwrap_or(cwd);

    if cwd.is_empty() {
        return value;
    }

    if let Some(rest) = value.strip_prefix(cwd) {
        if rest.is_empty() {
            ""
        } else if let Some(relative) = rest.strip_prefix('/') {
            relative
        } else {
            value
        }
    } else {
        value
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
            allow_local: false,
            field: field.map(|s| s.to_string()),
            pattern: pattern.map(|s| s.to_string()),
            action,
        }
    }

    fn make_local_rule(
        name: &str,
        tool: &str,
        field: Option<&str>,
        pattern: Option<&str>,
        action: ToolRuleAction,
    ) -> ToolRule {
        ToolRule {
            allow_local: true,
            ..make_rule(name, tool, field, pattern, action)
        }
    }

    fn tool_input_has_local_path(paths: &[PathBuf], cwd: &Path) -> bool {
        let tool_input = serde_json::json!({
            "path": paths.first().map(|path| path.to_string_lossy().to_string()),
            "file_path": paths
                .get(1)
                .map(|path| path.to_string_lossy().to_string()),
        });

        evaluate_local_paths(&tool_input, cwd)
            .as_ref()
            .is_some_and(LocalPathEvaluation::any_local)
    }

    #[test]
    fn test_empty_rules() {
        let engine = ToolRuleEngine::from_config(vec![], None);
        let result =
            engine.apply_rules_sync("Read", &serde_json::json!({"file_path": "/tmp/foo"}), "");
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

        let result =
            engine.apply_rules_sync("Read", &serde_json::json!({"file_path": "/tmp/foo"}), "");
        assert_eq!(
            result,
            ToolRuleResult::Allowed {
                rule_name: "allow-read".to_string()
            }
        );

        // Doesn't match other tools
        let result = engine.apply_rules_sync("Write", &serde_json::json!({}), "");
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

        let result = engine.apply_rules_sync("Write", &serde_json::json!({}), "");
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

        let result = engine.apply_rules_sync("Edit", &serde_json::json!({}), "");
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
            engine.apply_rules_sync("Read", &serde_json::json!({}), ""),
            ToolRuleResult::Asked {
                rule_name: "catch-all".to_string()
            }
        );
        assert_eq!(
            engine.apply_rules_sync("Write", &serde_json::json!({}), ""),
            ToolRuleResult::Asked {
                rule_name: "catch-all".to_string()
            }
        );
        assert_eq!(
            engine.apply_rules_sync("Bash", &serde_json::json!({}), ""),
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
        let result = engine.apply_rules_sync(
            "Write",
            &serde_json::json!({"file_path": "/home/user/.env"}),
            "",
        );
        assert_eq!(
            result,
            ToolRuleResult::Denied {
                rule_name: "deny-env-write".to_string(),
                reason: "Cannot write .env files".to_string(),
            }
        );

        // Doesn't match (different extension)
        let result = engine.apply_rules_sync(
            "Write",
            &serde_json::json!({"file_path": "/home/user/main.rs"}),
            "",
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
        let result = engine.apply_rules_sync("Write", &serde_json::json!({"content": "hello"}), "");
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
        let result = engine.apply_rules_sync("Test", &serde_json::json!({"count": 42}), "");
        assert_eq!(
            result,
            ToolRuleResult::Allowed {
                rule_name: "match-number".to_string()
            }
        );

        // Bool field (doesn't match "42")
        let result = engine.apply_rules_sync("Test", &serde_json::json!({"count": true}), "");
        assert_eq!(result, ToolRuleResult::NoMatch);

        // Array/object/null fields can't be matched
        let result = engine.apply_rules_sync("Test", &serde_json::json!({"count": [42]}), "");
        assert_eq!(result, ToolRuleResult::NoMatch);

        let result = engine.apply_rules_sync("Test", &serde_json::json!({"count": null}), "");
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

        let result = bool_engine.apply_rules_sync("Test", &serde_json::json!({"flag": true}), "");
        assert_eq!(
            result,
            ToolRuleResult::Allowed {
                rule_name: "match-bool".to_string()
            }
        );

        let result = bool_engine.apply_rules_sync("Test", &serde_json::json!({"flag": false}), "");
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
        let result =
            engine.apply_rules_sync("Write", &serde_json::json!({"file_path": "main.rs"}), "");
        assert_eq!(
            result,
            ToolRuleResult::Allowed {
                rule_name: "allow-specific".to_string()
            }
        );

        // Other file matches second rule
        let result =
            engine.apply_rules_sync("Write", &serde_json::json!({"file_path": "data.csv"}), "");
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
        let result =
            engine.apply_rules_sync("Write", &serde_json::json!({"file_path": "/home/.env"}), "");
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

        let result =
            engine.apply_rules_sync("Write", &serde_json::json!({"file_path": "/home/.env"}), "");
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

        let result = engine.apply_rules_sync(
            "Read",
            &serde_json::json!({"file_path": "/home/user/project/src/main.rs"}),
            "",
        );
        assert_eq!(
            result,
            ToolRuleResult::Allowed {
                rule_name: "allow-project-read".to_string()
            }
        );

        let result =
            engine.apply_rules_sync("Read", &serde_json::json!({"file_path": "/other/path"}), "");
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
        let result = engine.apply_rules_sync("Read", &serde_json::json!({}), "");
        assert_eq!(
            result,
            ToolRuleResult::Allowed {
                rule_name: "allow-read".to_string()
            }
        );

        // Write falls through to wildcard
        let result = engine.apply_rules_sync("Write", &serde_json::json!({}), "");
        assert_eq!(
            result,
            ToolRuleResult::Asked {
                rule_name: "ask-all".to_string()
            }
        );
    }

    #[test]
    fn test_allow_local_matches_path_and_file_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path();
        let existing_file = cwd.join("src/lib.rs");
        fs::create_dir_all(existing_file.parent().unwrap()).unwrap();
        fs::write(&existing_file, "fn lib() {}\n").unwrap();

        assert!(tool_input_has_local_path(
            &[PathBuf::from("src/lib.rs")],
            cwd
        ));
        assert!(tool_input_has_local_path(&[existing_file.clone()], cwd));

        let engine = ToolRuleEngine::from_config(
            vec![make_local_rule(
                "allow-local-read",
                "Read",
                Some("file_path"),
                Some(r"^src/.*\.rs$"),
                ToolRuleAction::Allow,
            )],
            None,
        );

        let result = engine.apply_rules_sync(
            "Read",
            &serde_json::json!({"file_path": existing_file}),
            cwd.to_str().unwrap(),
        );
        assert_eq!(
            result,
            ToolRuleResult::Allowed {
                rule_name: "allow-local-read".to_string()
            }
        );
    }

    #[test]
    fn test_allow_local_matches_nonexistent_targets_inside_cwd() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path();

        assert!(tool_input_has_local_path(
            &[PathBuf::from("nested/new/file.txt")],
            cwd
        ));
        assert!(tool_input_has_local_path(
            &[PathBuf::from("nested/./deeper/../file.txt")],
            cwd
        ));

        let engine = ToolRuleEngine::from_config(
            vec![make_local_rule(
                "allow-generated",
                "Write",
                Some("file_path"),
                Some(r"^nested/new/.*\.txt$"),
                ToolRuleAction::Allow,
            )],
            None,
        );

        let result = engine.apply_rules_sync(
            "Write",
            &serde_json::json!({"file_path": "nested/new/file.txt"}),
            cwd.to_str().unwrap(),
        );
        assert_eq!(
            result,
            ToolRuleResult::Allowed {
                rule_name: "allow-generated".to_string()
            }
        );
    }

    #[test]
    fn test_allow_local_rejects_parent_escape() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path();

        assert!(!tool_input_has_local_path(
            &[PathBuf::from("../outside.txt")],
            cwd
        ));
        assert!(!tool_input_has_local_path(
            &[PathBuf::from("nested/../../outside.txt")],
            cwd
        ));
    }

    #[test]
    fn test_allow_local_rejects_missing_path_fields() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path();

        let engine = ToolRuleEngine::from_config(
            vec![make_local_rule(
                "allow-local-read",
                "Read",
                None,
                None,
                ToolRuleAction::Allow,
            )],
            None,
        );

        let result = engine.apply_rules_sync(
            "Read",
            &serde_json::json!({"content": "x"}),
            cwd.to_str().unwrap(),
        );
        assert_eq!(result, ToolRuleResult::NoMatch);
    }

    #[test]
    fn test_allow_local_requires_both_locality_and_regex() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path();
        fs::create_dir_all(cwd.join("src")).unwrap();
        fs::write(cwd.join("src/lib.rs"), "fn lib() {}\n").unwrap();
        fs::write(cwd.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();

        let engine = ToolRuleEngine::from_config(
            vec![make_local_rule(
                "allow-local-rust-only",
                "Read",
                Some("path"),
                Some(r"^src/.*\.rs$"),
                ToolRuleAction::Allow,
            )],
            None,
        );

        let non_matching_regex = engine.apply_rules_sync(
            "Read",
            &serde_json::json!({"path": cwd.join("Cargo.toml")}),
            cwd.to_str().unwrap(),
        );
        assert_eq!(non_matching_regex, ToolRuleResult::NoMatch);
    }

    #[test]
    fn test_allow_local_still_blocks_matching_non_local_regex() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path();
        let outside_dir = tempfile::tempdir().unwrap();
        let outside_file = outside_dir.path().join("src/lib.rs");
        fs::create_dir_all(outside_file.parent().unwrap()).unwrap();
        fs::write(&outside_file, "fn lib() {}\n").unwrap();

        let engine = ToolRuleEngine::from_config(
            vec![make_local_rule(
                "allow-local-rust-only",
                "Read",
                Some("path"),
                Some(r"src/lib\.rs$"),
                ToolRuleAction::Allow,
            )],
            None,
        );

        let non_local = engine.apply_rules_sync(
            "Read",
            &serde_json::json!({"path": outside_file}),
            cwd.to_str().unwrap(),
        );
        assert_eq!(non_local, ToolRuleResult::NoMatch);
    }

    #[test]
    fn test_allow_local_rejects_when_target_field_is_missing() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path();
        fs::write(cwd.join("lib.rs"), "fn lib() {}\n").unwrap();

        let path_engine = ToolRuleEngine::from_config(
            vec![make_local_rule(
                "allow-local-path",
                "Read",
                Some("path"),
                Some(".*"),
                ToolRuleAction::Allow,
            )],
            None,
        );

        let result = path_engine.apply_rules_sync(
            "Read",
            &serde_json::json!({"file_path": cwd.join("lib.rs")}),
            cwd.to_str().unwrap(),
        );
        assert_eq!(result, ToolRuleResult::NoMatch);

        let file_path_engine = ToolRuleEngine::from_config(
            vec![make_local_rule(
                "allow-local-file-path",
                "Read",
                Some("file_path"),
                Some(".*"),
                ToolRuleAction::Allow,
            )],
            None,
        );

        let result = file_path_engine.apply_rules_sync(
            "Read",
            &serde_json::json!({"path": cwd.join("lib.rs")}),
            cwd.to_str().unwrap(),
        );
        assert_eq!(result, ToolRuleResult::NoMatch);
    }

    #[test]
    fn test_allow_local_rejects_missing_cwd() {
        let temp_dir = tempfile::tempdir().unwrap();
        let missing_cwd = temp_dir.path().join("missing");

        assert!(!tool_input_has_local_path(
            &[PathBuf::from("src/lib.rs")],
            &missing_cwd
        ));
    }

    #[test]
    fn test_allow_local_treats_non_string_path_as_absent() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path();
        let engine = ToolRuleEngine::from_config(
            vec![make_local_rule(
                "allow-any-local",
                "Read",
                None,
                None,
                ToolRuleAction::Allow,
            )],
            None,
        );

        let result = engine.apply_rules_sync(
            "Read",
            &serde_json::json!({"path": 42}),
            cwd.to_str().unwrap(),
        );
        assert_eq!(result, ToolRuleResult::NoMatch);
    }

    #[test]
    fn test_allow_local_matches_when_either_path_field_is_local() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path();
        let outside_dir = tempfile::tempdir().unwrap();

        assert!(tool_input_has_local_path(
            &[
                outside_dir.path().join("outside.txt"),
                cwd.join("inside.txt"),
            ],
            cwd,
        ));
    }

    #[test]
    fn test_allow_local_with_non_path_field_never_matches() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path();
        fs::write(cwd.join("local.txt"), "hello\n").unwrap();

        let engine = ToolRuleEngine::from_config(
            vec![make_local_rule(
                "bad-command-locality",
                "Read",
                Some("command"),
                Some("^cat"),
                ToolRuleAction::Allow,
            )],
            None,
        );

        let result = engine.apply_rules_sync(
            "Read",
            &serde_json::json!({
                "command": "cat local.txt",
                "path": cwd.join("local.txt"),
            }),
            cwd.to_str().unwrap(),
        );
        assert_eq!(result, ToolRuleResult::NoMatch);
    }

    #[test]
    fn test_allow_local_without_field_matches_local_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path();
        fs::write(cwd.join("local.txt"), "hello\n").unwrap();

        let engine = ToolRuleEngine::from_config(
            vec![make_local_rule(
                "allow-any-local",
                "Read",
                None,
                None,
                ToolRuleAction::Allow,
            )],
            None,
        );

        let result = engine.apply_rules_sync(
            "Read",
            &serde_json::json!({"path": cwd.join("local.txt")}),
            cwd.to_str().unwrap(),
        );
        assert_eq!(
            result,
            ToolRuleResult::Allowed {
                rule_name: "allow-any-local".to_string()
            }
        );

        let result = engine.apply_rules_sync(
            "Read",
            &serde_json::json!({"file_path": cwd.join("local.txt")}),
            cwd.to_str().unwrap(),
        );
        assert_eq!(
            result,
            ToolRuleResult::Allowed {
                rule_name: "allow-any-local".to_string()
            }
        );
    }

    #[test]
    fn test_allow_local_wildcard_matches_any_tool() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path();
        fs::write(cwd.join("local.txt"), "hello\n").unwrap();

        let engine = ToolRuleEngine::from_config(
            vec![make_local_rule(
                "deny-all-local",
                "*",
                None,
                None,
                ToolRuleAction::Deny {
                    value: "no local ops".to_string(),
                },
            )],
            None,
        );

        let result = engine.apply_rules_sync(
            "Edit",
            &serde_json::json!({"path": cwd.join("local.txt")}),
            cwd.to_str().unwrap(),
        );
        assert_eq!(
            result,
            ToolRuleResult::Denied {
                rule_name: "deny-all-local".to_string(),
                reason: "no local ops".to_string(),
            }
        );

        let result = engine.apply_rules_sync(
            "Edit",
            &serde_json::json!({"path": "/outside/local.txt"}),
            cwd.to_str().unwrap(),
        );
        assert_eq!(result, ToolRuleResult::NoMatch);
    }

    #[test]
    fn test_allow_local_wildcard_with_regex() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path();
        fs::create_dir_all(cwd.join("src")).unwrap();
        fs::write(cwd.join("src/lib.rs"), "fn lib() {}\n").unwrap();
        fs::write(cwd.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();

        let engine = ToolRuleEngine::from_config(
            vec![make_local_rule(
                "deny-any-local-rs",
                "*",
                Some("path"),
                Some(r"\.rs$"),
                ToolRuleAction::Deny {
                    value: "no rs".to_string(),
                },
            )],
            None,
        );

        // Matches: local path + regex hits
        let result = engine.apply_rules_sync(
            "Edit",
            &serde_json::json!({"path": cwd.join("src/lib.rs")}),
            cwd.to_str().unwrap(),
        );
        assert_eq!(
            result,
            ToolRuleResult::Denied {
                rule_name: "deny-any-local-rs".to_string(),
                reason: "no rs".to_string(),
            }
        );

        // No match: local path but regex misses
        let result = engine.apply_rules_sync(
            "Write",
            &serde_json::json!({"path": cwd.join("Cargo.toml")}),
            cwd.to_str().unwrap(),
        );
        assert_eq!(result, ToolRuleResult::NoMatch);
    }

    #[test]
    fn test_allow_local_falls_through_to_later_rule() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path();

        let engine = ToolRuleEngine::from_config(
            vec![
                make_local_rule(
                    "allow-local-rust",
                    "Write",
                    Some("path"),
                    Some(r"\.rs$"),
                    ToolRuleAction::Allow,
                ),
                make_rule(
                    "deny-all-writes",
                    "Write",
                    None,
                    None,
                    ToolRuleAction::Deny {
                        value: "writes denied".to_string(),
                    },
                ),
            ],
            None,
        );

        let result = engine.apply_rules_sync(
            "Write",
            &serde_json::json!({"path": "/outside/src/lib.rs"}),
            cwd.to_str().unwrap(),
        );
        assert_eq!(
            result,
            ToolRuleResult::Denied {
                rule_name: "deny-all-writes".to_string(),
                reason: "writes denied".to_string(),
            }
        );
    }

    #[test]
    fn test_allow_local_can_deny_local_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path();
        fs::write(cwd.join("secret.txt"), "shh\n").unwrap();

        let engine = ToolRuleEngine::from_config(
            vec![make_local_rule(
                "deny-local-read",
                "Read",
                None,
                None,
                ToolRuleAction::Deny {
                    value: "local reads denied".to_string(),
                },
            )],
            None,
        );

        let result = engine.apply_rules_sync(
            "Read",
            &serde_json::json!({"path": cwd.join("secret.txt")}),
            cwd.to_str().unwrap(),
        );
        assert_eq!(
            result,
            ToolRuleResult::Denied {
                rule_name: "deny-local-read".to_string(),
                reason: "local reads denied".to_string(),
            }
        );
    }

    #[test]
    fn test_allow_local_treats_path_through_file_as_local() {
        // A path like `Cargo.toml/child.txt` where `Cargo.toml` is a regular file (not a
        // directory) cannot actually be read, but `canonicalize_allow_missing` still resolves
        // it as local because `Cargo.toml` exists inside cwd. This ensures Deny rules catch
        // such paths rather than silently passing through.
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path();
        fs::write(cwd.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();

        let engine = ToolRuleEngine::from_config(
            vec![make_local_rule(
                "deny-local-read",
                "Read",
                None,
                None,
                ToolRuleAction::Deny {
                    value: "local reads denied".to_string(),
                },
            )],
            None,
        );

        let result = engine.apply_rules_sync(
            "Read",
            &serde_json::json!({"path": cwd.join("Cargo.toml/child.txt")}),
            cwd.to_str().unwrap(),
        );
        assert_eq!(
            result,
            ToolRuleResult::Denied {
                rule_name: "deny-local-read".to_string(),
                reason: "local reads denied".to_string(),
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_allow_local_accepts_symlink_within_cwd() {
        use std::os::unix::fs::symlink;

        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path().join("project");
        let inside = cwd.join("inside");
        fs::create_dir_all(&inside).unwrap();
        fs::write(inside.join("file.txt"), "data\n").unwrap();
        symlink(&inside, cwd.join("linked-inside")).unwrap();

        assert!(tool_input_has_local_path(
            &[PathBuf::from("linked-inside/file.txt")],
            &cwd
        ));
    }

    #[cfg(unix)]
    #[test]
    fn test_allow_local_rejects_broken_symlink() {
        use std::os::unix::fs::symlink;

        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path().join("project");
        let missing = temp_dir.path().join("missing-target");
        fs::create_dir_all(&cwd).unwrap();
        symlink(&missing, cwd.join("broken-link")).unwrap();

        assert!(!tool_input_has_local_path(
            &[PathBuf::from("broken-link/file.txt")],
            &cwd
        ));
    }

    #[cfg(unix)]
    #[test]
    fn test_allow_local_rejects_broken_symlink_as_leaf() {
        use std::os::unix::fs::symlink;

        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path().join("project");
        let missing = temp_dir.path().join("missing-target");
        fs::create_dir_all(&cwd).unwrap();
        symlink(&missing, cwd.join("broken-link")).unwrap();

        // "broken-link" is itself the broken symlink — no child path appended.
        // This exercises symlink_metadata on the first canonicalize failure iteration,
        // unlike the parent-component test which hits it on a later iteration.
        assert!(!tool_input_has_local_path(
            &[PathBuf::from("broken-link")],
            &cwd
        ));
    }

    #[cfg(unix)]
    #[test]
    fn test_allow_local_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path().join("project");
        let outside = temp_dir.path().join("outside");
        fs::create_dir_all(&cwd).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.txt"), "secret\n").unwrap();
        symlink(&outside, cwd.join("linked-outside")).unwrap();

        assert!(!tool_input_has_local_path(
            &[PathBuf::from("linked-outside/secret.txt")],
            &cwd
        ));
    }

    #[test]
    fn test_canonicalize_allow_missing_handles_non_directory_ancestor() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path();
        let file = cwd.join("Cargo.toml");
        fs::write(&file, "[package]\nname = \"x\"\n").unwrap();

        let canonical_cwd = fs::canonicalize(cwd).unwrap();
        let resolved = canonicalize_allow_missing(&cwd.join("Cargo.toml/child.txt")).unwrap();
        assert_eq!(resolved, canonical_cwd.join("Cargo.toml/child.txt"));
    }

    #[test]
    fn test_canonicalize_allow_missing_rejects_escape_in_missing_suffix() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path();

        let err = canonicalize_allow_missing(&cwd.join("missing/../..")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert!(err.to_string().contains("escapes"));
    }

    // ===== strip_cwd_prefix tests =====

    #[test]
    fn test_strip_cwd_prefix_basic() {
        assert_eq!(
            strip_cwd_prefix("/home/user/project/src/main.rs", "/home/user/project"),
            "src/main.rs"
        );
    }

    #[test]
    fn test_strip_cwd_prefix_trailing_slash_on_cwd() {
        assert_eq!(
            strip_cwd_prefix("/home/user/project/src/main.rs", "/home/user/project/"),
            "src/main.rs"
        );
    }

    #[test]
    fn test_strip_cwd_prefix_value_equals_cwd() {
        assert_eq!(
            strip_cwd_prefix("/home/user/project", "/home/user/project"),
            ""
        );
    }

    #[test]
    fn test_strip_cwd_prefix_value_equals_cwd_trailing_slash() {
        assert_eq!(
            strip_cwd_prefix("/home/user/project", "/home/user/project/"),
            ""
        );
    }

    #[test]
    fn test_strip_cwd_prefix_no_match() {
        assert_eq!(
            strip_cwd_prefix("/other/path/file.rs", "/home/user/project"),
            "/other/path/file.rs"
        );
    }

    #[test]
    fn test_strip_cwd_prefix_partial_dir_name() {
        assert_eq!(strip_cwd_prefix("/foobar/baz", "/foo"), "/foobar/baz");
    }

    #[test]
    fn test_strip_cwd_prefix_already_relative() {
        assert_eq!(
            strip_cwd_prefix("src/main.rs", "/home/user/project"),
            "src/main.rs"
        );
    }

    #[test]
    fn test_strip_cwd_prefix_empty_cwd() {
        assert_eq!(
            strip_cwd_prefix("/home/user/file.rs", ""),
            "/home/user/file.rs"
        );
    }

    #[test]
    fn test_strip_cwd_prefix_root_cwd() {
        // Root cwd "/" normalizes to empty after trailing-slash strip, so no stripping occurs
        assert_eq!(strip_cwd_prefix("/foo", "/"), "/foo");
    }

    // ===== cwd stripping in apply_rules =====

    #[test]
    fn test_cwd_stripping_matches_relative_pattern() {
        let rules = vec![make_rule(
            "allow-flake",
            "Read",
            Some("path"),
            Some(r"^flake\.nix$"),
            ToolRuleAction::Allow,
        )];
        let engine = ToolRuleEngine::from_config(rules, None);

        let result = engine.apply_rules_sync(
            "Read",
            &serde_json::json!({"path": "/tmp/project/flake.nix"}),
            "/tmp/project",
        );
        assert_eq!(
            result,
            ToolRuleResult::Allowed {
                rule_name: "allow-flake".to_string()
            }
        );
    }

    #[test]
    fn test_cwd_stripping_no_match_different_cwd() {
        let rules = vec![make_rule(
            "allow-flake",
            "Read",
            Some("path"),
            Some(r"^flake\.nix$"),
            ToolRuleAction::Allow,
        )];
        let engine = ToolRuleEngine::from_config(rules, None);

        // Path doesn't start with cwd, so no stripping — absolute path won't match "^flake\.nix$"
        let result = engine.apply_rules_sync(
            "Read",
            &serde_json::json!({"path": "/tmp/project/flake.nix"}),
            "/other/dir",
        );
        assert_eq!(result, ToolRuleResult::NoMatch);
    }

    #[test]
    fn test_cwd_stripping_absolute_pattern_still_works_without_cwd() {
        let rules = vec![make_rule(
            "allow-absolute",
            "Read",
            Some("path"),
            Some(r"^/tmp/project/"),
            ToolRuleAction::Allow,
        )];
        let engine = ToolRuleEngine::from_config(rules, None);

        let result = engine.apply_rules_sync(
            "Read",
            &serde_json::json!({"path": "/tmp/project/flake.nix"}),
            "",
        );
        assert_eq!(
            result,
            ToolRuleResult::Allowed {
                rule_name: "allow-absolute".to_string()
            }
        );
    }

    #[test]
    fn test_cwd_stripping_subdirectory_path() {
        let rules = vec![make_rule(
            "allow-src",
            "Write",
            Some("file_path"),
            Some(r"^src/"),
            ToolRuleAction::Allow,
        )];
        let engine = ToolRuleEngine::from_config(rules, None);

        let result = engine.apply_rules_sync(
            "Write",
            &serde_json::json!({"file_path": "/home/user/project/src/lib.rs"}),
            "/home/user/project",
        );
        assert_eq!(
            result,
            ToolRuleResult::Allowed {
                rule_name: "allow-src".to_string()
            }
        );
    }

    #[test]
    fn test_cwd_stripping_trailing_slash_on_cwd() {
        let rules = vec![make_rule(
            "allow-src",
            "Read",
            Some("file_path"),
            Some(r"^src/"),
            ToolRuleAction::Allow,
        )];
        let engine = ToolRuleEngine::from_config(rules, None);

        let result = engine.apply_rules_sync(
            "Read",
            &serde_json::json!({"file_path": "/home/user/project/src/main.rs"}),
            "/home/user/project/",
        );
        assert_eq!(
            result,
            ToolRuleResult::Allowed {
                rule_name: "allow-src".to_string()
            }
        );
    }

    #[test]
    fn test_cwd_stripping_value_equals_cwd() {
        // When value equals cwd, stripping produces "" — pattern "^$" matches empty string
        let rules = vec![make_rule(
            "match-empty",
            "Read",
            Some("path"),
            Some(r"^$"),
            ToolRuleAction::Allow,
        )];
        let engine = ToolRuleEngine::from_config(rules, None);

        let result = engine.apply_rules_sync(
            "Read",
            &serde_json::json!({"path": "/home/user/project"}),
            "/home/user/project",
        );
        assert_eq!(
            result,
            ToolRuleResult::Allowed {
                rule_name: "match-empty".to_string()
            }
        );
    }
}
