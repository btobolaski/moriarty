use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::{Datelike, NaiveDate, TimeZone, Utc};

use super::analyzer::*;
use super::pricing::{ModelCostsMap, ModelType, TokenCosts, TokenCounts};
use super::time_filter::TimeRangeFilter;

/// Helper function to create an empty time range filter for tests
fn empty_filter() -> TimeRangeFilter {
    TimeRangeFilter::new(None, None).expect("Empty filter should always be valid")
}

/// Helper to call aggregate_by_date with default unknown model tracking
fn aggregate_test(
    usages: Vec<DateBasedMessage>,
) -> std::collections::BTreeMap<NaiveDate, DailyUsage> {
    aggregate_test_with_lines(usages, Vec::new())
}

/// Helper to call aggregate_by_date with custom lines_changed data
fn aggregate_test_with_lines(
    usages: Vec<DateBasedMessage>,
    lines_changed: Vec<(NaiveDate, usize)>,
) -> std::collections::BTreeMap<NaiveDate, DailyUsage> {
    let mut unknown_models = HashSet::new();
    let mut total_unknown_tokens = TokenCounts::default();
    aggregate_by_date(
        usages,
        lines_changed,
        &mut unknown_models,
        &mut total_unknown_tokens,
    )
}

/// Helper to call aggregate_by_date and also return unknown model info
fn aggregate_test_with_unknowns(
    usages: Vec<DateBasedMessage>,
) -> (
    std::collections::BTreeMap<NaiveDate, DailyUsage>,
    HashSet<String>,
    TokenCounts,
) {
    let mut unknown_models = HashSet::new();
    let mut total_unknown_tokens = TokenCounts::default();
    let result = aggregate_by_date(
        usages,
        Vec::new(),
        &mut unknown_models,
        &mut total_unknown_tokens,
    );
    (result, unknown_models, total_unknown_tokens)
}

