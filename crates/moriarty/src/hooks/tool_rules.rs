//! Tool call validation rules for any Claude Code tool.
//!
//! This module provides a rule engine for permissioning arbitrary tool calls (Read, Write, Edit,
//! Bash, etc.) before Claude Code executes them. Unlike `bash_rules`, which operates on command
//! strings, tool rules combine a tool name with an optional legacy field regex and typed,
//! presence-aware conditions over top-level input keys. Rules may also require selected `path` or
//! `file_path` inputs to resolve within the canonicalized hook `cwd`, with safe handling of
//! non-existent targets. Regex values under `cwd` are normalized to relative paths.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    ffi::OsString,
    fs, io,
    path::{Component, Path, PathBuf},
};

use regex::Regex;
use tokio::task::spawn_blocking;
use tracing::{debug, warn};

use super::bash_rules::{
    RuleDiagnostic, RuleDiagnosticKind, classify_fragment_error, default_fragments,
    expand_fragments,
};
use crate::{
    permission_mode::{PermissionMode, is_mode_eligible},
    user_config::{ToolRule, ToolRuleAction, ToolRuleCondition},
};

const PATH_FIELD: &str = "path";
const FILE_PATH_FIELD: &str = "file_path";
const PATH_FIELDS: [&str; 2] = [PATH_FIELD, FILE_PATH_FIELD];

#[derive(Debug)]
enum CompiledCondition {
    /// Key exists in the top-level input object (value, including null, is irrelevant).
    Present { field: String },
    /// Key does not exist in the top-level input object.
    Absent { field: String },
    /// Key exists and its raw JSON value equals the stored value (type-sensitive recursive
    /// equality). Path values are compared raw — locality is a separate gate.
    Equals {
        field: String,
        value: serde_json::Value,
    },
    /// Scalar field value matches the compiled regex after cwd-stripping. The regex is
    /// pre-expanded and compiled at rule-load time; a compilation failure drops the entire rule.
    Matches { field: String, regex: Regex },
}

impl CompiledCondition {
    fn locality_field(&self) -> Option<&str> {
        match self {
            Self::Present { field } | Self::Equals { field, .. } | Self::Matches { field, .. } => {
                Some(field)
            }
            Self::Absent { .. } => None,
        }
    }
}

#[derive(Debug)]
struct LegacyPattern {
    field: String,
    regex: Regex,
}

/// Runtime representation of a tool rule with pre-compiled regex for the field pattern.
#[derive(Debug)]
struct CompiledToolRule {
    name: String,
    tool: String,
    modes: Option<BTreeSet<PermissionMode>>,
    allow_local: bool,
    legacy: Option<LegacyPattern>,
    conditions: Vec<CompiledCondition>,
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

    fn candidate_for_field(&self, field: &str) -> Option<&CandidatePathEvaluation> {
        match field {
            PATH_FIELD => self.path.as_ref(),
            FILE_PATH_FIELD => self.file_path.as_ref(),
            _ => None,
        }
    }

    fn resolved_local_path(&self, field: &str) -> Option<&Path> {
        self.candidate_for_field(field)
            .filter(|evaluation| evaluation.is_local)
            .map(|evaluation| evaluation.resolved_path.as_path())
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
}

/// Extracts only the `path` and `file_path` fields from the tool input so that only those
/// two small strings need to be moved into the `spawn_blocking` closure, avoiding a full
/// clone of a potentially-large input (e.g., a Write tool call's `content` field).
fn locality_input(tool_input: &serde_json::Value) -> serde_json::Value {
    serde_json::Value::Object(
        PATH_FIELDS
            .iter()
            .filter_map(|field| {
                tool_input
                    .get(*field)
                    .cloned()
                    .map(|value| ((*field).to_string(), value))
            })
            .collect(),
    )
}

fn is_missing_path_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
    )
        // Older Windows/Rust combinations may surface ERROR_DIRECTORY (267) as ErrorKind::Other.
        || cfg!(windows) && error.raw_os_error() == Some(267)
}

fn compile_rule_pattern(
    rule_name: &str,
    pattern: &str,
    condition_field: Option<&str>,
    fragments: &HashMap<String, String>,
) -> Result<Regex, RuleDiagnostic> {
    let add_context = |message: String| match condition_field {
        Some(field) => format!("Matches condition for field '{field}': {message}"),
        None => message,
    };

    let expanded = expand_fragments(pattern, fragments).map_err(|error| {
        let message = error.to_string();
        RuleDiagnostic {
            kind: classify_fragment_error(&message),
            rule_name: rule_name.to_string(),
            pattern: pattern.to_string(),
            message: add_context(message),
        }
    })?;

    Regex::new(&expanded).map_err(|error| RuleDiagnostic {
        rule_name: rule_name.to_string(),
        pattern: pattern.to_string(),
        kind: RuleDiagnosticKind::InvalidRegex,
        message: add_context(error.to_string()),
    })
}

fn compile_condition(
    rule_name: &str,
    condition: ToolRuleCondition,
    fragments: &HashMap<String, String>,
) -> Result<CompiledCondition, RuleDiagnostic> {
    match condition {
        ToolRuleCondition::Present { field } => Ok(CompiledCondition::Present { field }),
        ToolRuleCondition::Absent { field } => Ok(CompiledCondition::Absent { field }),
        ToolRuleCondition::Equals { field, value } => {
            Ok(CompiledCondition::Equals { field, value })
        }
        ToolRuleCondition::Matches { field, pattern } => {
            let regex = compile_rule_pattern(rule_name, &pattern, Some(&field), fragments)?;
            Ok(CompiledCondition::Matches { field, regex })
        }
    }
}

impl ToolRuleEngine {
    /// Compiles tool rules with pattern fragment expansion, logging and skipping malformed rules
    /// (incomplete `field`/`pattern` pairs, unexpandable fragments, or invalid regexes).
    pub fn from_config(rules: Vec<ToolRule>, fragments: Option<HashMap<String, String>>) -> Self {
        let (engine, diagnostics) = Self::compile_with_diagnostics(rules, fragments);
        for diagnostic in &diagnostics {
            warn!(
                rule_name = %diagnostic.rule_name,
                pattern = %diagnostic.pattern,
                error = %diagnostic.message,
                "Skipping tool rule the hook cannot compile"
            );
        }
        engine
    }

    /// Compiles tool rules, returning the engine and a diagnostic per dropped rule. Does not log;
    /// the caller decides how to surface dropped rules (the hook warns; `rules lint` errors).
    ///
    /// Condition compilation is atomic per rule: any unexpandable or invalid `Matches` regex in a
    /// condition emits one diagnostic and drops the entire rule. Dropping only the bad condition
    /// would broaden the remaining rule, potentially turning an intended restricted Allow into an
    /// unsafe Allow.
    pub(crate) fn compile_with_diagnostics(
        rules: Vec<ToolRule>,
        fragments: Option<HashMap<String, String>>,
    ) -> (Self, Vec<RuleDiagnostic>) {
        let mut merged_fragments = default_fragments();
        if let Some(user_frags) = fragments {
            merged_fragments.extend(user_frags);
        }

        let mut compiled = Vec::new();
        let mut diagnostics = Vec::new();

        for mut rule in rules {
            let field = rule.field.take();
            let pattern = rule.pattern.take();
            let conditions = std::mem::take(&mut rule.conditions);

            let legacy = match (field, pattern) {
                (Some(_), None) => {
                    diagnostics.push(RuleDiagnostic {
                        rule_name: rule.name,
                        pattern: String::new(),
                        kind: RuleDiagnosticKind::MissingFieldOrPattern,
                        message: "Rule has 'field' without 'pattern'".to_string(),
                    });
                    continue;
                }
                (None, Some(pattern)) => {
                    diagnostics.push(RuleDiagnostic {
                        rule_name: rule.name,
                        pattern,
                        kind: RuleDiagnosticKind::MissingFieldOrPattern,
                        message: "Rule has 'pattern' without 'field'".to_string(),
                    });
                    continue;
                }
                (Some(field), Some(pattern)) => {
                    match compile_rule_pattern(&rule.name, &pattern, None, &merged_fragments) {
                        Ok(regex) => Some(LegacyPattern { field, regex }),
                        Err(diagnostic) => {
                            diagnostics.push(diagnostic);
                            continue;
                        }
                    }
                }
                (None, None) => None,
            };

            let compiled_conditions = match conditions
                .into_iter()
                .map(|condition| compile_condition(&rule.name, condition, &merged_fragments))
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(conditions) => conditions,
                Err(diagnostic) => {
                    diagnostics.push(diagnostic);
                    continue;
                }
            };

            compiled.push(CompiledToolRule {
                name: rule.name,
                tool: rule.tool,
                modes: rule.modes,
                allow_local: rule.allow_local,
                legacy,
                conditions: compiled_conditions,
                action: rule.action,
            });
        }

