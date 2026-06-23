//! `moriarty rules` — inspect, validate, and author the bash/tool permission rules.
//!
//! These subcommands operate on `~/.config/moriarty/tool_rules.toml` (or a `--config` override) and
//! never run the hook; they help authors write rules that are safe and actually take effect.

// standard library
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::PathBuf,
    sync::LazyLock,
};

// 3rd party crates
use miette::{IntoDiagnostic, Result, WrapErr};
use regex::Regex;
use serde::Serialize;
use tabled::{Table, Tabled};

// local / workspace deps
use crate::{
    RulesCommand,
    cost_report::TimeRangeFilter,
    hooks::{
        bash_rules::{
            BashRuleEngine, RuleDiagnostic, RuleResult, default_fragments, expand_fragments,
        },
        command_split::{SplitOutcome, split_command},
        report::{CwdAggregation, ReportRow, RulesHashFilter, aggregate_with_cwd},
        result::PreToolResult,
        tool_rules::ToolRuleEngine,
    },
    user_config::{BashRule, BashRuleAction, UserConfig, load_user_config, load_user_config_from},
};

/// Generated-pattern shape for `rules suggest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum MatchMode {
    /// A fully-literal, fully-anchored match of one observed leaf command.
    Exact,
    /// A match on just the program name (the first token).
    Prefix,
    /// One pattern per program, generalizing safe-identifier subcommands across the observed
    /// leaves into a closed alternation (`^cargo (build|check)(\s|$)`), falling back to a program
    /// prefix when the second tokens are not all simple identifiers.
    Fuzzy,
}

/// Action assigned to rules emitted by `rules suggest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum SuggestAction {
    Ask,
    Deny,
    Allow,
}

pub async fn exec_rules(cmd: RulesCommand) -> Result<()> {
    match cmd {
        RulesCommand::Lint {
            config,
            json,
            strict,
        } => lint(config, json, strict).await,
        RulesCommand::ListFragments { config, json } => list_fragments(config, json).await,
        RulesCommand::Schema { json } => schema(json),
        RulesCommand::Starter { json } => starter(json),
        RulesCommand::Suggest {
            dir,
            start_time,
            end_time,
            timezone,
            result,
            limit,
            min_count,
            match_mode,
            action,
            all_rules,
            rules_hash,
            json,
        } => {
            suggest(SuggestOptions {
                dir,
                start_time,
                end_time,
                timezone,
                result,
                limit,
                min_count,
                match_mode,
                action,
                all_rules,
                rules_hash,
                json,
            })
            .await
        }
        RulesCommand::Replay {
            dir,
            config,
            start_time,
            end_time,
            timezone,
            result,
            all_rules,
            rules_hash,
            json,
        } => {
            replay(ReplayOptions {
                dir,
                config,
                start_time,
                end_time,
                timezone,
                result,
                all_rules,
                rules_hash,
                json,
            })
            .await
        }
    }
}

/// One reported issue. `kind` is the diagnostic label for errors, or `over-broad-allow` /
/// `shadowed` for `--strict` warnings.
#[derive(Debug, Serialize, PartialEq, Eq)]
struct LintFinding {
    rule_kind: &'static str,
    rule_name: String,
    pattern: String,
    kind: String,
    message: String,
}