/// Helper to write content to a temp file and parse it as a log file
async fn parse_test_log(content: &str) -> Vec<DateBasedMessage> {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");
    tokio::fs::write(&file_path, content).await.unwrap();
    parse_log_file(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap()
}

/// Helper function to create a time range filter from date strings
fn date_filter(start: Option<&str>, end: Option<&str>) -> TimeRangeFilter {
    TimeRangeFilter::new(start.map(|s| s.to_string()), end.map(|s| s.to_string())).unwrap()
}

fn test_date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

fn daily_usage_with(date: NaiveDate, model: ModelType, counts: TokenCounts) -> DailyUsage {
    let mut usage = DailyUsage::new(date);
    usage.add_usage(model, counts);
    usage
}

async fn parse_lines_changed_lines(lines: &[String]) -> Vec<(NaiveDate, usize)> {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = write_test_jsonl(temp_dir.path(), lines).await;
    parse_lines_changed(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap()
}

async fn parse_test_lines(lines: &[String]) -> Vec<DateBasedMessage> {
    parse_test_log(&lines.join("\n")).await
}

/// Asserts `parse_lines_changed` returned exactly one `(date, lines)` tuple.
fn assert_single_lines_changed_entry(
    result: &[(NaiveDate, usize)],
    date: NaiveDate,
    expected_lines: usize,
    label: &str,
) {
    assert_eq!(result.len(), 1, "{label} should produce one entry");
    assert_eq!(
        result[0].0, date,
        "{label} should attribute to correct date"
    );
    assert_eq!(
        result[0].1, expected_lines,
        "{label} should report {expected_lines} lines"
    );
}

/// Asserts a single per-date per-model aggregate entry with the expected token counts.
fn assert_aggregate_usage(
    result: &std::collections::BTreeMap<NaiveDate, DailyUsage>,
    date: NaiveDate,
    model: ModelType,
    input_tokens: usize,
    output_tokens: usize,
    label: &str,
) {
    assert_eq!(
        result[&date].per_model.get(model).input_tokens,
        input_tokens,
        "case {label}"
    );
    assert_eq!(
        result[&date].per_model.get(model).output_tokens,
        output_tokens,
        "case {label}"
    );
}

async fn parse_test_file(file_path: &Path) -> Vec<DateBasedMessage> {
    parse_log_file(file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap()
}

const STREAMING_GROUP_INPUT_TOKENS: usize = 8;
const STREAMING_GROUP_OUTPUT_TOKENS: usize = 835;
const STREAMING_GROUP_CACHE_WRITE_TOKENS: usize = 17_932;

/// Asserts the parsed streaming group retained the final token counts after
/// request-id deduplication.
fn assert_streaming_group_token_counts(counts: TokenCounts) {
    assert_eq!(counts.input_tokens, STREAMING_GROUP_INPUT_TOKENS);
    assert_eq!(
        counts.output_tokens, STREAMING_GROUP_OUTPUT_TOKENS,
        "Should have final output count"
    );
    assert_eq!(
        counts.cache_write_tokens, STREAMING_GROUP_CACHE_WRITE_TOKENS,
        "Should count cache_write only once"
    );
}

/// Asserts the cache-write cost falls inside the expected streaming-dedup band.
fn assert_cache_write_cost_range(cost: f64, min: f64, max: f64, label: &str) {
    assert!(
        cost > min && cost < max,
        "{label}: expected cache_write in ({min}, {max}), got ${cost}"
    );
}

/// Helper function to create a DateBasedMessage for tests
fn create_date_based_message(
    date: NaiveDate,
    model_type: ModelType,
    model_string: String,
    token_counts: TokenCounts,
) -> DateBasedMessage {
    DateBasedMessage {
        date,
        model_type,
        model_string,
        token_counts,
        request_id: None,
        timestamp: Utc
            .with_ymd_and_hms(date.year(), date.month(), date.day(), 0, 0, 0)
            .unwrap(),
    }
}

/// Shortcut: Sonnet date-based test message on the given date with the given counts.
fn sonnet_msg(date: NaiveDate, counts: TokenCounts) -> DateBasedMessage {
    create_date_based_message(
        date,
        ModelType::Sonnet,
        "claude-sonnet-4".to_string(),
        counts,
    )
}

/// Shortcut: Haiku date-based test message on the given date with the given counts.
fn haiku_msg(date: NaiveDate, counts: TokenCounts) -> DateBasedMessage {
    create_date_based_message(date, ModelType::Haiku, "claude-haiku-3".to_string(), counts)
}

/// Shortcut: Unknown-model test message with caller-provided model string.
fn unknown_msg(date: NaiveDate, model: &str, counts: TokenCounts) -> DateBasedMessage {
    create_date_based_message(date, ModelType::Unknown, model.to_string(), counts)
}

/// Identifiers for a single assistant log line fixture.
struct TestMessageMeta<'a> {
    timestamp: &'a str,
    request_id: Option<&'a str>,
    session_id: &'a str,
    msg_id: &'a str,
    uuid: &'a str,
}

/// Builds a single assistant JSONL log line with text content.
fn make_assistant_jsonl(
    model: &str,
    token_counts: TokenCounts,
    meta: TestMessageMeta<'_>,
) -> String {
    let req_id = match meta.request_id {
        Some(id) => format!("\"{}\"", id),
        None => "null".to_string(),
    };
    format!(
        r#"{{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"{}","version":"1.0.0","gitBranch":"main","message":{{"id":"{}","type":"message","role":"assistant","model":"{}","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{{"input_tokens":{},"cache_creation_input_tokens":{},"cache_read_input_tokens":{},"cache_creation":{{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0}},"output_tokens":{},"service_tier":null,"server_tool_use":null}}}},"requestId":{},"uuid":"{}","timestamp":"{}","isApiErrorMessage":null}}"#,
        meta.session_id,
        meta.msg_id,
        model,
        token_counts.input_tokens,
        token_counts.cache_write_tokens,
        token_counts.cache_read_tokens,
        token_counts.output_tokens,
        req_id,
        meta.uuid,
        meta.timestamp
    )
}

/// Builds a JSONL log line with tool_use content (for lines-changed tests).
fn make_tool_use_jsonl(
    model: &str,
    tools_json: &str,
    timestamp: &str,
    msg_id: &str,
    uuid: &str,
) -> String {
    format!(
        r#"{{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{{"id":"{}","type":"message","role":"assistant","model":"{}","container":null,"content":[{}],"stop_reason":null,"stop_sequence":null,"usage":{{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0}},"output_tokens":500,"service_tier":null,"server_tool_use":null}}}},"requestId":null,"uuid":"{}","timestamp":"{}","isApiErrorMessage":null}}"#,
        msg_id, model, tools_json, uuid, timestamp
    )
}

/// Writes JSONL lines to a temp file and returns the path.
async fn write_test_jsonl(dir: &Path, lines: &[String]) -> PathBuf {
    let file_path = dir.join("test.jsonl");
    tokio::fs::write(&file_path, lines.join("\n"))
        .await
        .unwrap();
    file_path
}

/// Returns a standard UUID string for test message N.
/// N must be 0–9 (single digit) to produce a valid 36-char UUID.
fn test_uuid(n: u8) -> String {
    assert!(n <= 9, "test_uuid: N must be 0–9 to produce a valid UUID");
    format!("00000000-0000-0000-0000-00000000000{}", n)
}

/// Returns a standard message ID for test message N.
fn test_msg_id(n: u8) -> String {
    format!("msg_{}", n)
}

/// Builds an assistant JSONL line with canonical test IDs for message `n`.
fn assistant_msg(
    model: &str,
    counts: TokenCounts,
    timestamp: &str,
    session: &str,
    n: u8,
) -> String {
    assistant_msg_req(model, counts, timestamp, session, n, None)
}

/// Same as [`assistant_msg`] but allows setting a request ID for streaming/dedup tests.
fn assistant_msg_req(
    model: &str,
    counts: TokenCounts,
    timestamp: &str,
    session: &str,
    n: u8,
    request_id: Option<&str>,
) -> String {
    make_assistant_jsonl(
        model,
        counts,
        TestMessageMeta {
            timestamp,
            request_id,
            session_id: session,
            msg_id: &test_msg_id(n),
            uuid: &test_uuid(n),
        },
    )
}

/// Builds a tool_use JSONL line with canonical test IDs for message `n`.
fn tool_use_line(model: &str, tools: &str, timestamp: &str, n: u8) -> String {
    make_tool_use_jsonl(model, tools, timestamp, &test_msg_id(n), &test_uuid(n))
}

/// Options for building a "forked conversation" assistant JSONL line used by the
/// cross-file deduplication tests. The resulting line has an array-shaped
/// `content` with a single text block, `userType: "external"`, and a fixed
/// `parentUuid` — the shape actually produced by Claude Code forked sessions.
///
/// **Constraints**: every field is interpolated verbatim into a JSON string
/// literal, so values must not contain `"`, `\`, or raw `{` / `}` characters.
/// A malformed value produces invalid JSON and a confusing parse failure in
/// the caller rather than a clear helper misuse.
struct ForkedMsgOpts<'a> {
    session_id: &'a str,
    msg_id: &'a str,
    request_id: &'a str,
    uuid: &'a str,
    timestamp: &'a str,
    text: &'a str,
    counts: TokenCounts,
}

/// Writes the same-or-different pair of pre-built JSONL messages to
/// `session-a.jsonl` / `session-b.jsonl` for cross-file dedup tests.
async fn write_two_sessions(dir: &Path, a_name: &str, a: &str, b_name: &str, b: &str) {
    tokio::fs::write(dir.join(format!("{a_name}.jsonl")), a)
        .await
        .unwrap();
    tokio::fs::write(dir.join(format!("{b_name}.jsonl")), b)
        .await
        .unwrap();
}

/// Asserts `actual` is within 1e-10 of `tokens * rate_per_million / 1_000_000`.
/// Used by the forked-conversation cost-dedup tests.
fn assert_cost_close(actual: f64, tokens: f64, rate_per_million: f64, label: &str) {
    let expected = tokens * rate_per_million / 1_000_000.0;
    assert!(
        (actual - expected).abs() < 1e-10,
        "{label}: expected ~{expected}, got {actual}",
    );
}

/// Runs a sequence of `TimeFilterCase`s against a shared log file, asserting
/// each case produces the expected `input_tokens` sequence from
/// `parse_log_file`.
async fn run_time_filter_cases(file: &Path, cases: &[TimeFilterCase]) {
    for (name, start, end, expected) in cases {
        let filter = date_filter(*start, *end);
        let result = parse_log_file(file, DateTimezone::Utc, &filter)
            .await
            .unwrap();
        let got: Vec<usize> = result
            .iter()
            .map(|msg| msg.token_counts.input_tokens)
            .collect();
        assert_eq!(got, *expected, "case {name}");
    }
}

/// Builds a forked-conversation assistant JSONL line for cross-file dedup tests.
fn forked_assistant_msg(opts: ForkedMsgOpts<'_>) -> String {
    format!(
        r#"{{"type":"assistant","parentUuid":"bb0252ce-8926-4f5c-b616-fa5743f365de","isSidechain":false,"userType":"external","cwd":"/test","sessionId":"{session_id}","version":"2.0.32","gitBranch":"main","message":{{"model":"claude-sonnet-4","id":"{msg_id}","type":"message","role":"assistant","content":[{{"type":"text","text":"{text}"}}],"stop_reason":null,"stop_sequence":null,"usage":{{"input_tokens":{input},"cache_creation_input_tokens":{cache_write},"cache_read_input_tokens":{cache_read},"cache_creation":{{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0}},"output_tokens":{output},"service_tier":"standard"}}}},"requestId":"{request_id}","uuid":"{uuid}","timestamp":"{timestamp}","isApiErrorMessage":null}}"#,
        session_id = opts.session_id,
        msg_id = opts.msg_id,
        text = opts.text,
        input = opts.counts.input_tokens,
        cache_write = opts.counts.cache_write_tokens,
        cache_read = opts.counts.cache_read_tokens,
        output = opts.counts.output_tokens,
        request_id = opts.request_id,
        uuid = opts.uuid,
        timestamp = opts.timestamp,
    )
}

#[test]
fn test_daily_usage_new() {
    let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
    let usage = DailyUsage::new(date);

    assert_eq!(usage.date, date);
    assert_eq!(usage.per_model.get(ModelType::Sonnet).input_tokens, 0);
    assert_eq!(usage.per_model.get(ModelType::Haiku).input_tokens, 0);
    assert_eq!(usage.per_model.get(ModelType::Opus).input_tokens, 0);
    assert_eq!(usage.per_model.get(ModelType::Opus4).input_tokens, 0);
    assert_eq!(usage.per_model.get(ModelType::Unknown).input_tokens, 0);
    assert_eq!(usage.lines_changed, 0);
}

#[test]
fn test_daily_usage_add_routes_tokens_to_correct_model() {
    // Adding usage for any single model must record tokens under that model only
    // and leave the other model slots at zero.
    let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
    let all_models = [
        ModelType::Sonnet,
        ModelType::Haiku,
        ModelType::Opus,
        ModelType::Opus4,
        ModelType::Unknown,
    ];

    for model in all_models {
        let mut usage = DailyUsage::new(date);
        usage.add_usage(model, TokenCounts::new(1000, 500, 100, 50));

        assert_eq!(usage.per_model.get(model).input_tokens, 1000, "{:?}", model);
        assert_eq!(usage.per_model.get(model).output_tokens, 500, "{:?}", model);

        for other in all_models {
            if other != model {
                assert_eq!(
                    usage.per_model.get(other).input_tokens,
                    0,
                    "{:?} should not leak into {:?}",
                    model,
                    other
                );
            }
        }
    }
}

#[test]
fn test_daily_usage_add_multiple_models() {
    let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
    let mut usage = DailyUsage::new(date);

    usage.add_usage(ModelType::Sonnet, TokenCounts::new(1000, 500, 0, 0));
    usage.add_usage(ModelType::Haiku, TokenCounts::new(2000, 1000, 0, 0));

    assert_eq!(usage.per_model.get(ModelType::Sonnet).input_tokens, 1000);
    assert_eq!(usage.per_model.get(ModelType::Haiku).input_tokens, 2000);
}

#[test]
fn test_daily_usage_add_accumulates() {
    let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
    let mut usage = DailyUsage::new(date);

    usage.add_usage(ModelType::Sonnet, TokenCounts::new(1000, 500, 100, 50));
    usage.add_usage(ModelType::Sonnet, TokenCounts::new(500, 250, 50, 25));

    assert_eq!(usage.per_model.get(ModelType::Sonnet).input_tokens, 1500);
    assert_eq!(usage.per_model.get(ModelType::Sonnet).output_tokens, 750);
    assert_eq!(
        usage.per_model.get(ModelType::Sonnet).cache_write_tokens,
        150
    );
    assert_eq!(usage.per_model.get(ModelType::Sonnet).cache_read_tokens, 75);
}

#[test]
fn test_daily_usage_calculate_costs() {
    let date = test_date(2025, 10, 23);
    let costs = daily_usage_with(
        date,
        ModelType::Sonnet,
        TokenCounts::new(1_000_000, 1_000_000, 0, 0),
    )
    .calculate_costs();

    assert_eq!(costs.date, date);
    assert_eq!(costs.per_model.get(ModelType::Sonnet).input, 3.0);
    assert_eq!(costs.per_model.get(ModelType::Sonnet).output, 15.0);
    assert_eq!(costs.lines_changed, 0);
}

#[test]
fn test_daily_usage_calculate_costs_opus4() {
    let date = test_date(2025, 10, 23);
    let costs = daily_usage_with(
        date,
        ModelType::Opus4,
        TokenCounts::new(1_000_000, 1_000_000, 0, 0),
    )
    .calculate_costs();

    assert_eq!(costs.date, date);
    assert_eq!(costs.per_model.get(ModelType::Opus4).input, 5.0);
    assert_eq!(costs.per_model.get(ModelType::Opus4).output, 25.0);
    assert_eq!(costs.per_model.get(ModelType::Opus4).total(), 30.0);
    assert_eq!(costs.lines_changed, 0);
}

#[test]
fn test_daily_costs_total() {
    let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
    let mut model_costs = ModelCostsMap::default();
    model_costs.set(
        ModelType::Sonnet,
        TokenCosts {
            input: 1.0,
            output: 2.0,
            cache_write: 0.5,
            cache_read: 0.25,
        },
    );
    model_costs.set(
        ModelType::Haiku,
        TokenCosts {
            input: 0.5,
            output: 1.0,
            cache_write: 0.25,
            cache_read: 0.1,
        },
    );
    let costs = DailyCosts {
        date,
        per_model: model_costs,
        lines_changed: 0,
    };

    assert!((costs.total() - 5.6).abs() < 1e-10);
}

#[test]
fn test_aggregate_by_date_empty() {
    let (result, unknown_models, _) = aggregate_test_with_unknowns(Vec::new());
    assert!(result.is_empty());
    assert!(unknown_models.is_empty());
}

#[test]
fn test_aggregate_by_date_usage_variants() {
    let cases = [
        (
            "single entry",
            vec![sonnet_msg(
                test_date(2025, 10, 23),
                TokenCounts::new(1000, 500, 0, 0),
            )],
            vec![(test_date(2025, 10, 23), ModelType::Sonnet, 1000, 500)],
        ),
        (
            "multiple dates",
            vec![
                sonnet_msg(test_date(2025, 10, 23), TokenCounts::new(1000, 500, 0, 0)),
                haiku_msg(test_date(2025, 10, 24), TokenCounts::new(2000, 1000, 0, 0)),
            ],
            vec![
                (test_date(2025, 10, 23), ModelType::Sonnet, 1000, 500),
                (test_date(2025, 10, 24), ModelType::Haiku, 2000, 1000),
            ],
        ),
        (
            "same date accumulates",
            vec![
                sonnet_msg(test_date(2025, 10, 23), TokenCounts::new(1000, 500, 0, 0)),
                sonnet_msg(test_date(2025, 10, 23), TokenCounts::new(500, 250, 0, 0)),
            ],
            vec![(test_date(2025, 10, 23), ModelType::Sonnet, 1500, 750)],
        ),
    ];

    for (label, usages, expected) in cases {
        let result = aggregate_test(usages);
        assert_eq!(result.len(), expected.len(), "{label}");
        for (date, model, input_tokens, output_tokens) in expected {
            assert_aggregate_usage(&result, date, model, input_tokens, output_tokens, label);
        }
    }
}

#[test]
fn test_aggregate_by_date_tracks_unknown_models() {
    let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();

    let usages = vec![
        unknown_msg(date, "gemini-pro", TokenCounts::new(1000, 500, 100, 50)),
        unknown_msg(date, "gpt-4", TokenCounts::new(500, 250, 0, 0)),
    ];
    let (result, unknown_models, total_unknown_tokens) = aggregate_test_with_unknowns(usages);

    assert_eq!(unknown_models.len(), 2);
    assert!(unknown_models.contains("gemini-pro"));
    assert!(unknown_models.contains("gpt-4"));
    assert_eq!(total_unknown_tokens.input_tokens, 1500);
    assert_eq!(total_unknown_tokens.output_tokens, 750);
    assert_eq!(total_unknown_tokens.cache_write_tokens, 100);
    assert_eq!(total_unknown_tokens.cache_read_tokens, 50);
    assert_eq!(
        result[&date].per_model.get(ModelType::Unknown).input_tokens,
        1500
    );
}

#[test]
fn test_aggregate_by_date_sorted_by_date() {
    let date1 = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
    let date2 = NaiveDate::from_ymd_opt(2025, 10, 21).unwrap();
    let date3 = NaiveDate::from_ymd_opt(2025, 10, 25).unwrap();

    let usages = vec![
        sonnet_msg(date1, TokenCounts::default()),
        sonnet_msg(date2, TokenCounts::default()),
        sonnet_msg(date3, TokenCounts::default()),
    ];
    let result = aggregate_test(usages);
    let dates: Vec<_> = result.keys().collect();

    assert_eq!(dates, vec![&date2, &date1, &date3]);
}

#[tokio::test]
async fn test_find_jsonl_files_variants() {
    enum Case {
        Empty,
        NoJsonl,
        Single,
        Recursive,
        Deep,
    }

    for case in [
        Case::Empty,
        Case::NoJsonl,
        Case::Single,
        Case::Recursive,
        Case::Deep,
    ] {
        let temp_dir = tempfile::tempdir().unwrap();
        let (label, expected_len, expected_suffix): (&str, usize, Option<&str>) = match case {
            Case::Empty => ("empty", 0, None),
            Case::NoJsonl => {
                tokio::fs::write(temp_dir.path().join("test.txt"), "content")
                    .await
                    .unwrap();
                tokio::fs::write(temp_dir.path().join("test.json"), "{}")
                    .await
                    .unwrap();
                ("no_jsonl", 0, None)
            }
            Case::Single => {
                tokio::fs::write(temp_dir.path().join("test.jsonl"), "")
                    .await
                    .unwrap();
                ("single", 1, Some("test.jsonl"))
            }
            Case::Recursive => {
                let subdir = temp_dir.path().join("subdir");
                tokio::fs::create_dir(&subdir).await.unwrap();
                tokio::fs::write(temp_dir.path().join("root.jsonl"), "")
                    .await
                    .unwrap();
                tokio::fs::write(subdir.join("nested.jsonl"), "")
                    .await
                    .unwrap();
                ("recursive", 2, None)
            }
            Case::Deep => {
                let deep_path = temp_dir.path().join("a").join("b").join("c");
                tokio::fs::create_dir_all(&deep_path).await.unwrap();
                tokio::fs::write(deep_path.join("deep.jsonl"), "")
                    .await
                    .unwrap();
                ("deep", 1, Some("deep.jsonl"))
            }
        };

        let result = find_jsonl_files(temp_dir.path()).await.unwrap();
        assert_eq!(result.len(), expected_len, "case: {label}");
        if let Some(expected_suffix) = expected_suffix {
            assert!(result[0].ends_with(expected_suffix), "case: {label}");
        }
    }
}

#[tokio::test]
async fn test_parse_log_file_empty_file() {
    let result = parse_test_log("").await;
    assert!(result.is_empty());
}

#[tokio::test]
async fn test_parse_log_file_extracts_usage_correctly() {
    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4-20250514","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":100,"cache_read_input_tokens":50,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
    let result = parse_test_log(log_content).await;

    assert_eq!(result.len(), 1);
    let msg = &result[0];

    assert_eq!(msg.date, NaiveDate::from_ymd_opt(2025, 10, 23).unwrap());
    assert_eq!(msg.model_type, ModelType::Sonnet);
    assert_eq!(msg.model_string, "claude-sonnet-4-20250514");
    assert_eq!(msg.token_counts.input_tokens, 1000);
    assert_eq!(msg.token_counts.output_tokens, 500);
    assert_eq!(msg.token_counts.cache_write_tokens, 100);
    assert_eq!(msg.token_counts.cache_read_tokens, 50);
}

#[tokio::test]
async fn test_parse_log_file_handles_multiple_assistant_messages() {
    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-haiku-3","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":1000,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-24T12:00:00Z","isApiErrorMessage":null}"#;
    let result = parse_test_log(log_content).await;

    assert_eq!(result.len(), 2);

    assert_eq!(result[0].model_type, ModelType::Sonnet);
    assert_eq!(result[0].token_counts.input_tokens, 1000);

    assert_eq!(result[1].model_type, ModelType::Haiku);
    assert_eq!(result[1].token_counts.input_tokens, 2000);
}

