//! `hooks report` — aggregate recorded PreToolUse hook results into a JSON report.
//!
//! Reads the hooks tracing logs, keeps the completed PreToolUse records (those carrying the
//! clean `result` field written by [`super::result`]), and groups them by the exact
//! `(tool name, arguments, result, deciding rule)` key so each row reports how often that exact
//! call occurred. Output is JSON on stdout; nothing else is written there. The same streaming
//! parser/filter fold supplies timestamp-preserving outcomes to `rules report` without retaining
//! every matching record.

// standard library
use std::{
    collections::HashMap,
    io::ErrorKind,
    path::{Path, PathBuf},
};

// 3rd party crates
use chrono::{DateTime, Utc};
use futures::{StreamExt, TryStreamExt, stream};
use miette::{IntoDiagnostic, Result, WrapErr};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

// local / workspace deps
use super::result::PreToolResult;
use crate::{cost_report::TimeRangeFilter, permission_mode::PermissionMode, persistence::FileType};

const COMPLETION_MESSAGE: &str = "PreToolUse hook completed";
/// Full files remain buffered while their records are folded, so bounding reads prevents old log
/// histories from multiplying peak memory while still overlapping filesystem I/O.
const MAX_CONCURRENT_LOG_READS: usize = 4;

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
    permission_mode: Option<String>,
    rules_hash: Option<String>,
    rule: Option<String>,
}

#[derive(Clone)]
struct HookRecord {
    timestamp: DateTime<Utc>,
    tool_name: String,
    tool_args: String,
    result: PreToolResult,
    /// The hook's working directory, used by the rules path to normalize commands as the hook did.
    cwd: Option<String>,
    /// Missing or unrecognized modes enable only unrestricted rules during replay.
    permission_mode: Option<PermissionMode>,
    /// Hash of the rule set in force when this decision was made (see [`crate::rules`]).
    rules_hash: Option<String>,
    /// Name of the rule whose action produced the decision; `None` when no rule decided.
    rule: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct ReportRow {
    pub(crate) tool_name: String,
    pub(crate) arguments: Value,
    pub(crate) result: PreToolResult,
    pub(crate) count: u64,
    /// The rule that decided these calls. Part of the grouping key, so one row never mixes
    /// decisions from different rules; omitted from the JSON when no rule decided (historical lines
    /// and passthrough/unconfigured outcomes), keeping those rows' serialization unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rule: Option<String>,
    /// The cwd these calls ran under. Skipped in `hooks report` output, which groups without cwd so
    /// its rows and counts are unchanged; populated (and part of the grouping key) only for the
    /// rules path via [`aggregate_with_cwd`]. Empty string for the `hooks report` grouping.
    #[serde(skip)]
    pub(crate) cwd: String,
    /// Internal rules utilities group and evaluate by mode; the public report deliberately omits it.
    #[serde(skip)]
    pub(crate) permission_mode: Option<PermissionMode>,
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
    /// Dropped because no `rules_hash` was recorded (older logs and config-load failures).
    pub(crate) no_hash: u64,
}

/// Rows from [`aggregate_with_cwd`] plus the hash-filter skip accounting.
pub(crate) struct CwdAggregation {
    pub(crate) rows: Vec<ReportRow>,
    pub(crate) skipped: HashSkipStats,
}

/// The effectiveness report needs each event's timestamp intact; aggregating through
/// [`ReportRow`] first would make timezone-aware daily grouping impossible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutcomeRecord {
    pub(crate) timestamp: DateTime<Utc>,
    pub(crate) cwd: Option<String>,
    pub(crate) result: PreToolResult,
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct RowKey {
    tool_name: String,
    tool_args: String,
    result: PreToolResult,
    rule: Option<String>,
    cwd: String,
    permission_mode: Option<PermissionMode>,
}

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
    let aggregation =
        aggregate_rows(dir, filter, tool, result, false, &RulesHashFilter::Any).await?;
    Ok(aggregation.rows)
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
    aggregate_rows(dir, filter, tool, result, true, hash_filter).await
}

