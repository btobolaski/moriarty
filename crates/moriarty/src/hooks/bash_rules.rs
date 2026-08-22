//! Bash command validation and modification rules.
//!
//! This module provides a rule engine for validating and modifying Bash tool use commands
//! before they are executed by Claude Code. Rules can deny dangerous commands, modify
//! commands to add safety flags, or explicitly allow specific patterns.

use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
    sync::Arc,
};

use miette::miette;
use regex::{Regex, RegexSet};
use serde::Serialize;
use tokio::{
    task::{JoinError, spawn_blocking},
    time::timeout,
};
use tracing::debug;

use super::{
    FILESYSTEM_EVALUATION_TIMEOUT,
    command_split::{
        AliasBinding, BailReason, FilterCommand, LeafCommand, ParsedRedirectTarget,
        RedirectEndpoint, SplitOutcome, split_command,
    },
    path_resolution::{RedirectResolutionContext, RedirectTargetResolution},
};
use crate::{
    permission_mode::{PermissionMode, is_mode_eligible},
    user_config::{
        BashPathAlias, BashRule, BashRuleAction, RedirectDirection, RedirectRuleAction, UserConfig,
    },
};

#[derive(Debug)]
struct CompiledCommandRules {
    regex_set: RegexSet,
    rules: Vec<CompiledCommandRule>,
}

#[derive(Debug)]
struct CompiledCommandRule {
    metadata: Arc<MatchedRuleMetadata>,
    regex: Regex,
    modes: Option<BTreeSet<PermissionMode>>,
    action: CommandAction,
}

#[derive(Debug)]
enum CommandAction {
    Allow,
    Deny(String),
    Modify(String),
    Ask,
    ArgumentFilter {
        remove: Option<Vec<String>>,
        add: Option<Vec<String>>,
        replace: Option<HashMap<String, String>>,
        reason: Option<String>,
    },
}

#[derive(Debug)]
struct CompiledRedirectRules {
    regex_set: RegexSet,
    rules: Vec<CompiledRedirectRule>,
}

#[derive(Debug)]
struct CompiledRedirectRule {
    metadata: Arc<MatchedRuleMetadata>,
    regex: Regex,
    modes: Option<BTreeSet<PermissionMode>>,
    action: Arc<RedirectRuleAction>,
}

#[derive(Clone, Copy)]
struct RedirectRewrite<'a> {
    bindings: &'a [AliasBinding],
    suffix: &'a str,
    count: usize,
}

impl<'a> RedirectRewrite<'a> {
    const NONE: Self = Self {
        bindings: &[],
        suffix: "",
        count: 0,
    };

    fn from_leaf(leaf: &'a LeafCommand) -> Self {
        Self {
            bindings: &leaf.redirect_bindings,
            suffix: &leaf.redirect_suffix,
            count: leaf.redirects.len(),
        }
    }
}

/// Includes `rule_name` in all match variants to support logging and debugging.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

/// Command and redirect rules use independent first-match-wins domains so neither policy can
/// authorize or shadow the other.
#[derive(Debug)]
pub struct BashRuleEngine {
    command_rules: CompiledCommandRules,
    redirect_rules: CompiledRedirectRules,
    path_aliases: BTreeSet<BashPathAlias>,
}

/// A reason a rule was dropped at compile time. Surfaced by `compile_with_diagnostics` so the
/// `rules lint` command can report rules the hook silently ignores; `from_config` logs them and
/// keeps the original fail-open-per-rule behavior on the hook hot path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuleDiagnostic {
    pub rule_name: String,
    pub pattern: String,
    pub kind: RuleDiagnosticKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleDiagnosticKind {
    /// A `{{fragment}}` reference had no definition.
    UndefinedFragment,
    /// Fragments referenced each other in a cycle.
    CircularFragment,
    /// Fragment expansion exceeded the maximum nesting depth.
    FragmentDepthExceeded,
    /// Fragment expansion performed too many total substitutions. Distinct from
    /// `FragmentDepthExceeded` because breadth blows up within the depth limit: a fragment
    /// referencing several others multiplies at every level of an acyclic graph.
    FragmentExpansionLimitExceeded,
    /// The expanded pattern was not a valid regex.
    InvalidRegex,
    /// A tool rule had only one of `field`/`pattern` (both are required together). Tool rules only.
    MissingFieldOrPattern,
}

impl RuleDiagnosticKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::UndefinedFragment => "undefined-fragment",
            Self::CircularFragment => "circular-fragment",
            Self::FragmentDepthExceeded => "fragment-depth-exceeded",
            Self::FragmentExpansionLimitExceeded => "fragment-expansion-limit-exceeded",
            Self::InvalidRegex => "invalid-regex",
            Self::MissingFieldOrPattern => "missing-field-or-pattern",
        }
    }
}

/// Classifies an [`expand_fragments`] error from its message text. Message-matching is the only
/// signal available because `expand_fragments` returns an opaque `miette::Report`.
pub(crate) fn classify_fragment_error(message: &str) -> RuleDiagnosticKind {
    if message.contains("Circular dependency") {
        RuleDiagnosticKind::CircularFragment
    } else if message.contains("exceeded maximum depth") {
        RuleDiagnosticKind::FragmentDepthExceeded
    } else if message.contains("exceeded maximum expansion count") {
        RuleDiagnosticKind::FragmentExpansionLimitExceeded
    } else {
        // The remaining `expand_fragments` failure is the undefined-fragment case.
        RuleDiagnosticKind::UndefinedFragment
    }
}

/// Expands {{fragment_name}} references in a pattern string.
///
/// Supports nested fragments (fragments referencing other fragments) by expanding each
/// reference depth-first. Cycle detection tracks the chain of fragments currently being
/// expanded rather than every fragment ever expanded, because the same fragment legitimately
/// appears at several points in a directed acyclic graph — a pattern using both `{{safe_arg}}`
/// and `{{safe_chars}}` when `safe_arg` itself references `safe_chars` is not a cycle.
///
/// # Arguments
/// * `pattern` - The pattern string potentially containing {{fragment}} references
/// * `fragments` - Map of fragment names to their regex values
///
/// # Errors
/// * Returns error if a referenced fragment doesn't exist in the map
/// * Returns error if circular dependencies are detected (fragments referencing each other)
/// * Returns error if nested expansion exceeds MAX_DEPTH (10 levels)
/// * Returns error if total substitutions exceed MAX_EXPANSIONS (256)
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
) -> miette::Result<String> {
    let fragment_pattern =
        Regex::new(r"\{\{([a-zA-Z_][a-zA-Z0-9_-]*)\}\}").expect("Fragment regex pattern is valid");

    FragmentExpander {
        root_pattern: pattern,
        fragments,
        fragment_pattern: &fragment_pattern,
        active: Vec::new(),
        expansions: 0,
    }
    .expand(pattern)
}