#[tokio::test]
async fn test_parse_log_file_ignores_non_assistant_messages() {
    let log_content = r#"{"type":"user","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"role":"user","content":"test"},"isMeta":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T11:00:00Z","toolUseResult":null,"thinkingMetadata":null,"isVisibleInTranscriptOnly":null,"isCompactSummary":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
    let result = parse_test_log(log_content).await;

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].model_type, ModelType::Sonnet);
}

#[tokio::test]
async fn test_analyze_directory_empty() {
    let temp_dir = tempfile::tempdir().unwrap();
    let result = analyze_directory(temp_dir.path(), DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

    assert!(result.daily_costs.is_empty());
    assert_eq!(result.files_parsed, 0);
    assert_eq!(result.files_failed, 0);
}

#[tokio::test]
async fn test_analyze_directory_with_invalid_jsonl() {
    let temp_dir = tempfile::tempdir().unwrap();
    tokio::fs::write(temp_dir.path().join("invalid.jsonl"), "not json")
        .await
        .unwrap();

    let result = analyze_directory(temp_dir.path(), DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();
    assert!(result.daily_costs.is_empty());
    assert_eq!(result.files_parsed, 0);
    assert_eq!(result.files_failed, 1);
}

#[test]
fn test_daily_usage_add_lines_changed() {
    let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
    let mut usage = DailyUsage::new(date);

    usage.add_lines_changed(100);
    assert_eq!(usage.lines_changed, 100);

    usage.add_lines_changed(50);
    assert_eq!(usage.lines_changed, 150);
}

#[test]
fn test_aggregate_by_date_with_lines_changed() {
    let date = test_date(2025, 10, 23);

    let usages = vec![sonnet_msg(date, TokenCounts::default())];

    let lines_changed = vec![(date, 100), (date, 50)];
    let result = aggregate_test_with_lines(usages, lines_changed);

    assert_eq!(result[&date].lines_changed, 150);
}

#[test]
fn test_aggregate_by_date_lines_changed_different_dates() {
    let date1 = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
    let date2 = NaiveDate::from_ymd_opt(2025, 10, 24).unwrap();

    let lines_changed = vec![(date1, 100), (date2, 200), (date1, 50)];
    let result = aggregate_test_with_lines(Vec::new(), lines_changed);

    assert_eq!(result[&date1].lines_changed, 150);
    assert_eq!(result[&date2].lines_changed, 200);
}

#[tokio::test]
async fn test_parse_lines_changed_counts_single_tool_calls() {
    // Each `parse_lines_changed` scenario is: one assistant message containing a
    // single tool_use JSON blob, produce one `(date, lines)` result matching the
    // tool's line impact.
    let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
    let cases: &[(&str, &str, usize)] = &[
        (
            "Edit",
            r#"{"type":"tool_use","id":"tool_1","name":"Edit","input":{"file_path":"/test.rs","old_string":"line1\nline2","new_string":"line1\nmodified\nline3"}}"#,
            3,
        ),
        (
            "Write",
            r#"{"type":"tool_use","id":"tool_1","name":"Write","input":{"file_path":"/test.rs","content":"line1\nline2\nline3\nline4"}}"#,
            4,
        ),
        (
            "NotebookEdit",
            r#"{"type":"tool_use","id":"tool_1","name":"NotebookEdit","input":{"notebook_path":"/test.ipynb","new_source":"print('hello')\nprint('world')"}}"#,
            2,
        ),
    ];

    for (label, tools, expected_lines) in cases {
        let result = parse_lines_changed_lines(&[tool_use_line(
            "claude-sonnet-4",
            tools,
            "2025-10-23T12:00:00Z",
            1,
        )])
        .await;
        assert_single_lines_changed_entry(&result, date, *expected_lines, label);
    }
}

#[tokio::test]
async fn test_parse_lines_changed_multiple_tools_same_message() {
    let tools = r#"{"type":"tool_use","id":"tool_1","name":"Edit","input":{"file_path":"/test.rs","old_string":"old","new_string":"new"}},{"type":"tool_use","id":"tool_2","name":"Write","input":{"file_path":"/test2.rs","content":"line1\nline2"}}"#;
    let result = parse_lines_changed_lines(&[tool_use_line(
        "claude-sonnet-4",
        tools,
        "2025-10-23T12:00:00Z",
        1,
    )])
    .await;
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].0, test_date(2025, 10, 23));
    assert_eq!(result[1].0, test_date(2025, 10, 23));
}