        (Self { rules: compiled }, diagnostics)
    }

    fn has_matching_allow_local_rule(&self, tool_name: &str, mode: Option<PermissionMode>) -> bool {
        self.rules.iter().any(|rule| {
            rule.allow_local
                && (rule.tool == "*" || rule.tool == tool_name)
                && is_mode_eligible(rule.modes.as_ref(), mode)
        })
    }

    fn apply_rules_core(
        &self,
        tool_name: &str,
        tool_input: &serde_json::Value,
        cwd: &str,
        mode: Option<PermissionMode>,
        local_evaluation: Option<&LocalPathEvaluation>,
    ) -> ToolRuleResult {
        for rule in &self.rules {
            if !is_mode_eligible(rule.modes.as_ref(), mode) {
                continue;
            }

            if rule.tool != "*" && rule.tool != tool_name {
                continue;
            }

            if rule.allow_local && !rule_matches_allow_local(rule, local_evaluation) {
                continue;
            }

            // A cached locality result may exist because an earlier or later rule needs it; only
            // this rule's own allow_local setting may change how its path regexes are normalized.
            let local_evaluation_for_rule = rule.allow_local.then_some(local_evaluation).flatten();

            if !conditions_match(rule, tool_input, cwd, local_evaluation_for_rule) {
                continue;
            }

            if !rule_matches_regex(rule, tool_input, cwd, local_evaluation_for_rule) {
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
    /// `^/home/user/project/src/`). With `allow_local = true`, condition-free rules retain their
    /// legacy path selection, while condition-bearing rules require every `path` or `file_path`
    /// selected by a presence-requiring predicate or legacy path field to resolve within the
    /// canonicalized hook cwd. The filesystem work runs on the blocking thread pool.
    pub async fn apply_rules(
        &self,
        tool_name: &str,
        tool_input: &serde_json::Value,
        cwd: &str,
        mode: Option<PermissionMode>,
    ) -> ToolRuleResult {
        let local_evaluation = if self.has_matching_allow_local_rule(tool_name, mode) {
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

        self.apply_rules_core(tool_name, tool_input, cwd, mode, local_evaluation.as_ref())
    }

    #[cfg(test)]
    fn apply_rules_sync(
        &self,
        tool_name: &str,
        tool_input: &serde_json::Value,
        cwd: &str,
        mode: Option<PermissionMode>,
    ) -> ToolRuleResult {
        let local_evaluation = self
            .has_matching_allow_local_rule(tool_name, mode)
            .then(|| evaluate_local_paths(&locality_input(tool_input), Path::new(cwd)))
            .flatten();

        self.apply_rules_core(tool_name, tool_input, cwd, mode, local_evaluation.as_ref())
    }
}

fn rule_matches_allow_local(
    rule: &CompiledToolRule,
    local_evaluation: Option<&LocalPathEvaluation>,
) -> bool {
    let Some(local_evaluation) = local_evaluation else {
        return false;
    };

    let mut path_fields: HashSet<&str> = HashSet::new();
    if let Some(field) = rule
        .legacy
        .as_ref()
        .map(|legacy| legacy.field.as_str())
        .filter(|field| is_path_field(field))
    {
        path_fields.insert(field);
    }
    path_fields.extend(
        rule.conditions
            .iter()
            .filter_map(CompiledCondition::locality_field)
            .filter(|field| is_path_field(field)),
    );

    if path_fields.is_empty() {
        // This also preserves condition-free rules: a legacy non-path field cannot establish
        // locality, while a tool-only rule accepts any available local path.
        return rule.legacy.is_none() && local_evaluation.any_local();
    }

    path_fields.iter().all(|field| {
        local_evaluation
            .candidate_for_field(field)
            .is_some_and(|evaluation| evaluation.is_local)
    })
}

fn is_path_field(field: &str) -> bool {
    PATH_FIELDS.contains(&field)
}

/// Keeps legacy field patterns and conjunctive `Matches` predicates aligned: only path fields
/// under an allow_local rule use the canonicalized path; every other field is matched raw after
/// cwd-prefix stripping.
fn regex_match_text(
    tool_input: &serde_json::Value,
    field: &str,
    cwd: &str,
    local_evaluation: Option<&LocalPathEvaluation>,
) -> Option<String> {
    if is_path_field(field)
        && let Some(local_evaluation) = local_evaluation
    {
        let resolved_path = local_evaluation.resolved_local_path(field)?;
        return Some(
            strip_cwd_prefix(
                &resolved_path.to_string_lossy(),
                &local_evaluation.canonical_cwd.to_string_lossy(),
            )
            .to_string(),
        );
    }

    let value = tool_input.get(field)?;
    let scalar = extract_field_value(value)?;
    Some(strip_cwd_prefix(&scalar, cwd).to_string())
}

/// Conditions require a top-level object so an `Absent` predicate cannot accidentally broaden a
/// rule when an upstream caller supplies malformed input.
fn conditions_match(
    rule: &CompiledToolRule,
    tool_input: &serde_json::Value,
    cwd: &str,
    local_evaluation: Option<&LocalPathEvaluation>,
) -> bool {
    if rule.conditions.is_empty() {
        return true;
    }

    let obj = match tool_input.as_object() {
        Some(obj) => obj,
        None => return false,
    };

    for cond in &rule.conditions {
        match cond {
            CompiledCondition::Present { field } => {
                if !obj.contains_key(field.as_str()) {
                    return false;
                }
            }
            CompiledCondition::Absent { field } => {
                if obj.contains_key(field.as_str()) {
                    return false;
                }
            }
            CompiledCondition::Equals { field, value } => match obj.get(field.as_str()) {
                Some(actual) if actual == value => {}
                _ => return false,
            },
            CompiledCondition::Matches { field, regex } => {
                let Some(value_for_matching) =
                    regex_match_text(tool_input, field, cwd, local_evaluation)
                else {
                    return false;
                };
                if !regex.is_match(&value_for_matching) {
                    return false;
                }
            }
        }
    }

    true
}

fn rule_matches_regex(
    rule: &CompiledToolRule,
    tool_input: &serde_json::Value,
    cwd: &str,
    local_evaluation: Option<&LocalPathEvaluation>,
) -> bool {
    let Some(legacy) = &rule.legacy else {
        return true;
    };

    regex_match_text(tool_input, &legacy.field, cwd, local_evaluation)
        .is_some_and(|value| legacy.regex.is_match(&value))
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
        path: evaluate_candidate_path(tool_input, PATH_FIELD, &canonical_cwd),
        file_path: evaluate_candidate_path(tool_input, FILE_PATH_FIELD, &canonical_cwd),
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
                return rebuild_missing_suffix(canonical, removed_components.into_iter().rev());
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
///
/// Shared with [`super::command_split`] so Bash leaf commands normalize in-cwd absolute paths the
/// same way tool-rule field matching does.
pub(crate) fn strip_cwd_prefix<'a>(value: &'a str, cwd: &str) -> &'a str {
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

    fn make_rule(name: &str, tool: &str) -> ToolRule {
        ToolRule {
            name: name.to_string(),
            tool: tool.to_string(),
            modes: None,
            allow_local: false,
            field: None,
            pattern: None,
            conditions: Vec::new(),
            action: ToolRuleAction::Allow,
        }
    }

    fn local_allow(name: &str, conditions: Vec<ToolRuleCondition>) -> ToolRule {
        make_rule(name, "Read").local().with_conditions(conditions)
    }

    trait ToolRuleTestExt {
        fn local(self) -> Self;
        fn with_field(self, field: &str) -> Self;
        fn with_pattern(self, pattern: &str) -> Self;
        fn with_legacy(self, field: &str, pattern: &str) -> Self;
        fn with_conditions(self, conditions: Vec<ToolRuleCondition>) -> Self;
        fn with_modes(self, modes: impl IntoIterator<Item = PermissionMode>) -> Self;
        fn with_action(self, action: ToolRuleAction) -> Self;
    }

    impl ToolRuleTestExt for ToolRule {
        fn local(mut self) -> Self {
            self.allow_local = true;
            self
        }

        fn with_field(mut self, field: &str) -> Self {
            self.field = Some(field.to_string());
            self
        }

        fn with_pattern(mut self, pattern: &str) -> Self {
            self.pattern = Some(pattern.to_string());
            self
        }

        fn with_legacy(self, field: &str, pattern: &str) -> Self {
            self.with_field(field).with_pattern(pattern)
        }

        fn with_conditions(mut self, conditions: Vec<ToolRuleCondition>) -> Self {
            self.conditions = conditions;
            self
        }

        fn with_modes(mut self, modes: impl IntoIterator<Item = PermissionMode>) -> Self {
            self.modes = Some(modes.into_iter().collect());
            self
        }

        fn with_action(mut self, action: ToolRuleAction) -> Self {
            self.action = action;
            self
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

    /// Keeps individual tests focused on rule behavior instead of repeating engine setup.
    fn apply_single_rule(
        rule: &ToolRule,
        tool: &str,
        input: &serde_json::Value,
        cwd: &str,
    ) -> ToolRuleResult {
        apply_rules(vec![rule.clone()], None, tool, input, cwd)
    }

    fn assert_rule_nomatch(rule: &ToolRule, tool: &str, input: serde_json::Value, cwd: &str) {
        assert_eq!(
            apply_single_rule(rule, tool, &input, cwd),
            ToolRuleResult::NoMatch
        );
    }

    fn assert_rule_allowed(rule: &ToolRule, tool: &str, input: serde_json::Value, cwd: &str) {
        assert_eq!(
            apply_single_rule(rule, tool, &input, cwd),
            ToolRuleResult::Allowed {
                rule_name: rule.name.clone(),
            }
        );
    }

    /// Keeps deny assertions from drifting when callers update a rule name but
    /// forget to update the expected result in lockstep.
    fn assert_rule_denied(
        rule: ToolRule,
        tool: &str,
        input: serde_json::Value,
        cwd: &str,
        reason: &str,
    ) {
        let rule_name = rule.name.clone();
        assert_eq!(
            apply_single_rule(&rule, tool, &input, cwd),
            ToolRuleResult::Denied {
                rule_name,
                reason: reason.to_string(),
            }
        );
    }

    fn assert_rule_asked(rule: ToolRule, tool: &str, input: serde_json::Value, cwd: &str) {
        let rule_name = rule.name.clone();
        assert_eq!(
            apply_single_rule(&rule, tool, &input, cwd),
            ToolRuleResult::Asked { rule_name }
        );
    }

    fn assert_engine_result(
        engine: &ToolRuleEngine,
        tool: &str,
        input: serde_json::Value,
        cwd: &str,
        expected: ToolRuleResult,
    ) {
        assert_eq!(engine.apply_rules_sync(tool, &input, cwd, None), expected);
    }

    fn assert_allow_cases(
        rule: ToolRule,
        tool: &str,
        cwd: &str,
        matches: &[serde_json::Value],
        misses: &[serde_json::Value],
    ) {
        let expected = ToolRuleResult::Allowed {
            rule_name: rule.name.clone(),
        };
        let engine = ToolRuleEngine::from_config(vec![rule], None);
        for input in matches {
            assert_eq!(
                engine.apply_rules_sync(tool, input, cwd, None),
                expected,
                "expected match for {input}"
            );
        }
        for input in misses {
            assert_eq!(
                engine.apply_rules_sync(tool, input, cwd, None),
                ToolRuleResult::NoMatch,
                "expected no match for {input}"
            );
        }
    }

    /// Creates a temp project and writes `content` at `relative_path`, returning
    /// the directory guard, canonical cwd path, and the created file path.
    fn temp_project_with_file(
        relative_path: &str,
        content: &str,
    ) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path().to_path_buf();
        let file_path = cwd.join(relative_path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&file_path, content).unwrap();
        (temp_dir, cwd, file_path)
    }

    fn lib_rs_project() -> (tempfile::TempDir, PathBuf, PathBuf) {
        temp_project_with_file("src/lib.rs", "fn lib() {}\n")
    }

    fn temp_outside_file(relative_path: &str, content: &str) -> (tempfile::TempDir, PathBuf) {
        let (temp_dir, _cwd, file_path) = temp_project_with_file(relative_path, content);
        (temp_dir, file_path)
    }

    fn assert_no_local_path(paths: &[PathBuf], cwd: &Path) {
        assert!(!tool_input_has_local_path(paths, cwd));
    }

    fn assert_has_local_path(paths: &[PathBuf], cwd: &Path) {
        assert!(tool_input_has_local_path(paths, cwd));
    }

    /// Keeps multi-rule tests focused on precedence instead of repeating engine setup.
    fn apply_rules(
        rules: Vec<ToolRule>,
        fragments: Option<HashMap<String, String>>,
        tool: &str,
        input: &serde_json::Value,
        cwd: &str,
    ) -> ToolRuleResult {
        ToolRuleEngine::from_config(rules, fragments).apply_rules_sync(tool, input, cwd, None)
    }

    #[test]
    fn test_empty_rules() {
        assert_eq!(
            apply_rules(
                vec![],
                None,
                "Read",
                &serde_json::json!({"file_path": "/tmp/foo"}),
                "",
            ),
            ToolRuleResult::NoMatch
        );
    }

    #[test]
    fn test_tool_name_only_allow() {
        let rule = make_rule("allow-read", "Read");
        let engine = ToolRuleEngine::from_config(vec![rule], None);

        assert_engine_result(
            &engine,
            "Read",
            serde_json::json!({"file_path": "/tmp/foo"}),
            "",
            ToolRuleResult::Allowed {
                rule_name: "allow-read".to_string(),
            },
        );
        // Doesn't match other tools
        assert_engine_result(
            &engine,
            "Write",
            serde_json::json!({}),
            "",
            ToolRuleResult::NoMatch,
        );
    }

    #[test]
    fn permission_modes_gate_ordered_tool_rules() {
        let plan_deny = make_rule("plan-only", "Read")
            .with_modes([PermissionMode::Plan])
            .with_action(ToolRuleAction::Deny {
                value: "plan deny".to_string(),
            });
        let disabled =
            make_rule("disabled", "Read")
                .with_modes([])
                .with_action(ToolRuleAction::Deny {
                    value: "disabled".to_string(),
                });
        let engine = ToolRuleEngine::from_config(
            vec![disabled, plan_deny, make_rule("fallback", "Read")],
            None,
        );
        let input = serde_json::json!({});

        assert_eq!(
            engine.apply_rules_sync("Read", &input, "", Some(PermissionMode::Plan)),
            ToolRuleResult::Denied {
                rule_name: "plan-only".to_string(),
                reason: "plan deny".to_string(),
            }
        );
        assert_eq!(
            engine.apply_rules_sync("Read", &input, "", Some(PermissionMode::Default)),
            ToolRuleResult::Allowed {
                rule_name: "fallback".to_string(),
            }
        );
        assert_eq!(
            engine.apply_rules_sync("Read", &input, "", None),
            ToolRuleResult::Allowed {
                rule_name: "fallback".to_string(),
            }
        );
    }

    #[test]
    fn unrestricted_tool_rule_applies_with_or_without_a_current_mode() {
        let engine = ToolRuleEngine::from_config(vec![make_rule("all", "Read")], None);
        let input = serde_json::json!({});
        assert!(matches!(
            engine.apply_rules_sync("Read", &input, "", Some(PermissionMode::Plan)),
            ToolRuleResult::Allowed { .. }
        ));
        assert!(matches!(
            engine.apply_rules_sync("Read", &input, "", None),
            ToolRuleResult::Allowed { .. }
        ));
    }

    #[test]
    fn locality_preflight_requires_a_mode_eligible_rule() {
        let engine = ToolRuleEngine::from_config(
            vec![
                make_rule("plan-local", "Read")
                    .local()
                    .with_modes([PermissionMode::Plan]),
            ],
            None,
        );
        assert!(!engine.has_matching_allow_local_rule("Read", Some(PermissionMode::Default)));
        assert!(!engine.has_matching_allow_local_rule("Read", None));
        assert!(engine.has_matching_allow_local_rule("Read", Some(PermissionMode::Plan)));
    }

    #[test]
    fn test_tool_name_deny() {
        let rule = make_rule("deny-write", "Write").with_action(ToolRuleAction::Deny {
            value: "Writes not allowed".to_string(),
        });
        assert_rule_denied(
            rule,
            "Write",
            serde_json::json!({}),
            "",
            "Writes not allowed",
        );
    }

    #[test]
    fn test_tool_name_ask() {
        let rule = make_rule("ask-edit", "Edit").with_action(ToolRuleAction::Ask);
        assert_rule_asked(rule, "Edit", serde_json::json!({}), "");
    }

    #[test]
    fn test_wildcard_matches_any_tool() {
        let rule = make_rule("catch-all", "*").with_action(ToolRuleAction::Ask);
        let expected = ToolRuleResult::Asked {
            rule_name: "catch-all".to_string(),
        };
        let engine = ToolRuleEngine::from_config(vec![rule], None);

        assert_engine_result(&engine, "Read", serde_json::json!({}), "", expected.clone());
        assert_engine_result(
            &engine,
            "Write",
            serde_json::json!({}),
            "",
            expected.clone(),
        );
        assert_engine_result(&engine, "Bash", serde_json::json!({}), "", expected);
    }

    #[test]
    fn test_field_pattern_matching() {
        let rule = make_rule("deny-env-write", "Write")
            .with_legacy("file_path", r"\.env$")
            .with_action(ToolRuleAction::Deny {
                value: "Cannot write .env files".to_string(),
            });
        let engine = ToolRuleEngine::from_config(vec![rule], None);

        assert_engine_result(
            &engine,
            "Write",
            serde_json::json!({"file_path": "/home/user/.env"}),
            "",
            ToolRuleResult::Denied {
                rule_name: "deny-env-write".to_string(),
                reason: "Cannot write .env files".to_string(),
            },
        );
        // Doesn't match (different extension)
        assert_engine_result(
            &engine,
            "Write",
            serde_json::json!({"file_path": "/home/user/main.rs"}),
            "",
            ToolRuleResult::NoMatch,
        );
    }

    #[test]
    fn test_field_pattern_missing_field_in_input() {
        let rule = make_rule("deny-env", "Write")
            .with_legacy("file_path", r"\.env$")
            .with_action(ToolRuleAction::Deny {
                value: "no".to_string(),
            });
        assert_rule_nomatch(&rule, "Write", serde_json::json!({"content": "hello"}), "");
    }

    #[test]
    fn test_field_value_extraction_types() {
        let rule = make_rule("match-number", "Test").with_legacy("count", "^42$");
        let engine = ToolRuleEngine::from_config(vec![rule], None);
        let allowed = ToolRuleResult::Allowed {
            rule_name: "match-number".to_string(),
        };

        assert_engine_result(
            &engine,
            "Test",
            serde_json::json!({"count": 42}),
            "",
            allowed,
        );
        assert_engine_result(
            &engine,
            "Test",
            serde_json::json!({"count": true}),
            "",
            ToolRuleResult::NoMatch,
        );
        assert_engine_result(
            &engine,
            "Test",
            serde_json::json!({"count": [42]}),
            "",
            ToolRuleResult::NoMatch,
        );
        assert_engine_result(
            &engine,
            "Test",
            serde_json::json!({"count": null}),
            "",
            ToolRuleResult::NoMatch,
        );

        // Bool positive match (bools are converted to "true"/"false" strings)
        let bool_rule = make_rule("match-bool", "Test").with_legacy("flag", "^true$");
        let bool_engine = ToolRuleEngine::from_config(vec![bool_rule], None);
        let bool_allowed = ToolRuleResult::Allowed {
            rule_name: "match-bool".to_string(),
        };

        assert_engine_result(
            &bool_engine,
            "Test",
            serde_json::json!({"flag": true}),
            "",
            bool_allowed,
        );
        assert_engine_result(
            &bool_engine,
            "Test",
            serde_json::json!({"flag": false}),
            "",
            ToolRuleResult::NoMatch,
        );
    }

    #[test]
    fn test_first_match_wins() {
        let rules = vec![
            make_rule("allow-specific", "Write").with_legacy("file_path", r"\.rs$"),
            make_rule("deny-all-writes", "Write").with_action(ToolRuleAction::Deny {
                value: "Writes denied".to_string(),
            }),
        ];
        let engine = ToolRuleEngine::from_config(rules, None);

        assert_engine_result(
            &engine,
            "Write",
            serde_json::json!({"file_path": "main.rs"}),
            "",
            ToolRuleResult::Allowed {
                rule_name: "allow-specific".to_string(),
            },
        );
        assert_engine_result(
            &engine,
            "Write",
            serde_json::json!({"file_path": "data.csv"}),
            "",
            ToolRuleResult::Denied {
                rule_name: "deny-all-writes".to_string(),
                reason: "Writes denied".to_string(),
            },
        );
    }

    #[test]
    fn test_incomplete_field_pattern_skipped() {
        let result = apply_rules(
            vec![
                make_rule("bad-field-only", "Write")
                    .with_field("file_path")
                    .with_action(ToolRuleAction::Deny {
                        value: "bad".to_string(),
                    }),
                make_rule("bad-pattern-only", "Write")
                    .with_pattern(r"\.env$")
                    .with_action(ToolRuleAction::Deny {
                        value: "bad".to_string(),
                    }),
                make_rule("fallback", "Write").with_action(ToolRuleAction::Ask),
            ],
            None,
            "Write",
            &serde_json::json!({"file_path": "/home/.env"}),
            "",
        );
        assert_eq!(
            result,
            ToolRuleResult::Asked {
                rule_name: "fallback".to_string()
            }
        );
    }

    #[test]
    fn test_invalid_regex_skipped() {
        let result = apply_rules(
            vec![
                make_rule("bad-regex", "Write")
                    .with_legacy("file_path", "[invalid(")
                    .with_action(ToolRuleAction::Deny {
                        value: "bad".to_string(),
                    }),
                make_rule("fallback", "Write"),
            ],
            None,
            "Write",
            &serde_json::json!({"file_path": "/home/.env"}),
            "",
        );
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
        let rule =
            make_rule("allow-project-read", "Read").with_legacy("file_path", "^{{project}}/");
        let engine = ToolRuleEngine::from_config(vec![rule], Some(fragments));

        assert_eq!(
            engine.apply_rules_sync(
                "Read",
                &serde_json::json!({"file_path": "/home/user/project/src/main.rs"}),
                "",
                None,
            ),
            ToolRuleResult::Allowed {
                rule_name: "allow-project-read".to_string()
            }
        );
        assert_eq!(
            engine.apply_rules_sync(
                "Read",
                &serde_json::json!({"file_path": "/other/path"}),
                "",
                None,
            ),
            ToolRuleResult::NoMatch
        );
    }

    #[test]
    fn test_specific_tool_before_wildcard() {
        let rules = vec![
            make_rule("allow-read", "Read"),
            make_rule("ask-all", "*").with_action(ToolRuleAction::Ask),
        ];
        let engine = ToolRuleEngine::from_config(rules, None);

        assert_eq!(
            engine.apply_rules_sync("Read", &serde_json::json!({}), "", None),
            ToolRuleResult::Allowed {
                rule_name: "allow-read".to_string()
            }
        );
        assert_eq!(
            engine.apply_rules_sync("Write", &serde_json::json!({}), "", None),
            ToolRuleResult::Asked {
                rule_name: "ask-all".to_string()
            }
        );
    }

    #[test]
    fn test_allow_local_matches_path_and_file_path() {
        let (_temp_dir, cwd, existing_file) = lib_rs_project();

        assert!(tool_input_has_local_path(
            &[PathBuf::from("src/lib.rs")],
            &cwd
        ));
        assert!(tool_input_has_local_path(
            std::slice::from_ref(&existing_file),
            &cwd
        ));

        let rule = make_rule("allow-local-read", "Read")
            .local()
            .with_legacy("file_path", r"^src/.*\.rs$");
        assert_rule_allowed(
            &rule,
            "Read",
            serde_json::json!({"file_path": existing_file}),
            cwd.to_str().unwrap(),
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

        let rule = make_rule("allow-generated", "Write")
            .local()
            .with_legacy("file_path", r"^nested/new/.*\.txt$");
        assert_eq!(
            apply_single_rule(
                &rule,
                "Write",
                &serde_json::json!({"file_path": "nested/new/file.txt"}),
                cwd.to_str().unwrap()
            ),
            ToolRuleResult::Allowed {
                rule_name: "allow-generated".to_string()
            }
        );
    }

    #[test]
    fn test_allow_local_rejects_parent_escape() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path();

        for path in ["../outside.txt", "nested/../../outside.txt"] {
            assert_no_local_path(&[PathBuf::from(path)], cwd);
        }
    }

    #[test]
    fn test_allow_local_nonmatching_input_variants() {
        let (_temp_dir, cwd, local_file) = temp_project_with_file("local.txt", "hello\n");
        let cwd_str = cwd.to_str().unwrap();

        let cases = [
            (
                "missing path fields",
                make_rule("allow-local-read", "Read").local(),
                serde_json::json!({"content": "x"}),
            ),
            (
                "non-string path",
                make_rule("allow-any-local", "Read").local(),
                serde_json::json!({"path": 42}),
            ),
            (
                "non-path field",
                make_rule("bad-command-locality", "Read")
                    .local()
                    .with_legacy("command", "^cat"),
                serde_json::json!({"command": "cat local.txt", "path": local_file}),
            ),
        ];

        for (label, rule, input) in cases {
            assert_eq!(
                apply_single_rule(&rule, "Read", &input, cwd_str),
                ToolRuleResult::NoMatch,
                "case {label}"
            );
        }
    }

    #[test]
    fn test_allow_local_requires_both_locality_and_regex() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path();
        fs::create_dir_all(cwd.join("src")).unwrap();
        fs::write(cwd.join("src/lib.rs"), "fn lib() {}\n").unwrap();
        fs::write(cwd.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();

        let rule = make_rule("allow-local-rust-only", "Read")
            .local()
            .with_legacy("path", r"^src/.*\.rs$");
        assert_rule_nomatch(
            &rule,
            "Read",
            serde_json::json!({"path": cwd.join("Cargo.toml")}),
            cwd.to_str().unwrap(),
        );
    }

    #[test]
    fn test_allow_local_still_blocks_matching_non_local_regex() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path();
        let outside_dir = tempfile::tempdir().unwrap();
        let outside_file = outside_dir.path().join("src/lib.rs");
        fs::create_dir_all(outside_file.parent().unwrap()).unwrap();
        fs::write(&outside_file, "fn lib() {}\n").unwrap();

        let rule = make_rule("allow-local-rust-only", "Read")
            .local()
            .with_legacy("path", r"src/lib\.rs$");
        assert_rule_nomatch(
            &rule,
            "Read",
            serde_json::json!({"path": outside_file}),
            cwd.to_str().unwrap(),
        );
    }

    #[test]
    fn test_allow_local_rejects_when_target_field_is_missing() {
        let (_temp_dir, cwd, lib_file) = temp_project_with_file("lib.rs", "fn lib() {}\n");
        let cwd_str = cwd.to_str().unwrap();

        for (field, input) in [
            ("path", serde_json::json!({"file_path": lib_file})),
            ("file_path", serde_json::json!({"path": cwd.join("lib.rs")})),
        ] {
            let rule = make_rule(&format!("allow-local-{field}"), "Read")
                .local()
                .with_legacy(field, ".*");
            assert_rule_nomatch(&rule, "Read", input, cwd_str);
        }
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
    fn test_allow_local_matches_when_either_path_field_is_local() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path();
        let outside_dir = tempfile::tempdir().unwrap();

        assert_has_local_path(
            &[
                outside_dir.path().join("outside.txt"),
                cwd.join("inside.txt"),
            ],
            cwd,
        );
    }

    #[test]
    fn test_allow_local_without_field_matches_local_path() {
        let (_temp_dir, cwd, local_file) = temp_project_with_file("local.txt", "hello\n");
        let cwd_str = cwd.to_str().unwrap();

        let rule = make_rule("allow-any-local", "Read").local();
        let engine = ToolRuleEngine::from_config(vec![rule], None);
        let expected = ToolRuleResult::Allowed {
            rule_name: "allow-any-local".to_string(),
        };

        for input in [
            serde_json::json!({"path": local_file}),
            serde_json::json!({"file_path": cwd.join("local.txt")}),
        ] {
            assert_engine_result(&engine, "Read", input, cwd_str, expected.clone());
        }
    }

    #[test]
    fn test_allow_local_wildcard_variants() {
        let cases = [
            (
                "condition-selected local path",
                make_rule("deny-condition-selected-local", "*")
                    .local()
                    .with_conditions(vec![cond_present("path")])
                    .with_action(ToolRuleAction::Deny {
                        value: "no local ops".to_string(),
                    }),
                vec![("local.txt", "hello\n")],
                "local.txt",
                ToolRuleResult::Denied {
                    rule_name: "deny-condition-selected-local".to_string(),
                    reason: "no local ops".to_string(),
                },
                None,
                ToolRuleResult::NoMatch,
            ),
            (
                "deny local rust paths only",
                make_rule("deny-any-local-rs", "*")
                    .local()
                    .with_legacy("path", r"\.rs$")
                    .with_action(ToolRuleAction::Deny {
                        value: "no rs".to_string(),
                    }),
                vec![
                    ("src/lib.rs", "fn lib() {}\n"),
                    ("Cargo.toml", "[package]\nname = \"x\"\n"),
                ],
                "src/lib.rs",
                ToolRuleResult::Denied {
                    rule_name: "deny-any-local-rs".to_string(),
                    reason: "no rs".to_string(),
                },
                Some("Cargo.toml"),
                ToolRuleResult::NoMatch,
            ),
        ];

        for (
            label,
            rule,
            files,
            matching_path,
            matching_expected,
            nonmatching_path,
            nonmatching_expected,
        ) in cases
        {
            let temp_dir = tempfile::tempdir().unwrap();
            let cwd = temp_dir.path();
            for (relative_path, contents) in files {
                let file_path = cwd.join(relative_path);
                if let Some(parent) = file_path.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::write(file_path, contents).unwrap();
            }
            let engine = ToolRuleEngine::from_config(vec![rule], None);
            let cwd_str = cwd.to_str().unwrap();

            assert_eq!(
                engine.apply_rules_sync(
                    "Edit",
                    &serde_json::json!({"path": cwd.join(matching_path)}),
                    cwd_str,
                    None,
                ),
                matching_expected,
                "case {label}: matching path"
            );
            let nonmatching_input = match nonmatching_path {
                Some(path) => serde_json::json!({"path": cwd.join(path)}),
                None => serde_json::json!({"path": "/outside/local.txt"}),
            };
            assert_eq!(
                engine.apply_rules_sync("Write", &nonmatching_input, cwd_str, None),
                nonmatching_expected,
                "case {label}: nonmatching path"
            );
        }
    }

    #[test]
    fn test_allow_local_falls_through_to_later_rule() {
        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path();

        let result = apply_rules(
            vec![
                make_rule("allow-local-rust", "Write")
                    .local()
                    .with_legacy("path", r"\.rs$"),
                make_rule("deny-all-writes", "Write").with_action(ToolRuleAction::Deny {
                    value: "writes denied".to_string(),
                }),
            ],
            None,
            "Write",
            &serde_json::json!({"path": "/outside/src/lib.rs"}),
            cwd.to_str().unwrap(),
        );
        assert_eq!(
            result,
            ToolRuleResult::Denied {
                rule_name: "deny-all-writes".to_string(),
                reason: "writes denied".to_string()
            }
        );
    }

    #[test]
    fn test_allow_local_denied_variants() {
        let cases = [
            (
                "direct local file",
                "secret.txt",
                "shh\n",
                PathBuf::from("secret.txt"),
            ),
            (
                "path-through-file ancestor",
                "Cargo.toml",
                "[package]\nname = \"x\"\n",
                // `Cargo.toml` is a regular file, not a directory;
                // `canonicalize_allow_missing` still resolves `Cargo.toml/child.txt`
                // as local because `Cargo.toml` exists inside cwd. This ensures
                // Deny rules catch such paths rather than silently passing through.
                PathBuf::from("Cargo.toml/child.txt"),
            ),
        ];

        for (label, seed_path, contents, target) in cases {
            let (_temp_dir, cwd, _) = temp_project_with_file(seed_path, contents);
            let rule =
                make_rule("deny-local-read", "Read")
                    .local()
                    .with_action(ToolRuleAction::Deny {
                        value: "local reads denied".to_string(),
                    });
            assert_eq!(
                apply_single_rule(
                    &rule,
                    "Read",
                    &serde_json::json!({"path": cwd.join(target)}),
                    cwd.to_str().unwrap(),
                ),
                ToolRuleResult::Denied {
                    rule_name: "deny-local-read".to_string(),
                    reason: "local reads denied".to_string(),
                },
                "case {label}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_allow_local_symlink_variants() {
        use std::os::unix::fs::symlink;

        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path().join("project");
        let inside = cwd.join("inside");
        fs::create_dir_all(&inside).unwrap();
        fs::write(inside.join("file.txt"), "data\n").unwrap();
        symlink(&inside, cwd.join("linked-inside")).unwrap();
        assert_has_local_path(&[PathBuf::from("linked-inside/file.txt")], &cwd);

        let temp_dir = tempfile::tempdir().unwrap();
        let cwd = temp_dir.path().join("project");
        let missing = temp_dir.path().join("missing-target");
        fs::create_dir_all(&cwd).unwrap();
        symlink(&missing, cwd.join("broken-link")).unwrap();
        let broken_cases = [
            (
                "broken parent component",
                // `broken-link/file.txt` exercises symlink failure on a non-leaf
                // parent component during canonicalization.
                "broken-link/file.txt",
            ),
            (
                "broken symlink leaf",
                // `broken-link` (with no child) exercises the first canonicalize
                // failure iteration via `symlink_metadata` on the leaf itself.
                "broken-link",
            ),
        ];
        for (label, path) in broken_cases {
            assert!(
                !tool_input_has_local_path(&[PathBuf::from(path)], &cwd),
                "case {label}"
            );
        }
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
    fn test_strip_cwd_prefix_variants() {
        // (value, cwd, expected, case_name)
        let cases: &[(&str, &str, &str, &str)] = &[
            (
                "/home/user/project/src/main.rs",
                "/home/user/project",
                "src/main.rs",
                "basic",
            ),
            (
                "/home/user/project/src/main.rs",
                "/home/user/project/",
                "src/main.rs",
                "trailing_slash_on_cwd",
            ),
            (
                "/home/user/project",
                "/home/user/project",
                "",
                "value_equals_cwd",
            ),
            (
                "/home/user/project",
                "/home/user/project/",
                "",
                "value_equals_cwd_trailing_slash",
            ),
            (
                "/other/path/file.rs",
                "/home/user/project",
                "/other/path/file.rs",
                "no_match",
            ),
            ("/foobar/baz", "/foo", "/foobar/baz", "partial_dir_name"),
            (
                "src/main.rs",
                "/home/user/project",
                "src/main.rs",
                "already_relative",
            ),
            ("/home/user/file.rs", "", "/home/user/file.rs", "empty_cwd"),
            // Root cwd "/" normalizes to empty after trailing-slash strip,
            // so no stripping occurs.
            ("/foo", "/", "/foo", "root_cwd"),
        ];

        for (value, cwd, expected, name) in cases {
            assert_eq!(
                strip_cwd_prefix(value, cwd),
                *expected,
                "case {name}: strip_cwd_prefix({value:?}, {cwd:?})"
            );
        }
    }

    // ===== cwd stripping in apply_rules =====

    #[test]
    fn test_cwd_stripping_matches_relative_pattern() {
        let rule = make_rule("allow-flake", "Read").with_legacy("path", r"^flake\.nix$");
        assert_rule_allowed(
            &rule,
            "Read",
            serde_json::json!({"path": "/tmp/project/flake.nix"}),
            "/tmp/project",
        );
    }

    #[test]
    fn test_cwd_stripping_no_match_different_cwd() {
        let rule = make_rule("allow-flake", "Read").with_legacy("path", r"^flake\.nix$");
        assert_eq!(
            apply_single_rule(
                &rule,
                "Read",
                &serde_json::json!({"path": "/tmp/project/flake.nix"}),
                "/other/dir"
            ),
            ToolRuleResult::NoMatch
        );
    }

    #[test]
    fn test_cwd_stripping_absolute_pattern_still_works_without_cwd() {
        let rule = make_rule("allow-absolute", "Read").with_legacy("path", r"^/tmp/project/");
        assert_rule_allowed(
            &rule,
            "Read",
            serde_json::json!({"path": "/tmp/project/flake.nix"}),
            "",
        );
    }

    #[test]
    fn test_cwd_stripping_subdirectory_path() {
        let rule = make_rule("allow-src", "Write").with_legacy("file_path", r"^src/");
        assert_rule_allowed(
            &rule,
            "Write",
            serde_json::json!({"file_path": "/home/user/project/src/lib.rs"}),
            "/home/user/project",
        );
    }

    #[test]
    fn test_cwd_stripping_trailing_slash_on_cwd() {
        let rule = make_rule("allow-src", "Read").with_legacy("file_path", r"^src/");
        assert_rule_allowed(
            &rule,
            "Read",
            serde_json::json!({"file_path": "/home/user/project/src/main.rs"}),
            "/home/user/project/",
        );
    }

    #[test]
    fn test_compile_with_diagnostics_reports_dropped_rules_and_keeps_good_ones() {
        let incomplete_legacy = make_rule("missing-pattern", "Write")
            .with_field("file_path")
            .with_conditions(vec![cond_present("content")]);
        let rules = vec![
            incomplete_legacy,
            make_rule("bad-regex", "Read").with_legacy("path", "[invalid("),
            make_rule("good", "Read"),
        ];
        let (engine, diagnostics) = ToolRuleEngine::compile_with_diagnostics(rules, None);

        assert_eq!(engine.rules.len(), 1);
        // The valid tool-name-only rule still compiled and is enforced.
        assert_eq!(
            engine.apply_rules_sync("Read", &serde_json::json!({}), "", None),
            ToolRuleResult::Allowed {
                rule_name: "good".to_string()
            }
        );

        let kinds: Vec<RuleDiagnosticKind> = diagnostics.iter().map(|d| d.kind).collect();
        assert_eq!(diagnostics.len(), 2, "diagnostics: {diagnostics:?}");
        assert!(kinds.contains(&RuleDiagnosticKind::MissingFieldOrPattern));
        assert!(kinds.contains(&RuleDiagnosticKind::InvalidRegex));
    }

    #[test]
    fn test_cwd_stripping_value_equals_cwd() {
        // When value equals cwd, stripping produces "" — pattern "^$" matches empty string
        let rule = make_rule("match-empty", "Read").with_legacy("path", r"^$");
        assert_rule_allowed(
            &rule,
            "Read",
            serde_json::json!({"path": "/home/user/project"}),
            "/home/user/project",
        );
    }

    // ===== condition tests =====

    fn cond_present(field: &str) -> ToolRuleCondition {
        ToolRuleCondition::Present {
            field: field.to_string(),
        }
    }

    fn cond_absent(field: &str) -> ToolRuleCondition {
        ToolRuleCondition::Absent {
            field: field.to_string(),
        }
    }

    fn cond_equals(field: &str, value: serde_json::Value) -> ToolRuleCondition {
        ToolRuleCondition::Equals {
            field: field.to_string(),
            value,
        }
    }

    fn cond_matches(field: &str, pattern: &str) -> ToolRuleCondition {
        ToolRuleCondition::Matches {
            field: field.to_string(),
            pattern: pattern.to_string(),
        }
    }

    #[test]
    fn test_conditions_present_and_absent() {
        let rule = make_rule("has-action-no-budget", "subagent")
            .with_conditions(vec![cond_present("action"), cond_absent("turnBudget")]);
        assert_allow_cases(
            rule,
            "subagent",
            "",
            &[serde_json::json!({"action": null, "async": true})],
            &[
                serde_json::json!({"action": null, "turnBudget": null}),
                serde_json::json!({"async": true}),
                serde_json::json!({"turnBudget": 5}),
            ],
        );
    }

    #[test]
    fn test_conditions_equals_type_sensitive() {
        let rule = make_rule("async-true", "subagent")
            .with_conditions(vec![cond_equals("async", serde_json::json!(true))]);
        assert_allow_cases(
            rule,
            "subagent",
            "",
            &[serde_json::json!({"async": true})],
            &[
                serde_json::json!({"async": "true"}),
                serde_json::json!({"async": false}),
                serde_json::json!({"async": null}),
                serde_json::json!({}),
            ],
        );
    }

    #[test]
    fn test_conditions_equals_array_and_object() {
        let settings = serde_json::json!({"items": ["one", 2, false], "mode": "safe"});
        let rule = make_rule("exact-settings", "Test")
            .with_conditions(vec![cond_equals("settings", settings.clone())]);
        assert_allow_cases(
            rule,
            "Test",
            "",
            &[serde_json::json!({"settings": settings})],
            &[serde_json::json!({"settings": {"items": ["one", 2, true], "mode": "safe"}})],
        );
    }

    #[test]
    fn test_conditions_matches_scalar_and_strips_cwd() {
        let rule = make_rule("scalar-42", "Test")
            .with_conditions(vec![cond_matches("value", r"^(worker-\d+|42|nested)$")]);
        assert_allow_cases(
            rule,
            "Test",
            "/tmp/project",
            &[
                serde_json::json!({"value": "worker-123"}),
                serde_json::json!({"value": 42}),
                serde_json::json!({"value": "/tmp/project/nested"}),
            ],
            &[
                serde_json::json!({"value": "builder-1"}),
                serde_json::json!({"value": [42]}),
                serde_json::json!({"value": null}),
                serde_json::json!({}),
            ],
        );
    }

    #[test]
    fn test_conditions_multiple_matches_expand_fragments_and_conjoin() {
        let fragments = Some(HashMap::from([
            ("worker".to_string(), "worker-[0-9]+".to_string()),
            ("start".to_string(), "(start|resume)".to_string()),
        ]));
        let rule = make_rule("worker-start", "subagent").with_conditions(vec![
            cond_matches("agent", "^{{worker}}$"),
            cond_matches("mode", "^{{start}}$"),
        ]);
        let engine = ToolRuleEngine::from_config(vec![rule], fragments);
        let expected = ToolRuleResult::Allowed {
            rule_name: "worker-start".to_string(),
        };

        assert_engine_result(
            &engine,
            "subagent",
            serde_json::json!({"agent": "worker-42", "mode": "start"}),
            "",
            expected,
        );
        assert_engine_result(
            &engine,
            "subagent",
            serde_json::json!({"agent": "worker-42", "mode": "status"}),
            "",
            ToolRuleResult::NoMatch,
        );
    }

    #[test]
    fn test_conditions_plus_legacy_field() {
        let rule = make_rule("cond-plus-legacy", "Write")
            .with_legacy("file_path", r"\.rs$")
            .with_conditions(vec![cond_present("content")]);
        assert_allow_cases(
            rule,
            "Write",
            "",
            &[serde_json::json!({"file_path": "main.rs", "content": "fn main() {}"})],
            &[
                serde_json::json!({"file_path": "main.rs"}),
                serde_json::json!({"file_path": "data.txt", "content": "x"}),
            ],
        );
    }

    #[test]
    fn test_conditions_treat_dotted_fields_as_literal_keys() {
        let rule =
            make_rule("no-literal-dotted-key", "Test").with_conditions(vec![cond_absent("a.b")]);
        assert_allow_cases(
            rule,
            "Test",
            "",
            &[serde_json::json!({"a": {"b": 1}})],
            &[serde_json::json!({"a.b": 1})],
        );
    }

    #[test]
    fn test_conditions_non_object_input_cannot_satisfy_absent() {
        let rule = make_rule("action-absent", "Test").with_conditions(vec![cond_absent("action")]);
        assert_allow_cases(
            rule,
            "Test",
            "",
            &[serde_json::json!({})],
            &[
                serde_json::json!("not an object"),
                serde_json::json!(42),
                serde_json::json!([1, 2, 3]),
                serde_json::json!(null),
            ],
        );
    }

    #[test]
    fn test_conditions_bad_patterns_drop_whole_rules() {
        let rules = vec![
            make_rule("undefined-fragment", "Test").with_conditions(vec![
                cond_matches("name", "^ok$"),
                cond_matches("path", "^{{nope}}"),
            ]),
            make_rule("invalid-regex", "Test")
                .with_conditions(vec![cond_matches("name", "[invalid(")]),
            make_rule("fallback", "Test").with_action(ToolRuleAction::Ask),
        ];
        let (engine, diagnostics) = ToolRuleEngine::compile_with_diagnostics(rules, None);

        assert_eq!(engine.rules.len(), 1);
        assert_eq!(engine.rules[0].name, "fallback");
        assert_eq!(diagnostics[0].pattern, r"^{{nope}}");
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.kind)
                .collect::<Vec<_>>(),
            vec![
                RuleDiagnosticKind::UndefinedFragment,
                RuleDiagnosticKind::InvalidRegex,
            ]
        );
        assert_engine_result(
            &engine,
            "Test",
            serde_json::json!({"name": "ok"}),
            "",
            ToolRuleResult::Asked {
                rule_name: "fallback".to_string(),
            },
        );
    }

    // ===== condition + allow_local tests =====

    #[test]
    fn test_non_local_rule_uses_raw_path_when_another_rule_needs_locality() {
        // The engine evaluates locality once per tool call. That cache must not make a later
        // non-local rule canonicalize its regex input merely because an earlier rule needs it.
        let temp_dir = tempfile::tempdir().unwrap();
        let local_rule = make_rule("ask-local", "Read")
            .local()
            .with_conditions(vec![cond_present("path")])
            .with_action(ToolRuleAction::Ask);
        let raw_path_rule = make_rule("allow-external-prefix", "Read")
            .with_conditions(vec![cond_matches("path", r"^/outside/")]);
        let engine = ToolRuleEngine::from_config(vec![local_rule, raw_path_rule], None);

        assert_engine_result(
            &engine,
            "Read",
            serde_json::json!({"path": "/outside/file.txt"}),
            temp_dir.path().to_str().unwrap(),
            ToolRuleResult::Allowed {
                rule_name: "allow-external-prefix".to_string(),
            },
        );
    }

    #[test]
    fn test_conditions_allow_local_multiple_path_fields() {
        // When conditions reference both path and file_path, both must be local.
        let (_temp_dir, cwd, lib_file) = lib_rs_project();
        fs::write(cwd.join("Cargo.toml"), "[package]\n").unwrap();
        let cwd_str = cwd.to_str().unwrap();

        let rule = local_allow(
            "both-paths-local",
            vec![cond_present("path"), cond_present("file_path")],
        );

        assert_rule_allowed(
            &rule,
            "Read",
            serde_json::json!({"path": lib_file, "file_path": cwd.join("Cargo.toml")}),
            cwd_str,
        );

        for input in [
            serde_json::json!({"path": "/outside/file.txt", "file_path": cwd.join("Cargo.toml")}),
            serde_json::json!({"file_path": cwd.join("Cargo.toml")}),
        ] {
            assert_rule_nomatch(&rule, "Read", input, cwd_str);
        }
    }

    #[test]
    fn test_conditions_allow_local_without_selected_paths_uses_fallback() {
        let (_temp_dir, cwd, local_file) = temp_project_with_file("local.txt", "hello\n");
        let rule = local_allow(
            "local-cat",
            vec![cond_absent("path"), cond_matches("command", "^cat")],
        );

        assert_rule_allowed(
            &rule,
            "Read",
            serde_json::json!({"file_path": local_file, "command": "cat local.txt"}),
            cwd.to_str().unwrap(),
        );
    }

    #[test]
    fn test_conditions_allow_local_equals_raw() {
        // Equals stays raw even for paths — locality is a separate gate.
        let (_temp_dir, cwd, lib_file) = lib_rs_project();
        let cwd_str = cwd.to_str().unwrap();

        let equals_path = |name: &str, path: &Path| {
            local_allow(name, vec![cond_equals("path", serde_json::json!(path))])
        };
        let exact_rule = equals_path("path-equals-exact", &lib_file);
        assert_rule_allowed(
            &exact_rule,
            "Read",
            serde_json::json!({"path": lib_file}),
            cwd_str,
        );

        let (_outside_dir, outside_file) = temp_outside_file("outside.txt", "outside\n");
        let external_rule = equals_path("external-path-equals", &outside_file);
        assert_rule_nomatch(
            &external_rule,
            "Read",
            serde_json::json!({"path": outside_file}),
            cwd_str,
        );
    }

    #[test]
    fn test_conditions_allow_local_matches_path_fields_use_canonical_paths() {
        // Under allow_local, Matches uses the canonicalized local path before cwd-stripping.
        let (_temp_dir, cwd, _lib_file) = lib_rs_project();
        let cwd_str = cwd.to_str().unwrap();
        let path_rule = local_allow(
            "local-src-only",
            vec![cond_matches("path", r"^src/lib\.rs$")],
        );
        let file_path_rule = local_allow(
            "local-file-src-only",
            vec![cond_matches("file_path", r"^src/lib\.rs$")],
        );

        assert_rule_allowed(
            &path_rule,
            "Read",
            serde_json::json!({"path": cwd.join("src/lib.rs")}),
            cwd_str,
        );
        assert_rule_allowed(
            &file_path_rule,
            "Read",
            serde_json::json!({"file_path": cwd.join("src/lib.rs")}),
            cwd_str,
        );

        let (_outside_dir, outside_file) = temp_outside_file("src/lib.rs", "fn lib() {}\n");
        assert_rule_nomatch(
            &path_rule,
            "Read",
            serde_json::json!({"path": outside_file}),
            cwd_str,
        );
    }

    #[test]
    fn test_conditions_allow_local_legacy_nonpath_without_selected_path_fails() {
        let (_temp_dir, cwd, lib_file) = lib_rs_project();
        let rule = make_rule("legacy-nonpath", "Read")
            .local()
            .with_legacy("command", r"^cat\s+")
            .with_conditions(vec![cond_present("content")]);

        assert_rule_nomatch(
            &rule,
            "Read",
            serde_json::json!({
                "command": "cat src/lib.rs",
                "content": "requested",
                "path": lib_file,
            }),
            cwd.to_str().unwrap(),
        );
    }

    #[test]
    fn test_conditions_allow_local_legacy_and_condition_paths_must_be_local() {
        let (_temp_dir, cwd, lib_file) = lib_rs_project();
        let main_file = cwd.join("src/main.rs");
        fs::write(&main_file, "fn main() {}\n").unwrap();
        let rule = make_rule("both-path-sources", "Read")
            .local()
            .with_legacy("file_path", r"\.rs$")
            .with_conditions(vec![cond_present("path")]);
        let cwd_str = cwd.to_str().unwrap();

        assert_rule_allowed(
            &rule,
            "Read",
            serde_json::json!({"path": lib_file.clone(), "file_path": main_file.clone()}),
            cwd_str,
        );
        for input in [
            serde_json::json!({"path": "/outside/src/lib.rs", "file_path": main_file}),
            serde_json::json!({"path": lib_file, "file_path": "/outside/src/main.rs"}),
        ] {
            assert_rule_nomatch(&rule, "Read", input, cwd_str);
        }
    }

    #[test]
    fn test_conditions_allow_local_path_condition_with_legacy_nonpath_regex() {
        // A condition-selected path supplies locality, so a legacy regex on a different field
        // must still read that non-path field raw instead of trying to resolve it as a path.
        let (_temp_dir, cwd, lib_file) = lib_rs_project();
        let rule = make_rule("local-cat", "Read")
            .local()
            .with_legacy("command", r"^cat\s+")
            .with_conditions(vec![cond_present("path")]);

        assert_rule_allowed(
            &rule,
            "Read",
            serde_json::json!({"path": lib_file, "command": "cat src/lib.rs"}),
            cwd.to_str().unwrap(),
        );
        assert_rule_nomatch(
            &rule,
            "Read",
            serde_json::json!({"path": "/outside/file.txt", "command": "cat /outside/file.txt"}),
            cwd.to_str().unwrap(),
        );
    }
}