impl LintFinding {
    fn from_diagnostic(rule_kind: &'static str, diagnostic: &RuleDiagnostic) -> Self {
        Self {
            rule_kind,
            rule_name: diagnostic.rule_name.clone(),
            pattern: diagnostic.pattern.clone(),
            kind: diagnostic.kind.label().to_string(),
            message: diagnostic.message.clone(),
        }
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct LintReport {
    /// Rules the hook silently drops (a rule the user wrote is not in effect).
    errors: Vec<LintFinding>,
    /// `--strict` advisories: likely-shadowed and over-broad rules.
    warnings: Vec<LintFinding>,
    /// Number of dropped rules; nonzero means the lint fails.
    ignored_count: usize,
}

async fn lint(config_path: Option<PathBuf>, json: bool, strict: bool) -> Result<()> {
    let config = load_user_config_from(config_path.as_deref()).await?;
    let report = build_lint_report(&config, strict)?;

    if json {
        let rendered = serde_json::to_string_pretty(&report)
            .into_diagnostic()
            .wrap_err("Failed to serialize lint report")?;
        println!("{rendered}");
    } else {
        print_human(&report);
    }

    // A dropped rule means a rule the user wrote is not enforced — fail so CI catches it. Exit
    // directly (rather than returning Err) to avoid printing a miette report over the clean output.
    if report.ignored_count > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn build_lint_report(config: &UserConfig, strict: bool) -> Result<LintReport> {
    let fragments = config.pattern_fragments.clone();
    let bash_rules = config.bash_rules.clone().unwrap_or_default();
    let tool_rules = config.tool_rules.clone().unwrap_or_default();

    let mut errors = Vec::new();

    let (_engine, bash_diagnostics) =
        BashRuleEngine::compile_with_diagnostics(bash_rules.clone(), fragments.clone())?;
    errors.extend(
        bash_diagnostics
            .iter()
            .map(|diagnostic| LintFinding::from_diagnostic("bash", diagnostic)),
    );

    let (_engine, tool_diagnostics) =
        ToolRuleEngine::compile_with_diagnostics(tool_rules, fragments.clone());
    errors.extend(
        tool_diagnostics
            .iter()
            .map(|diagnostic| LintFinding::from_diagnostic("tool", diagnostic)),
    );

    let warnings = if strict {
        strict_bash_warnings(&bash_rules, fragments.as_ref())
    } else {
        Vec::new()
    };

    let ignored_count = errors.len();
    Ok(LintReport {
        errors,
        warnings,
        ignored_count,
    })
}

/// Best-effort `--strict` advisories over the bash rules: over-broad Allow rules and rules an
/// earlier rule likely shadows. Both are heuristic, so they are warnings, never errors.
fn strict_bash_warnings(
    rules: &[BashRule],
    user_fragments: Option<&HashMap<String, String>>,
) -> Vec<LintFinding> {
    let mut warnings = Vec::new();

    for rule in rules {
        if matches!(rule.action, BashRuleAction::Allow) && is_over_broad(&rule.pattern) {
            warnings.push(LintFinding {
                rule_kind: "bash",
                rule_name: rule.name.clone(),
                pattern: rule.pattern.clone(),
                kind: "over-broad-allow".to_string(),
                message: "Allow rule matches effectively every command".to_string(),
            });
        }
    }

    warnings.extend(shadow_warnings(rules, user_fragments));
    warnings
}

/// Flags a rule when an earlier rule's regex matches a literal probe derived from this rule's
/// pattern — i.e. the earlier rule fires first and this one is unreachable for that input. This is
/// deliberately approximate (a single literal probe per rule), so it can miss or over-report.
fn shadow_warnings(
    rules: &[BashRule],
    user_fragments: Option<&HashMap<String, String>>,
) -> Vec<LintFinding> {
    let mut fragments = default_fragments();
    if let Some(user_fragments) = user_fragments {
        fragments.extend(user_fragments.clone());
    }

    // Only successfully-compiling rules participate; rules that fail to compile are already errors.
    let compiled: Vec<(&BashRule, Regex)> = rules
        .iter()
        .filter_map(|rule| {
            let expanded = expand_fragments(&rule.pattern, &fragments).ok()?;
            Regex::new(&expanded).ok().map(|regex| (rule, regex))
        })
        .collect();

    let mut warnings = Vec::new();
    for (later_index, (later_rule, _)) in compiled.iter().enumerate() {
        let probe = literal_probe(&later_rule.pattern);
        if probe.is_empty() {
            continue;
        }
        if let Some((earlier_rule, _)) = compiled[..later_index]
            .iter()
            .find(|(_, earlier_regex)| earlier_regex.is_match(&probe))
        {
            warnings.push(LintFinding {
                rule_kind: "bash",
                rule_name: later_rule.name.clone(),
                pattern: later_rule.pattern.clone(),
                kind: "shadowed".to_string(),
                message: format!(
                    "Likely shadowed by earlier rule '{}' (best-effort heuristic)",
                    earlier_rule.name
                ),
            });
        }
    }
    warnings
}

fn is_over_broad(pattern: &str) -> bool {
    matches!(pattern.trim(), "" | ".*" | "^.*$" | "^.*" | ".*$")
}

/// Derives a representative literal string from a regex by stripping anchors/word-boundaries and
/// unescaping the common escapes that appear in rule patterns. Used only as a shadow-detection
/// probe, so an imperfect result simply yields a weaker (still safe) heuristic.
fn literal_probe(pattern: &str) -> String {
    let trimmed = pattern.trim();
    let trimmed = trimmed.strip_prefix('^').unwrap_or(trimmed);
    let trimmed = trimmed.strip_suffix('$').unwrap_or(trimmed);
    trimmed
        .replace(r"\b", "")
        .replace(r"\s", " ")
        .replace(r"\.", ".")
        .replace(r"\/", "/")
        .replace(r"\-", "-")
        .replace('\\', "")
}

fn print_human(report: &LintReport) {
    for finding in &report.errors {
        println!(
            "ERROR  {} rule '{}' [{}]: {}",
            finding.rule_kind, finding.rule_name, finding.kind, finding.message
        );
        if !finding.pattern.is_empty() {
            println!("         pattern: {}", finding.pattern);
        }
    }
    for finding in &report.warnings {
        println!(
            "WARN   {} rule '{}' [{}]: {}",
            finding.rule_kind, finding.rule_name, finding.kind, finding.message
        );
    }
    if report.errors.is_empty() && report.warnings.is_empty() {
        println!("No issues found.");
    }
    println!();
    println!(
        "{} rule(s) will be silently ignored by the hook.",
        report.ignored_count
    );
}

// ===== rules list-fragments =====

#[derive(Debug, Serialize, Tabled, PartialEq, Eq)]
struct FragmentRow {
    #[tabled(rename = "Fragment")]
    name: String,
    #[tabled(rename = "Source")]
    source: &'static str,
    #[tabled(rename = "Value")]
    value: String,
}

/// Merges built-in default fragments with the user's `pattern_fragments`, marking which source
/// each came from (a user fragment overriding a default is reported as `user`).
fn fragment_rows(user: Option<&HashMap<String, String>>) -> Vec<FragmentRow> {
    let defaults = default_fragments();
    let mut names: BTreeSet<String> = defaults.keys().cloned().collect();
    if let Some(user) = user {
        names.extend(user.keys().cloned());
    }

    names
        .into_iter()
        .map(|name| match user.and_then(|user| user.get(&name)) {
            Some(value) => FragmentRow {
                name,
                source: "user",
                value: value.clone(),
            },
            None => FragmentRow {
                value: defaults.get(&name).cloned().unwrap_or_default(),
                name,
                source: "default",
            },
        })
        .collect()
}

async fn list_fragments(config_path: Option<PathBuf>, json: bool) -> Result<()> {
    let config = load_user_config_from(config_path.as_deref()).await?;
    let rows = fragment_rows(config.pattern_fragments.as_ref());

    if json {
        let rendered = serde_json::to_string_pretty(&rows)
            .into_diagnostic()
            .wrap_err("Failed to serialize fragments")?;
        println!("{rendered}");
    } else {
        println!("{}", Table::new(&rows));
        println!();
        println!(
            "Reference a fragment in any rule pattern with {{{{name}}}}; it is expanded before the regex compiles."
        );
    }
    Ok(())
}

// ===== rules schema =====

/// Canonical example config exercising every rule kind and action variant. Kept in sync with the
/// config types by the `schema_round_trips_through_user_config` test.
const SCHEMA_TOML: &str = r#"# Reusable regex fragments, referenced from patterns as {{name}}.
[pattern_fragments]
safe_chars = "[^|&;$`]"

# bash_rules permission Bash commands. The hook splits compound commands and evaluates each leaf.
[[bash_rules]]
name = "deny-rm-rf"
pattern = "^rm\\s+-rf\\b"
action = { type = "Deny", value = "Dangerous recursive delete" }

[[bash_rules]]
name = "add-docker-dry-run"
pattern = "^(docker\\s+system\\s+prune)$"
action = { type = "Modify", value = "$1 --dry-run" }

[[bash_rules]]
name = "allow-ls"
pattern = "^ls\\b"
action = { type = "Allow" }

[[bash_rules]]
name = "ask-docker"
pattern = "^docker\\b"
action = { type = "Ask" }

[[bash_rules]]
name = "strip-cargo-doc-open"
pattern = "^cargo doc\\b"
action = { type = "ArgumentFilter", remove = ["--open"], reason = "Browser flag removed" }

# tool_rules permission any tool call (Read, Write, Edit, …); first match wins.
[[tool_rules]]
name = "allow-read"
tool = "Read"
action = { type = "Allow" }

[[tool_rules]]
name = "deny-env-write"
tool = "Write"
field = "file_path"
pattern = "\\.env$"
action = { type = "Deny", value = "Cannot write .env files" }

[[tool_rules]]
name = "ask-local-edit"
tool = "Edit"
allow_local = true
action = { type = "Ask" }
"#;

fn schema(json: bool) -> Result<()> {
    if json {
        let config: UserConfig = toml::from_str(SCHEMA_TOML)
            .into_diagnostic()
            .wrap_err("schema example is not valid config")?;
        let rendered = serde_json::to_string_pretty(&config)
            .into_diagnostic()
            .wrap_err("Failed to serialize schema config")?;
        println!("{rendered}");
    } else {
        print!("{SCHEMA_TOML}");
    }
    Ok(())
}

// ===== rules starter =====

/// Common read-only commands that are safe to auto-allow once compound splitting has separated
/// them from any operators. Writes via redirection are still capped at Ask by the engine.
const STARTER_COMMANDS: &[&str] = &[
    "echo", "ls", "cat", "head", "tail", "wc", "sort", "uniq", "grep", "rg", "pwd", "which",
    "file", "stat", "basename", "dirname", "true", "date", "env",
];

fn starter_rules() -> Vec<BashRule> {
    STARTER_COMMANDS
        .iter()
        .map(|command| BashRule {
            name: format!("allow-{command}"),
            // `\b` after the program name matches `echo`, `echo hi`, and `echo "x"` but not
            // `echoes`. Operators are already split off, so no `{{safe_arg}}` exclusion is needed.
            pattern: format!(r"^{command}\b"),
            action: BashRuleAction::Allow,
        })
        .collect()
}

fn starter(json: bool) -> Result<()> {
    let rules = starter_rules();

    if json {
        let rendered = serde_json::to_string_pretty(&rules)
            .into_diagnostic()
            .wrap_err("Failed to serialize starter rules")?;
        println!("{rendered}");
        return Ok(());
    }

    let config = UserConfig {
        pattern_fragments: None,
        bash_rules: Some(rules),
        tool_rules: None,
    };
    let toml = toml::to_string_pretty(&config)
        .into_diagnostic()
        .wrap_err("Failed to render starter rules")?;

    println!("# Starter pack: allow-rules for common read-only commands.");
    println!("# The compound splitter evaluates each leaf of a command independently, so these");
    println!("# simple prefix patterns stay safe inside `&&` / `||` / `|` / `;` chains.");
    println!("# A redirect to a real file (e.g. `> out.txt`) is still capped at Ask, even here.");
    println!();
    print!("{toml}");
    Ok(())
}

// ===== rules suggest =====

/// A grouped struct keeps `suggest` under clippy's argument-count limit and mirrors the CLI fields.
struct SuggestOptions {
    dir: Option<PathBuf>,
    start_time: Option<String>,
    end_time: Option<String>,
    timezone: String,
    result: PreToolResult,
    limit: usize,
    min_count: u64,
    match_mode: MatchMode,
    action: Option<SuggestAction>,
    all_rules: bool,
    rules_hash: Option<String>,
    json: bool,
}

/// Mirrors the `rules replay` CLI fields; grouped to stay under clippy's argument-count limit.
struct ReplayOptions {
    dir: Option<PathBuf>,
    config: Option<PathBuf>,
    start_time: Option<String>,
    end_time: Option<String>,
    timezone: String,
    result: Option<PreToolResult>,
    all_rules: bool,
    rules_hash: Option<String>,
    json: bool,
}

/// Resolves how `replay`/`suggest` scope recorded history to a rule set, returning the filter plus
/// the hash to display (`None` under `--all-rules`). The default — neither `--all-rules` nor an
/// explicit `--rules-hash` — is the rule set currently installed at the default config path, which
/// for `replay` is the migration *source*, independent of any `--config` candidate being tested.
async fn resolve_hash_filter(
    all_rules: bool,
    rules_hash: Option<String>,
) -> Result<(RulesHashFilter, Option<String>)> {
    if all_rules {
        return Ok((RulesHashFilter::Any, None));
    }
    if let Some(hash) = rules_hash {
        return Ok((RulesHashFilter::Only(hash.clone()), Some(hash)));
    }
    let active_hash = load_user_config().await?.effective_hash();
    Ok((
        RulesHashFilter::Only(active_hash.clone()),
        Some(active_hash),
    ))
}

/// A filtered run must never silently look like it covered all of history, so every output mode
/// (human, TOML header, JSON-on-stderr) renders this same scope line.
fn rules_scope_note(
    active_hash: &Option<String>,
    skipped: &crate::hooks::report::HashSkipStats,
) -> String {
    match active_hash {
        None => "Rule set: all (--all-rules); no hash filter applied.".to_string(),
        Some(hash) => format!(
            "Rule set: {hash}; skipped {} record(s) from other rule sets and {} predating rule-hash logging \
             (use --all-rules to include them).",
            skipped.other_rules, skipped.no_hash
        ),
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct Suggestion {
    rule: BashRule,
    count: u64,
    /// Every recorded command that contributed to this suggestion, so a generalized (prefix/fuzzy)
    /// or shared-leaf pattern is reviewable against what was actually observed.
    observed_commands: Vec<String>,
}

/// Returns suggestions and a scope note without rendering, so callers can assert on the raw
/// results without capturing stdout.
async fn collect_suggestions(opts: &SuggestOptions) -> Result<(Vec<Suggestion>, String)> {
    let action = opts.action.unwrap_or_else(|| default_action(opts.result));

    // Allow rules must be exact: a prefix or fuzzy Allow would auto-approve more than was observed.
    if matches!(action, SuggestAction::Allow) && !matches!(opts.match_mode, MatchMode::Exact) {
        return Err(miette::miette!(
            "Refusing to suggest Allow rules with --match {}; use --match exact for literal, fully-anchored Allow rules",
            match_mode_label(opts.match_mode)
        ));
    }

    let filter = TimeRangeFilter::new(
        opts.start_time.clone(),
        opts.end_time.clone(),
        crate::cost_report::parse_timezone(&opts.timezone)?,
    )?;
    let (hash_filter, active_hash) =
        resolve_hash_filter(opts.all_rules, opts.rules_hash.clone()).await?;
    let engine = {
        let config = load_user_config().await?;
        BashRuleEngine::from_config(
            config.bash_rules.unwrap_or_default(),
            config.pattern_fragments,
        )?
    };
    let CwdAggregation { rows, skipped } = aggregate_with_cwd(
        opts.dir.clone(),
        &filter,
        Some("Bash"),
        Some(opts.result),
        &hash_filter,
    )
    .await?;
    let suggestions = build_suggestions(
        &rows,
        opts.match_mode,
        action,
        opts.min_count,
        opts.limit,
        Some(&engine),
    );
    let scope = rules_scope_note(&active_hash, &skipped);
    Ok((suggestions, scope))
}

async fn suggest(opts: SuggestOptions) -> Result<()> {
    let (suggestions, scope) = collect_suggestions(&opts).await?;

    if opts.json {
        // The scope note goes to stderr so stdout stays the suggestions array that callers parse.
        eprintln!("{scope}");
        let rendered = serde_json::to_string_pretty(&suggestions)
            .into_diagnostic()
            .wrap_err("Failed to serialize suggestions")?;
        println!("{rendered}");
        return Ok(());
    }

    if suggestions.is_empty() {
        println!("# {scope}");
        println!("# No commands matched (try lowering --min-count or widening the time range).");
        return Ok(());
    }

    let config = UserConfig {
        pattern_fragments: None,
        bash_rules: Some(suggestions.iter().map(|s| s.rule.clone()).collect()),
        tool_rules: None,
    };
    let toml = toml::to_string_pretty(&config)
        .into_diagnostic()
        .wrap_err("Failed to render suggestions")?;

    println!("# Suggested rules from the hook logs. Review before adding them to tool_rules.toml.");
    println!("# {scope}");
    println!("# Re-run with --json to see how often each command was seen.");
    println!();
    print!("{toml}");
    Ok(())
}

/// When `--action` is unspecified, mirror the mined outcome: deny-derived suggestions default to
/// Deny, everything else to Ask (so a frequently-prompted command becomes an explicit decision).
fn default_action(result: PreToolResult) -> SuggestAction {
    match result {
        PreToolResult::Deny => SuggestAction::Deny,
        _ => SuggestAction::Ask,
    }
}

fn match_mode_label(mode: MatchMode) -> &'static str {
    match mode {
        MatchMode::Exact => "exact",
        MatchMode::Prefix => "prefix",
        MatchMode::Fuzzy => "fuzzy",
    }
}

/// Accumulates everything observed for one leaf text (or one generated pattern): the summed
/// occurrence count and the full commands it came from, kept sorted for deterministic output.
#[derive(Default)]
struct ObservedGroup {
    count: u64,
    observed: BTreeSet<String>,
}

impl ObservedGroup {
    fn absorb(&mut self, count: u64, observed_command: &str) {
        self.count += count;
        self.observed.insert(observed_command.to_string());
    }

    fn merge(&mut self, other: &ObservedGroup) {
        self.count += other.count;
        self.observed.extend(other.observed.iter().cloned());
    }
}

/// True when the current `BashRuleEngine` already allows a leaf and the leaf doesn't write to a
/// real file — i.e. the leaf won't prompt the user even without a new rule. `real_file_write` caps
/// Allow → Ask in the compound engine, so a write-redirecting leaf must stay in the suggestions.
fn is_already_allowed(engine: Option<&BashRuleEngine>, text: &str, real_file_write: bool) -> bool {
    match engine {
        Some(engine) => {
            !real_file_write && matches!(engine.apply_rules(text), RuleResult::Allowed { .. })
        }
        None => false,
    }
}

/// Pure core of `suggest`: turns aggregated Bash rows into anchored rule suggestions.
///
/// Each observed command is first split into the leaf simple-commands the hook actually evaluates
/// (with the row's recorded cwd, so leaves are normalized exactly as they were matched), because a
/// whole-compound literal like `^git status && ls$` can never match the per-leaf engine. Counts for
/// the same leaf are summed across every compound it appeared in. A command the splitter bails on is
/// kept whole — the hook cannot analyze it either, so only a whole-command pattern is meaningful.
fn build_suggestions(
    rows: &[ReportRow],
    match_mode: MatchMode,
    action: SuggestAction,
    min_count: u64,
    limit: usize,
    engine: Option<&BashRuleEngine>,
) -> Vec<Suggestion> {
    let mut leaves: BTreeMap<String, ObservedGroup> = BTreeMap::new();
    for row in rows {
        let Some(command) = row.bash_command() else {
            continue;
        };
        // A leaf repeated within one compound counts once per recorded occurrence, not once per
        // appearance, so the count still reflects how often the user saw a prompt.
        let texts: BTreeSet<String> = match split_command(command, &row.cwd) {
            SplitOutcome::Commands(parts) => parts
                .into_iter()
                .filter(|leaf| !is_already_allowed(engine, &leaf.text, leaf.real_file_write))
                .map(|leaf| leaf.text)
                .collect(),
            SplitOutcome::Bail(_) => BTreeSet::from([command.to_string()]),
        };
        for text in texts {
            leaves.entry(text).or_default().absorb(row.count, command);
        }
    }

    // One candidate per generated pattern: exact mode yields one per leaf; prefix/fuzzy cluster all
    // of a program's leaves into one pattern, merging their counts and observed commands.
    let mut patterns: BTreeMap<String, (String, ObservedGroup)> = BTreeMap::new();
    match match_mode {
        MatchMode::Exact => {
            for (text, group) in &leaves {
                let pattern = format!("^{}$", regex::escape(text));
                let entry = patterns
                    .entry(pattern)
                    .or_insert_with(|| (program_token(text), ObservedGroup::default()));
                entry.1.merge(group);
            }
        }
        MatchMode::Prefix => {
            for (program, cluster) in cluster_by_program(&leaves) {
                let pattern = format!(r"^{}(\s|$)", regex::escape(&program));
                let entry = patterns
                    .entry(pattern)
                    .or_insert_with(|| (program_token(&program), ObservedGroup::default()));
                entry.1.merge(&cluster.merged);
            }
        }
        MatchMode::Fuzzy => {
            for (program, cluster) in cluster_by_program(&leaves) {
                let pattern = fuzzy_pattern(&program, &cluster.subcommands);
                let entry = patterns
                    .entry(pattern)
                    .or_insert_with(|| (program_token(&program), ObservedGroup::default()));
                entry.1.merge(&cluster.merged);
            }
        }
    }

    let mut suggestions: Vec<Suggestion> = patterns
        .into_iter()
        .filter(|(_, (_, group))| group.count >= min_count)
        .map(|(pattern, (program, group))| Suggestion {
            rule: BashRule {
                name: format!("suggested-{}-{}", program, short_hash(&pattern)),
                pattern,
                action: to_bash_action(action),
            },
            count: group.count,
            observed_commands: group.observed.into_iter().collect(),
        })
        .collect();

    // Most frequent first; the pattern breaks ties so output is deterministic.
    suggestions.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.rule.pattern.cmp(&b.rule.pattern))
    });
    suggestions.truncate(limit);
    suggestions
}

/// One program's worth of observed leaves for prefix/fuzzy clustering.
#[derive(Default)]
struct ProgramCluster {
    merged: ObservedGroup,
    /// The second token of every leaf, or `None` for a leaf with no second token (bare program) or
    /// one whose tokens cannot be parsed — either disqualifies the cluster from an alternation.
    subcommands: Vec<Option<String>>,
}

/// Groups leaves by their program (first token). Leaves that cannot be tokenized are dropped: with
/// no reliable program there is nothing safe to anchor a generalized pattern on.
fn cluster_by_program(
    leaves: &BTreeMap<String, ObservedGroup>,
) -> BTreeMap<String, ProgramCluster> {
    let mut clusters: BTreeMap<String, ProgramCluster> = BTreeMap::new();
    for (text, group) in leaves {
        let Ok(tokens) = shell_words::split(text) else {
            continue;
        };
        let Some(program) = tokens.first().filter(|program| !program.is_empty()) else {
            continue;
        };
        let cluster = clusters.entry(program.clone()).or_default();
        cluster.merged.merge(group);
        cluster.subcommands.push(tokens.get(1).cloned());
    }
    clusters
}

/// Generalizes one program's observed leaves: when every leaf has a simple-identifier second token
/// (`cargo build`, `cargo check`), the distinct subcommands form a closed alternation; any bare
/// invocation, flag, path, or other non-identifier second token falls back to a program prefix. The
/// alternation never widens beyond tokens that were actually observed.
fn fuzzy_pattern(program: &str, subcommands: &[Option<String>]) -> String {
    static IDENTIFIER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new("^[a-zA-Z][a-zA-Z0-9_-]*$").expect("static regex is valid"));
    let all_safe = !subcommands.is_empty()
        && subcommands
            .iter()
            .all(|sub| sub.as_deref().is_some_and(|sub| IDENTIFIER.is_match(sub)));
    if !all_safe {
        return format!(r"^{}(\s|$)", regex::escape(program));
    }