#[tokio::test]
async fn test_parse_lines_changed_empty_file() {
    let result = parse_lines_changed_lines(&[]).await;
    assert!(result.is_empty());
}

#[tokio::test]
async fn test_parse_lines_changed_ignores_non_counted_cases() {
    let cases = [
        ("empty", Vec::new()),
        (
            "assistant without tool uses",
            vec![assistant_msg(
                "claude-sonnet-4",
                TokenCounts::new(1000, 500, 0, 0),
                "2025-10-23T12:00:00Z",
                "00000000-0000-0000-0000-000000000000",
                1,
            )],
        ),
        (
            "non-modifying read tool",
            vec![tool_use_line(
                "claude-sonnet-4",
                r#"{"type":"tool_use","id":"tool_1","name":"Read","input":{"file_path":"/test.rs"}}"#,
                "2025-10-23T12:00:00Z",
                1,
            )],
        ),
        (
            "all synthetic tool uses",
            vec![
                make_tool_use_jsonl(
                    "<synthetic>",
                    r#"{"type":"tool_use","id":"tool_1","name":"Edit","input":{"file_path":"/test.rs","old_string":"old","new_string":"new"}}"#,
                    "2025-10-23T12:00:00Z",
                    &test_msg_id(1),
                    &test_uuid(1),
                ),
                make_tool_use_jsonl(
                    "<synthetic>",
                    r#"{"type":"tool_use","id":"tool_2","name":"Write","input":{"file_path":"/test2.rs","content":"test"}}"#,
                    "2025-10-23T12:00:00Z",
                    &test_msg_id(2),
                    &test_uuid(2),
                ),
            ],
        ),
    ];

    for (label, lines) in cases {
        let result = parse_lines_changed_lines(&lines).await;
        assert!(result.is_empty(), "case {label}: expected empty result");
    }
}

