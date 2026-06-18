//! `hooks report` — aggregate recorded PreToolUse hook results into a JSON report.
//!
//! Reads the hooks tracing logs, keeps the completed PreToolUse records (those carrying the
//! clean `result` field written by [`super::result`]), and groups them by the exact
//! `(tool name, arguments, result, deciding rule)` key so each row reports how often that exact
//! call occurred. Output is JSON on stdout; nothing else is written there.

// standard library
use std::{
    collections::HashMap,
    io::ErrorKind,
    path::{Path, PathBuf},
};

// 3rd party crates
use chrono::{DateTime, Utc};
use miette::{IntoDiagnostic, Result, WrapErr};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// local / workspace deps
use super::result::PreToolResult;
use crate::cost_report::TimeRangeFilter;
use crate::persistence::FileType;

const COMPLETION_MESSAGE: &str = "PreToolUse hook completed";

/// Tracing-subscriber's per-line JSON envelope. Only the fields the report needs are modeled and
/// `deny_unknown_fields` is intentionally omitted: the envelope schema (`level`, `target`,
/// `filename`, `line_number`, `threadId`, and other per-event `fields` keys such as `hook_output`)
/// is owned by tracing-subscriber, not by this codebase.
#[derive(Debug, Deserialize)]
struct LogEnvelope {
    timestamp: DateTime<Utc>,
    fields: LogEventFields,
}

#[derive(Debug, Deserialize)]
struct LogEventFields {
    message: String,
    tool_name: Option<String>,
    tool_args: Option<String>,
    result: Option<PreToolResult>,
    // Provenance the rules commands need; optional because lines predating each field still parse.
    cwd: Option<String>,
    rules_hash: Option<String>,
    rule: Option<String>,
}

struct HookRecord {
    timestamp: DateTime<Utc>,
    tool_name: String,
    tool_args: String,
    result: PreToolResult,
    /// The hook's working directory, used by the rules path to normalize commands as the hook did.
    cwd: Option<String>,
    /// Hash of the rule set in force when this decision was made (see [`crate::rules`]).
    rules_hash: Option<String>,
    /// Name of the rule whose action produced the decision; `None` when no rule decided.
    rule: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
pub(crate) struct ReportRow {
    pub(crate) tool_name: String,
    pub(crate) arguments: Value,
    pub(crate) result: PreToolResult,
    pub(crate) count: u64,
    /// The rule that decided these calls. Part of the grouping key, so one row never mixes
    /// decisions from different rules; omitted from the JSON when no rule decided (legacy lines and
    /// passthrough/unconfigured outcomes), keeping those rows' serialization unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rule: Option<String>,
    /// The cwd these calls ran under. Skipped in `hooks report` output, which groups without cwd so
    /// its rows and counts are unchanged; populated (and part of the grouping key) only for the
    /// rules path via [`aggregate_with_cwd`]. Empty string for the `hooks report` grouping.
    #[serde(skip)]
    pub(crate) cwd: String,
}

impl ReportRow {
    /// The recorded Bash command, when this row is a Bash call whose arguments kept the wire shape.
    ///
    /// `arguments` deliberately stays a raw [`Value`] rather than a typed per-tool struct: the
    /// report aggregates every tool's input verbatim, and a log line truncated past the size cap
    /// degrades to a plain string — exactly the rows this accessor must skip gracefully rather
    /// than reject at parse time. Routing all Bash-command extraction through here keeps "row had
    /// no usable command" a single named code path instead of scattered `.get("command")` probes.
    pub(crate) fn bash_command(&self) -> Option<&str> {
        if self.tool_name != "Bash" {
            return None;
        }
        self.arguments.get("command").and_then(Value::as_str)
    }
}

/// Restricts the rules path to records produced by a particular rule set, so `rules replay`/`rules
/// suggest` reason only about the rules in force rather than the union of every historical config.
pub(crate) enum RulesHashFilter {
    /// Keep only records stamped with this exact rule-set hash.
    Only(String),
    /// Keep every record regardless of its rule-set hash (`--all-rules`).
    Any,
}

/// Records dropped by a [`RulesHashFilter::Only`] pass, surfaced so callers can report (never hide)
/// how much history a hash filter excluded.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct HashSkipStats {
    /// Dropped because the record's `rules_hash` differs from the requested one.
    pub(crate) other_rules: u64,
    /// Dropped because the record predates rule-hash logging (no `rules_hash`).
    pub(crate) no_hash: u64,
}

