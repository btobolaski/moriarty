use std::{
    cmp::Ordering,
    collections::{hash_map::Entry, HashMap},
    path::{Path, PathBuf},
};

use async_walkdir::WalkDir;
use futures::StreamExt;
use miette::{Context, IntoDiagnostic};
use tokio::{fs::read_to_string, sync::mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{event, Level};

use crate::logs::{AnalyzableLog, LineWithCost};

const JSONL_EXTENSION: &str = "jsonl";
/// Keep parsing concurrency intentionally small because JSON decoding is CPU-bound and we only
/// need enough parallelism to overlap file I/O with a few parser tasks.
const MAX_CONCURRENT_PARSE_TASKS: usize = 4;

async fn send_output<T>(
    tx: &mpsc::Sender<miette::Result<T>>,
    result: miette::Result<T>,
    path: Option<&Path>,
) -> bool {
    match tx.send(result).await {
        Ok(()) => true,
        Err(error) => {
            let error = miette::miette!("failed to write to output: {error}");

            if let Some(path) = path {
                event!(Level::ERROR, ?error, path=%path.display(), "failed to write to channel");
            } else {
                event!(Level::ERROR, ?error, "failed to write to channel");
            }
            false
        }
    }
}

/// The outcome of scanning a directory tree for billable log entries.
#[derive(Debug, Clone)]
pub struct AnalysisResult<LogType>
where
    LogType: AnalyzableLog,
{
    /// Deduplicated billable log entries.
    pub lines: Vec<LineWithCost<LogType>>,
    /// True when at least one file failed to enumerate, read, or parse.
    pub had_errors: bool,
}

/// This ignores symlinks for now. That is an area for possible future improvement but, that would
/// require tracking visited directories in order to prevent infinitely recursing.
fn jsonl_files<LogType: AnalyzableLog>(path: PathBuf) -> mpsc::Receiver<miette::Result<PathBuf>> {
    let (tx, rx) = mpsc::channel(10);

    tokio::spawn(async move {
        let mut walker = WalkDir::new(path);

        while let Some(entry) = walker.next().await {
            let entry = match entry.into_diagnostic().context("failed to get file entry") {
                Ok(entry) => entry,
                Err(error) => {
                    // A walker failure aborts the entire traversal, so we log it here before
                    // returning regardless of whether the receiver is still listening.
                    event!(Level::ERROR, ?error, "failed to get file entry");
                    let _ = send_output(&tx, Err(error), None).await;
                    return;
                }
            };

            let path = entry.path();

            match entry
                .file_type()
                .await
                .into_diagnostic()
                .context("failed to get file type")
            {
                Ok(file_type) => {
                    if !file_type.is_file() {
                        continue;
                    }

                    if path.extension().and_then(|extension| extension.to_str())
                        != Some(JSONL_EXTENSION)
                    {
                        continue;
                    }

                    if !LogType::should_parse_file(&path) {
                        continue;
                    }

                    if !send_output(&tx, Ok(path.clone()), Some(&path)).await {
                        return;
                    }
                }
                Err(error) => {
                    event!(Level::ERROR, ?error, path=%path.display(), "failed to read file type");

                    if !send_output(&tx, Err(error), None).await {
                        return;
                    }
                }
            }
        }
    });

    rx
}

fn log_read_file_error(path: &Path, error: &miette::Report) {
    event!(
        Level::ERROR,
        error = ?error,
        path = %path.display(),
        "failed to read file"
    );
}

fn log_parse_line_error(path: &Path, line_number: usize, line: &str, error: &miette::Report) {
    event!(
        Level::ERROR,
        ?error,
        path=%path.display(),
        line_number,
        %line,
        "failed to parse line"
    );
}

async fn read_file_contents(path: &Path) -> miette::Result<String> {
    let contents = read_to_string(path).await;

    contents
        .into_diagnostic()
        .context("failed to read file")
        .inspect_err(|error| log_read_file_error(path, error))
}

fn parse_log_line<LogType: AnalyzableLog>(
    path: &Path,
    line_number: usize,
    line: &str,
) -> miette::Result<LogType> {
    LogType::parse(line).inspect_err(|error| log_parse_line_error(path, line_number, line, error))
}

async fn parse_file<LogType: AnalyzableLog>(
    path: PathBuf,
) -> miette::Result<Vec<LineWithCost<LogType>>> {
    let contents = read_file_contents(&path).await?;

    // Spawning and immediately awaiting looks unusual, but it keeps CPU-heavy JSON parsing on
    // executor worker threads. Without this, `buffer_unordered` would still overlap file reads,
    // but each parse would run inline and effectively serialize the decoding work.
    tokio::spawn(async move {
        let mut output = Vec::new();
        let mut current_session_id: Option<String> = None;

        for (line_number, line) in contents.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }

            let log = parse_log_line::<LogType>(&path, line_number + 1, line)?;

            if let Some(session_id) = log.session_id() {
                current_session_id = Some(session_id);
            }

            if let Some(line_with_cost) = LineWithCost::from_log(log, current_session_id.clone()) {
                output.push(line_with_cost);
            }
        }

        Ok(output)
    })
    .await
    .into_diagnostic()
    .context("failed to read the outcome of the parse")?
}