#[tokio::test]
async fn test_parse_log_file_synthetic_filter_variants() {
    let sess = "00000000-0000-0000-0000-000000000000";
    let ts = "2025-10-23T12:00:00Z";
    let cases = [
        (
            [
                assistant_msg(
                    "<synthetic>",
                    TokenCounts::new(1000, 500, 0, 0),
                    ts,
                    sess,
                    1,
                ),
                assistant_msg(
                    "claude-sonnet-4",
                    TokenCounts::new(2000, 1000, 100, 50),
                    ts,
                    sess,
                    2,
                ),
            ],
            Some(("claude-sonnet-4", 2000)),
        ),
        (
            [
                assistant_msg(
                    "<SYNTHETIC>",
                    TokenCounts::new(1000, 500, 0, 0),
                    ts,
                    sess,
                    1,
                ),
                assistant_msg(
                    "<Synthetic>",
                    TokenCounts::new(1500, 750, 0, 0),
                    ts,
                    sess,
                    2,
                ),
            ],
            None,
        ),
        (
            [
                assistant_msg(
                    "<synthetic>",
                    TokenCounts::new(1000, 500, 0, 0),
                    ts,
                    sess,
                    1,
                ),
                assistant_msg(
                    "<synthetic>",
                    TokenCounts::new(2000, 1000, 0, 0),
                    ts,
                    sess,
                    2,
                ),
            ],
            None,
        ),
    ];

    for (index, (lines, expected)) in cases.into_iter().enumerate() {
        let label = format!("synthetic-case-{index}");
        let result = parse_test_lines(&lines).await;
        if let Some((model, input_tokens)) = expected {
            assert_eq!(result.len(), 1, "case {label}");
            assert_eq!(result[0].model_string, model, "case {label}");
            assert_eq!(
                result[0].token_counts.input_tokens, input_tokens,
                "case {label}"
            );
        } else {
            assert!(result.is_empty(), "case {label}: expected empty result");
        }
    }
}

#[tokio::test]
async fn test_parse_lines_changed_synthetic_filter_variants() {
    let ts = "2025-10-23T12:00:00Z";
    let cases = [
        (
            vec![
                tool_use_line(
                    "<synthetic>",
                    r#"{"type":"tool_use","id":"tool_1","name":"Edit","input":{"file_path":"/test.rs","old_string":"old","new_string":"new\ncode"}}"#,
                    ts,
                    1,
                ),
                tool_use_line(
                    "claude-sonnet-4",
                    r#"{"type":"tool_use","id":"tool_2","name":"Edit","input":{"file_path":"/test2.rs","old_string":"a","new_string":"b\nc"}}"#,
                    ts,
                    2,
                ),
            ],
            Some((test_date(2025, 10, 23), 3, "synthetic filter")),
        ),
        (
            vec![
                tool_use_line(
                    "<SYNTHETIC>",
                    r#"{"type":"tool_use","id":"tool_1","name":"Edit","input":{"file_path":"/test.rs","old_string":"old","new_string":"new"}}"#,
                    ts,
                    1,
                ),
                tool_use_line(
                    "<Synthetic>",
                    r#"{"type":"tool_use","id":"tool_2","name":"Write","input":{"file_path":"/test2.rs","content":"line1\nline2"}}"#,
                    ts,
                    2,
                ),
            ],
            None,
        ),
    ];

    for (index, (lines, expected)) in cases.into_iter().enumerate() {
        let label = format!("synthetic-lines-case-{index}");
        let result = parse_lines_changed_lines(&lines).await;
        if let Some((date, expected_lines, expected_label)) = expected {
            assert_single_lines_changed_entry(&result, date, expected_lines, expected_label);
        } else {
            assert!(result.is_empty(), "case {label}: expected empty result");
        }
    }
}