/// Depth-first expansion state for one [`expand_fragments`] call.
struct FragmentExpander<'a> {
    /// Reported in the undefined-fragment error so it names the pattern the user wrote rather than
    /// the fragment body the dangling reference happened to sit in.
    root_pattern: &'a str,
    fragments: &'a HashMap<String, String>,
    fragment_pattern: &'a Regex,
    /// The chain of fragments whose bodies are currently being expanded, innermost last. A
    /// reference to a name already on this chain is the only true cycle; the same fragment reached
    /// by two different paths is an acyclic graph and expands normally.
    active: Vec<String>,
    /// Counted across the whole expansion rather than per branch, because it bounds the size of the
    /// result and sibling references multiply.
    expansions: usize,
}

impl FragmentExpander<'_> {
    /// Maximum nesting depth chosen to allow reasonable fragment composition
    /// (e.g., safe_chars -> safe_arg -> safe_pipe) while preventing
    /// resource exhaustion from deeply nested or circular references.
    const MAX_DEPTH: usize = 10;
    /// Depth alone does not bound the expanded pattern: a fragment referencing several others
    /// multiplies at every level, so an acyclic graph well inside [`Self::MAX_DEPTH`] can still
    /// expand into an enormous regex. Set generously so no plausible hand-written set trips it.
    const MAX_EXPANSIONS: usize = 256;

    fn expand(&mut self, text: &str) -> miette::Result<String> {
        if self.active.len() > Self::MAX_DEPTH {
            return Err(miette!(
                "Pattern fragment expansion exceeded maximum depth of {}. \
                 This likely indicates overly deep nesting.",
                Self::MAX_DEPTH
            ));
        }

        // Copied out of `self` so the borrows below are independent of the `&mut self` the
        // recursive call needs.
        let fragments = self.fragments;
        let fragment_pattern = self.fragment_pattern;

        let mut result = String::new();
        let mut last_end = 0;

        for cap in fragment_pattern.captures_iter(text) {
            let full_match = cap.get(0).unwrap();
            let fragment_name = &cap[1];

            let fragment_value = fragments.get(fragment_name).ok_or_else(|| {
                miette!(
                    "Undefined pattern fragment '{}' referenced in pattern: {}",
                    fragment_name,
                    self.root_pattern
                )
            })?;

            if self.active.iter().any(|name| name == fragment_name) {
                return Err(miette!(
                    "Circular dependency detected in pattern fragments: '{}' references itself through other fragments",
                    fragment_name
                ));
            }

            self.expansions += 1;
            if self.expansions > Self::MAX_EXPANSIONS {
                return Err(miette!(
                    "Pattern fragment expansion exceeded maximum expansion count of {}. \
                     This likely indicates fragments that multiply across many references.",
                    Self::MAX_EXPANSIONS
                ));
            }

            result.push_str(&text[last_end..full_match.start()]);

            self.active.push(fragment_name.to_string());
            let expanded = self.expand(fragment_value);
            self.active.pop();
            result.push_str(&expanded?);

            last_end = full_match.end();
        }

        result.push_str(&text[last_end..]);
        Ok(result)
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct RuleMatchExplanation {
    pub rule_name: String,
    /// The pattern after `{{fragment}}` expansion (what the regex engine actually compiled).
    pub expanded_pattern: String,
    pub action_summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RedirectTraceDecision {
    Allowed,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum RedirectTraceState {
    Matched {
        matched: RuleMatchExplanation,
        decision: RedirectTraceDecision,
    },
    Failed {
        failure: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct RedirectEndpointTrace {
    pub original_target: String,
    pub direction: RedirectDirection,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_local: Option<bool>,
    #[serde(flatten)]
    pub state: RedirectTraceState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SubCommandTrace {
    pub original: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias_expanded: Option<String>,
    pub normalized: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<AliasBinding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_confirmation: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub redirects: Vec<RedirectEndpointTrace>,
    pub matched: Option<RuleMatchExplanation>,
}

/// Keeps rewritten text paired with its reason after a fail-closed `Asked` result loses both.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct RewrittenBailTrace {
    pub command: String,
    pub reason: BailReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redirect: Option<RedirectEndpointTrace>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CommandTrace {
    pub original: String,
    /// Bindings that were consumed as analysis metadata rather than executable leaves.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<AliasBinding>,
    /// Per-leaf evaluation in execution order; empty when `bail` is set.
    pub sub_commands: Vec<SubCommandTrace>,
    /// Kept separately so explain can show policy applied after a Modify rewrite or ArgumentFilter recheck.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rewritten_sub_commands: Vec<SubCommandTrace>,
    /// Preserves diagnostics when an unanalyzable rewrite is downgraded to confirmation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rewritten_bail: Option<RewrittenBailTrace>,
    /// Set when the command could not be analyzed and fell back to whole-command evaluation.
    pub bail: Option<BailReason>,
    pub final_result: RuleResult,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub contributors: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct MatchedRuleMetadata {
    rule_name: String,
    expanded_pattern: String,
    action_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MatchedCommandRule {
    metadata: Arc<MatchedRuleMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MatchedRedirectRule {
    metadata: Arc<MatchedRuleMetadata>,
    action: Arc<RedirectRuleAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CommandDecision {
    NoMatch,
    Allow {
        rule: MatchedCommandRule,
    },
    Deny {
        rule: MatchedCommandRule,
        reason: String,
    },
    Modify {
        rule: MatchedCommandRule,
        new_command: String,
    },
    Ask {
        rule: MatchedCommandRule,
    },
    ArgumentFilter {
        rule: MatchedCommandRule,
        new_command: String,
        reason: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EndpointFailureStage {
    StaticAnalysis,
    RuntimeResolution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedRedirectTarget {
    match_text: String,
    is_local: bool,
    kind: RedirectEndpointKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FilesystemAnalysis {
    Resolved {
        target: ResolvedRedirectTarget,
        matched_rule: Option<MatchedRedirectRule>,
    },
    Failed {
        stage: EndpointFailureStage,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EndpointAnalysis {
    Descriptor {
        original_target: String,
        direction: RedirectDirection,
        match_text: String,
        matched_rule: Option<MatchedRedirectRule>,
    },
    Filesystem {
        original_target: String,
        direction: RedirectDirection,
        resolution: FilesystemAnalysis,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RedirectAuthorizationView<'a> {
    Authorized(&'a MatchedRedirectRule),
    Denied(&'a MatchedRedirectRule, &'a str),
    Unmatched,
    Unresolvable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeniedRedirectEndpoint {
    endpoint: EndpointAnalysis,
    rule: MatchedRedirectRule,
    reason: String,
}

impl DeniedRedirectEndpoint {
    fn from_denied(endpoint: EndpointAnalysis) -> Option<Self> {
        let (rule, reason) = endpoint.denial()?;
        let rule = rule.clone();
        let reason = reason.to_string();
        Some(Self {
            endpoint,
            rule,
            reason,
        })
    }

    pub(crate) fn endpoint(&self) -> &EndpointAnalysis {
        &self.endpoint
    }

    fn denial(&self) -> (&MatchedRedirectRule, &str) {
        (&self.rule, &self.reason)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LeafIdentity {
    original: String,
    alias_expanded: Option<String>,
    normalized: String,
    bindings: Vec<AliasBinding>,
    requires_confirmation: Option<String>,
    command_shape: Option<FilterCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EndpointCoverage {
    Skipped,
    Analyzed(Vec<EndpointAnalysis>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PolicyLeafAnalysis {
    identity: LeafIdentity,
    command: CommandDecision,
    endpoints: EndpointCoverage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RedirectLeafAnalysis {
    identity: LeafIdentity,
    endpoints: Vec<EndpointAnalysis>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BailDecision {
    Deny {
        rule: MatchedCommandRule,
        reason: String,
    },
    RedirectDeny {
        rule: MatchedRedirectRule,
        reason: String,
    },
    NoMatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PolicyAnalysis {
    Bail {
        command: String,
        reason: BailReason,
        decision: BailDecision,
    },
    Leaves(Vec<PolicyLeafAnalysis>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RedirectCheckAnalysis {
    Bail {
        command: String,
        reason: BailReason,
        denied_endpoint: Option<DeniedRedirectEndpoint>,
    },
    Leaves(Vec<RedirectLeafAnalysis>),
}

/// `evaluate_sync` constructs this check only when the recheck outcome is `Modify`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FilterContinuation {
    None,
    ModifyRedirectCheck(RedirectCheckAnalysis),
}

/// Each non-`None` variant is valid only beside the original source outcome that names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OriginalContinuation {
    None,
    ModifyRedirectCheck(RedirectCheckAnalysis),
    ArgumentFilterRecheck {
        recheck: PolicyAnalysis,
        continuation: FilterContinuation,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Evaluation {
    original: PolicyAnalysis,
    continuation: OriginalContinuation,
}

#[derive(Debug)]
pub(crate) struct EvaluationContext {
    cwd: String,
    mode: Option<PermissionMode>,
    resolution: RedirectResolutionContext,
}

impl EvaluationContext {
    pub(crate) fn new(cwd: &str, mode: Option<PermissionMode>) -> Self {
        let home = std::env::var_os("HOME");
        Self {
            cwd: cwd.to_string(),
            mode,
            resolution: RedirectResolutionContext::new(
                Path::new(cwd),
                home.as_deref().map(Path::new),
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EvaluationPurpose {
    Decision,
    Diagnostics,
}

#[derive(Debug)]
pub(crate) enum LiveEvaluationFailure {
    Timeout,
    Join(JoinError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PolicyOutcome<'a> {
    Allow {
        rule: &'a MatchedCommandRule,
    },
    Deny {
        rule: &'a MatchedCommandRule,
        reason: &'a str,
    },
    RedirectDeny {
        rule: &'a MatchedRedirectRule,
        reason: &'a str,
    },
    Modify {
        rule: &'a MatchedCommandRule,
        new_command: &'a str,
    },
    Ask {
        rule: &'a MatchedCommandRule,
    },
    ArgumentFilter {
        rule: &'a MatchedCommandRule,
        new_command: &'a str,
        reason: Option<&'a str>,
    },
    NoMatch,
}

impl CommandAction {
    fn summary(&self) -> String {
        match self {
            Self::Allow => "Allow".to_string(),
            Self::Deny(reason) => format!("Deny: {reason}"),
            Self::Modify(command) => format!("Modify → {command}"),
            Self::Ask => "Ask".to_string(),
            Self::ArgumentFilter { reason, .. } => match reason {
                Some(reason) => format!("ArgumentFilter ({reason})"),
                None => "ArgumentFilter".to_string(),
            },
        }
    }
}

fn redirect_action_summary(action: &RedirectRuleAction) -> String {
    match action {
        RedirectRuleAction::Allow {
            allow_local,
            direction,
        } => {
            let mut qualifiers = Vec::new();
            if *direction != RedirectDirection::Output {
                qualifiers.push(direction.as_str());
            }
            if *allow_local {
                qualifiers.push("local only");
            }
            if qualifiers.is_empty() {
                "AllowRedirect".to_string()
            } else {
                format!("AllowRedirect ({})", qualifiers.join(", "))
            }
        }
        RedirectRuleAction::Deny { value, direction } if *direction == RedirectDirection::Both => {
            format!("DenyRedirect: {value}")
        }
        RedirectRuleAction::Deny { value, direction } => {
            format!("DenyRedirect ({}): {value}", direction.as_str())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RedirectEndpointKind {
    Descriptor,
    Filesystem,
    DeviceOrSpecial,
}

impl RedirectEndpointKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Descriptor => "descriptor",
            Self::Filesystem => "filesystem",
            Self::DeviceOrSpecial => "device or special file",
        }
    }

    pub(crate) fn is_filesystem(self) -> bool {
        matches!(self, Self::Filesystem)
    }
}

impl CompiledCommandRules {
    fn first_match(
        &self,
        text: &str,
        mode: Option<PermissionMode>,
    ) -> Option<&CompiledCommandRule> {
        self.regex_set.matches(text).iter().find_map(|index| {
            let rule = &self.rules[index];
            debug_assert!(rule.regex.is_match(text));
            is_mode_eligible(rule.modes.as_ref(), mode).then_some(rule)
        })
    }
}

impl CompiledRedirectRules {
    fn deny_coverage(&self, mode: Option<PermissionMode>) -> BTreeSet<RedirectDirection> {
        let mut coverage = BTreeSet::new();
        for rule in &self.rules {
            if is_mode_eligible(rule.modes.as_ref(), mode)
                && let Some(direction) = rule.action.deny_direction()
            {
                coverage.extend(
                    RedirectDirection::ALL
                        .into_iter()
                        .filter(|endpoint| direction.overlaps(*endpoint)),
                );
            }
        }
        coverage
    }

    fn first_match(
        &self,
        text: &str,
        mode: Option<PermissionMode>,
        endpoint_direction: RedirectDirection,
        is_filesystem: bool,
        is_local: bool,
    ) -> Option<&CompiledRedirectRule> {
        self.regex_set.matches(text).iter().find_map(|index| {
            let rule = &self.rules[index];
            debug_assert!(rule.regex.is_match(text));
            (is_mode_eligible(rule.modes.as_ref(), mode)
                && rule
                    .action
                    .matches(endpoint_direction, is_filesystem, is_local))
            .then_some(rule)
        })
    }
}

impl BashRuleEngine {
    /// Compiles rules with pattern fragment expansion, logging and skipping any rule that fails to
    /// expand or compile (fail-open per rule, preserving the hook hot path's behavior).
    pub fn from_config(config: UserConfig) -> miette::Result<Self> {
        let (mut engine, diagnostics) = Self::compile_with_diagnostics(
            config.bash_rules.unwrap_or_default(),
            config.pattern_fragments,
        )?;
        engine.path_aliases = config.bash_path_aliases;
        for diagnostic in &diagnostics {
            tracing::error!(
                rule_name = %diagnostic.rule_name,
                pattern = %diagnostic.pattern,
                error = %diagnostic.message,
                "Skipping bash rule the hook cannot compile"
            );
        }
        Ok(engine)
    }

    /// Compiles rules, returning the engine alongside a diagnostic for every rule that was dropped.
    ///
    /// Unlike [`Self::from_config`], this does not log; the caller decides how to surface dropped
    /// rules (the hook logs them; `rules lint` reports them as errors).
    pub(crate) fn compile_with_diagnostics(
        rules: Vec<BashRule>,
        user_fragments: Option<HashMap<String, String>>,
    ) -> miette::Result<(Self, Vec<RuleDiagnostic>)> {
        let mut fragments = default_fragments();
        if let Some(user_fragments) = user_fragments {
            fragments.extend(user_fragments);
        }

        let mut command_patterns = Vec::new();
        let mut command_rules = Vec::new();
        let mut redirect_patterns = Vec::new();
        let mut redirect_rules = Vec::new();
        let mut diagnostics = Vec::new();

        for rule in rules {
            let expanded_pattern = match expand_fragments(&rule.pattern, &fragments) {
                Ok(pattern) => pattern,
                Err(error) => {
                    let message = error.to_string();
                    diagnostics.push(RuleDiagnostic {
                        kind: classify_fragment_error(&message),
                        rule_name: rule.name,
                        pattern: rule.pattern,
                        message,
                    });
                    continue;
                }
            };
            let regex = match Regex::new(&expanded_pattern) {
                Ok(regex) => regex,
                Err(error) => {
                    diagnostics.push(RuleDiagnostic {
                        rule_name: rule.name,
                        pattern: rule.pattern,
                        kind: RuleDiagnosticKind::InvalidRegex,
                        message: error.to_string(),
                    });
                    continue;
                }
            };

            if let Some(action) = RedirectRuleAction::from_config(&rule.action) {
                redirect_patterns.push(expanded_pattern.clone());
                redirect_rules.push(CompiledRedirectRule {
                    metadata: Arc::new(MatchedRuleMetadata {
                        rule_name: rule.name,
                        expanded_pattern,
                        action_summary: redirect_action_summary(&action),
                    }),
                    regex,
                    modes: rule.modes,
                    action: Arc::new(action),
                });
                continue;
            }

            let action = match rule.action {
                BashRuleAction::Allow => CommandAction::Allow,
                BashRuleAction::Deny { value } => CommandAction::Deny(value),
                BashRuleAction::Modify { value } => CommandAction::Modify(value),
                BashRuleAction::Ask => CommandAction::Ask,
                BashRuleAction::ArgumentFilter {
                    remove,
                    add,
                    replace,
                    reason,
                } => CommandAction::ArgumentFilter {
                    remove,
                    add,
                    replace,
                    reason,
                },
                BashRuleAction::AllowRedirect { .. } | BashRuleAction::DenyRedirect { .. } => {
                    continue;
                }
            };
            command_patterns.push(expanded_pattern.clone());
            command_rules.push(CompiledCommandRule {
                metadata: Arc::new(MatchedRuleMetadata {
                    rule_name: rule.name,
                    expanded_pattern,
                    action_summary: action.summary(),
                }),
                regex,
                modes: rule.modes,
                action,
            });
        }

        Ok((
            Self {
                command_rules: CompiledCommandRules {
                    regex_set: RegexSet::new(command_patterns).map_err(|error| {
                        miette!("Failed to build command RegexSet from patterns: {error}")
                    })?,
                    rules: command_rules,
                },
                redirect_rules: CompiledRedirectRules {
                    regex_set: RegexSet::new(redirect_patterns).map_err(|error| {
                        miette!("Failed to build redirect RegexSet from patterns: {error}")
                    })?,
                    rules: redirect_rules,
                },
                path_aliases: BTreeSet::new(),
            },
            diagnostics,
        ))
    }

    fn matched_command_rule(rule: &CompiledCommandRule) -> MatchedCommandRule {
        MatchedCommandRule {
            metadata: Arc::clone(&rule.metadata),
        }
    }

    fn match_command_decision(
        &self,
        command: &str,
        command_shape: Option<&FilterCommand>,
        redirect_rewrite: RedirectRewrite<'_>,
        mode: Option<PermissionMode>,
    ) -> CommandDecision {
        let Some(rule) = self.command_rules.first_match(command, mode) else {
            return CommandDecision::NoMatch;
        };
        debug!(rule_name = %rule.metadata.rule_name, command, "Bash rule matched");
        let matched = Self::matched_command_rule(rule);
        match &rule.action {
            CommandAction::Allow => CommandDecision::Allow { rule: matched },
            CommandAction::Deny(reason) => CommandDecision::Deny {
                rule: matched,
                reason: reason.clone(),
            },
            CommandAction::Modify(value) => {
                let captures = rule
                    .regex
                    .captures(command)
                    .expect("Invariant violation: RegexSet and Regex desynchronized");
                let replacement = expand_captures(&captures, value);
                let mut rewritten = replacement.clone();
                rewritten.push_str(redirect_rewrite.suffix);
                if !self.redirects_remain_attached(&replacement, &rewritten, redirect_rewrite.count)
                {
                    return CommandDecision::Ask { rule: matched };
                }

                let prefix = redirect_rewrite
                    .bindings
                    .iter()
                    .map(|binding| format!("{}={}; ", binding.name, binding.value))
                    .collect::<String>();
                let new_command = format!("{prefix}{rewritten}");
                CommandDecision::Modify {
                    rule: matched,
                    new_command,
                }
            }
            CommandAction::Ask => CommandDecision::Ask { rule: matched },
            CommandAction::ArgumentFilter {
                remove,
                add,
                replace,
                reason,
            } => {
                let Some(command_shape) = command_shape else {
                    return CommandDecision::Ask { rule: matched };
                };
                match filter_arguments(command_shape, remove, add, replace) {
                    Ok(new_command) => CommandDecision::ArgumentFilter {
                        rule: matched,
                        new_command,
                        reason: reason.clone(),
                    },
                    Err(error) => {
                        debug!(
                            rule_name = %rule.metadata.rule_name,
                            command,
                            error = %error,
                            "Failed to filter command arguments, asking user"
                        );
                        CommandDecision::Ask { rule: matched }
                    }
                }
            }
        }
    }

    fn redirects_remain_attached(
        &self,
        replacement: &str,
        rewritten: &str,
        redirect_count: usize,
    ) -> bool {
        if redirect_count == 0 {
            return true;
        }
        // Attachment depends on parse shape and redirect counts, not cwd normalization, so the
        // structural comparison deliberately reparses without a cwd.
        let (before, after) = match (
            split_command(replacement, "", &self.path_aliases),
            split_command(rewritten, "", &self.path_aliases),
        ) {
            (SplitOutcome::Commands(before), SplitOutcome::Commands(after)) => (before, after),
            // The continuation reparses the same rewritten command and prompts on this bailout.
            (_, SplitOutcome::Bail { .. }) => return true,
            (SplitOutcome::Bail { .. }, SplitOutcome::Commands(_)) => return false,
        };
        if before.len() != after.len() || before.is_empty() {
            return false;
        }
        let last = before.len() - 1;
        before
            .iter()
            .zip(&after)
            .enumerate()
            .all(|(index, (before_leaf, after_leaf))| {
                before_leaf.match_text == after_leaf.match_text
                    && after_leaf.redirects.len()
                        == before_leaf.redirects.len() + usize::from(index == last) * redirect_count
            })
    }

    pub(crate) fn evaluate_sync(
        &self,
        command: &str,
        context: &EvaluationContext,
        purpose: EvaluationPurpose,
    ) -> Evaluation {
        let original = self.evaluate_original(command, context, purpose);
        let continuation = match original.outcome() {
            PolicyOutcome::Modify { new_command, .. } => OriginalContinuation::ModifyRedirectCheck(
                self.analyze_redirect_check(new_command, context),
            ),
            PolicyOutcome::ArgumentFilter { new_command, .. } => {
                let recheck = self.evaluate_original(new_command, context, purpose);
                let continuation = match recheck.outcome() {
                    PolicyOutcome::Modify { new_command, .. } => {
                        FilterContinuation::ModifyRedirectCheck(
                            self.analyze_redirect_check(new_command, context),
                        )
                    }
                    _ => FilterContinuation::None,
                };
                OriginalContinuation::ArgumentFilterRecheck {
                    recheck,
                    continuation,
                }
            }
            _ => OriginalContinuation::None,
        };
        Evaluation {
            original,
            continuation,
        }
    }

    pub(crate) fn evaluate_original(
        &self,
        command: &str,
        context: &EvaluationContext,
        purpose: EvaluationPurpose,
    ) -> PolicyAnalysis {
        match split_command(command, &context.cwd, &self.path_aliases) {
            SplitOutcome::Bail { reason, redirects } => {
                let decision = match self.match_command_decision(
                    command,
                    None,
                    RedirectRewrite::NONE,
                    context.mode,
                ) {
                    CommandDecision::Deny { rule, reason } => BailDecision::Deny { rule, reason },
                    _ => self.redirect_denied_endpoint(redirects, context).map_or(
                        BailDecision::NoMatch,
                        |endpoint| {
                            let (rule, reason) = endpoint.denial();
                            BailDecision::RedirectDeny {
                                rule: rule.clone(),
                                reason: reason.to_string(),
                            }
                        },
                    ),
                };
                PolicyAnalysis::Bail {
                    command: command.to_string(),
                    reason,
                    decision,
                }
            }
            SplitOutcome::Commands(leaves) => {
                let redirect_denies = if leaves.iter().any(|leaf| !leaf.redirects.is_empty()) {
                    self.redirect_rules.deny_coverage(context.mode)
                } else {
                    BTreeSet::new()
                };
                PolicyAnalysis::Leaves(
                    leaves
                        .into_iter()
                        .map(|leaf| {
                            self.analyze_policy_leaf(leaf, context, purpose, &redirect_denies)
                        })
                        .collect(),
                )
            }
        }
    }

    fn analyze_policy_leaf(
        &self,
        leaf: LeafCommand,
        context: &EvaluationContext,
        purpose: EvaluationPurpose,
        redirect_denies: &BTreeSet<RedirectDirection>,
    ) -> PolicyLeafAnalysis {
        let command = self.match_command_decision(
            &leaf.match_text,
            leaf.filter_command.as_ref(),
            RedirectRewrite::from_leaf(&leaf),
            context.mode,
        );
        let (identity, redirects) = leaf_identity_and_redirects(leaf);
        let analyze_endpoints = matches!(purpose, EvaluationPurpose::Diagnostics)
            || matches!(command, CommandDecision::Allow { .. })
            || (redirects
                .iter()
                .any(|endpoint| redirect_denies.contains(&endpoint.direction))
                && matches!(
                    command,
                    CommandDecision::NoMatch | CommandDecision::Ask { .. }
                ));
        let endpoints = if analyze_endpoints {
            EndpointCoverage::Analyzed(
                redirects
                    .into_iter()
                    .map(|endpoint| self.analyze_endpoint(endpoint, context))
                    .collect(),
            )
        } else {
            EndpointCoverage::Skipped
        };
        PolicyLeafAnalysis {
            identity,
            command,
            endpoints,
        }
    }

    fn redirect_denied_endpoint(
        &self,
        redirects: Vec<RedirectEndpoint>,
        context: &EvaluationContext,
    ) -> Option<DeniedRedirectEndpoint> {
        let coverage = self.redirect_rules.deny_coverage(context.mode);
        redirects
            .into_iter()
            .filter(|endpoint| coverage.contains(&endpoint.direction))
            .map(|endpoint| self.analyze_endpoint(endpoint, context))
            .find_map(DeniedRedirectEndpoint::from_denied)
    }

    fn analyze_redirect_check(
        &self,
        command: &str,
        context: &EvaluationContext,
    ) -> RedirectCheckAnalysis {
        match split_command(command, &context.cwd, &self.path_aliases) {
            SplitOutcome::Bail { reason, redirects } => RedirectCheckAnalysis::Bail {
                command: command.to_string(),
                reason,
                denied_endpoint: self.redirect_denied_endpoint(redirects, context),
            },
            SplitOutcome::Commands(leaves) => RedirectCheckAnalysis::Leaves(
                leaves
                    .into_iter()
                    .map(|leaf| {
                        let (identity, redirects) = leaf_identity_and_redirects(leaf);
                        RedirectLeafAnalysis {
                            endpoints: redirects
                                .into_iter()
                                .map(|endpoint| self.analyze_endpoint(endpoint, context))
                                .collect(),
                            identity,
                        }
                    })
                    .collect(),
            ),
        }
    }

    fn analyze_endpoint(
        &self,
        endpoint: RedirectEndpoint,
        context: &EvaluationContext,
    ) -> EndpointAnalysis {
        let direction = endpoint.direction;
        match endpoint.target {
            ParsedRedirectTarget::Descriptor { match_text } => {
                let matched_rule = self.match_redirect_rule(
                    &match_text,
                    context.mode,
                    direction,
                    RedirectEndpointKind::Descriptor,
                    false,
                );
                EndpointAnalysis::Descriptor {
                    original_target: endpoint.original_target,
                    direction,
                    match_text,
                    matched_rule,
                }
            }
            ParsedRedirectTarget::Unresolvable { reason } => EndpointAnalysis::Filesystem {
                original_target: endpoint.original_target,
                direction,
                resolution: FilesystemAnalysis::Failed {
                    stage: EndpointFailureStage::StaticAnalysis,
                    reason,
                },
            },
            ParsedRedirectTarget::Filesystem {
                path,
                expand_home_tilde,
            } => match context.resolution.resolve(&path, expand_home_tilde) {
                Ok(RedirectTargetResolution {
                    match_text,
                    is_local,
                    is_device_or_special,
                    ..
                }) => {
                    let kind = if is_device_or_special {
                        RedirectEndpointKind::DeviceOrSpecial
                    } else {
                        RedirectEndpointKind::Filesystem
                    };
                    let matched_rule = self.match_redirect_rule(
                        &match_text,
                        context.mode,
                        direction,
                        kind,
                        is_local,
                    );
                    EndpointAnalysis::Filesystem {
                        original_target: endpoint.original_target,
                        direction,
                        resolution: FilesystemAnalysis::Resolved {
                            target: ResolvedRedirectTarget {
                                match_text,
                                is_local,
                                kind,
                            },
                            matched_rule,
                        },
                    }
                }
                Err(error) => EndpointAnalysis::Filesystem {
                    original_target: endpoint.original_target,
                    direction,
                    resolution: FilesystemAnalysis::Failed {
                        stage: EndpointFailureStage::RuntimeResolution,
                        reason: format!("failed to resolve redirect target: {error}"),
                    },
                },
            },
        }
    }

    fn match_redirect_rule(
        &self,
        match_text: &str,
        mode: Option<PermissionMode>,
        direction: RedirectDirection,
        kind: RedirectEndpointKind,
        is_local: bool,
    ) -> Option<MatchedRedirectRule> {
        self.redirect_rules
            .first_match(match_text, mode, direction, kind.is_filesystem(), is_local)
            .map(|rule| MatchedRedirectRule {
                metadata: Arc::clone(&rule.metadata),
                action: Arc::clone(&rule.action),
            })
    }

    pub(crate) async fn evaluate_live(
        self: &Arc<Self>,
        command: &str,
        cwd: &str,
        mode: Option<PermissionMode>,
    ) -> Result<Evaluation, LiveEvaluationFailure> {
        let engine = Arc::clone(self);
        let command = command.to_string();
        let cwd = cwd.to_string();
        match timeout(
            FILESYSTEM_EVALUATION_TIMEOUT,
            spawn_blocking(move || {
                let context = EvaluationContext::new(&cwd, mode);
                engine.evaluate_sync(&command, &context, EvaluationPurpose::Decision)
            }),
        )
        .await
        {
            Ok(Ok(evaluation)) => Ok(evaluation),
            Ok(Err(error)) => Err(LiveEvaluationFailure::Join(error)),
            Err(_) => Err(LiveEvaluationFailure::Timeout),
        }
    }
}

impl CommandDecision {
    fn outcome(&self) -> PolicyOutcome<'_> {
        match self {
            Self::NoMatch => PolicyOutcome::NoMatch,
            Self::Allow { rule } => PolicyOutcome::Allow { rule },
            Self::Deny { rule, reason } => PolicyOutcome::Deny { rule, reason },
            Self::Modify { rule, new_command } => PolicyOutcome::Modify { rule, new_command },
            Self::Ask { rule } => PolicyOutcome::Ask { rule },
            Self::ArgumentFilter {
                rule,
                new_command,
                reason,
            } => PolicyOutcome::ArgumentFilter {
                rule,
                new_command,
                reason: reason.as_deref(),
            },
        }
    }

    pub(crate) fn matched_rule(&self) -> Option<&MatchedCommandRule> {
        match self {
            Self::NoMatch => None,
            Self::Allow { rule }
            | Self::Deny { rule, .. }
            | Self::Modify { rule, .. }
            | Self::Ask { rule }
            | Self::ArgumentFilter { rule, .. } => Some(rule),
        }
    }
}

impl EndpointAnalysis {
    fn authorization(&self) -> RedirectAuthorizationView<'_> {
        match self.matched_rule() {
            Some(rule) => match rule.action.as_ref() {
                RedirectRuleAction::Allow { .. } => RedirectAuthorizationView::Authorized(rule),
                RedirectRuleAction::Deny { value, .. } => {
                    RedirectAuthorizationView::Denied(rule, value)
                }
            },
            None if self.failure_stage().is_some() => RedirectAuthorizationView::Unresolvable,
            None => RedirectAuthorizationView::Unmatched,
        }
    }

    fn denial(&self) -> Option<(&MatchedRedirectRule, &str)> {
        match self.authorization() {
            RedirectAuthorizationView::Denied(rule, reason) => Some((rule, reason)),
            _ => None,
        }
    }

    fn matched_rule(&self) -> Option<&MatchedRedirectRule> {
        match self {
            Self::Descriptor { matched_rule, .. }
            | Self::Filesystem {
                resolution: FilesystemAnalysis::Resolved { matched_rule, .. },
                ..
            } => matched_rule.as_ref(),
            Self::Filesystem {
                resolution: FilesystemAnalysis::Failed { .. },
                ..
            } => None,
        }
    }

    pub(crate) fn direction(&self) -> RedirectDirection {
        match self {
            Self::Descriptor { direction, .. } | Self::Filesystem { direction, .. } => *direction,
        }
    }

    pub(crate) fn failure_stage(&self) -> Option<EndpointFailureStage> {
        match self {
            Self::Filesystem {
                resolution: FilesystemAnalysis::Failed { stage, .. },
                ..
            } => Some(*stage),
            Self::Descriptor { .. }
            | Self::Filesystem {
                resolution: FilesystemAnalysis::Resolved { .. },
                ..
            } => None,
        }
    }
}

impl MatchedCommandRule {
    pub(crate) fn rule_name(&self) -> &str {
        &self.metadata.rule_name
    }

    pub(crate) fn expanded_pattern(&self) -> &str {
        &self.metadata.expanded_pattern
    }

    pub(crate) fn action_summary(&self) -> &str {
        &self.metadata.action_summary
    }
}

impl MatchedRedirectRule {
    pub(crate) fn rule_name(&self) -> &str {
        &self.metadata.rule_name
    }

    pub(crate) fn expanded_pattern(&self) -> &str {
        &self.metadata.expanded_pattern
    }

    pub(crate) fn action_summary(&self) -> &str {
        &self.metadata.action_summary
    }

    pub(crate) fn is_deny(&self) -> bool {
        self.action.is_deny()
    }
}

impl ResolvedRedirectTarget {
    pub(crate) fn match_text(&self) -> &str {
        &self.match_text
    }

    pub(crate) fn is_local(&self) -> bool {
        self.is_local
    }

    pub(crate) fn kind(&self) -> RedirectEndpointKind {
        self.kind
    }
}

fn leaf_identity_and_redirects(leaf: LeafCommand) -> (LeafIdentity, Vec<RedirectEndpoint>) {
    let identity = LeafIdentity {
        original: leaf.original,
        alias_expanded: leaf.alias_expanded,
        normalized: leaf.match_text,
        bindings: leaf.bindings,
        requires_confirmation: leaf.requires_confirmation,
        command_shape: leaf.filter_command,
    };
    (identity, leaf.redirects)
}

impl LeafIdentity {
    pub(crate) fn original(&self) -> &str {
        &self.original
    }

    pub(crate) fn alias_expanded(&self) -> Option<&str> {
        self.alias_expanded.as_deref()
    }

    pub(crate) fn normalized(&self) -> &str {
        &self.normalized
    }

    pub(crate) fn bindings(&self) -> &[AliasBinding] {
        &self.bindings
    }

    pub(crate) fn requires_confirmation(&self) -> Option<&str> {
        self.requires_confirmation.as_deref()
    }

    pub(crate) fn command_shape(&self) -> Option<&FilterCommand> {
        self.command_shape.as_ref()
    }
}

impl EndpointCoverage {
    pub(crate) fn analyzed(&self) -> Option<&[EndpointAnalysis]> {
        match self {
            Self::Analyzed(endpoints) => Some(endpoints),
            Self::Skipped => None,
        }
    }
}

impl PolicyLeafAnalysis {
    pub(crate) fn identity(&self) -> &LeafIdentity {
        &self.identity
    }

    pub(crate) fn command(&self) -> &CommandDecision {
        &self.command
    }

    pub(crate) fn endpoints(&self) -> &EndpointCoverage {
        &self.endpoints
    }

    pub(crate) fn source_command_allowed(&self) -> bool {
        matches!(self.command, CommandDecision::Allow { .. })
    }

    pub(crate) fn outcome(&self) -> PolicyOutcome<'_> {
        let outcome = self.command.outcome();
        if matches!(
            outcome,
            PolicyOutcome::Deny { .. }
                | PolicyOutcome::Modify { .. }
                | PolicyOutcome::ArgumentFilter { .. }
        ) {
            return outcome;
        }
        let endpoints = match &self.endpoints {
            EndpointCoverage::Analyzed(endpoints) => endpoints.as_slice(),
            EndpointCoverage::Skipped => &[],
        };
        if let Some((rule, reason)) = endpoints.iter().find_map(EndpointAnalysis::denial) {
            return PolicyOutcome::RedirectDeny { rule, reason };
        }
        let PolicyOutcome::Allow { rule } = outcome else {
            return outcome;
        };
        if matches!(self.endpoints, EndpointCoverage::Skipped) {
            return PolicyOutcome::Ask { rule };
        }
        if endpoints.iter().all(|endpoint| {
            matches!(
                endpoint.authorization(),
                RedirectAuthorizationView::Authorized(_)
            )
        }) && self.identity.requires_confirmation.is_none()
        {
            outcome
        } else {
            PolicyOutcome::Ask { rule }
        }
    }
}

impl PolicyAnalysis {
    pub(crate) fn leaves(&self) -> Option<&[PolicyLeafAnalysis]> {
        match self {
            Self::Leaves(leaves) => Some(leaves),
            Self::Bail { .. } => None,
        }
    }

    fn outcome(&self) -> PolicyOutcome<'_> {
        match self {
            Self::Bail { decision, .. } => match decision {
                BailDecision::Deny { rule, reason } => PolicyOutcome::Deny { rule, reason },
                BailDecision::RedirectDeny { rule, reason } => {
                    PolicyOutcome::RedirectDeny { rule, reason }
                }
                BailDecision::NoMatch => PolicyOutcome::NoMatch,
            },
            Self::Leaves(leaves) if leaves.len() == 1 => leaves[0].outcome(),
            Self::Leaves(leaves) => {
                let best = leaves.iter().map(PolicyLeafAnalysis::outcome).fold(
                    None,
                    |best: Option<PolicyOutcome<'_>>, outcome| match best {
                        Some(current) if policy_rank(current) >= policy_rank(outcome) => best,
                        _ => Some(outcome),
                    },
                );
                match best {
                    Some(
                        outcome @ (PolicyOutcome::Allow { .. }
                        | PolicyOutcome::Deny { .. }
                        | PolicyOutcome::RedirectDeny { .. }
                        | PolicyOutcome::Ask { .. }),
                    ) => outcome,
                    _ => PolicyOutcome::NoMatch,
                }
            }
        }
    }
}

impl RedirectLeafAnalysis {
    pub(crate) fn identity(&self) -> &LeafIdentity {
        &self.identity
    }

    pub(crate) fn endpoints(&self) -> &[EndpointAnalysis] {
        &self.endpoints
    }
}

impl RedirectCheckAnalysis {
    fn denial(&self) -> Option<(&MatchedRedirectRule, &str)> {
        match self {
            Self::Bail {
                denied_endpoint: Some(endpoint),
                ..
            } => Some(endpoint.denial()),
            Self::Bail {
                denied_endpoint: None,
                ..
            } => None,
            Self::Leaves(leaves) => leaves
                .iter()
                .find_map(|leaf| leaf.endpoints.iter().find_map(EndpointAnalysis::denial)),
        }
    }

    fn allows_rewrite(&self) -> bool {
        match self {
            Self::Bail { .. } => false,
            Self::Leaves(leaves) => leaves.iter().all(|leaf| {
                leaf.identity.requires_confirmation.is_none()
                    && leaf.endpoints.iter().all(|endpoint| {
                        matches!(
                            endpoint.authorization(),
                            RedirectAuthorizationView::Authorized(_)
                        )
                    })
            }),
        }
    }
}

impl Evaluation {
    pub(crate) fn original_analysis(&self) -> &PolicyAnalysis {
        &self.original
    }

    pub(crate) fn continuation(&self) -> &OriginalContinuation {
        &self.continuation
    }

    pub(crate) fn original_outcome(&self) -> PolicyOutcome<'_> {
        self.original.outcome()
    }

    pub(crate) fn outcome(&self) -> PolicyOutcome<'_> {
        let original = self.original_outcome();
        match (&self.continuation, original) {
            (OriginalContinuation::None, _) => original,
            (
                OriginalContinuation::ModifyRedirectCheck(check),
                PolicyOutcome::Modify { rule, new_command },
            ) => {
                if let Some((rule, reason)) = check.denial() {
                    PolicyOutcome::RedirectDeny { rule, reason }
                } else if check.allows_rewrite() {
                    PolicyOutcome::Modify { rule, new_command }
                } else {
                    PolicyOutcome::Ask { rule }
                }
            }
            (
                OriginalContinuation::ArgumentFilterRecheck {
                    recheck,
                    continuation,
                },
                original @ PolicyOutcome::ArgumentFilter { .. },
            ) => match (recheck.outcome(), continuation) {
                (PolicyOutcome::Allow { .. }, _) => original,
                (PolicyOutcome::ArgumentFilter { rule, .. }, _) => PolicyOutcome::Ask { rule },
                (
                    PolicyOutcome::Modify { rule, new_command },
                    FilterContinuation::ModifyRedirectCheck(check),
                ) => {
                    if let Some((rule, reason)) = check.denial() {
                        PolicyOutcome::RedirectDeny { rule, reason }
                    } else if check.allows_rewrite() {
                        PolicyOutcome::Modify { rule, new_command }
                    } else {
                        PolicyOutcome::Ask { rule }
                    }
                }
                (outcome, _) => outcome,
            },
            _ => original,
        }
    }

    pub(crate) fn rule_result(&self) -> RuleResult {
        self.outcome().into()
    }

    pub(crate) fn contributors(&self) -> Vec<String> {
        let mut contributors = Vec::new();
        collect_policy_contributors(&self.original, &mut contributors);
        match &self.continuation {
            OriginalContinuation::None => {}
            OriginalContinuation::ModifyRedirectCheck(check) => {
                collect_redirect_contributors(check, &mut contributors);
            }
            OriginalContinuation::ArgumentFilterRecheck {
                recheck,
                continuation,
            } => {
                collect_policy_contributors(recheck, &mut contributors);
                if let FilterContinuation::ModifyRedirectCheck(check) = continuation {
                    collect_redirect_contributors(check, &mut contributors);
                }
            }
        }
        contributors
            .into_iter()
            .map(|contributor| contributor.rule_name().to_string())
            .collect()
    }
}

impl From<PolicyOutcome<'_>> for RuleResult {
    fn from(outcome: PolicyOutcome<'_>) -> Self {
        match outcome {
            PolicyOutcome::Allow { rule } => Self::Allowed {
                rule_name: rule.rule_name().to_string(),
            },
            PolicyOutcome::Deny { rule, reason } => Self::Denied {
                rule_name: rule.rule_name().to_string(),
                reason: reason.to_string(),
            },
            PolicyOutcome::RedirectDeny { rule, reason } => Self::Denied {
                rule_name: rule.rule_name().to_string(),
                reason: reason.to_string(),
            },
            PolicyOutcome::Modify { rule, new_command } => Self::Modified {
                rule_name: rule.rule_name().to_string(),
                new_command: new_command.to_string(),
            },
            PolicyOutcome::Ask { rule } => Self::Asked {
                rule_name: rule.rule_name().to_string(),
            },
            PolicyOutcome::ArgumentFilter {
                rule,
                new_command,
                reason,
            } => Self::ArgumentFiltered {
                rule_name: rule.rule_name().to_string(),
                new_command: new_command.to_string(),
                reason: reason.map(str::to_string),
            },
            PolicyOutcome::NoMatch => Self::NoMatch,
        }
    }
}

fn policy_rank(outcome: PolicyOutcome<'_>) -> u8 {
    match outcome {
        PolicyOutcome::Deny { .. } | PolicyOutcome::RedirectDeny { .. } => 4,
        PolicyOutcome::Ask { .. } => 3,
        PolicyOutcome::NoMatch
        | PolicyOutcome::Modify { .. }
        | PolicyOutcome::ArgumentFilter { .. } => 2,
        PolicyOutcome::Allow { .. } => 1,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MatchedContributor<'a> {
    Command(&'a MatchedCommandRule),
    Redirect(&'a MatchedRedirectRule),
}

impl MatchedContributor<'_> {
    fn rule_name(&self) -> &str {
        match self {
            Self::Command(rule) => rule.rule_name(),
            Self::Redirect(rule) => rule.rule_name(),
        }
    }
}

fn push_matched_contributor<'a>(
    contributors: &mut Vec<MatchedContributor<'a>>,
    contributor: MatchedContributor<'a>,
) {
    if !contributor.rule_name().is_empty() && !contributors.contains(&contributor) {
        contributors.push(contributor);
    }
}

fn collect_policy_contributors<'a>(
    policy: &'a PolicyAnalysis,
    contributors: &mut Vec<MatchedContributor<'a>>,
) {
    match policy {
        PolicyAnalysis::Bail {
            decision: BailDecision::Deny { rule, .. },
            ..
        } => push_matched_contributor(contributors, MatchedContributor::Command(rule)),
        PolicyAnalysis::Bail {
            decision: BailDecision::RedirectDeny { rule, .. },
            ..
        } => push_matched_contributor(contributors, MatchedContributor::Redirect(rule)),
        PolicyAnalysis::Bail {
            decision: BailDecision::NoMatch,
            ..
        } => {}
        PolicyAnalysis::Leaves(leaves) => {
            for leaf in leaves {
                if let Some(rule) = leaf.command.matched_rule() {
                    push_matched_contributor(contributors, MatchedContributor::Command(rule));
                }
                if let EndpointCoverage::Analyzed(endpoints) = &leaf.endpoints {
                    for endpoint in endpoints {
                        if let Some(rule) = endpoint.matched_rule()
                            && (matches!(leaf.command, CommandDecision::Allow { .. })
                                || rule.is_deny())
                        {
                            push_matched_contributor(
                                contributors,
                                MatchedContributor::Redirect(rule),
                            );
                        }
                    }
                }
            }
        }
    }
}

fn collect_redirect_contributors<'a>(
    check: &'a RedirectCheckAnalysis,
    contributors: &mut Vec<MatchedContributor<'a>>,
) {
    match check {
        RedirectCheckAnalysis::Bail {
            denied_endpoint: Some(endpoint),
            ..
        } => {
            let (rule, _) = endpoint.denial();
            push_matched_contributor(contributors, MatchedContributor::Redirect(rule));
        }
        RedirectCheckAnalysis::Bail {
            denied_endpoint: None,
            ..
        } => {}
        RedirectCheckAnalysis::Leaves(leaves) => {
            for leaf in leaves {
                for endpoint in &leaf.endpoints {
                    if let Some(rule) = endpoint.matched_rule() {
                        push_matched_contributor(contributors, MatchedContributor::Redirect(rule));
                    }
                }
            }
        }
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

fn quote_command_word(word: &str) -> String {
    if !word.is_empty()
        && word.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'_' | b'@' | b'%' | b'+' | b'=' | b':' | b',' | b'.' | b'/' | b'-'
                )
        })
    {
        word.to_string()
    } else {
        shell_words::quote(word).into_owned()
    }
}

fn filter_arguments(
    filter_command: &FilterCommand,
    remove: &Option<Vec<String>>,
    add: &Option<Vec<String>>,
    replace: &Option<HashMap<String, String>>,
) -> miette::Result<String> {
    let command = &filter_command.source;
    let mut edits = Vec::new();
    for argument in &filter_command.arguments {
        let removed = remove.as_ref().is_some_and(|patterns| {
            patterns.iter().any(|pattern| {
                argument.value == *pattern
                    || argument
                        .value
                        .strip_prefix(pattern)
                        .is_some_and(|suffix| suffix.starts_with('='))
            })
        });
        // Removal wins so replacement cannot reintroduce an explicitly forbidden argument.
        let replacement = if removed {
            Some(String::new())
        } else {
            replace
                .as_ref()
                .and_then(|replacements| replacements.get(&argument.value))
                .map(|replacement| quote_command_word(replacement))
        };
        if let Some(replacement) = replacement {
            let mut range = argument.range.clone();
            if replacement.is_empty() {
                while let Some((index, ch)) = command[..range.start].char_indices().next_back()
                    && ch.is_whitespace()
                {
                    range.start = index;
                }
            }
            edits.push((range, replacement));
        }
    }

    let mut filtered = command.to_string();
    for (range, replacement) in edits.into_iter().rev() {
        if filtered.get(range.clone()).is_none() {
            return Err(miette!(
                "ArgumentFilter received invalid brush source spans"
            ));
        }
        filtered.replace_range(range, &replacement);
    }
    if let Some(additions) = add
        && !additions.is_empty()
    {
        if !filtered.chars().last().is_some_and(char::is_whitespace) {
            filtered.push(' ');
        }
        filtered.push_str(
            &additions
                .iter()
                .map(|argument| quote_command_word(argument))
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    Ok(format!(
        "{}{}{}",
        filter_command.rewrite_prefix, filtered, filter_command.rewrite_suffix
    ))
}

#[cfg(test)]
#[path = "bash_rules_tests/mod.rs"]
mod tests;
