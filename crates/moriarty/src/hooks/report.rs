//! `hooks report` — aggregate recorded PreToolUse hook results into a JSON report.
//!
//! Reads the hooks tracing logs, keeps the completed PreToolUse records (those carrying the
//! clean `result` field written by [`super::result`]), and groups them by the exact
//! `(tool name, arguments, result)` triple so each row reports how often that exact call
//! occurred. Output is JSON on stdout; nothing else is written there.

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
}

struct HookRecord {
    timestamp: DateTime<Utc>,
    tool_name: String,
    tool_args: String,
    result: PreToolResult,
}

#[derive(Debug, Serialize, PartialEq)]
pub(crate) struct ReportRow {
    pub(crate) tool_name: String,
    pub(crate) arguments: Value,
    pub(crate) result: PreToolResult,
    pub(crate) count: u64,
}

pub async fn run(
    dir: Option<PathBuf>,
    start_time: Option<String>,
    end_time: Option<String>,
    tool: Option<String>,
    result: Option<PreToolResult>,
) -> Result<()> {
    let filter = TimeRangeFilter::new(start_time, end_time)?;
    let rows = aggregate(dir, &filter, tool.as_deref(), result).await?;

    let json = serde_json::to_string_pretty(&rows)
        .into_diagnostic()
        .wrap_err("Failed to serialize hook report")?;
    println!("{json}");
    Ok(())
}

/// Reads the hook logs and aggregates them into `(tool, arguments, result)` rows, sorted by count.
/// Shared by `hooks report` and `rules suggest`/`rules replay`.
pub(crate) async fn aggregate(
    dir: Option<PathBuf>,
    filter: &TimeRangeFilter,
    tool: Option<&str>,
    result: Option<PreToolResult>,
) -> Result<Vec<ReportRow>> {
    let log_dir = resolve_log_dir(dir).await?;
    let records = read_records(&log_dir).await?;
    Ok(build_rows(records, filter, tool, result))
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
    })
}

fn build_rows(
    records: Vec<HookRecord>,
    filter: &TimeRangeFilter,
    tool: Option<&str>,
    result: Option<PreToolResult>,
) -> Vec<ReportRow> {
    let mut counts: HashMap<(String, String, PreToolResult), u64> = HashMap::new();

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

        *counts
            .entry((record.tool_name, record.tool_args, record.result))
            .or_insert(0) += 1;
    }

    let mut entries: Vec<((String, String, PreToolResult), u64)> = counts.into_iter().collect();

    // Most frequent first; tool name, raw arguments, then result fully order ties so the report is
    // deterministic regardless of HashMap iteration order. Sorting on the raw `tool_args` key also
    // avoids re-serializing the parsed arguments for every comparison.
    entries.sort_by(
        |((a_tool, a_args, a_result), a_count), ((b_tool, b_args, b_result), b_count)| {
            b_count
                .cmp(a_count)
                .then_with(|| a_tool.cmp(b_tool))
                .then_with(|| a_args.cmp(b_args))
                .then_with(|| a_result.as_str().cmp(b_result.as_str()))
        },
    );

    entries
        .into_iter()
        .map(|((tool_name, tool_args, result), count)| ReportRow {
            arguments: arguments_value(tool_args),
            tool_name,
            result,
            count,
        })
        .collect()
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

    fn ts(value: &str) -> DateTime<Utc> {
        value.parse().expect("test timestamp should be RFC 3339")
    }

    fn record(timestamp: &str, tool: &str, args: &str, result: PreToolResult) -> HookRecord {
        HookRecord {
            timestamp: ts(timestamp),
            tool_name: tool.to_string(),
            tool_args: args.to_string(),
            result,
        }
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
        let unrestricted = TimeRangeFilter::new(None, None).unwrap();
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

        let rows = build_rows(records, &unrestricted, None, None);

        assert_eq!(
            rows,
            vec![
                ReportRow {
                    tool_name: "Bash".to_string(),
                    arguments: serde_json::json!({ "command": "ls" }),
                    result: PreToolResult::Allow,
                    count: 2,
                },
                ReportRow {
                    tool_name: "Bash".to_string(),
                    arguments: serde_json::json!({ "command": "ls" }),
                    result: PreToolResult::Deny,
                    count: 1,
                },
                ReportRow {
                    tool_name: "Read".to_string(),
                    arguments: serde_json::json!({ "file_path": "/a" }),
                    result: PreToolResult::Passthrough,
                    count: 1,
                },
            ]
        );
    }

    #[test]
    fn build_rows_breaks_result_ties_deterministically() {
        // Identical tool, args, and count; only the result differs. The result tiebreaker orders
        // them by the lowercase label so the output never depends on HashMap iteration order.
        let unrestricted = TimeRangeFilter::new(None, None).unwrap();
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

        let rows = build_rows(records, &unrestricted, None, None);

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
        let unrestricted = TimeRangeFilter::new(None, None).unwrap();
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
        );

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

        let rows = build_rows(records, &filter, None, None);

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
        let filter = TimeRangeFilter::new(Some("2026-06-03".to_string()), None).unwrap();
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

        let rows = build_rows(records, &filter, None, None);

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
        run(Some(dir.path().to_path_buf()), None, None, None, None)
            .await
            .expect("report over a valid directory should succeed");
    }
}