// ============================================================================
// Tests for streaming message deduplication
// ============================================================================
// Streaming API responses send multiple partial messages with the same requestId.
// These tests verify that we correctly deduplicate by keeping only the message
// with the highest output_tokens (the final, complete message).

/// Writes a 4-message streaming group for `req-123`; the final message has 835 output tokens.
async fn write_streaming_group_file(dir: &Path) -> PathBuf {
    let sess = "session-1";
    let lines: Vec<String> = (1..=4)
        .map(|i| {
            let output = if i == 4 { 835 } else { 2 };
            let ts = format!("2025-10-23T12:00:0{}Z", i - 1);
            assistant_msg_req(
                "claude-sonnet-4",
                TokenCounts::new(8, output, 17932, 0),
                &ts,
                sess,
                i,
                Some("req-123"),
            )
        })
        .collect();

    write_test_jsonl(dir, &lines).await
}

#[tokio::test]
async fn test_parse_log_file_deduplicates_streaming_messages_by_request_id() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = write_streaming_group_file(temp_dir.path()).await;

    let result = parse_test_file(&file_path).await;
    assert_eq!(
        result.len(),
        1,
        "streaming dedup should produce one message"
    );
    let msg = &result[0];

    assert_eq!(msg.model_type, ModelType::Sonnet);
    assert_streaming_group_token_counts(msg.token_counts);
}

#[tokio::test]
async fn test_parse_log_file_by_session_deduplicates_streaming_messages() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = write_streaming_group_file(temp_dir.path()).await;

    let result = parse_log_file_by_session(&file_path, &empty_filter())
        .await
        .unwrap();
    assert_eq!(
        result.len(),
        1,
        "streaming dedup should produce one session message"
    );
    let msg = &result[0];

    assert_eq!(msg.session_id, "session-1");
    assert_streaming_group_token_counts(msg.token_counts);
}

#[tokio::test]
async fn test_parse_log_file_handles_multiple_streaming_groups() {
    let temp_dir = tempfile::tempdir().unwrap();
    let sess = "session-1";
    let lines = vec![
        // Request group 1: 3 messages, final has 835 output tokens
        assistant_msg_req(
            "claude-sonnet-4",
            TokenCounts::new(8, 2, 17932, 0),
            "2025-10-23T12:00:00Z",
            sess,
            1,
            Some("req-123"),
        ),
        assistant_msg_req(
            "claude-sonnet-4",
            TokenCounts::new(8, 2, 17932, 0),
            "2025-10-23T12:00:01Z",
            sess,
            2,
            Some("req-123"),
        ),
        assistant_msg_req(
            "claude-sonnet-4",
            TokenCounts::new(8, 835, 17932, 0),
            "2025-10-23T12:00:02Z",
            sess,
            3,
            Some("req-123"),
        ),
        // Request group 2: 2 messages, final has 867 output tokens
        assistant_msg_req(
            "claude-sonnet-4",
            TokenCounts::new(12, 2, 5361, 17932),
            "2025-10-23T12:01:00Z",
            sess,
            4,
            Some("req-456"),
        ),
        assistant_msg_req(
            "claude-sonnet-4",
            TokenCounts::new(12, 867, 5361, 17932),
            "2025-10-23T12:01:01Z",
            sess,
            5,
            Some("req-456"),
        ),
    ];
    let file_path = write_test_jsonl(temp_dir.path(), &lines).await;

    let result = parse_log_file(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

    assert_eq!(
        result.len(),
        2,
        "Should have 2 entries (one per request), but got {}",
        result.len()
    );
    assert_eq!(result[0].token_counts.output_tokens, 835);
    assert_eq!(result[0].token_counts.cache_write_tokens, 17932);
    assert_eq!(result[1].token_counts.output_tokens, 867);
    assert_eq!(result[1].token_counts.cache_write_tokens, 5361);
    assert_eq!(result[1].token_counts.cache_read_tokens, 17932);
}

#[tokio::test]
async fn test_parse_log_file_preserves_non_streaming_messages() {
    let temp_dir = tempfile::tempdir().unwrap();
    let line = assistant_msg(
        "claude-sonnet-4",
        TokenCounts::new(1000, 500, 0, 0),
        "2025-10-23T12:00:00Z",
        "session-1",
        1,
    );
    let file_path = write_test_jsonl(temp_dir.path(), &[line]).await;

    let result = parse_log_file(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].token_counts.input_tokens, 1000);
    assert_eq!(result[0].token_counts.output_tokens, 500);
}

#[test]
fn test_aggregate_by_date_streaming_cost_variants() {
    let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
    let cases = [
        (
            vec![sonnet_msg(date, TokenCounts::new(8, 835, 17932, 0))],
            17932,
            0.06,
            0.07,
            "deduplicated streaming data",
        ),
        (
            vec![
                sonnet_msg(date, TokenCounts::new(8, 2, 17932, 0)),
                sonnet_msg(date, TokenCounts::new(8, 2, 17932, 0)),
                sonnet_msg(date, TokenCounts::new(8, 2, 17932, 0)),
                sonnet_msg(date, TokenCounts::new(8, 835, 17932, 0)),
            ],
            17932 * 4,
            0.26,
            0.28,
            "buggy non-deduplicated data",
        ),
    ];

    for (usages, expected_cache_write_tokens, min, max, label) in cases {
        let result = aggregate_test(usages);
        assert_eq!(result.len(), 1, "case {label}");
        let daily_usage = &result[&date];
        assert_eq!(
            daily_usage
                .per_model
                .get(ModelType::Sonnet)
                .cache_write_tokens,
            expected_cache_write_tokens,
            "case {label}"
        );

        let daily_costs = daily_usage.calculate_costs();
        assert_cache_write_cost_range(
            daily_costs.per_model.get(ModelType::Sonnet).cache_write,
            min,
            max,
            label,
        );
    }
}