/// Rows from [`aggregate_with_cwd`] plus the hash-filter skip accounting.
pub(crate) struct CwdAggregation {
    pub(crate) rows: Vec<ReportRow>,
    pub(crate) skipped: HashSkipStats,
}

/// Grouping key for [`build_rows`]: tool name, raw `tool_args`, result, deciding rule, cwd.
type RowKey = (String, String, PreToolResult, Option<String>, String);

pub async fn run(
    dir: Option<PathBuf>,
    start_time: Option<String>,
    end_time: Option<String>,
    tool: Option<String>,
    result: Option<PreToolResult>,
    timezone: crate::cost_report::DateTimezone,
) -> Result<()> {
    let filter = TimeRangeFilter::new(start_time, end_time, timezone)?;
    let rows = aggregate(dir, &filter, tool.as_deref(), result).await?;

    let json = serde_json::to_string_pretty(&rows)
        .into_diagnostic()
        .wrap_err("Failed to serialize hook report")?;
    println!("{json}");
    Ok(())
}

/// Reads the hook logs and aggregates them into `(tool, arguments, result)` rows, sorted by count.
/// Used by `hooks report`; `cwd` is not part of the grouping key, so identical calls from different
/// directories merge into one row (the report's historical behavior).
pub(crate) async fn aggregate(
    dir: Option<PathBuf>,
    filter: &TimeRangeFilter,
    tool: Option<&str>,
    result: Option<PreToolResult>,
) -> Result<Vec<ReportRow>> {
    let log_dir = resolve_log_dir(dir).await?;
    let records = read_records(&log_dir).await?;
    Ok(build_rows(records, filter, tool, result, false, &RulesHashFilter::Any).rows)
}

/// The rules path needs per-cwd rows (a command only re-normalizes correctly against the directory
/// it ran under), while [`aggregate`]'s output shape is `hooks report`'s public JSON and must not
/// split rows by directory — hence two entry points instead of one parameterized signature.
/// The returned [`CwdAggregation`] reports how many records `hash_filter` excluded so callers can
/// surface (never hide) what a rule-set filter dropped.
pub(crate) async fn aggregate_with_cwd(
    dir: Option<PathBuf>,
    filter: &TimeRangeFilter,
    tool: Option<&str>,
    result: Option<PreToolResult>,
    hash_filter: &RulesHashFilter,
) -> Result<CwdAggregation> {
    let log_dir = resolve_log_dir(dir).await?;
    let records = read_records(&log_dir).await?;
    Ok(build_rows(records, filter, tool, result, true, hash_filter))
}

async fn resolve_log_dir(dir: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(dir) = dir {
        return Ok(dir);
    }

    let log_file = FileType::State.build_path("hooks/hooks.log").await?;
    log_file
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| miette::miette!("Could not determine the hooks log directory"))
}

async fn read_records(log_dir: &Path) -> Result<Vec<HookRecord>> {
    let mut entries = match tokio::fs::read_dir(log_dir).await {
        Ok(entries) => entries,
        // A missing log directory means no hooks have run yet; an empty report is correct.
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).into_diagnostic().wrap_err_with(|| {
                format!("Failed to read hooks log directory {}", log_dir.display())
            });
        }
    };

    let mut records = Vec::new();
    while let Some(entry) = entries.next_entry().await.into_diagnostic()? {
        // Daily rotation produces `hooks.log.YYYY-MM-DD`; match the whole family.
        if !entry.file_name().to_string_lossy().starts_with("hooks.log") {
            continue;
        }

        let contents = tokio::fs::read_to_string(entry.path())
            .await
            .into_diagnostic()
            .wrap_err_with(|| {
                format!("Failed to read hooks log file {}", entry.path().display())
            })?;

        records.extend(contents.lines().filter_map(parse_record));
    }

    Ok(records)
}