/// Folds timestamp-preserving outcomes as their files are read, so daily/directory reports retain
/// only their final buckets rather than every matching hook record.
pub(crate) async fn fold_outcomes<T, F>(
    dir: Option<PathBuf>,
    filter: &TimeRangeFilter,
    hash_filter: &RulesHashFilter,
    initial: T,
    mut fold: F,
) -> Result<(T, HashSkipStats)>
where
    F: FnMut(&mut T, OutcomeRecord),
{
    let filters = RecordFilters {
        time: filter,
        tool: None,
        result: None,
        hash: hash_filter,
    };
    fold_filtered_records(dir, filters, initial, |value, record| {
        fold(
            value,
            OutcomeRecord {
                timestamp: record.timestamp,
                cwd: record.cwd,
                result: record.result,
            },
        );
    })
    .await
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

async fn log_files(log_dir: &Path) -> Result<Vec<PathBuf>> {
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

    let mut paths = Vec::new();
    while let Some(entry) = entries.next_entry().await.into_diagnostic()? {
        // Daily rotation produces `hooks.log.YYYY-MM-DD`; match the whole family.
        if entry.file_name().to_string_lossy().starts_with("hooks.log") {
            paths.push(entry.path());
        }
    }
    Ok(paths)
}

async fn read_log_file(path: PathBuf) -> Result<String> {
    tokio::fs::read_to_string(&path)
        .await
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to read hooks log file {}", path.display()))
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
        permission_mode: parse_recorded_permission_mode(envelope.fields.permission_mode),
        // The log writes `""` (not an absent key) for "no rules hash" / "no deciding rule", so an
        // empty value must mean None here — otherwise the hash filter would misclassify a
        // config-load-failure line as belonging to some other rule set.
        rules_hash: envelope.fields.rules_hash.filter(|hash| !hash.is_empty()),
        rule: envelope.fields.rule.filter(|rule| !rule.is_empty()),
    })
}

fn parse_recorded_permission_mode(mode: Option<String>) -> Option<PermissionMode> {
    let mode = mode.filter(|mode| !mode.is_empty())?;
    match serde_json::from_value(Value::String(mode.clone())) {
        Ok(mode) => Some(mode),
        Err(error) => {
            warn!(permission_mode = %mode, %error, "Treating unrecognized recorded permission mode as mode-less");
            None
        }
    }
}

struct RecordFilters<'a> {
    time: &'a TimeRangeFilter,
    tool: Option<&'a str>,
    result: Option<PreToolResult>,
    hash: &'a RulesHashFilter,
}

impl RecordFilters<'_> {
    /// Hash filtering follows the other predicates so skip counts describe only otherwise-in-scope
    /// records; every report consumer shares this ordering.
    fn keep(&self, record: HookRecord, skipped: &mut HashSkipStats) -> Option<HookRecord> {
        if self
            .tool
            .is_some_and(|tool| tool != record.tool_name.as_str())
            || self.result.is_some_and(|result| result != record.result)
            || !self.time.contains(&record.timestamp)
        {
            return None;
        }

        if let RulesHashFilter::Only(wanted) = self.hash {
            match &record.rules_hash {
                Some(hash) if hash == wanted => {}
                Some(_) => {
                    skipped.other_rules += 1;
                    return None;
                }
                None => {
                    skipped.no_hash += 1;
                    return None;
                }
            }
        }

        Some(record)
    }
}

async fn fold_filtered_records<T, F>(
    dir: Option<PathBuf>,
    filters: RecordFilters<'_>,
    mut value: T,
    mut fold: F,
) -> Result<(T, HashSkipStats)>
where
    F: FnMut(&mut T, HookRecord),
{
    let log_dir = resolve_log_dir(dir).await?;
    let mut reads = stream::iter(log_files(&log_dir).await?)
        .map(read_log_file)
        .buffer_unordered(MAX_CONCURRENT_LOG_READS);

    let mut skipped = HashSkipStats::default();

    while let Some(contents) = reads.try_next().await? {
        for record in contents.lines().filter_map(parse_record) {
            if let Some(record) = filters.keep(record, &mut skipped) {
                fold(&mut value, record);
            }
        }
    }

    Ok((value, skipped))
}