#[tokio::test]
async fn test_parse_log_file_handles_zero_output_tokens() {
    // Edge case: all messages in streaming group have zero output_tokens
    // Should keep the first message encountered
    let temp_dir = tempfile::tempdir().unwrap();
    let sess = "00000000-0000-0000-0000-000000000000";
    let lines: Vec<String> = (1..=3)
        .map(|i| {
            let ts = format!("2025-10-23T12:00:0{}Z", i - 1);
            assistant_msg_req(
                "claude-sonnet-4",
                TokenCounts::new(10, 0, 100, 0),
                &ts,
                sess,
                i,
                Some("req-123"),
            )
        })
        .collect();
    let file_path = write_test_jsonl(temp_dir.path(), &lines).await;

    let result = parse_log_file(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
    let msg = &result[0];
    // When all have zero output_tokens, keeps the first encountered
    assert_eq!(msg.token_counts.output_tokens, 0);
    assert_eq!(msg.token_counts.input_tokens, 10);
    assert_eq!(msg.token_counts.cache_write_tokens, 100);
}

// ============================================================================
// TIME FILTERING INTEGRATION TESTS
// ============================================================================

/// Writes assistant messages at the given timestamps for `session-1`.
///
/// Each message has input_tokens = `1000 * i` and output_tokens = `100 * i` where `i` is
/// the 1-based index, so callers can differentiate messages by their token counts.
///
/// # Panics
///
/// Panics if `timestamps.len() > 9`; message indices are used as the trailing
/// digit of a UUID via [`test_uuid`], which rejects `n > 9`.
async fn write_messages_at_times(dir: &Path, timestamps: &[&str]) -> PathBuf {
    let sess = "session-1";
    let lines: Vec<String> = timestamps
        .iter()
        .enumerate()
        .map(|(idx, ts)| {
            let i = idx + 1;
            assistant_msg(
                "claude-sonnet-4",
                TokenCounts::new(1000 * i, 100 * i, 0, 0),
                ts,
                sess,
                (idx + 1) as u8,
            )
        })
        .collect();
    write_test_jsonl(dir, &lines).await
}

/// Creates 3 time-filter test messages at 10:00, 12:00, 14:00 with input_tokens 1000/2000/3000.
async fn write_three_time_messages(dir: &Path) -> PathBuf {
    write_messages_at_times(
        dir,
        &[
            "2025-01-01T10:00:00Z",
            "2025-01-01T12:00:00Z",
            "2025-01-01T14:00:00Z",
        ],
    )
    .await
}

/// Row in the time-filter table-driven tests: `(case_name, start, end,
/// expected_input_token_counts)`.
type TimeFilterCase = (
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
    &'static [usize],
);

/// Table-driven test covering start-only, end-only, and range filtering over
/// the three-message fixture at 10:00/12:00/14:00.
#[tokio::test]
async fn test_parse_log_file_time_filter_variants() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = write_three_time_messages(temp_dir.path()).await;

    let cases: &[TimeFilterCase] = &[
        (
            "start_only",
            Some("2025-01-01T11:00:00Z"),
            None,
            &[2000, 3000],
        ),
        ("end_only", None, Some("2025-01-01T12:00:00Z"), &[1000]),
        (
            "range",
            Some("2025-01-01T11:00:00Z"),
            Some("2025-01-01T13:00:00Z"),
            &[2000],
        ),
    ];

    run_time_filter_cases(&file_path, cases).await;
}

#[tokio::test]
async fn test_parse_log_file_filter_excludes_all_messages() {
    let temp_dir = tempfile::tempdir().unwrap();
    let line = assistant_msg(
        "claude-sonnet-4",
        TokenCounts::new(1000, 100, 0, 0),
        "2025-01-01T12:00:00Z",
        "session-1",
        1,
    );
    let file_path = write_test_jsonl(temp_dir.path(), &[line]).await;

    let filter = date_filter(Some("2024-01-01"), Some("2024-12-31"));
    let result = parse_log_file(&file_path, DateTimezone::Utc, &filter)
        .await
        .unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn test_parse_log_file_filter_with_date_only() {
    let temp_dir = tempfile::tempdir().unwrap();
    let sess = "session-1";
    let lines = vec![
        assistant_msg(
            "claude-sonnet-4",
            TokenCounts::new(1000, 100, 0, 0),
            "2025-01-31T00:00:00Z",
            sess,
            1,
        ),
        assistant_msg(
            "claude-sonnet-4",
            TokenCounts::new(2000, 200, 0, 0),
            "2025-01-31T12:00:00Z",
            sess,
            2,
        ),
        assistant_msg(
            "claude-sonnet-4",
            TokenCounts::new(3000, 300, 0, 0),
            "2025-01-31T23:59:59Z",
            sess,
            3,
        ),
        assistant_msg(
            "claude-sonnet-4",
            TokenCounts::new(4000, 400, 0, 0),
            "2025-02-01T00:00:00Z",
            sess,
            4,
        ),
    ];
    let file_path = write_test_jsonl(temp_dir.path(), &lines).await;

    let filter = date_filter(Some("2025-01-31"), Some("2025-01-31"));
    let result = parse_log_file(&file_path, DateTimezone::Utc, &filter)
        .await
        .unwrap();

    // Should include all 3 messages from Jan 31, but not Feb 1
    assert_eq!(result.len(), 3);
}

/// Creates 3 messages at 11:00, 12:00, 13:00 with input_tokens 1000/2000/3000 for boundary tests.
async fn write_boundary_test_messages(dir: &Path) -> PathBuf {
    write_messages_at_times(
        dir,
        &[
            "2025-01-01T11:00:00Z",
            "2025-01-01T12:00:00Z",
            "2025-01-01T13:00:00Z",
        ],
    )
    .await
}

/// Boundary-behaviour test: start is inclusive, end is exclusive.
#[tokio::test]
async fn test_parse_log_file_filter_boundaries() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = write_boundary_test_messages(temp_dir.path()).await;

    let cases: &[TimeFilterCase] = &[
        // start=12:00 includes the 12:00 message (inclusive)
        (
            "start_inclusive",
            Some("2025-01-01T12:00:00Z"),
            None,
            &[2000, 3000],
        ),
        // end=12:00 excludes the 12:00 message (exclusive)
        ("end_exclusive", None, Some("2025-01-01T12:00:00Z"), &[1000]),
    ];

    run_time_filter_cases(&file_path, cases).await;
}

#[tokio::test]
async fn test_parse_log_file_by_session_respects_filter() {
    let temp_dir = tempfile::tempdir().unwrap();
    let sess = "session-1";
    let lines = vec![
        assistant_msg(
            "claude-sonnet-4",
            TokenCounts::new(1000, 100, 0, 0),
            "2025-01-01T10:00:00Z",
            sess,
            1,
        ),
        assistant_msg(
            "claude-sonnet-4",
            TokenCounts::new(2000, 200, 0, 0),
            "2025-01-01T12:00:00Z",
            sess,
            2,
        ),
    ];
    let file_path = write_test_jsonl(temp_dir.path(), &lines).await;

    let filter = date_filter(Some("2025-01-01T11:00:00Z"), None);
    let result = parse_log_file_by_session(&file_path, &filter)
        .await
        .unwrap();

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].token_counts.input_tokens, 2000);
}

#[tokio::test]
async fn test_parse_lines_changed_respects_time_filter() {
    let temp_dir = tempfile::tempdir().unwrap();
    let write1 = r#"{"type":"tool_use","id":"tool_1","name":"Write","input":{"file_path":"/test/file1.txt","content":"line1\nline2\n"}}"#;
    let write2 = r#"{"type":"tool_use","id":"tool_2","name":"Write","input":{"file_path":"/test/file2.txt","content":"line1\nline2\nline3\n"}}"#;
    let lines = vec![
        tool_use_line("claude-sonnet-4", write1, "2025-01-01T10:00:00Z", 1),
        tool_use_line("claude-sonnet-4", write2, "2025-01-01T12:00:00Z", 2),
    ];
    let file_path = write_test_jsonl(temp_dir.path(), &lines).await;

    let filter = date_filter(Some("2025-01-01T11:00:00Z"), None);
    let result = parse_lines_changed(&file_path, DateTimezone::Utc, &filter)
        .await
        .unwrap();

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].1, 3);
}