    let distinct: BTreeSet<&str> = subcommands.iter().flatten().map(String::as_str).collect();
    let alternation = distinct
        .into_iter()
        .map(regex::escape)
        .collect::<Vec<_>>()
        .join("|");
    format!(r"^{} ({})(\s|$)", regex::escape(program), alternation)
}

/// The command's program basename, for a readable rule name (`/usr/bin/ls` → `ls`).
fn program_token(command: &str) -> String {
    shell_words::split(command)
        .ok()
        .and_then(|parts| parts.into_iter().next())
        .map(|program| program.rsplit('/').next().unwrap_or(&program).to_string())
        .filter(|program| !program.is_empty())
        .unwrap_or_else(|| "cmd".to_string())
}

/// A short, stable hash of the full command, disambiguating rule names for the same program.
fn short_hash(command: &str) -> String {
    crate::hashing::hash_string(command)
        .strip_prefix("sha256:")
        .unwrap_or_default()
        .chars()
        .take(8)
        .collect()
}

fn to_bash_action(action: SuggestAction) -> BashRuleAction {
    match action {
        SuggestAction::Ask => BashRuleAction::Ask,
        SuggestAction::Allow => BashRuleAction::Allow,
        SuggestAction::Deny => BashRuleAction::Deny {
            value: "Suggested deny — review before keeping".to_string(),
        },
    }
}