fn count_row(counts: &mut HashMap<RowKey, u64>, record: HookRecord, include_cwd: bool) {
    let (cwd, permission_mode) = if include_cwd {
        (record.cwd.unwrap_or_default(), record.permission_mode)
    } else {
        (String::new(), None)
    };
    *counts
        .entry(RowKey {
            tool_name: record.tool_name,
            tool_args: record.tool_args,
            result: record.result,
            rule: record.rule,
            cwd,
            permission_mode,
        })
        .or_insert(0) += 1;
}

fn finish_rows(counts: HashMap<RowKey, u64>, skipped: HashSkipStats) -> CwdAggregation {
    let mut entries: Vec<(RowKey, u64)> = counts.into_iter().collect();

    // Most frequent first; tool name, raw arguments, result, rule, then cwd fully order ties so
    // output is deterministic regardless of HashMap or concurrent file-read order.
    entries.sort_by(|(a, a_count), (b, b_count)| {
        b_count
            .cmp(a_count)
            .then_with(|| a.tool_name.cmp(&b.tool_name))
            .then_with(|| a.tool_args.cmp(&b.tool_args))
            .then_with(|| a.result.as_str().cmp(b.result.as_str()))
            .then_with(|| a.rule.cmp(&b.rule))
            .then_with(|| a.cwd.cmp(&b.cwd))
            .then_with(|| a.permission_mode.cmp(&b.permission_mode))
    });

    let rows = entries
        .into_iter()
        .map(|(key, count)| ReportRow {
            arguments: arguments_value(key.tool_args),
            tool_name: key.tool_name,
            result: key.result,
            count,
            rule: key.rule,
            cwd: key.cwd,
            permission_mode: key.permission_mode,
        })
        .collect();
    CwdAggregation { rows, skipped }
}

async fn aggregate_rows(
    dir: Option<PathBuf>,
    filter: &TimeRangeFilter,
    tool: Option<&str>,
    result: Option<PreToolResult>,
    include_cwd: bool,
    hash_filter: &RulesHashFilter,
) -> Result<CwdAggregation> {
    let filters = RecordFilters {
        time: filter,
        tool,
        result,
        hash: hash_filter,
    };
    let (counts, skipped) =
        fold_filtered_records(dir, filters, HashMap::new(), |counts, record| {
            count_row(counts, record, include_cwd)
        })
        .await?;
    Ok(finish_rows(counts, skipped))
}