// ============================================================================
// CROSS-FILE DEDUPLICATION TESTS
// ============================================================================
// When conversations are forked in Claude Code, all messages are copied to
// the new session file, resulting in duplicate messages with the same
// requestId and message.id appearing in multiple .jsonl files.
//
// This test verifies that analyze_directory properly deduplicates these
// cross-file duplicates to prevent inflating cost calculations.

#[tokio::test]
async fn test_analyze_directory_deduplicates_forked_conversation() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Simulate a conversation fork where the same message appears in two session files
    // This mirrors the real-world scenario from the grep output where msg_01GRWfK8FJtwwQVzS6LypbBp
    // appears in both a3933ed9-1121-4630-b197-3f2ddcaa8810.jsonl and
    // a4b1a3a4-f5e4-479a-9b96-26b4b57f04fd.jsonl

    // The exact same message (same requestId, message.id, uuid, and usage) in both files
    let shared_message = forked_assistant_msg(ForkedMsgOpts {
        session_id: "session-original",
        msg_id: "msg_01GRWfK8FJtwwQVzS6LypbBp",
        request_id: "req_011CUwGzDJueepzBaSgFrpQA",
        uuid: "70cd1811-0ecc-45b0-9f53-75bcb0758246",
        timestamp: "2025-11-08T23:18:07.276Z",
        text: "I'll list the directories",
        counts: TokenCounts::new(100, 500, 5000, 2000),
    });

    // Same shared message in two session files — tokens must be counted once.
    write_two_sessions(
        temp_dir.path(),
        "session-original",
        &shared_message,
        "session-forked",
        &shared_message,
    )
    .await;

    let result = analyze_directory(temp_dir.path(), DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

    assert_eq!(result.files_parsed, 2, "Should have parsed both files");
    assert_eq!(result.daily_costs.len(), 1, "Should have one day of usage");
    let daily = &result.daily_costs[0];
    let per_model = daily.per_model.get(ModelType::Sonnet);

    // If any assertion fails, values were doubled — cross-file dedup didn't run.
    assert_cost_close(per_model.input, 100.0, 3.0, "input counted once");
    assert_cost_close(per_model.output, 500.0, 15.0, "output counted once");
    assert_cost_close(
        per_model.cache_write,
        5000.0,
        3.75,
        "cache_write counted once",
    );
    assert_cost_close(
        per_model.cache_read,
        2000.0,
        0.30,
        "cache_read counted once",
    );
}

#[tokio::test]
async fn test_analyze_directory_keeps_message_with_more_tokens() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Simulate a forked conversation where the same requestId appears in two files
    // but with different output_tokens (one has partial streaming response, other has complete)

    // File 1: Partial streaming response (100 output tokens)
    let partial_message = forked_assistant_msg(ForkedMsgOpts {
        session_id: "session-1",
        msg_id: "msg_test",
        request_id: "req_test_different_tokens",
        uuid: "70cd1811-0ecc-45b0-9f53-75bcb0758246",
        timestamp: "2025-11-08T10:00:00.000Z",
        text: "Partial response",
        counts: TokenCounts::new(50, 100, 1000, 0),
    });

    // File 2: Complete streaming response (500 output tokens)
    let complete_message = forked_assistant_msg(ForkedMsgOpts {
        session_id: "session-2",
        msg_id: "msg_test",
        request_id: "req_test_different_tokens",
        uuid: "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
        timestamp: "2025-11-08T10:00:00.000Z",
        text: "Complete response",
        counts: TokenCounts::new(50, 500, 1000, 0),
    });

    write_two_sessions(
        temp_dir.path(),
        "session-1",
        &partial_message,
        "session-2",
        &complete_message,
    )
    .await;

    let result = analyze_directory(temp_dir.path(), DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

    assert_eq!(result.files_parsed, 2);
    assert_eq!(result.daily_costs.len(), 1);
    let per_model = result.daily_costs[0].per_model.get(ModelType::Sonnet);

    // Should keep the 500-token message, not sum to 600.
    assert_cost_close(per_model.output, 500.0, 15.0, "keeps higher output tokens");
    assert_cost_close(per_model.input, 50.0, 3.0, "input counted once");
}

#[tokio::test]
async fn test_analyze_directory_keeps_oldest_when_tokens_equal() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Simulate the rare case where the same requestId appears with equal tokens
    // but different timestamps (file modification, reprocessing, or clock skew)

    // File 1: Earlier timestamp (10:00)
    let earlier_message = forked_assistant_msg(ForkedMsgOpts {
        session_id: "session-1",
        msg_id: "msg_test",
        request_id: "req_test_timestamp_tiebreak",
        uuid: "11111111-0ecc-45b0-9f53-75bcb0758246",
        timestamp: "2025-11-08T10:00:00.000Z",
        text: "Earlier message",
        counts: TokenCounts::new(100, 500, 2000, 0),
    });

    // File 2: Later timestamp (12:00) with same tokens
    let later_message = forked_assistant_msg(ForkedMsgOpts {
        session_id: "session-2",
        msg_id: "msg_test",
        request_id: "req_test_timestamp_tiebreak",
        uuid: "22222222-0ecc-45b0-9f53-75bcb0758246",
        timestamp: "2025-11-08T12:00:00.000Z",
        text: "Later message",
        counts: TokenCounts::new(100, 500, 2000, 0),
    });

    write_two_sessions(
        temp_dir.path(),
        "session-1",
        &earlier_message,
        "session-2",
        &later_message,
    )
    .await;

    let result = analyze_directory(temp_dir.path(), DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

    assert_eq!(result.files_parsed, 2);
    assert_eq!(result.daily_costs.len(), 1);
    let per_model = result.daily_costs[0].per_model.get(ModelType::Sonnet);

    // With equal tokens, dedup should still keep exactly one (oldest timestamp).
    // We can't directly verify which timestamp was kept, but if dedup didn't
    // run, output tokens would be doubled.
    assert_cost_close(per_model.output, 500.0, 15.0, "dedup on equal tokens");
}

#[tokio::test]
async fn test_analyze_directory_by_session_deduplicates_forked_conversation() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Test that analyze_directory_by_session also handles cross-file deduplication
    // This mirrors the test for analyze_directory but uses session-based aggregation

    let shared_message = forked_assistant_msg(ForkedMsgOpts {
        session_id: "session-original",
        msg_id: "msg_test_session",
        request_id: "req_test_session",
        uuid: "33333333-0ecc-45b0-9f53-75bcb0758246",
        timestamp: "2025-11-08T15:00:00.000Z",
        text: "Test message",
        counts: TokenCounts::new(200, 400, 3000, 1000),
    });

    write_two_sessions(
        temp_dir.path(),
        "session-original",
        &shared_message,
        "session-forked",
        &shared_message,
    )
    .await;

    let result = analyze_directory_by_session(temp_dir.path(), &empty_filter())
        .await
        .unwrap();

    assert_eq!(result.files_parsed, 2);

    // Two sessions exist, but the duplicate message must be counted once globally.
    let total_output_cost: f64 = result
        .session_costs
        .iter()
        .map(|s| s.per_model.get(ModelType::Sonnet).output)
        .sum();
    assert_cost_close(total_output_cost, 400.0, 15.0, "session dedup cross-file");
}