// ===== rules replay =====

/// One recorded Bash command whose recomputed decision differs from what was logged.
#[derive(Debug, Serialize, PartialEq, Eq)]
struct ReplayRow {
    command: String,
    recorded: PreToolResult,
    computed: PreToolResult,
    count: u64,
    /// `lost-allow` (regression), `newly-allowed` (intended improvement), or `changed`.
    classification: &'static str,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct ReplayReport {
    /// Only the commands whose decision changed; unchanged commands are omitted.
    divergences: Vec<ReplayRow>,
    /// Previously-Allowed commands that the candidate no longer auto-allows (the regression gate).
    lost_allow_count: usize,
    /// Commands the candidate now auto-allows that previously prompted/denied (the migration goal).
    newly_allowed_count: usize,
    /// Total recorded commands evaluated (after any `--result` filter).
    total_evaluated: usize,
    /// Hash of the rule set whose recorded decisions were replayed (`None` under `--all-rules`).
    replayed_rules_hash: Option<String>,
    /// Records excluded because they came from a different rule set.
    skipped_other_rules: u64,
    /// Records excluded because they predate rule-hash logging.
    skipped_no_hash: u64,
}

async fn replay(opts: ReplayOptions) -> Result<()> {
    let config = load_user_config_from(opts.config.as_deref()).await?;
    let engine = BashRuleEngine::from_config(
        config.bash_rules.unwrap_or_default(),
        config.pattern_fragments,
    )?;

    // Replay defaults to all recorded history, but `--start-time`/`--end-time` bound the window so a
    // long-lived log doesn't force every candidate to clear every command ever run.
    let filter = TimeRangeFilter::new(
        opts.start_time,
        opts.end_time,
        crate::cost_report::parse_timezone(&opts.timezone)?,
    )?;
    let (hash_filter, replayed_hash) = resolve_hash_filter(opts.all_rules, opts.rules_hash).await?;
    let CwdAggregation { rows, skipped } =
        aggregate_with_cwd(opts.dir, &filter, Some("Bash"), None, &hash_filter).await?;
    let report = build_replay_report(&rows, &engine, opts.result, replayed_hash, skipped);

    if opts.json {
        let rendered = serde_json::to_string_pretty(&report)
            .into_diagnostic()
            .wrap_err("Failed to serialize replay report")?;
        println!("{rendered}");
    } else {
        print_replay(&report);
    }

    // The migration acceptance gate: a candidate that drops a previously-auto-approved command is a
    // regression, so fail loudly for CI/scripts.
    if report.lost_allow_count > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// Pure core of `replay`: recompute each recorded Bash command through the candidate engine and
/// classify the divergences. Each row carries the cwd it ran under so commands normalize exactly as
/// the hook did; rows from before cwd was logged fall back to an empty cwd (normalization disabled).
fn build_replay_report(
    rows: &[ReportRow],
    engine: &BashRuleEngine,
    result_filter: Option<PreToolResult>,
    replayed_rules_hash: Option<String>,
    skipped: crate::hooks::report::HashSkipStats,
) -> ReplayReport {
    let mut divergences = Vec::new();
    let mut lost_allow_count = 0;
    let mut newly_allowed_count = 0;
    let mut total_evaluated = 0;

    for row in rows {
        if result_filter.is_some_and(|filter| filter != row.result) {
            continue;
        }
        let Some(command) = row.bash_command() else {
            continue;
        };
        total_evaluated += 1;

        let computed = classify_result(&engine.apply_rules_compound(command, &row.cwd));
        if computed == row.result {
            continue;
        }

        let classification = if row.result == PreToolResult::Allow {
            lost_allow_count += 1;
            "lost-allow"
        } else if computed == PreToolResult::Allow {
            newly_allowed_count += 1;
            "newly-allowed"
        } else {
            "changed"
        };

        divergences.push(ReplayRow {
            command: command.to_string(),
            recorded: row.result,
            computed,
            count: row.count,
            classification,
        });
    }

    ReplayReport {
        divergences,
        lost_allow_count,
        newly_allowed_count,
        total_evaluated,
        replayed_rules_hash,
        skipped_other_rules: skipped.other_rules,
        skipped_no_hash: skipped.no_hash,
    }
}

/// Maps an engine decision to the same `PreToolResult` the hook would log: a bash `NoMatch` becomes
/// `Ask` (the hook prompts), and `ArgumentFiltered` becomes `Modify`.
fn classify_result(result: &RuleResult) -> PreToolResult {
    match result {
        RuleResult::Allowed { .. } => PreToolResult::Allow,
        RuleResult::Denied { .. } => PreToolResult::Deny,
        RuleResult::Modified { .. } | RuleResult::ArgumentFiltered { .. } => PreToolResult::Modify,
        RuleResult::Asked { .. } | RuleResult::NoMatch => PreToolResult::Ask,
    }
}

fn print_replay(report: &ReplayReport) {
    match &report.replayed_rules_hash {
        Some(hash) => println!(
            "Rule set: {hash}; skipped {} record(s) from other rule sets and {} predating rule-hash \
             logging (use --all-rules to include them).",
            report.skipped_other_rules, report.skipped_no_hash
        ),
        None => println!("Rule set: all (--all-rules); no hash filter applied."),
    }
    println!(
        "Replayed {} recorded Bash command(s) against the candidate config.",
        report.total_evaluated
    );
    println!(
        "  Lost auto-approval (regression): {}",
        report.lost_allow_count
    );
    println!(
        "  Newly auto-allowed (improvement): {}",
        report.newly_allowed_count
    );

    if report.divergences.is_empty() {
        println!("\nNo decisions changed.");
    } else {
        println!();
        for row in &report.divergences {
            println!(
                "  [{}] {} → {} (×{}): {}",
                row.classification,
                row.recorded.as_str(),
                row.computed.as_str(),
                row.count,
                row.command
            );
        }
    }

    if report.lost_allow_count > 0 {
        println!(
            "\nFAIL: {} previously-auto-approved command(s) would now prompt or be denied.",
            report.lost_allow_count
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::user_config::{BashRule, BashRuleAction, ToolRule, ToolRuleAction};
    use std::env;

    fn config_with_bash(rules: Vec<BashRule>) -> UserConfig {
        UserConfig {
            pattern_fragments: None,
            bash_rules: Some(rules),
            tool_rules: None,
        }
    }

    fn allow(name: &str, pattern: &str) -> BashRule {
        BashRule {
            name: name.to_string(),
            pattern: pattern.to_string(),
            action: BashRuleAction::Allow,
        }
    }

    #[test]
    fn reports_invalid_regex_as_error() {
        let config = config_with_bash(vec![allow("bad", "[invalid(")]);
        let report = build_lint_report(&config, false).unwrap();
        assert_eq!(report.ignored_count, 1);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].kind, "invalid-regex");
        assert_eq!(report.errors[0].rule_kind, "bash");
    }

    #[test]
    fn reports_undefined_fragment_as_error() {
        let config = config_with_bash(vec![allow("frag", "^{{nope}}")]);
        let report = build_lint_report(&config, false).unwrap();
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].kind, "undefined-fragment");
    }

    #[test]
    fn reports_tool_rule_missing_field_pattern() {
        let config = UserConfig {
            pattern_fragments: None,
            bash_rules: None,
            tool_rules: Some(vec![ToolRule {
                name: "half".to_string(),
                tool: "Read".to_string(),
                allow_local: false,
                field: Some("path".to_string()),
                pattern: None,
                action: ToolRuleAction::Allow,
            }]),
        };
        let report = build_lint_report(&config, false).unwrap();
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].rule_kind, "tool");
        assert_eq!(report.errors[0].kind, "missing-field-or-pattern");
    }

    #[test]
    fn clean_config_has_no_errors() {
        let config = config_with_bash(vec![allow("ls", r"^ls($|\s)")]);
        let report = build_lint_report(&config, true).unwrap();
        assert_eq!(report.ignored_count, 0);
        assert!(report.errors.is_empty());
    }

    #[test]
    fn strict_flags_over_broad_allow() {
        let config = config_with_bash(vec![allow("any", ".*")]);
        let report = build_lint_report(&config, true).unwrap();
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.kind == "over-broad-allow")
        );
        // Over-broad is a warning, not a dropped rule.
        assert_eq!(report.ignored_count, 0);
    }

    #[test]
    fn strict_flags_shadowed_rule() {
        // `^ls` matches everything `^ls -la` does, so the later, more specific rule is unreachable.
        let config = config_with_bash(vec![
            allow("broad-ls", r"^ls"),
            allow("specific-ls", r"^ls -la$"),
        ]);
        let report = build_lint_report(&config, true).unwrap();
        let shadowed: Vec<_> = report
            .warnings
            .iter()
            .filter(|warning| warning.kind == "shadowed")
            .collect();
        assert_eq!(shadowed.len(), 1, "warnings: {:?}", report.warnings);
        assert_eq!(shadowed[0].rule_name, "specific-ls");
    }

    #[test]
    fn non_strict_emits_no_warnings() {
        let config = config_with_bash(vec![allow("any", ".*")]);
        let report = build_lint_report(&config, false).unwrap();
        assert!(report.warnings.is_empty());
    }

    // ===== list-fragments / schema / starter =====

    #[test]
    fn fragment_rows_mark_default_and_user_sources() {
        let mut user = HashMap::new();
        user.insert("safe_chars".to_string(), "[a-z]".to_string()); // overrides a default
        user.insert("my_custom".to_string(), "xyz".to_string()); // user-only
        let rows = fragment_rows(Some(&user));

        let find = |name: &str| rows.iter().find(|row| row.name == name).unwrap();
        assert_eq!(find("safe_chars").source, "user");
        assert_eq!(find("safe_chars").value, "[a-z]");
        assert_eq!(find("my_custom").source, "user");
        assert_eq!(find("identifier").source, "default");
    }

    #[test]
    fn schema_round_trips_through_user_config() {
        // Guards the canonical example against drift from the config types.
        toml::from_str::<UserConfig>(SCHEMA_TOML).expect("schema TOML must parse");
    }

    #[test]
    fn starter_rules_round_trip_through_toml() {
        let config = UserConfig {
            pattern_fragments: None,
            bash_rules: Some(starter_rules()),
            tool_rules: None,
        };
        let toml = toml::to_string_pretty(&config).unwrap();
        let parsed: UserConfig = toml::from_str(&toml).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn starter_pack_auto_allows_the_north_star_command() {
        use crate::hooks::bash_rules::RuleResult;

        const NORTH_STAR: &str = r#"echo "===== Is there a lib.rs? =====" && ls crates/moriarty/src/lib.rs 2>/dev/null && echo "FOUND lib.rs" || echo "NO lib.rs (binary only via main.rs)"; echo; echo "===== Cargo.toml deps =====" && cat crates/moriarty/Cargo.toml; echo; cat Cargo.toml 2>/dev/null | head -60"#;

        let engine = BashRuleEngine::from_config(starter_rules(), None).unwrap();
        assert!(
            matches!(
                engine.apply_rules_compound(NORTH_STAR, ""),
                RuleResult::Allowed { .. }
            ),
            "starter pack should auto-allow the north-star command"
        );
    }

    // ===== suggest =====

    fn bash_row(command: &str, count: u64, result: PreToolResult) -> ReportRow {
        bash_row_in(command, count, result, "")
    }

    fn bash_row_in(command: &str, count: u64, result: PreToolResult, cwd: &str) -> ReportRow {
        ReportRow {
            tool_name: "Bash".to_string(),
            arguments: serde_json::json!({ "command": command }),
            result,
            count,
            rule: None,
            cwd: cwd.to_string(),
        }
    }

    #[test]
    fn suggest_exact_produces_fully_anchored_literal() {
        let rows = vec![bash_row("ls -la", 3, PreToolResult::Ask)];
        let suggestions =
            build_suggestions(&rows, MatchMode::Exact, SuggestAction::Ask, 2, 10, None);

        assert_eq!(suggestions.len(), 1);
        let suggestion = &suggestions[0];
        assert_eq!(suggestion.count, 3);
        assert_eq!(suggestion.observed_commands, vec!["ls -la".to_string()]);
        assert!(suggestion.rule.name.starts_with("suggested-ls-"));
        assert!(matches!(suggestion.rule.action, BashRuleAction::Ask));

        let regex = Regex::new(&suggestion.rule.pattern).unwrap();
        assert!(regex.is_match("ls -la"));
        assert!(
            !regex.is_match("ls -la extra"),
            "exact must not match a superstring"
        );
        assert!(!regex.is_match("ls"));
    }

    #[test]
    fn suggest_prefix_anchors_on_program_token() {
        let rows = vec![bash_row("cargo build --release", 5, PreToolResult::Ask)];
        let suggestions =
            build_suggestions(&rows, MatchMode::Prefix, SuggestAction::Ask, 2, 10, None);

        let regex = Regex::new(&suggestions[0].rule.pattern).unwrap();
        assert!(regex.is_match("cargo build --release"));
        assert!(regex.is_match("cargo test"));
        assert!(
            !regex.is_match("cargolike"),
            "prefix must respect a word boundary"
        );
    }

    #[test]
    fn suggest_respects_min_count_and_limit() {
        let rows = vec![
            bash_row("a", 5, PreToolResult::Ask),
            bash_row("b", 1, PreToolResult::Ask), // below min_count
            bash_row("c", 4, PreToolResult::Ask),
            bash_row("d", 3, PreToolResult::Ask),
        ];
        let suggestions =
            build_suggestions(&rows, MatchMode::Exact, SuggestAction::Ask, 2, 2, None);

        assert_eq!(
            suggestions.len(),
            2,
            "min_count drops 'b', limit keeps top 2"
        );
        assert_eq!(suggestions[0].observed_commands, vec!["a".to_string()]);
        assert_eq!(suggestions[1].observed_commands, vec!["c".to_string()]);
    }

    #[test]
    fn default_action_follows_result() {
        assert!(matches!(
            default_action(PreToolResult::Deny),
            SuggestAction::Deny
        ));
        assert!(matches!(
            default_action(PreToolResult::Ask),
            SuggestAction::Ask
        ));
    }

    #[tokio::test]
    async fn suggest_rejects_allow_with_prefix_match() {
        // The guard runs before any log reading, so the (unused) dir is irrelevant.
        let err = suggest(SuggestOptions {
            dir: Some(PathBuf::from("/tmp/moriarty-suggest-guard")),
            start_time: None,
            end_time: None,
            timezone: "utc".to_string(),
            result: PreToolResult::Ask,
            limit: 10,
            min_count: 2,
            match_mode: MatchMode::Prefix,
            action: Some(SuggestAction::Allow),
            all_rules: false,
            rules_hash: None,
            json: false,
        })
        .await
        .expect_err("Allow + prefix must be rejected");
        assert!(err.to_string().contains("Allow rules with --match prefix"));
    }

    #[test]
    fn suggested_rules_round_trip_through_toml() {
        let rows = vec![bash_row("git status", 3, PreToolResult::Ask)];
        let suggestions =
            build_suggestions(&rows, MatchMode::Exact, SuggestAction::Ask, 2, 10, None);
        let config = UserConfig {
            pattern_fragments: None,
            bash_rules: Some(suggestions.iter().map(|s| s.rule.clone()).collect()),
            tool_rules: None,
        };
        let toml = toml::to_string_pretty(&config).unwrap();
        let parsed: UserConfig = toml::from_str(&toml).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn suggest_splits_compounds_into_per_leaf_suggestions() {
        // A whole-compound literal could never match the per-leaf engine, so each leaf gets its own
        // exact suggestion, and the generated pattern must actually allow that leaf when adopted.
        let rows = vec![bash_row("git status && ls -la", 3, PreToolResult::Ask)];
        let suggestions =
            build_suggestions(&rows, MatchMode::Exact, SuggestAction::Allow, 2, 10, None);

        let patterns: Vec<&str> = suggestions
            .iter()
            .map(|suggestion| suggestion.rule.pattern.as_str())
            .collect();
        assert_eq!(patterns, vec![r"^git status$", r"^ls \-la$"]);
        for suggestion in &suggestions {
            assert_eq!(suggestion.count, 3);
            assert_eq!(
                suggestion.observed_commands,
                vec!["git status && ls -la".to_string()]
            );
        }

        let rules: Vec<BashRule> = suggestions.iter().map(|s| s.rule.clone()).collect();
        let engine = BashRuleEngine::from_config(rules, None).unwrap();
        assert!(
            matches!(
                engine.apply_rules_compound("git status && ls -la", ""),
                RuleResult::Allowed { .. }
            ),
            "adopting the suggestions must allow the observed compound"
        );
    }

    #[test]
    fn suggest_sums_counts_for_a_leaf_shared_across_compounds() {
        let rows = vec![
            bash_row("git status && cargo test", 2, PreToolResult::Ask),
            bash_row("git status", 3, PreToolResult::Ask),
        ];
        let suggestions =
            build_suggestions(&rows, MatchMode::Exact, SuggestAction::Ask, 1, 10, None);

        let git = suggestions
            .iter()
            .find(|suggestion| suggestion.rule.pattern == "^git status$")
            .expect("the shared git leaf should be suggested");
        assert_eq!(
            git.count, 5,
            "counts sum across every compound the leaf appeared in"
        );
        assert_eq!(
            git.observed_commands,
            vec![
                "git status".to_string(),
                "git status && cargo test".to_string()
            ],
            "observed commands are reported in sorted order"
        );
    }

    #[test]
    fn suggest_normalizes_leaves_with_the_recorded_cwd() {
        let rows = vec![bash_row_in(
            "cat /work/proj/src/main.rs",
            3,
            PreToolResult::Ask,
            "/work/proj",
        )];
        let suggestions =
            build_suggestions(&rows, MatchMode::Exact, SuggestAction::Ask, 2, 10, None);

        assert_eq!(
            suggestions[0].rule.pattern, r"^cat src/main\.rs$",
            "the leaf is normalized exactly as the hook matched it"
        );
    }

    #[test]
    fn suggest_keeps_a_bailed_command_whole() {
        // The hook cannot analyze a command with a substitution, so suggesting per-leaf pieces would
        // be meaningless; the whole observed text is kept, exactly as before.
        let rows = vec![bash_row("echo $(date)", 4, PreToolResult::Ask)];
        let suggestions =
            build_suggestions(&rows, MatchMode::Exact, SuggestAction::Ask, 2, 10, None);

        assert_eq!(suggestions.len(), 1);
        assert_eq!(
            suggestions[0].rule.pattern,
            format!("^{}$", regex::escape("echo $(date)"))
        );
    }

    #[test]
    fn suggest_prefix_merges_same_program_rows() {
        let rows = vec![
            bash_row("cargo build", 2, PreToolResult::Ask),
            bash_row("cargo check --all", 3, PreToolResult::Ask),
        ];
        let suggestions =
            build_suggestions(&rows, MatchMode::Prefix, SuggestAction::Ask, 2, 10, None);

        assert_eq!(suggestions.len(), 1, "one prefix rule per program");
        assert_eq!(suggestions[0].rule.pattern, r"^cargo(\s|$)");
        assert_eq!(suggestions[0].count, 5);
        assert_eq!(suggestions[0].observed_commands.len(), 2);
    }

    #[test]
    fn suggest_fuzzy_generalizes_safe_subcommands_into_an_alternation() {
        let rows = vec![
            bash_row("cargo build", 2, PreToolResult::Ask),
            bash_row("cargo build --release", 3, PreToolResult::Ask),
            bash_row("cargo check", 2, PreToolResult::Ask),
        ];
        let suggestions =
            build_suggestions(&rows, MatchMode::Fuzzy, SuggestAction::Ask, 2, 10, None);

        assert_eq!(suggestions.len(), 1);
        let suggestion = &suggestions[0];
        assert_eq!(suggestion.rule.pattern, r"^cargo (build|check)(\s|$)");
        assert_eq!(suggestion.count, 7);
        assert_eq!(suggestion.observed_commands.len(), 3);

        let regex = Regex::new(&suggestion.rule.pattern).unwrap();
        assert!(regex.is_match("cargo build"));
        assert!(regex.is_match("cargo build --release"));
        assert!(regex.is_match("cargo check"));
        assert!(
            !regex.is_match("cargo publish"),
            "the alternation is closed"
        );
        assert!(
            !regex.is_match("cargo buildx"),
            "subcommand needs its own boundary"
        );
    }

    #[test]
    fn suggest_fuzzy_falls_back_to_prefix_on_non_identifier_second_token() {
        // `-la` is a flag and `src/main.rs` is a path; neither is a safe-identifier subcommand, so
        // the cluster degrades to a program prefix instead of inventing an alternation over them.
        let rows = vec![
            bash_row("ls -la", 3, PreToolResult::Ask),
            bash_row("cat src/main.rs", 2, PreToolResult::Ask),
        ];
        let suggestions =
            build_suggestions(&rows, MatchMode::Fuzzy, SuggestAction::Ask, 2, 10, None);

        let patterns: Vec<&str> = suggestions
            .iter()
            .map(|suggestion| suggestion.rule.pattern.as_str())
            .collect();
        assert_eq!(patterns, vec![r"^ls(\s|$)", r"^cat(\s|$)"]);
    }

    #[test]
    fn suggest_fuzzy_escapes_regex_metacharacters_in_program_names() {
        // `g++` contains regex metacharacters; the generated pattern must escape them so it
        // compiles and matches only the literal program.
        let rows = vec![
            bash_row("g++ -c main.cpp", 2, PreToolResult::Ask),
            bash_row("g++ -o out main.o", 2, PreToolResult::Ask),
        ];
        let suggestions =
            build_suggestions(&rows, MatchMode::Fuzzy, SuggestAction::Ask, 2, 10, None);

        assert_eq!(suggestions.len(), 1);
        let regex = Regex::new(&suggestions[0].rule.pattern)
            .expect("escaped program must produce a valid regex");
        assert!(regex.is_match("g++ -c main.cpp"));
        assert!(
            !regex.is_match("gxx -c main.cpp"),
            "the + must match literally"
        );
    }

    #[tokio::test]
    async fn suggest_json_smoke_over_an_explicit_dir() {
        // End-to-end through the async path: log reading, hash filtering (--all-rules), leaf
        // splitting, and JSON rendering all compose without error.
        let dir = tempfile::tempdir().unwrap();
        let line = serde_json::json!({
            "timestamp": "2026-06-10T01:00:00Z",
            "fields": {
                "message": "PreToolUse hook completed",
                "tool_name": "Bash",
                "tool_args": "{\"command\":\"git status && ls\"}",
                "cwd": "/tmp",
                "rules_hash": "sha256:test",
                "result": "ask"
            }
        })
        .to_string();
        tokio::fs::write(dir.path().join("hooks.log.2026-06-10"), format!("{line}\n"))
            .await
            .unwrap();

        suggest(SuggestOptions {
            dir: Some(dir.path().to_path_buf()),
            start_time: None,
            end_time: None,
            timezone: "utc".to_string(),
            result: PreToolResult::Ask,
            limit: 10,
            min_count: 1,
            match_mode: MatchMode::Exact,
            action: None,
            all_rules: true,
            rules_hash: None,
            json: true,
        })
        .await
        .expect("suggest should succeed over an explicit log dir");
    }

    #[test]
    fn suggest_fuzzy_bare_program_disqualifies_the_alternation() {
        // A bare `cargo` has no second token; an alternation would stop matching it, so the cluster
        // falls back to the prefix that covers every observed shape.
        let rows = vec![
            bash_row("cargo", 2, PreToolResult::Ask),
            bash_row("cargo build", 2, PreToolResult::Ask),
        ];
        let suggestions =
            build_suggestions(&rows, MatchMode::Fuzzy, SuggestAction::Ask, 2, 10, None);

        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].rule.pattern, r"^cargo(\s|$)");
    }

    #[tokio::test]
    async fn suggest_rejects_allow_with_fuzzy_match() {
        let err = suggest(SuggestOptions {
            dir: Some(PathBuf::from("/tmp/moriarty-suggest-guard")),
            start_time: None,
            end_time: None,
            timezone: "utc".to_string(),
            result: PreToolResult::Ask,
            limit: 10,
            min_count: 2,
            match_mode: MatchMode::Fuzzy,
            action: Some(SuggestAction::Allow),
            all_rules: false,
            rules_hash: None,
            json: false,
        })
        .await
        .expect_err("Allow + fuzzy must be rejected");
        assert!(err.to_string().contains("Allow rules with --match fuzzy"));
    }

    // ===== replay =====

    #[test]
    fn replay_flags_lost_allow_regression() {
        let engine =
            BashRuleEngine::from_config(vec![allow("allow-ls", r"^ls($|\s)")], None).unwrap();
        let rows = vec![
            bash_row("ls -la", 5, PreToolResult::Allow), // still allowed → unchanged
            bash_row("git status", 3, PreToolResult::Allow), // candidate no longer allows → lost
            bash_row("cargo build", 2, PreToolResult::Ask), // still prompts → unchanged
        ];
        let report = build_replay_report(&rows, &engine, None, None, Default::default());

        assert_eq!(report.total_evaluated, 3);
        assert_eq!(report.lost_allow_count, 1);
        let lost: Vec<_> = report
            .divergences
            .iter()
            .filter(|row| row.classification == "lost-allow")
            .collect();
        assert_eq!(lost.len(), 1);
        assert_eq!(lost[0].command, "git status");
    }

    #[test]
    fn replay_reports_newly_allowed_as_improvement() {
        let engine =
            BashRuleEngine::from_config(vec![allow("allow-ls", r"^ls($|\s)")], None).unwrap();
        let rows = vec![bash_row("ls -la", 4, PreToolResult::Ask)]; // was prompted, now allowed
        let report = build_replay_report(&rows, &engine, None, None, Default::default());

        assert_eq!(report.newly_allowed_count, 1);
        assert_eq!(report.lost_allow_count, 0);
        assert_eq!(report.divergences[0].classification, "newly-allowed");
    }

    #[test]
    fn replay_result_filter_limits_scope() {
        let engine =
            BashRuleEngine::from_config(vec![allow("allow-ls", r"^ls($|\s)")], None).unwrap();
        let rows = vec![
            bash_row("ls -la", 4, PreToolResult::Ask), // ask → would be newly-allowed
            bash_row("git x", 2, PreToolResult::Allow), // allow → would be lost-allow
        ];
        // Filtering to recorded=Allow evaluates only the git row.
        let report = build_replay_report(
            &rows,
            &engine,
            Some(PreToolResult::Allow),
            None,
            Default::default(),
        );

        assert_eq!(report.total_evaluated, 1);
        assert_eq!(report.lost_allow_count, 1);
        assert_eq!(report.newly_allowed_count, 0);
    }

    #[test]
    fn replay_empty_rows_yield_an_empty_report() {
        let engine = BashRuleEngine::from_config(vec![], None).unwrap();
        let report = build_replay_report(&[], &engine, None, None, Default::default());
        assert_eq!(report.total_evaluated, 0);
        assert_eq!(report.lost_allow_count, 0);
        assert_eq!(report.newly_allowed_count, 0);
        assert!(report.divergences.is_empty());
    }

    #[test]
    fn replay_skips_rows_without_a_command_field() {
        let engine = BashRuleEngine::from_config(vec![allow("allow-ls", r"^ls")], None).unwrap();
        let rows = vec![ReportRow {
            tool_name: "Bash".to_string(),
            arguments: serde_json::json!({ "not_command": "ls" }),
            result: PreToolResult::Allow,
            count: 3,
            rule: None,
            cwd: String::new(),
        }];
        let report = build_replay_report(&rows, &engine, None, None, Default::default());
        // A Bash row missing its command is skipped, not counted or misclassified.
        assert_eq!(report.total_evaluated, 0);
        assert!(report.divergences.is_empty());
    }

    #[test]
    fn replay_normalizes_in_cwd_absolute_paths_with_the_recorded_cwd() {
        // The hook recorded an Allow for an in-cwd absolute path. A relative-path allow-rule only
        // matches once the command is normalized against the cwd it ran under, so replaying with the
        // recorded cwd must reproduce the Allow (no lost-approval) rather than diverging.
        let engine =
            BashRuleEngine::from_config(vec![allow("allow-cat-src", r"^cat src/")], None).unwrap();
        let rows = vec![bash_row_in(
            "cat /work/proj/src/main.rs",
            1,
            PreToolResult::Allow,
            "/work/proj",
        )];

        let report = build_replay_report(&rows, &engine, None, None, Default::default());

        assert_eq!(
            report.lost_allow_count, 0,
            "cwd normalization reproduces the Allow"
        );
        assert!(report.divergences.is_empty());
    }

    #[test]
    fn replay_without_recorded_cwd_skips_normalization() {
        // A legacy row (empty cwd) cannot be normalized, so the same absolute-path command no longer
        // matches the relative allow-rule and is reported as a lost auto-approval.
        let engine =
            BashRuleEngine::from_config(vec![allow("allow-cat-src", r"^cat src/")], None).unwrap();
        let rows = vec![bash_row_in(
            "cat /work/proj/src/main.rs",
            1,
            PreToolResult::Allow,
            "",
        )];

        let report = build_replay_report(&rows, &engine, None, None, Default::default());

        assert_eq!(report.lost_allow_count, 1);
    }

    #[test]
    fn suggest_empty_rows_yield_no_suggestions() {
        assert!(
            build_suggestions(&[], MatchMode::Exact, SuggestAction::Ask, 2, 10, None).is_empty()
        );
    }

    #[test]
    fn suggest_filters_already_allowed_leaves() {
        // Already-allowed leaves wouldn't prompt the user, so suggesting a rule for them would
        // be noise.
        let engine =
            BashRuleEngine::from_config(vec![allow("allow-head", r"^head($|\s)")], None).unwrap();
        let rows = vec![
            bash_row("head -5 Cargo.toml", 4, PreToolResult::Ask),
            bash_row("tail -20 Cargo.toml", 3, PreToolResult::Ask),
        ];
        let suggestions = build_suggestions(
            &rows,
            MatchMode::Exact,
            SuggestAction::Ask,
            1,
            10,
            Some(&engine),
        );

        assert_eq!(suggestions.len(), 1);
        let suggestion = &suggestions[0];
        assert_eq!(suggestion.rule.pattern, r"^tail \-20 Cargo\.toml$",);
        assert_eq!(suggestion.count, 3);
    }

    #[test]
    fn suggest_filters_leaves_in_a_compound_command() {
        let engine =
            BashRuleEngine::from_config(vec![allow("allow-head", r"^head($|\s)")], None).unwrap();
        let rows = vec![bash_row(
            "head -5 Cargo.toml && tail -20 Cargo.toml",
            4,
            PreToolResult::Ask,
        )];
        let suggestions = build_suggestions(
            &rows,
            MatchMode::Exact,
            SuggestAction::Ask,
            1,
            10,
            Some(&engine),
        );

        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].rule.pattern, r"^tail \-20 Cargo\.toml$",);
        assert_eq!(suggestions[0].count, 4);
    }

    #[test]
    fn suggest_keeps_real_file_write_leaves_even_when_allowed() {
        // The compound engine caps Allow → Ask when a leaf writes to a real file, so those
        // leaves still reach the user. The filter must not drop them even when `apply_rules`
        // alone says Allowed.
        let engine =
            BashRuleEngine::from_config(vec![allow("allow-echo", r"^echo($|\s)")], None).unwrap();
        let rows = vec![bash_row("echo hi > output.txt", 5, PreToolResult::Ask)];
        let suggestions = build_suggestions(
            &rows,
            MatchMode::Exact,
            SuggestAction::Ask,
            1,
            10,
            Some(&engine),
        );

        assert_eq!(
            suggestions.len(),
            1,
            "real-file-write leaf must survive the filter"
        );
        assert_eq!(suggestions[0].rule.pattern, r"^echo hi > output\.txt$",);
    }

    #[test]
    fn suggest_keeps_bailed_commands_even_when_allowed() {
        // A bailed command (e.g., one with command substitution) is never `Allowed` by the
        // compound engine — it is capped to Ask — so `apply_rules()` alone returning `Allowed`
        // for a bailed leaf must not filter it out.
        let engine =
            BashRuleEngine::from_config(vec![allow("allow-echo", r"^echo($|\s)")], None).unwrap();
        let rows = vec![bash_row("echo $(date)", 4, PreToolResult::Ask)];
        let suggestions = build_suggestions(
            &rows,
            MatchMode::Exact,
            SuggestAction::Ask,
            1,
            10,
            Some(&engine),
        );

        assert_eq!(suggestions.len(), 1);
        assert_eq!(
            suggestions[0].rule.pattern,
            format!("^{}$", regex::escape("echo $(date)"))
        );
    }

    #[tokio::test]
    async fn suggest_with_active_config_filters_already_allowed_leaves() {
        // Uses --all-rules to skip the record-hash filter so the test doesn't need to match
        // the config's effective hash, while still exercising the engine-loading codepath.
        let _config_guard = crate::test_helpers::setup_isolated_xdg_config();
        let moriarty_dir = PathBuf::from(env::var("XDG_CONFIG_HOME").unwrap()).join("moriarty");
        tokio::fs::create_dir_all(&moriarty_dir).await.unwrap();
        tokio::fs::write(
            moriarty_dir.join("tool_rules.toml"),
            r#"[[bash_rules]]
name = "allow-head"
pattern = "^head($|\\s)"
action = { type = "Allow" }
"#,
        )
        .await
        .unwrap();

        let log_dir = tempfile::tempdir().unwrap();
        let line = serde_json::json!({
            "timestamp": "2026-06-10T01:00:00Z",
            "fields": {
                "message": "PreToolUse hook completed",
                "tool_name": "Bash",
                "tool_args": r#"{"command":"head -5 Cargo.toml"}"#,
                "cwd": "/tmp",
                "rules_hash": "sha256:test",
                "result": "ask"
            }
        })
        .to_string();
        let line2 = serde_json::json!({
            "timestamp": "2026-06-10T02:00:00Z",
            "fields": {
                "message": "PreToolUse hook completed",
                "tool_name": "Bash",
                "tool_args": r#"{"command":"tail -20 Cargo.toml"}"#,
                "cwd": "/tmp",
                "rules_hash": "sha256:test",
                "result": "ask"
            }
        })
        .to_string();
        tokio::fs::write(
            log_dir.path().join("hooks.log.2026-06-10"),
            format!("{line}\n{line2}\n"),
        )
        .await
        .unwrap();

        let opts = SuggestOptions {
            dir: Some(log_dir.path().to_path_buf()),
            start_time: None,
            end_time: None,
            timezone: "utc".to_string(),
            result: PreToolResult::Ask,
            limit: 10,
            min_count: 1,
            match_mode: MatchMode::Exact,
            action: None,
            all_rules: true,
            rules_hash: None,
            json: true,
        };
        let (suggestions, _scope) = collect_suggestions(&opts).await.unwrap();

        assert_eq!(
            suggestions.len(),
            1,
            "already-allowed head must be filtered"
        );
        assert!(
            suggestions[0].rule.pattern.contains("tail"),
            "only tail should survive the filter"
        );
        assert!(
            suggestions.iter().all(|s| !s.rule.pattern.contains("head")),
            "no head suggestion should appear"
        );
    }

    #[tokio::test]
    async fn suggest_empty_when_all_leaves_are_already_allowed() {
        let _config_guard = crate::test_helpers::setup_isolated_xdg_config();
        let moriarty_dir = PathBuf::from(env::var("XDG_CONFIG_HOME").unwrap()).join("moriarty");
        tokio::fs::create_dir_all(&moriarty_dir).await.unwrap();
        tokio::fs::write(
            moriarty_dir.join("tool_rules.toml"),
            r#"[[bash_rules]]
name = "allow-head"
pattern = "^head($|\\s)"
action = { type = "Allow" }
"#,
        )
        .await
        .unwrap();

        let log_dir = tempfile::tempdir().unwrap();
        let line = serde_json::json!({
            "timestamp": "2026-06-10T01:00:00Z",
            "fields": {
                "message": "PreToolUse hook completed",
                "tool_name": "Bash",
                "tool_args": r#"{"command":"head -5 Cargo.toml"}"#,
                "cwd": "/tmp",
                "rules_hash": "sha256:test",
                "result": "ask"
            }
        })
        .to_string();
        tokio::fs::write(
            log_dir.path().join("hooks.log.2026-06-10"),
            format!("{line}\n"),
        )
        .await
        .unwrap();

        let opts = SuggestOptions {
            dir: Some(log_dir.path().to_path_buf()),
            start_time: None,
            end_time: None,
            timezone: "utc".to_string(),
            result: PreToolResult::Ask,
            limit: 10,
            min_count: 1,
            match_mode: MatchMode::Exact,
            action: None,
            all_rules: true,
            rules_hash: None,
            json: true,
        };
        let (suggestions, _scope) = collect_suggestions(&opts).await.unwrap();

        assert!(
            suggestions.is_empty(),
            "all-head-allowed: no suggestions expected, got {:?}",
            suggestions
        );
    }

    #[test]
    fn lint_empty_rule_lists_have_no_findings() {
        // Some(vec![]) (an explicitly-empty list) must not error, unlike a dropped rule.
        let config = UserConfig {
            pattern_fragments: None,
            bash_rules: Some(vec![]),
            tool_rules: Some(vec![]),
        };
        let report = build_lint_report(&config, true).unwrap();
        assert_eq!(report.ignored_count, 0);
        assert!(report.errors.is_empty());
        assert!(report.warnings.is_empty());
    }
}