fn should_replace_existing_line<LogType: AnalyzableLog>(
    existing: &LineWithCost<LogType>,
    candidate: &LineWithCost<LogType>,
) -> bool {
    match existing.cost.total().cmp(&candidate.cost.total()) {
        // Duplicate log ids often come from partial and final views of the same model response.
        // Keeping the higher-cost version avoids silently undercounting when one copy is missing
        // some billed usage.
        Ordering::Greater => false,
        Ordering::Less => true,
        // When two duplicates report the same total cost, keep the earliest occurrence so the
        // choice is stable and deterministic across repeated scans.
        Ordering::Equal => existing.timestamp > candidate.timestamp,
    }
}

fn deduplicate_lines<LogType: AnalyzableLog>(
    output: &mut HashMap<LogType::ModelId, HashMap<LogType::LogId, LineWithCost<LogType>>>,
    lines: Vec<LineWithCost<LogType>>,
) {
    for line in lines {
        let model_lines = output.entry(line.model.clone()).or_default();

        match model_lines.entry(line.id.clone()) {
            Entry::Occupied(mut original) => {
                if should_replace_existing_line(original.get(), &line) {
                    original.insert(line);
                }
            }
            Entry::Vacant(vacant) => {
                vacant.insert(line);
            }
        }
    }
}