/// `tool_args` is the tool input serialized to a JSON string, so parse it back to emit real JSON.
/// Inputs larger than the log truncation limit are stored with a marker that is no longer valid
/// JSON; in that case the raw logged text is preserved verbatim as a string.
fn arguments_value(tool_args: String) -> Value {
    serde_json::from_str(&tool_args).unwrap_or(Value::String(tool_args))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

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
            permission_mode: None,
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
            permission_mode: None,
            rules_hash: rules_hash.map(str::to_string),
            rule: None,
        }
    }

    fn build_rows(
        records: Vec<HookRecord>,
        filter: &TimeRangeFilter,
        tool: Option<&str>,
        result: Option<PreToolResult>,
        include_cwd: bool,
        hash_filter: &RulesHashFilter,
    ) -> CwdAggregation {
        let filters = RecordFilters {
            time: filter,
            tool,
            result,
            hash: hash_filter,
        };
        let mut skipped = HashSkipStats::default();
        let mut counts = HashMap::new();

        for record in records {
            if let Some(record) = filters.keep(record, &mut skipped) {
                count_row(&mut counts, record, include_cwd);
            }
        }

        finish_rows(counts, skipped)
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

        // Only the h1 record survives; the other-rule-set and the record without a hash are
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

    #[tokio::test]
    async fn outcome_fold_preserves_event_fields_and_applies_time_filter() {
        let dir = tempfile::tempdir().unwrap();
        let line = |timestamp: &str,
                    tool: &str,
                    result: &str,
                    cwd: Option<&str>,
                    rules_hash: Option<&str>| {
            serde_json::json!({
                "timestamp": timestamp,
                "fields": {
                    "message": COMPLETION_MESSAGE,
                    "tool_name": tool,
                    "tool_args": "{}",
                    "result": result,
                    "cwd": cwd,
                    "rules_hash": rules_hash
                }
            })
            .to_string()
        };
        let lines = [
            line(
                "2026-06-03T01:00:00Z",
                "Bash",
                "allow",
                Some("/work/a"),
                Some("h1"),
            ),
            line(
                "2026-06-03T02:00:00Z",
                "Read",
                "deny",
                Some("/work/b"),
                Some("h1"),
            ),
            line("2026-06-03T03:00:00Z", "Edit", "ask", None, Some("h2")),
            line("2026-06-03T04:00:00Z", "Write", "modify", None, None),
            line(
                "2026-06-04T00:00:00Z",
                "Bash",
                "passthrough",
                None,
                Some("h1"),
            ),
        ];
        tokio::fs::write(
            dir.path().join("hooks.log.2026-06-03"),
            format!("{}\n", lines.join("\n")),
        )
        .await
        .unwrap();
        let filter = TimeRangeFilter::new(
            Some("2026-06-03".to_string()),
            Some("2026-06-03".to_string()),
            DateTimezone::Utc,
        )
        .unwrap();

        let (all_history, skipped) = fold_outcomes(
            Some(dir.path().to_path_buf()),
            &filter,
            &RulesHashFilter::Any,
            Vec::new(),
            |records, record| records.push(record),
        )
        .await
        .unwrap();
        assert_eq!(
            all_history.len(),
            4,
            "the next day's record is filtered out"
        );
        assert_eq!(
            &all_history[..2],
            [
                OutcomeRecord {
                    timestamp: ts("2026-06-03T01:00:00Z"),
                    cwd: Some("/work/a".to_string()),
                    result: PreToolResult::Allow,
                },
                OutcomeRecord {
                    timestamp: ts("2026-06-03T02:00:00Z"),
                    cwd: Some("/work/b".to_string()),
                    result: PreToolResult::Deny,
                },
            ]
        );
        assert_eq!(skipped, HashSkipStats::default());
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
            permission_mode: None,
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
        // rule; both must come back as None so the hash filter classifies them as no-hash.
        let line = serde_json::json!({
            "timestamp": "2026-06-03T12:00:00Z",
            "fields": {
                "message": COMPLETION_MESSAGE,
                "tool_name": "Bash",
                "tool_args": "{\"command\":\"ls\"}",
                "cwd": "/work",
                "permission_mode": "",
                "rules_hash": "",
                "rule": "",
                "result": "ask"
            }
        })
        .to_string();

        let record = parse_record(&line).expect("the line should parse");
        assert_eq!(record.permission_mode, None);
        assert_eq!(record.rules_hash, None);
        assert_eq!(record.rule, None);
        assert_eq!(record.cwd.as_deref(), Some("/work"));
    }

    #[test]
    fn recorded_mode_parsing_handles_known_and_missing_values() {
        let line = serde_json::json!({
            "timestamp": "2026-06-03T12:00:00Z",
            "fields": {
                "message": COMPLETION_MESSAGE,
                "tool_name": "Bash",
                "tool_args": "{\"command\":\"ls\"}",
                "permission_mode": "dontAsk",
                "result": "ask"
            }
        })
        .to_string();
        assert_eq!(
            parse_record(&line).unwrap().permission_mode,
            Some(PermissionMode::DontAsk)
        );
        let mode_less =
            completion_line("2026-06-03T12:00:00Z", "Bash", r#"{"command":"ls"}"#, "ask");
        assert_eq!(parse_record(&mode_less).unwrap().permission_mode, None);
    }

    #[test]
    fn only_internal_cwd_aware_rows_split_by_mode() {
        let make = |permission_mode| HookRecord {
            timestamp: ts("2026-06-03T12:00:00Z"),
            tool_name: "Bash".to_string(),
            tool_args: r#"{"command":"ls"}"#.to_string(),
            result: PreToolResult::Ask,
            cwd: Some("/work".to_string()),
            permission_mode,
            rules_hash: None,
            rule: None,
        };
        let records = vec![
            make(Some(PermissionMode::Plan)),
            make(Some(PermissionMode::Default)),
        ];
        let filter = TimeRangeFilter::new(None, None, DateTimezone::Utc).unwrap();
        let public = build_rows(
            records.clone(),
            &filter,
            None,
            None,
            false,
            &RulesHashFilter::Any,
        )
        .rows;
        assert_eq!((public.len(), public[0].count), (1, 2));
        assert!(
            serde_json::to_value(&public[0])
                .unwrap()
                .get("permission_mode")
                .is_none()
        );

        let internal = build_rows(records, &filter, None, None, true, &RulesHashFilter::Any).rows;
        assert_eq!(internal.len(), 2);
        assert_eq!(
            internal
                .iter()
                .map(|row| row.permission_mode)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([Some(PermissionMode::Default), Some(PermissionMode::Plan)])
        );
    }

    #[test]
    fn unknown_recorded_mode_is_fail_closed_as_mode_less() {
        assert_eq!(
            parse_recorded_permission_mode(Some("futureMode".to_string())),
            None
        );
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
            permission_mode: None,
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
    fn parse_record_skips_historical_lines_without_result() {
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
                    permission_mode: None,
                },
                ReportRow {
                    tool_name: "Bash".to_string(),
                    arguments: serde_json::json!({ "command": "ls" }),
                    result: PreToolResult::Deny,
                    count: 1,
                    rule: None,
                    cwd: String::new(),
                    permission_mode: None,
                },
                ReportRow {
                    tool_name: "Read".to_string(),
                    arguments: serde_json::json!({ "file_path": "/a" }),
                    result: PreToolResult::Passthrough,
                    count: 1,
                    rule: None,
                    cwd: String::new(),
                    permission_mode: None,
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
    async fn streaming_aggregate_reads_rotated_files_in_deterministic_order() {
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

        let filter = TimeRangeFilter::new(None, None, DateTimezone::Utc).unwrap();
        let rows = aggregate(Some(dir.path().to_path_buf()), &filter, None, None)
            .await
            .unwrap();

        // Equal-count rows are fully ordered after the concurrent file reads; malformed lines and
        // unrelated files never enter the aggregation.
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].tool_name, "Bash");
        assert_eq!(rows[0].arguments, serde_json::json!({ "command": "ls" }));
        assert_eq!(rows[0].result, PreToolResult::Allow);
        assert_eq!(rows[1].tool_name, "Read");
        assert_eq!(rows[1].arguments, serde_json::json!({ "file_path": "/a" }));
        assert_eq!(rows[1].result, PreToolResult::Passthrough);
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
    async fn aggregate_reports_log_file_read_errors_with_path() {
        let dir = tempfile::tempdir().unwrap();
        let unreadable = dir.path().join("hooks.log.unreadable");
        tokio::fs::create_dir(&unreadable).await.unwrap();
        let filter = TimeRangeFilter::new(None, None, DateTimezone::Utc).unwrap();

        let error = aggregate(Some(dir.path().to_path_buf()), &filter, None, None)
            .await
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains(&unreadable.display().to_string()),
            "error should identify the unreadable log path: {error:?}"
        );
    }

    #[tokio::test]
    async fn streaming_aggregate_returns_empty_when_directory_missing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        let filter = TimeRangeFilter::new(None, None, DateTimezone::Utc).unwrap();

        let rows = aggregate(Some(missing), &filter, None, None).await.unwrap();

        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn aggregate_streams_and_merges_rows_without_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let line = |cwd: &str| {
            serde_json::json!({
                "timestamp": "2026-06-03T01:00:00Z",
                "fields": {
                    "message": COMPLETION_MESSAGE,
                    "tool_name": "Bash",
                    "tool_args": r#"{"command":"ls"}"#,
                    "cwd": cwd,
                    "result": "allow"
                }
            })
            .to_string()
        };
        tokio::fs::write(
            dir.path().join("hooks.log.2026-06-03"),
            format!("{}\n{}\n", line("/work/a"), line("/work/b")),
        )
        .await
        .unwrap();
        let filter = TimeRangeFilter::new(None, None, DateTimezone::Utc).unwrap();

        let rows = aggregate(Some(dir.path().to_path_buf()), &filter, None, None)
            .await
            .unwrap();

        assert_eq!(
            rows,
            vec![ReportRow {
                tool_name: "Bash".to_string(),
                arguments: serde_json::json!({ "command": "ls" }),
                result: PreToolResult::Allow,
                count: 2,
                rule: None,
                cwd: String::new(),
                permission_mode: None,
            }]
        );
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

        // Exercises the full streaming fold through row aggregation and JSON serialization.
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