/// Returns a record only for completed PreToolUse lines that carry the clean result field.
///
/// Lines that are not JSON, belong to other events, or predate the result field are skipped so a
/// single odd line never fails the whole report.
fn parse_record(line: &str) -> Option<HookRecord> {
    let envelope: LogEnvelope = serde_json::from_str(line).ok()?;
    if envelope.fields.message != COMPLETION_MESSAGE {
        return None;
    }

    Some(HookRecord {
        timestamp: envelope.timestamp,
        tool_name: envelope.fields.tool_name?,
        tool_args: envelope.fields.tool_args?,
        result: envelope.fields.result?,
        cwd: envelope.fields.cwd,
        // The log writes `""` (not an absent key) for "no rules hash" / "no deciding rule", so an
        // empty value must mean None here — otherwise the hash filter would misclassify a
        // config-load-failure line as belonging to some other rule set.
        rules_hash: envelope.fields.rules_hash.filter(|hash| !hash.is_empty()),
        rule: envelope.fields.rule.filter(|rule| !rule.is_empty()),
    })
}

/// Groups records by `(tool, arguments, result)`. When `include_cwd` is set, the cwd also joins the
/// key (and is carried onto each row) so the rules path sees per-directory rows; otherwise cwd is
/// the empty string and identical calls from different directories merge — `hooks report`'s shape.
/// `hash_filter` drops records from other rule sets (counted in the returned [`HashSkipStats`]); the
/// `hooks report` path passes [`RulesHashFilter::Any`], so it never drops on hash and reports zero.
fn build_rows(
    records: Vec<HookRecord>,
    filter: &TimeRangeFilter,
    tool: Option<&str>,
    result: Option<PreToolResult>,
    include_cwd: bool,
    hash_filter: &RulesHashFilter,
) -> CwdAggregation {
    let mut counts: HashMap<RowKey, u64> = HashMap::new();
    let mut skipped = HashSkipStats::default();

    for record in records {
        if tool.is_some_and(|tool| tool != record.tool_name) {
            continue;
        }
        if result.is_some_and(|result| result != record.result) {
            continue;
        }
        if !filter.contains(&record.timestamp) {
            continue;
        }

        // Hash filtering runs after the tool/result/time filters so the skip counts reflect only
        // records that were otherwise in scope.
        if let RulesHashFilter::Only(wanted) = hash_filter {
            match &record.rules_hash {
                Some(hash) if hash == wanted => {}
                Some(_) => {
                    skipped.other_rules += 1;
                    continue;
                }
                None => {
                    skipped.no_hash += 1;
                    continue;
                }
            }
        }

        let cwd = if include_cwd {
            record.cwd.unwrap_or_default()
        } else {
            String::new()
        };
        *counts
            .entry((
                record.tool_name,
                record.tool_args,
                record.result,
                record.rule,
                cwd,
            ))
            .or_insert(0) += 1;
    }

    let mut entries: Vec<(RowKey, u64)> = counts.into_iter().collect();

    // Most frequent first; tool name, raw arguments, result, rule, then cwd fully order ties so the
    // report is deterministic regardless of HashMap iteration order. Sorting on the raw `tool_args`
    // key also avoids re-serializing the parsed arguments for every comparison.
    entries.sort_by(
        |((a_tool, a_args, a_result, a_rule, a_cwd), a_count),
         ((b_tool, b_args, b_result, b_rule, b_cwd), b_count)| {
            b_count
                .cmp(a_count)
                .then_with(|| a_tool.cmp(b_tool))
                .then_with(|| a_args.cmp(b_args))
                .then_with(|| a_result.as_str().cmp(b_result.as_str()))
                .then_with(|| a_rule.cmp(b_rule))
                .then_with(|| a_cwd.cmp(b_cwd))
        },
    );

    let rows = entries
        .into_iter()
        .map(
            |((tool_name, tool_args, result, rule, cwd), count)| ReportRow {
                arguments: arguments_value(tool_args),
                tool_name,
                result,
                count,
                rule,
                cwd,
            },
        )
        .collect();
    CwdAggregation { rows, skipped }
}