/// Recursively scans `path` for `*.jsonl` files, parses them in parallel, and returns
/// deduplicated billable entries.
///
/// Duplicate entries are keyed by `(ModelId, LogId)`: higher-cost entries win, and equal-cost
/// ties keep the earliest timestamp. Errors are logged through `tracing`, and `had_errors` reports
/// whether any file failed during the scan. Failure is per-file: a single parse error discards all
/// lines from that file, while lines from other fully parsed files are still returned.
pub async fn analyze_directory<LogType: AnalyzableLog>(path: PathBuf) -> AnalysisResult<LogType> {
    let (line_map, had_errors) = ReceiverStream::new(jsonl_files::<LogType>(path))
        .map(|maybe_path| async move {
            match maybe_path {
                Ok(path) => parse_file::<LogType>(path).await,
                Err(error) => Err(error),
            }
        })
        .buffer_unordered(MAX_CONCURRENT_PARSE_TASKS)
        .fold(
            (HashMap::new(), false),
            |(mut output, had_errors), result| async move {
                match result {
                    Ok(lines) => {
                        deduplicate_lines(&mut output, lines);
                        (output, had_errors)
                    }
                    // All stream errors are logged before they reach this fold, so this only
                    // needs to record that the overall scan had partial failures.
                    Err(_) => (output, true),
                }
            },
        )
        .await;

    AnalysisResult {
        lines: line_map
            .into_values()
            .flat_map(|inner_map| inner_map.into_values())
            .collect(),
        had_errors,
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, path::Path};

    use chrono::{DateTime, TimeZone, Utc};
    use rust_decimal::{prelude::ToPrimitive, Decimal};
    use serde::Deserialize;
    use serde_json::json;
    use tempfile::TempDir;
    use tokio::fs::{create_dir_all, write};

    use claude_logs::LogLine as ClaudeLogLine;
    use pi_logs::PiLogLine;

    use super::*;
    use crate::{
        logs::{parse_json_line, ClaudeModelPricing, ClaudeTokenCounts, LlmCost},
        test_support::{
            claude_assistant_json, claude_usage_json, CLAUDE_SESSION_ID, CLAUDE_TIMESTAMP,
        },
    };

    #[derive(Debug, Clone, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct MockLog {
        id: String,
        model: String,
        timestamp: DateTime<Utc>,
        input_cost: Decimal,
    }

    impl AnalyzableLog for MockLog {
        type LogId = String;
        type ModelId = String;

        fn cost(&self) -> Option<LlmCost> {
            Some(LlmCost {
                input: self.input_cost,
                cache_write: Decimal::ZERO,
                cache_read: Decimal::ZERO,
                output: Decimal::ZERO,
            })
        }

        fn token_count(&self, token_type: crate::logs::TokenType) -> Option<u64> {
            match token_type {
                crate::logs::TokenType::Input => self.input_cost.to_u64(),
                crate::logs::TokenType::Output
                | crate::logs::TokenType::CacheWrite
                | crate::logs::TokenType::CacheRead => Some(0),
            }
        }

        fn timestamp(&self) -> DateTime<Utc> {
            self.timestamp
        }

        fn identifier(&self) -> Self::LogId {
            self.id.clone()
        }

        fn model(&self) -> Option<Self::ModelId> {
            Some(self.model.clone())
        }

        fn session_id(&self) -> Option<String> {
            None
        }

        fn parse(value: &str) -> miette::Result<Self> {
            parse_json_line(value, "failed to parse mock log line")
        }
    }

    struct ExpectedStoredLine<'a> {
        model: &'a str,
        id: &'a str,
        input_cost: i64,
        timestamp_seconds: i64,
    }

    fn decimal(units: i64) -> Decimal {
        Decimal::new(units, 0)
    }

    const LATER_CLAUDE_TIMESTAMP: &str = "2026-04-25T01:48:35.742Z";
    const PI_SESSION_ID: &str = "019dc252-e50e-766c-8182-d654b46881b0";

    fn pi_assistant_log(id: &str, timestamp: &str) -> String {
        json!({
            "type": "message",
            "id": id,
            "parentId": "u1",
            "timestamp": timestamp,
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": "hello"}],
                "api": "anthropic-messages",
                "provider": "anthropic",
                "model": "claude-sonnet-4-5",
                "usage": {
                    "input": 10,
                    "output": 5,
                    "cacheRead": 2,
                    "cacheWrite": 1,
                    "totalTokens": 18,
                    "cost": {
                        "input": "3",
                        "output": "5",
                        "cacheRead": "2",
                        "cacheWrite": "1",
                        "total": "11"
                    }
                },
                "stopReason": "stop",
                "timestamp": 1_700_000_000
            }
        })
        .to_string()
    }

    fn pi_session_log(session_id: &str, timestamp: &str) -> String {
        json!({
            "type": "session",
            "version": 1,
            "id": session_id,
            "timestamp": timestamp,
            "cwd": "/tmp/project"
        })
        .to_string()
    }

    fn timestamp(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + seconds, 0)
            .single()
            .unwrap()
    }

    fn line(
        id: &str,
        model: &str,
        input_cost: Decimal,
        timestamp: DateTime<Utc>,
    ) -> LineWithCost<MockLog> {
        let log = MockLog {
            id: id.to_string(),
            model: model.to_string(),
            timestamp,
            input_cost,
        };

        LineWithCost {
            id: log.id.clone(),
            model: log.model.clone(),
            timestamp,
            session_id: None,
            log: Box::new(log),
            cost: LlmCost {
                input: input_cost,
                cache_write: Decimal::ZERO,
                cache_read: Decimal::ZERO,
                output: Decimal::ZERO,
            },
        }
    }

    fn temp_dir() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    fn mock_log_json(id: &str, model: &str, timestamp: &str, input_cost: &str) -> String {
        json!({
            "id": id,
            "model": model,
            "timestamp": timestamp,
            "inputCost": input_cost,
        })
        .to_string()
    }

    fn duplicate_pair(
        first_input_cost: i64,
        first_timestamp_seconds: i64,
        second_input_cost: i64,
        second_timestamp_seconds: i64,
    ) -> Vec<LineWithCost<MockLog>> {
        vec![
            line(
                "msg-1",
                "model-a",
                decimal(first_input_cost),
                timestamp(first_timestamp_seconds),
            ),
            line(
                "msg-1",
                "model-a",
                decimal(second_input_cost),
                timestamp(second_timestamp_seconds),
            ),
        ]
    }

    // All deduplication table cases use duplicate_pair with 5 as the winning cost; only the
    // timestamp of the retained entry varies across scenarios.
    fn expected_duplicate_result(timestamp_seconds: i64) -> ExpectedStoredLine<'static> {
        ExpectedStoredLine {
            model: "model-a",
            id: "msg-1",
            input_cost: 5,
            timestamp_seconds,
        }
    }

    fn log_entry(id: &str, timestamp: &str, input_cost: &str) -> String {
        mock_log_json(id, "model-a", timestamp, input_cost)
    }

    fn file(relative_path: &'static str, contents: String) -> (&'static str, String) {
        (relative_path, contents)
    }

    fn file_set(relative_path: &'static str, contents: String) -> Vec<(&'static str, String)> {
        vec![file(relative_path, contents)]
    }

    fn single_log_file(
        relative_path: &'static str,
        id: &str,
        timestamp: &str,
        input_cost: &str,
    ) -> Vec<(&'static str, String)> {
        file_set(
            relative_path,
            format!("{}\n", log_entry(id, timestamp, input_cost)),
        )
    }

    fn paired_log_file(
        relative_path: &'static str,
        separator: &str,
        terminator: &str,
    ) -> Vec<(&'static str, String)> {
        file_set(
            relative_path,
            format!(
                "{}{}{}{}",
                log_entry("msg-1", CLAUDE_TIMESTAMP, "3"),
                separator,
                log_entry("msg-2", LATER_CLAUDE_TIMESTAMP, "4"),
                terminator
            ),
        )
    }

    fn expected_id_set(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|id| (*id).to_string()).collect()
    }

    fn stored_line<'a>(
        output: &'a HashMap<String, HashMap<String, LineWithCost<MockLog>>>,
        model: &str,
        id: &str,
    ) -> &'a LineWithCost<MockLog> {
        output.get(model).and_then(|lines| lines.get(id)).unwrap()
    }

    fn assert_stored_line(
        output: &HashMap<String, HashMap<String, LineWithCost<MockLog>>>,
        expected: ExpectedStoredLine<'_>,
    ) {
        let stored = stored_line(output, expected.model, expected.id);
        assert_eq!(stored.cost.total(), decimal(expected.input_cost));
        assert_eq!(stored.timestamp, timestamp(expected.timestamp_seconds));
    }

    fn result_ids(result: &AnalysisResult<MockLog>) -> HashSet<String> {
        result.lines.iter().map(|line| line.id.clone()).collect()
    }

    fn assert_single_result_line(result: &AnalysisResult<MockLog>, id: &str, input_cost: i64) {
        assert_eq!(result.lines.len(), 1);
        assert_eq!(result.lines[0].id, id);
        assert_eq!(result.lines[0].cost.total(), decimal(input_cost));
    }

    fn assert_empty_success(result: &AnalysisResult<MockLog>) {
        assert!(!result.had_errors);
        assert!(result.lines.is_empty());
    }

    async fn write_log_files(root: &Path, files: &[(&str, String)]) {
        for (relative_path, contents) in files {
            let path = root.join(relative_path);

            if let Some(parent) = path.parent() {
                create_dir_all(parent).await.unwrap();
            }

            write(path, contents).await.unwrap();
        }
    }

    async fn analyze_with_files(files: &[(&str, String)]) -> AnalysisResult<MockLog> {
        let temp_dir = temp_dir();
        write_log_files(temp_dir.path(), files).await;
        analyze_directory::<MockLog>(temp_dir.path().to_path_buf()).await
    }

    #[derive(Clone, Copy)]
    struct ClaudeDedupCase<'a> {
        name: &'a str,
        request_id: Option<&'a str>,
        message_id: &'a str,
        lower_uuid: &'a str,
        higher_uuid: &'a str,
        expected_id: &'a str,
    }

    fn expected_claude_dedup_cost() -> LlmCost {
        ClaudeModelPricing::SONNET.calculate_cost(&ClaudeTokenCounts {
            input_tokens: 2_000_000,
            ..Default::default()
        })
    }

    async fn assert_claude_dedup_case(case: ClaudeDedupCase<'_>) {
        let temp_dir = temp_dir();
        let lower_cost = claude_assistant_json(
            None,
            case.request_id,
            case.message_id,
            case.lower_uuid,
            "claude-sonnet-4-20250514",
            claude_usage_json(1, 1, 0, 0),
        );
        let higher_cost = claude_assistant_json(
            None,
            case.request_id,
            case.message_id,
            case.higher_uuid,
            "claude-sonnet-4-20250514",
            claude_usage_json(2_000_000, 0, 0, 0),
        );
        let contents = format!("{}\n{}\n", lower_cost, higher_cost);

        write_log_files(temp_dir.path(), &[("session.jsonl", contents)]).await;

        let result = analyze_directory::<ClaudeLogLine>(temp_dir.path().to_path_buf()).await;

        assert!(!result.had_errors, "case {}", case.name);
        assert_eq!(result.lines.len(), 1, "case {}", case.name);
        assert_eq!(result.lines[0].id, case.expected_id, "case {}", case.name);
        assert_eq!(
            result.lines[0].cost,
            expected_claude_dedup_cost(),
            "case {}",
            case.name
        );
    }

    #[tokio::test]
    async fn parse_file_attaches_claude_session_id_to_billable_lines() {
        let temp_dir = temp_dir();
        let path = temp_dir.path().join("claude.jsonl");
        let contents = format!(
            "{}\n",
            claude_assistant_json(
                None,
                Some("req-1"),
                "msg-1",
                "22222222-2222-4222-8222-222222222222",
                "claude-sonnet-4-20250514",
                claude_usage_json(1, 0, 0, 0),
            )
        );

        write(&path, contents).await.unwrap();

        let lines = parse_file::<ClaudeLogLine>(path).await.unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].session_id.as_deref(), Some(CLAUDE_SESSION_ID));
    }

    #[tokio::test]
    async fn parse_file_inherits_pi_session_id_from_session_header() {
        let temp_dir = temp_dir();
        let path = temp_dir.path().join("pi.jsonl");
        let contents = format!(
            "{}\n{}\n",
            pi_session_log(PI_SESSION_ID, CLAUDE_TIMESTAMP),
            pi_assistant_log("a1", LATER_CLAUDE_TIMESTAMP),
        );

        write(&path, contents).await.unwrap();

        let lines = parse_file::<PiLogLine>(path).await.unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].session_id.as_deref(), Some(PI_SESSION_ID));
    }

    #[tokio::test]
    async fn parse_file_leaves_pi_session_id_empty_before_session_header() {
        let temp_dir = temp_dir();
        let path = temp_dir.path().join("pi.jsonl");
        let contents = format!(
            "{}\n{}\n",
            pi_assistant_log("a1", CLAUDE_TIMESTAMP),
            pi_session_log(PI_SESSION_ID, LATER_CLAUDE_TIMESTAMP),
        );

        write(&path, contents).await.unwrap();

        let lines = parse_file::<PiLogLine>(path).await.unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].session_id, None);
    }

    #[test]
    fn should_replace_existing_line_prefers_higher_cost_then_earlier_timestamp() {
        let lower_cost = line("msg-1", "model-a", decimal(3), timestamp(10));
        let higher_cost = line("msg-1", "model-a", decimal(5), timestamp(20));
        let earlier_equal_cost = line("msg-1", "model-a", decimal(5), timestamp(10));
        let later_equal_cost = line("msg-1", "model-a", decimal(5), timestamp(30));

        assert!(should_replace_existing_line(&lower_cost, &higher_cost));
        assert!(should_replace_existing_line(
            &higher_cost,
            &earlier_equal_cost
        ));
        assert!(!should_replace_existing_line(
            &higher_cost,
            &later_equal_cost
        ));
    }

    #[test]
    fn deduplicate_lines_prefers_expected_entry() {
        let cases = [
            (
                "higher cost wins",
                duplicate_pair(3, 10, 5, 20),
                expected_duplicate_result(20),
            ),
            (
                "earlier timestamp wins on cost tie",
                duplicate_pair(5, 20, 5, 10),
                expected_duplicate_result(10),
            ),
            (
                "existing higher cost stays",
                duplicate_pair(5, 10, 3, 20),
                expected_duplicate_result(10),
            ),
            (
                "identical duplicate keeps existing entry",
                duplicate_pair(5, 10, 5, 10),
                expected_duplicate_result(10),
            ),
            (
                "later equal cost does not replace earlier entry",
                duplicate_pair(5, 10, 5, 20),
                expected_duplicate_result(10),
            ),
        ];

        for (name, lines, expected) in cases {
            let mut output = HashMap::new();
            deduplicate_lines(&mut output, lines);
            assert_stored_line(&output, expected);
            assert_eq!(output.len(), 1, "case {name} created unexpected models");
        }
    }

    #[test]
    fn deduplicate_lines_treats_same_id_in_different_models_as_distinct() {
        let mut output = HashMap::new();

        deduplicate_lines(
            &mut output,
            vec![
                line("msg-1", "model-a", decimal(3), timestamp(10)),
                line("msg-1", "model-b", decimal(5), timestamp(10)),
            ],
        );

        assert_eq!(output.len(), 2);
        assert_eq!(
            stored_line(&output, "model-a", "msg-1").cost.total(),
            decimal(3)
        );
        assert_eq!(
            stored_line(&output, "model-b", "msg-1").cost.total(),
            decimal(5)
        );
    }

    #[tokio::test]
    async fn analyze_directory_returns_empty_result_for_empty_dir() {
        let temp_dir = temp_dir();
        let result = analyze_directory::<MockLog>(temp_dir.path().to_path_buf()).await;

        assert_empty_success(&result);
    }

    #[tokio::test]
    async fn analyze_directory_returns_had_errors_for_nonexistent_root() {
        let temp_dir = temp_dir();
        let missing_dir = temp_dir.path().join("missing");
        let result = analyze_directory::<MockLog>(missing_dir).await;

        assert!(result.had_errors);
        assert!(result.lines.is_empty());
    }

    #[tokio::test]
    async fn analyze_directory_handles_expected_file_layouts() {
        let cases = [
            (
                "deduplicates across files",
                vec![
                    file(
                        "one.jsonl",
                        format!("{}\n", log_entry("msg-1", CLAUDE_TIMESTAMP, "3")),
                    ),
                    file(
                        "two.jsonl",
                        format!("{}\n", log_entry("msg-1", LATER_CLAUDE_TIMESTAMP, "5")),
                    ),
                ],
                false,
                &["msg-1"][..],
                Some(("msg-1", 5)),
            ),
            (
                "discovers nested jsonl files",
                single_log_file(
                    "nested/deeper/deep.jsonl",
                    "msg-deep",
                    CLAUDE_TIMESTAMP,
                    "4",
                ),
                false,
                &["msg-deep"][..],
                Some(("msg-deep", 4)),
            ),
            // These layouts exercise line splitting behavior, so they intentionally skip a
            // single-line cost assertion and only validate the discovered ids.
            (
                "handles windows line endings",
                paired_log_file("windows.jsonl", "\r\n", "\r\n"),
                false,
                &["msg-1", "msg-2"][..],
                None,
            ),
            (
                "tolerates blank lines",
                paired_log_file("blank-lines.jsonl", "\n\n", "\n"),
                false,
                &["msg-1", "msg-2"][..],
                None,
            ),
            (
                "handles missing trailing newline",
                paired_log_file("no-trailing-newline.jsonl", "\n", ""),
                false,
                &["msg-1", "msg-2"][..],
                None,
            ),
            (
                "reports partial failures",
                vec![
                    file(
                        "valid.jsonl",
                        format!("{}\n", log_entry("msg-1", CLAUDE_TIMESTAMP, "3")),
                    ),
                    file("invalid.jsonl", "not-json\n".to_string()),
                    file("ignored.txt", "not a log\n".to_string()),
                ],
                true,
                &["msg-1"][..],
                Some(("msg-1", 3)),
            ),
        ];

        for (name, files, expected_had_errors, expected_ids, expected_single_line) in cases {
            let result = analyze_with_files(&files).await;

            assert_eq!(result.had_errors, expected_had_errors, "case {name}");
            assert_eq!(
                result_ids(&result),
                expected_id_set(expected_ids),
                "case {name}"
            );
            assert_eq!(
                result.lines.len(),
                expected_ids.len(),
                "case {name}: unexpected line count"
            );

            if let Some((id, input_cost)) = expected_single_line {
                assert_single_result_line(&result, id, input_cost);
            }
        }
    }

    #[tokio::test]
    async fn claude_analyze_directory_deduplicates_assistants_by_preferred_identifier() {
        let cases = [
            ClaudeDedupCase {
                name: "request id",
                request_id: Some("req-shared"),
                message_id: "msg_shared",
                lower_uuid: "55555555-5555-4555-8555-555555555555",
                higher_uuid: "66666666-6666-4666-8666-666666666666",
                expected_id: "req-shared",
            },
            ClaudeDedupCase {
                name: "message id fallback",
                request_id: None,
                message_id: "msg_shared",
                lower_uuid: "55555555-5555-4555-8555-555555555555",
                higher_uuid: "66666666-6666-4666-8666-666666666666",
                expected_id: "msg_shared",
            },
            ClaudeDedupCase {
                name: "uuid fallback",
                request_id: None,
                message_id: "",
                lower_uuid: "55555555-5555-4555-8555-555555555555",
                higher_uuid: "55555555-5555-4555-8555-555555555555",
                expected_id: "55555555-5555-4555-8555-555555555555",
            },
        ];

        for case in cases {
            assert_claude_dedup_case(case).await;
        }
    }

    #[tokio::test]
    async fn analyze_directory_ignores_empty_files() {
        let result = analyze_with_files(&[("empty.jsonl", String::new())]).await;

        assert_empty_success(&result);
    }

    #[tokio::test]
    async fn analyze_directory_ignores_whitespace_only_files() {
        let result = analyze_with_files(&[("blank.jsonl", "\n   \n\t\n".to_string())]).await;

        assert_empty_success(&result);
    }

    #[tokio::test]
    async fn analyze_directory_ignores_non_jsonl_files() {
        let result = analyze_with_files(&[
            ("data.json", "{}".to_string()),
            ("notes.txt", "hello".to_string()),
        ])
        .await;

        assert_empty_success(&result);
    }

    #[tokio::test]
    async fn analyze_directory_discards_entire_file_on_parse_error() {
        let result = analyze_with_files(&[(
            "mixed.jsonl",
            format!(
                "{}\nnot-json\n{}\n",
                mock_log_json("msg-1", "model-a", CLAUDE_TIMESTAMP, "3"),
                mock_log_json("msg-2", "model-a", LATER_CLAUDE_TIMESTAMP, "4")
            ),
        )])
        .await;

        assert!(result.had_errors);
        assert!(result.lines.is_empty());
    }

    #[tokio::test]
    async fn analyze_directory_skips_claude_history_jsonl() {
        let temp_dir = temp_dir();
        let claude_log = claude_assistant_json(
            None,
            Some("req-1"),
            "msg-1",
            "22222222-2222-4222-8222-222222222222",
            "claude-sonnet-4-20250514",
            claude_usage_json(1, 0, 0, 0),
        );
        // `~/.claude/history.jsonl` follows a different schema than the per-session
        // transcripts, so the Claude implementation must skip it entirely instead of
        // reporting a parse error that would mark the whole scan as having had errors.
        write_log_files(
            temp_dir.path(),
            &[
                ("history.jsonl", "{\"display\":\"prompt\"}\n".to_string()),
                ("session.jsonl", format!("{}\n", claude_log)),
            ],
        )
        .await;

        let result = analyze_directory::<ClaudeLogLine>(temp_dir.path().to_path_buf()).await;

        assert!(!result.had_errors);
        assert_eq!(result.lines.len(), 1);
    }
}