/// `tool_args` is the tool input serialized to a JSON string, so parse it back to emit real JSON.
/// Inputs larger than the log truncation limit are stored with a marker that is no longer valid
/// JSON; in that case the raw logged text is preserved verbatim as a string.
fn arguments_value(tool_args: String) -> Value {
    serde_json::from_str(&tool_args).unwrap_or(Value::String(tool_args))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cost_report::DateTimezone;

    fn ts(value: &str) -> DateTime<Utc> {
        value.parse().expect("test timestamp should be RFC 3339")
    }

    fn record(timestamp: &str, tool: &str, args: &str, result: PreToolResult) -> HookRecord {
        HookRecord {
            timestamp: ts(timestamp),
            tool_name: tool.to_string(),
            tool_args: args.to_string(),
            result,
            cwd: None,
            rules_hash: None,
            rule: None,
        }
    }

    fn record_with_hash(
        timestamp: &str,
        args: &str,
        result: PreToolResult,
        rules_hash: Option<&str>,
    ) -> HookRecord {
        HookRecord {
            timestamp: ts(timestamp),
            tool_name: "Bash".to_string(),
            tool_args: args.to_string(),
            result,
            cwd: None,
            rules_hash: rules_hash.map(str::to_string),
            rule: None,
        }
    }

    #[test]
    fn build_rows_only_filter_keeps_matching_hash_and_counts_skips() {
        let unrestricted = TimeRangeFilter::new(None, None, DateTimezone::Utc).unwrap();
        let records = vec![
            record_with_hash(
                "2026-06-03T01:00:00Z",
                r#"{"command":"ls"}"#,
                PreToolResult::Allow,
                Some("h1"),
            ),
            record_with_hash(
                "2026-06-03T02:00:00Z",
                r#"{"command":"rm"}"#,
                PreToolResult::Allow,
                Some("h2"),
            ),
            record_with_hash(
                "2026-06-03T03:00:00Z",
                r#"{"command":"cat"}"#,
                PreToolResult::Allow,
                None,
            ),
        ];

        let aggregation = build_rows(
            records,
            &unrestricted,
            Some("Bash"),
            None,
            true,
            &RulesHashFilter::Only("h1".to_string()),
        );

        // Only the h1 record survives; the other-rule-set and the unstamped legacy record are
        // excluded but counted so the caller can report them.
        assert_eq!(aggregation.rows.len(), 1);
        assert_eq!(
            aggregation.rows[0].arguments,
            serde_json::json!({ "command": "ls" })
        );
        assert_eq!(
            aggregation.skipped,
            HashSkipStats {
                other_rules: 1,
                no_hash: 1
            }
        );
    }

    #[test]
    fn build_rows_groups_by_deciding_rule_and_omits_absent_rule_from_json() {
        // Two identical calls decided by different rules must not merge into one row, and a row
        // with no deciding rule must serialize exactly as before (no `rule` key).
        let unrestricted = TimeRangeFilter::new(None, None, DateTimezone::Utc).unwrap();
        let with_rule = |rule: Option<&str>| HookRecord {
            timestamp: ts("2026-06-03T01:00:00Z"),
            tool_name: "Bash".to_string(),
            tool_args: r#"{"command":"ls"}"#.to_string(),
            result: PreToolResult::Allow,
            cwd: None,
            rules_hash: None,
            rule: rule.map(str::to_string),
        };
        let records = vec![
            with_rule(Some("allow-ls")),
            with_rule(Some("allow-read-commands")),
            with_rule(None),
        ];

        let rows = build_rows(
            records,
            &unrestricted,
            None,
            None,
            false,
            &RulesHashFilter::Any,
        )
        .rows;

        assert_eq!(rows.len(), 3, "each deciding rule gets its own row");
        let mut rules: Vec<Option<&str>> = rows.iter().map(|row| row.rule.as_deref()).collect();
        rules.sort_unstable();
        assert_eq!(
            rules,
            vec![None, Some("allow-ls"), Some("allow-read-commands")],
            "rows carry exactly the rules that decided them"
        );
        let no_rule_row = rows
            .iter()
            .find(|row| row.rule.is_none())
            .expect("the rule-less record keeps its own row");
        let json = serde_json::to_value(no_rule_row).unwrap();
        assert!(
            json.get("rule").is_none(),
            "a row without a deciding rule serializes without a rule key, exactly as before"
        );
        let attributed = serde_json::to_value(
            rows.iter()
                .find(|row| row.rule.as_deref() == Some("allow-ls"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(attributed["rule"], "allow-ls");
    }

    #[test]
    fn parse_record_treats_empty_provenance_as_absent() {
        // The completion log writes "" (not an absent key) when there is no rules hash or deciding
        // rule; both must come back as None so the hash filter classifies them as legacy/no-hash.
        let line = serde_json::json!({
            "timestamp": "2026-06-03T12:00:00Z",
            "fields": {
                "message": COMPLETION_MESSAGE,
                "tool_name": "Bash",
                "tool_args": "{\"command\":\"ls\"}",
                "cwd": "/work",
                "rules_hash": "",
                "rule": "",
                "result": "ask"
            }
        })
        .to_string();

        let record = parse_record(&line).expect("the line should parse");
        assert_eq!(record.rules_hash, None);
        assert_eq!(record.rule, None);
        assert_eq!(record.cwd.as_deref(), Some("/work"));
    }

    #[test]
    fn bash_command_gates_on_tool_name_and_argument_shape() {
        let row = |tool: &str, arguments: Value| ReportRow {
            tool_name: tool.to_string(),
            arguments,
            result: PreToolResult::Allow,
            count: 1,
            rule: None,
            cwd: String::new(),
        };

        assert_eq!(
            row("Read", serde_json::json!({ "command": "ls" })).bash_command(),
            None,
            "a non-Bash row never yields a command, even when a command key is present"
        );
        assert_eq!(
            row("Bash", serde_json::json!({ "command": "ls" })).bash_command(),
            Some("ls")
        );
        assert_eq!(
            row("Bash", Value::String("truncated raw text".to_string())).bash_command(),
            None,
            "a truncation-degraded arguments string is skipped, not misread"
        );
    }

    #[test]
    fn build_rows_any_filter_keeps_every_rule_set() {
        let unrestricted = TimeRangeFilter::new(None, None, DateTimezone::Utc).unwrap();
        let records = vec![
            record_with_hash(
                "2026-06-03T01:00:00Z",
                r#"{"command":"ls"}"#,
                PreToolResult::Allow,
                Some("h1"),
            ),
            record_with_hash(
                "2026-06-03T02:00:00Z",
                r#"{"command":"rm"}"#,
                PreToolResult::Allow,
                None,
            ),
        ];

        let aggregation = build_rows(
            records,
            &unrestricted,
            Some("Bash"),
            None,
            true,
            &RulesHashFilter::Any,
        );

        assert_eq!(aggregation.rows.len(), 2);
        assert_eq!(aggregation.skipped, HashSkipStats::default());
    }

    fn completion_line(timestamp: &str, tool: &str, args: &str, result: &str) -> String {
        // Mirrors the tracing JSON envelope shape, including unmodeled keys, to prove the parser
        // tolerates them.
        serde_json::json!({
            "timestamp": timestamp,
            "level": "INFO",
            "fields": {
                "message": COMPLETION_MESSAGE,
                "tool_name": tool,
                "tool_args": args,
                "result": result,
                "hook_output": "HookOutput { .. }"
            },
            "target": "moriarty::hooks",
            "filename": "crates/moriarty/src/hooks/mod.rs",
            "line_number": 267,
            "threadId": "ThreadId(1)"
        })
        .to_string()
    }

    #[test]
    fn parse_record_extracts_completed_pretool_fields() {
        let line = completion_line(
            "2026-06-03T12:00:00Z",
            "Bash",
            r#"{"command":"ls"}"#,
            "allow",
        );
        let record = parse_record(&line).expect("a completed PreToolUse line should parse");

        assert_eq!(record.tool_name, "Bash");
        assert_eq!(record.tool_args, r#"{"command":"ls"}"#);
        assert_eq!(record.result, PreToolResult::Allow);
        assert_eq!(record.timestamp, ts("2026-06-03T12:00:00Z"));
    }

    #[test]
    fn parse_record_skips_other_event_messages() {
        let line = serde_json::json!({
            "timestamp": "2026-06-03T12:00:00Z",
            "fields": { "message": "Stop hook completed", "hook_output": "..." }
        })
        .to_string();
        assert!(parse_record(&line).is_none());
    }

    #[test]
    fn parse_record_skips_legacy_lines_without_result() {
        // A completion line written before the clean result field existed.
        let line = serde_json::json!({
            "timestamp": "2026-06-03T12:00:00Z",
            "fields": {
                "message": COMPLETION_MESSAGE,
                "tool_name": "Read",
                "tool_args": "{\"file_path\":\"/tmp/x\"}",
                "hook_output": "HookOutput { .. }"
            }
        })
        .to_string();
        assert!(parse_record(&line).is_none());
    }

    #[test]
    fn parse_record_skips_non_json_lines() {
        assert!(parse_record("not json at all").is_none());
        assert!(parse_record("").is_none());
    }

    #[test]
    fn arguments_value_parses_json_objects() {
        assert_eq!(
            arguments_value(r#"{"command":"ls"}"#.to_string()),
            serde_json::json!({ "command": "ls" })
        );
    }

    #[test]
    fn arguments_value_preserves_truncated_text_as_string() {
        let truncated = r#"{"command":"ls ... [truncated 42 bytes]"#.to_string();
        assert_eq!(arguments_value(truncated.clone()), Value::String(truncated));
    }

    #[test]
    fn build_rows_counts_exact_triples_and_sorts_by_count() {
        let unrestricted = TimeRangeFilter::new(None, None, DateTimezone::Utc).unwrap();
        let records = vec![
            record(
                "2026-06-03T01:00:00Z",
                "Bash",
                r#"{"command":"ls"}"#,
                PreToolResult::Allow,
            ),
            record(
                "2026-06-03T02:00:00Z",
                "Bash",
                r#"{"command":"ls"}"#,
                PreToolResult::Allow,
            ),
            record(
                "2026-06-03T03:00:00Z",
                "Read",
                r#"{"file_path":"/a"}"#,
                PreToolResult::Passthrough,
            ),
            // Same tool + args but a different result is a distinct row.
            record(
                "2026-06-03T04:00:00Z",
                "Bash",
                r#"{"command":"ls"}"#,
                PreToolResult::Deny,
            ),
        ];

        let rows = build_rows(
            records,
            &unrestricted,
            None,
            None,
            false,
            &RulesHashFilter::Any,
        )
        .rows;

        assert_eq!(
            rows,
            vec![
                ReportRow {
                    tool_name: "Bash".to_string(),
                    arguments: serde_json::json!({ "command": "ls" }),
                    result: PreToolResult::Allow,
                    count: 2,
                    rule: None,
                    cwd: String::new(),
                },
                ReportRow {
                    tool_name: "Bash".to_string(),
                    arguments: serde_json::json!({ "command": "ls" }),
                    result: PreToolResult::Deny,
                    count: 1,
                    rule: None,
                    cwd: String::new(),
                },
                ReportRow {
                    tool_name: "Read".to_string(),
                    arguments: serde_json::json!({ "file_path": "/a" }),
                    result: PreToolResult::Passthrough,
                    count: 1,
                    rule: None,
                    cwd: String::new(),
                },
            ]
        );
    }

    #[test]
    fn build_rows_breaks_result_ties_deterministically() {
        // Identical tool, args, and count; only the result differs. The result tiebreaker orders
        // them by the lowercase label so the output never depends on HashMap iteration order.
        let unrestricted = TimeRangeFilter::new(None, None, DateTimezone::Utc).unwrap();
        let records = vec![
            record(
                "2026-06-03T02:00:00Z",
                "Bash",
                r#"{"command":"ls"}"#,
                PreToolResult::Deny,
            ),
            record(
                "2026-06-03T01:00:00Z",
                "Bash",
                r#"{"command":"ls"}"#,
                PreToolResult::Allow,
            ),
        ];

        let rows = build_rows(
            records,
            &unrestricted,
            None,
            None,
            false,
            &RulesHashFilter::Any,
        )
        .rows;

        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0].result,
            PreToolResult::Allow,
            "\"allow\" sorts before \"deny\""
        );
        assert_eq!(rows[1].result, PreToolResult::Deny);
    }

    #[test]
    fn build_rows_applies_tool_and_result_filters() {
        let unrestricted = TimeRangeFilter::new(None, None, DateTimezone::Utc).unwrap();
        let records = vec![
            record(
                "2026-06-03T01:00:00Z",
                "Bash",
                r#"{"command":"ls"}"#,
                PreToolResult::Allow,
            ),
            record(
                "2026-06-03T02:00:00Z",
                "Bash",
                r#"{"command":"rm"}"#,
                PreToolResult::Deny,
            ),
            record(
                "2026-06-03T03:00:00Z",
                "Read",
                r#"{"file_path":"/a"}"#,
                PreToolResult::Allow,
            ),
        ];

        let rows = build_rows(
            records,
            &unrestricted,
            Some("Bash"),
            Some(PreToolResult::Deny),
            false,
            &RulesHashFilter::Any,
        )
        .rows;

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].tool_name, "Bash");
        assert_eq!(rows[0].result, PreToolResult::Deny);
        assert_eq!(rows[0].arguments, serde_json::json!({ "command": "rm" }));
    }

    #[test]
    fn build_rows_applies_time_range_filter() {
        // A date-only end maps to that day's exclusive end (2026-06-04T00:00:00Z), so the range is
        // the whole of 2026-06-03.
        let filter = TimeRangeFilter::new(
            Some("2026-06-03".to_string()),
            Some("2026-06-03".to_string()),
            DateTimezone::Utc,
        )
        .unwrap();
        let records = vec![
            record(
                "2026-06-02T23:59:59Z",
                "Bash",
                r#"{"command":"ls"}"#,
                PreToolResult::Allow,
            ),
            record(
                "2026-06-03T12:00:00Z",
                "Bash",
                r#"{"command":"ls"}"#,
                PreToolResult::Allow,
            ),
            record(
                "2026-06-04T00:00:00Z",
                "Bash",
                r#"{"command":"ls"}"#,
                PreToolResult::Allow,
            ),
        ];

        let rows = build_rows(records, &filter, None, None, false, &RulesHashFilter::Any).rows;

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].count, 1, "only the 2026-06-03 record is in range");
    }

    #[tokio::test]
    async fn read_records_reads_rotated_files_and_skips_others() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(
            dir.path().join("hooks.log.2026-06-03"),
            format!(
                "{}\n{}\n",
                completion_line(
                    "2026-06-03T01:00:00Z",
                    "Bash",
                    r#"{"command":"ls"}"#,
                    "allow"
                ),
                "not a json line"
            ),
        )
        .await
        .unwrap();
        tokio::fs::write(
            dir.path().join("hooks.log.2026-06-04"),
            completion_line(
                "2026-06-04T01:00:00Z",
                "Read",
                r#"{"file_path":"/a"}"#,
                "passthrough",
            ),
        )
        .await
        .unwrap();
        // An unrelated file must be ignored.
        tokio::fs::write(dir.path().join("other.txt"), "ignored")
            .await
            .unwrap();

        let mut records = read_records(dir.path()).await.unwrap();
        records.sort_by(|a, b| a.tool_name.cmp(&b.tool_name));

        // The non-JSON line and the unrelated file are dropped; both rotated completion lines parse.
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].tool_name, "Bash");
        assert_eq!(records[0].timestamp, ts("2026-06-03T01:00:00Z"));
        assert_eq!(records[1].tool_name, "Read");
        assert_eq!(records[1].result, PreToolResult::Passthrough);
    }

    #[test]
    fn parse_record_skips_completion_lines_missing_a_field() {
        // `result` is present but `tool_name` is absent, so the record cannot be built.
        let line = serde_json::json!({
            "timestamp": "2026-06-03T12:00:00Z",
            "fields": {
                "message": COMPLETION_MESSAGE,
                "tool_args": "{\"command\":\"ls\"}",
                "result": "allow"
            }
        })
        .to_string();
        assert!(parse_record(&line).is_none());
    }

    #[test]
    fn build_rows_applies_start_only_filter() {
        let filter =
            TimeRangeFilter::new(Some("2026-06-03".to_string()), None, DateTimezone::Utc).unwrap();
        let records = vec![
            record(
                "2026-06-02T12:00:00Z",
                "Bash",
                r#"{"command":"ls"}"#,
                PreToolResult::Allow,
            ),
            record(
                "2026-06-03T00:00:00Z",
                "Bash",
                r#"{"command":"ls"}"#,
                PreToolResult::Allow,
            ),
        ];

        let rows = build_rows(records, &filter, None, None, false, &RulesHashFilter::Any).rows;

        // The start boundary is inclusive, so the midnight record is kept and the earlier one dropped.
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].count, 1);
    }

    #[tokio::test]
    async fn read_records_returns_empty_when_directory_missing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");

        let records = read_records(&missing).await.unwrap();

        assert!(records.is_empty());
    }

    #[tokio::test]
    async fn run_succeeds_over_an_explicit_dir() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(
            dir.path().join("hooks.log.2026-06-03"),
            format!(
                "{}\n",
                completion_line(
                    "2026-06-03T01:00:00Z",
                    "Bash",
                    r#"{"command":"ls"}"#,
                    "allow"
                )
            ),
        )
        .await
        .unwrap();

        // Exercises the full path: resolve_log_dir (explicit dir) -> read_records -> build_rows ->
        // JSON serialization. Output goes to stdout; we only assert the run itself succeeds.
        run(
            Some(dir.path().to_path_buf()),
            None,
            None,
            None,
            None,
            DateTimezone::Utc,
        )
        .await
        .expect("report over a valid directory should succeed");
    }
}
