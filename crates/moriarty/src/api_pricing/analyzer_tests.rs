use super::analyzer::*;
use super::pricing::{ModelType, TokenCosts, TokenCounts};
use super::time_filter::TimeRangeFilter;
use chrono::{Datelike, NaiveDate, TimeZone, Utc};
use std::collections::HashSet;

/// Helper function to create an empty time range filter for tests
fn empty_filter() -> TimeRangeFilter {
    TimeRangeFilter::new(None, None).expect("Empty filter should always be valid")
}

/// Helper function to create a time range filter from date strings
fn date_filter(start: Option<&str>, end: Option<&str>) -> TimeRangeFilter {
    TimeRangeFilter::new(start.map(|s| s.to_string()), end.map(|s| s.to_string())).unwrap()
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

#[test]
fn test_daily_usage_new() {
    let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
    let usage = DailyUsage::new(date);

    assert_eq!(usage.date, date);
    assert_eq!(usage.sonnet_usage.input_tokens, 0);
    assert_eq!(usage.haiku_usage.input_tokens, 0);
    assert_eq!(usage.opus_usage.input_tokens, 0);
    assert_eq!(usage.unknown_usage.input_tokens, 0);
    assert_eq!(usage.lines_changed, 0);
}

#[test]
fn test_daily_usage_add_sonnet() {
    let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
    let mut usage = DailyUsage::new(date);

    let counts = TokenCounts {
        input_tokens: 1000,
        output_tokens: 500,
        cache_write_tokens: 100,
        cache_read_tokens: 50,
    };

    usage.add_usage(ModelType::Sonnet, counts);

    assert_eq!(usage.sonnet_usage.input_tokens, 1000);
    assert_eq!(usage.sonnet_usage.output_tokens, 500);
    assert_eq!(usage.haiku_usage.input_tokens, 0);
    assert_eq!(usage.unknown_usage.input_tokens, 0);
}

#[test]
fn test_daily_usage_add_opus() {
    let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
    let mut usage = DailyUsage::new(date);

    let counts = TokenCounts {
        input_tokens: 1000,
        output_tokens: 500,
        cache_write_tokens: 100,
        cache_read_tokens: 50,
    };

    usage.add_usage(ModelType::Opus, counts);

    assert_eq!(usage.opus_usage.input_tokens, 1000);
    assert_eq!(usage.opus_usage.output_tokens, 500);
    assert_eq!(usage.sonnet_usage.input_tokens, 0);
    assert_eq!(usage.haiku_usage.input_tokens, 0);
}

#[test]
fn test_daily_usage_add_multiple_models() {
    let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
    let mut usage = DailyUsage::new(date);

    usage.add_usage(
        ModelType::Sonnet,
        TokenCounts {
            input_tokens: 1000,
            output_tokens: 500,
            cache_write_tokens: 0,
            cache_read_tokens: 0,
        },
    );

    usage.add_usage(
        ModelType::Haiku,
        TokenCounts {
            input_tokens: 2000,
            output_tokens: 1000,
            cache_write_tokens: 0,
            cache_read_tokens: 0,
        },
    );

    assert_eq!(usage.sonnet_usage.input_tokens, 1000);
    assert_eq!(usage.haiku_usage.input_tokens, 2000);
}

#[test]
fn test_daily_usage_add_accumulates() {
    let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
    let mut usage = DailyUsage::new(date);

    usage.add_usage(
        ModelType::Sonnet,
        TokenCounts {
            input_tokens: 1000,
            output_tokens: 500,
            cache_write_tokens: 100,
            cache_read_tokens: 50,
        },
    );

    usage.add_usage(
        ModelType::Sonnet,
        TokenCounts {
            input_tokens: 500,
            output_tokens: 250,
            cache_write_tokens: 50,
            cache_read_tokens: 25,
        },
    );

    assert_eq!(usage.sonnet_usage.input_tokens, 1500);
    assert_eq!(usage.sonnet_usage.output_tokens, 750);
    assert_eq!(usage.sonnet_usage.cache_write_tokens, 150);
    assert_eq!(usage.sonnet_usage.cache_read_tokens, 75);
}

#[test]
fn test_daily_usage_calculate_costs() {
    let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
    let mut usage = DailyUsage::new(date);

    usage.add_usage(
        ModelType::Sonnet,
        TokenCounts {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            cache_write_tokens: 0,
            cache_read_tokens: 0,
        },
    );

    let costs = usage.calculate_costs();

    assert_eq!(costs.date, date);
    assert_eq!(costs.sonnet_costs.input, 3.0);
    assert_eq!(costs.sonnet_costs.output, 15.0);
    assert_eq!(costs.lines_changed, 0);
}

#[test]
fn test_daily_costs_total() {
    let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
    let costs = DailyCosts {
        date,
        sonnet_costs: TokenCosts {
            input: 1.0,
            output: 2.0,
            cache_write: 0.5,
            cache_read: 0.25,
        },
        haiku_costs: TokenCosts {
            input: 0.5,
            output: 1.0,
            cache_write: 0.25,
            cache_read: 0.1,
        },
        opus_costs: TokenCosts {
            input: 0.0,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
        },
        lines_changed: 0,
    };

    assert!((costs.total() - 5.6).abs() < 1e-10);
}

#[test]
fn test_aggregate_by_date_empty() {
    let usages = Vec::new();
    let mut unknown_models = HashSet::new();
    let mut total_unknown_tokens = TokenCounts::default();

    let result = aggregate_by_date(
        usages,
        Vec::new(),
        &mut unknown_models,
        &mut total_unknown_tokens,
    );

    assert!(result.is_empty());
    assert!(unknown_models.is_empty());
}

#[test]
fn test_aggregate_by_date_single_entry() {
    let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
    let counts = TokenCounts {
        input_tokens: 1000,
        output_tokens: 500,
        cache_write_tokens: 0,
        cache_read_tokens: 0,
    };

    let usages = vec![create_date_based_message(
        date,
        ModelType::Sonnet,
        "claude-sonnet-4".to_string(),
        counts,
    )];
    let mut unknown_models = HashSet::new();
    let mut total_unknown_tokens = TokenCounts::default();

    let result = aggregate_by_date(
        usages,
        Vec::new(),
        &mut unknown_models,
        &mut total_unknown_tokens,
    );

    assert_eq!(result.len(), 1);
    assert!(result.contains_key(&date));
    assert_eq!(result[&date].sonnet_usage.input_tokens, 1000);
}

#[test]
fn test_aggregate_by_date_multiple_dates() {
    let date1 = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
    let date2 = NaiveDate::from_ymd_opt(2025, 10, 24).unwrap();

    let usages = vec![
        create_date_based_message(
            date1,
            ModelType::Sonnet,
            "claude-sonnet-4".to_string(),
            TokenCounts {
                input_tokens: 1000,
                output_tokens: 500,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
            },
        ),
        create_date_based_message(
            date2,
            ModelType::Haiku,
            "claude-haiku-3".to_string(),
            TokenCounts {
                input_tokens: 2000,
                output_tokens: 1000,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
            },
        ),
    ];
    let mut unknown_models = HashSet::new();
    let mut total_unknown_tokens = TokenCounts::default();

    let result = aggregate_by_date(
        usages,
        Vec::new(),
        &mut unknown_models,
        &mut total_unknown_tokens,
    );

    assert_eq!(result.len(), 2);
    assert_eq!(result[&date1].sonnet_usage.input_tokens, 1000);
    assert_eq!(result[&date2].haiku_usage.input_tokens, 2000);
}

#[test]
fn test_aggregate_by_date_same_date_accumulates() {
    let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();

    let usages = vec![
        create_date_based_message(
            date,
            ModelType::Sonnet,
            "claude-sonnet-4".to_string(),
            TokenCounts {
                input_tokens: 1000,
                output_tokens: 500,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
            },
        ),
        create_date_based_message(
            date,
            ModelType::Sonnet,
            "claude-sonnet-4".to_string(),
            TokenCounts {
                input_tokens: 500,
                output_tokens: 250,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
            },
        ),
    ];
    let mut unknown_models = HashSet::new();
    let mut total_unknown_tokens = TokenCounts::default();

    let result = aggregate_by_date(
        usages,
        Vec::new(),
        &mut unknown_models,
        &mut total_unknown_tokens,
    );

    assert_eq!(result.len(), 1);
    assert_eq!(result[&date].sonnet_usage.input_tokens, 1500);
    assert_eq!(result[&date].sonnet_usage.output_tokens, 750);
}

#[test]
fn test_aggregate_by_date_tracks_unknown_models() {
    let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();

    let usages = vec![
        create_date_based_message(
            date,
            ModelType::Unknown,
            "claude-opus-4".to_string(),
            TokenCounts {
                input_tokens: 1000,
                output_tokens: 500,
                cache_write_tokens: 100,
                cache_read_tokens: 50,
            },
        ),
        create_date_based_message(
            date,
            ModelType::Unknown,
            "gpt-4".to_string(),
            TokenCounts {
                input_tokens: 500,
                output_tokens: 250,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
            },
        ),
    ];
    let mut unknown_models = HashSet::new();
    let mut total_unknown_tokens = TokenCounts::default();

    let result = aggregate_by_date(
        usages,
        Vec::new(),
        &mut unknown_models,
        &mut total_unknown_tokens,
    );

    assert_eq!(unknown_models.len(), 2);
    assert!(unknown_models.contains("claude-opus-4"));
    assert!(unknown_models.contains("gpt-4"));
    assert_eq!(total_unknown_tokens.input_tokens, 1500);
    assert_eq!(total_unknown_tokens.output_tokens, 750);
    assert_eq!(total_unknown_tokens.cache_write_tokens, 100);
    assert_eq!(total_unknown_tokens.cache_read_tokens, 50);
    assert_eq!(result[&date].unknown_usage.input_tokens, 1500);
}

#[test]
fn test_aggregate_by_date_sorted_by_date() {
    let date1 = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
    let date2 = NaiveDate::from_ymd_opt(2025, 10, 21).unwrap();
    let date3 = NaiveDate::from_ymd_opt(2025, 10, 25).unwrap();

    let usages = vec![
        create_date_based_message(
            date1,
            ModelType::Sonnet,
            "claude-sonnet-4".to_string(),
            TokenCounts::default(),
        ),
        create_date_based_message(
            date2,
            ModelType::Sonnet,
            "claude-sonnet-4".to_string(),
            TokenCounts::default(),
        ),
        create_date_based_message(
            date3,
            ModelType::Sonnet,
            "claude-sonnet-4".to_string(),
            TokenCounts::default(),
        ),
    ];
    let mut unknown_models = HashSet::new();
    let mut total_unknown_tokens = TokenCounts::default();

    let result = aggregate_by_date(
        usages,
        Vec::new(),
        &mut unknown_models,
        &mut total_unknown_tokens,
    );
    let dates: Vec<_> = result.keys().collect();

    assert_eq!(dates, vec![&date2, &date1, &date3]);
}

#[tokio::test]
async fn test_find_jsonl_files_empty_directory() {
    let temp_dir = tempfile::tempdir().unwrap();
    let result = find_jsonl_files(temp_dir.path()).await.unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn test_find_jsonl_files_no_jsonl_files() {
    let temp_dir = tempfile::tempdir().unwrap();
    tokio::fs::write(temp_dir.path().join("test.txt"), "content")
        .await
        .unwrap();
    tokio::fs::write(temp_dir.path().join("test.json"), "{}")
        .await
        .unwrap();

    let result = find_jsonl_files(temp_dir.path()).await.unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn test_find_jsonl_files_single_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    tokio::fs::write(temp_dir.path().join("test.jsonl"), "")
        .await
        .unwrap();

    let result = find_jsonl_files(temp_dir.path()).await.unwrap();
    assert_eq!(result.len(), 1);
    assert!(result[0].ends_with("test.jsonl"));
}

#[tokio::test]
async fn test_find_jsonl_files_recursive() {
    let temp_dir = tempfile::tempdir().unwrap();
    let subdir = temp_dir.path().join("subdir");
    tokio::fs::create_dir(&subdir).await.unwrap();

    tokio::fs::write(temp_dir.path().join("root.jsonl"), "")
        .await
        .unwrap();
    tokio::fs::write(subdir.join("nested.jsonl"), "")
        .await
        .unwrap();

    let result = find_jsonl_files(temp_dir.path()).await.unwrap();
    assert_eq!(result.len(), 2);
}

#[tokio::test]
async fn test_find_jsonl_files_deep_nesting() {
    let temp_dir = tempfile::tempdir().unwrap();
    let deep_path = temp_dir.path().join("a").join("b").join("c");
    tokio::fs::create_dir_all(&deep_path).await.unwrap();

    tokio::fs::write(deep_path.join("deep.jsonl"), "")
        .await
        .unwrap();

    let result = find_jsonl_files(temp_dir.path()).await.unwrap();
    assert_eq!(result.len(), 1);
    assert!(result[0].ends_with("deep.jsonl"));
}

#[tokio::test]
async fn test_parse_log_file_empty_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("empty.jsonl");
    tokio::fs::write(&file_path, "").await.unwrap();

    let result = parse_log_file(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn test_parse_log_file_extracts_usage_correctly() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4-20250514","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":100,"cache_read_input_tokens":50,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    let result = parse_log_file(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

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
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-haiku-3","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":1000,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-24T12:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    let result = parse_log_file(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

    assert_eq!(result.len(), 2);

    assert_eq!(result[0].model_type, ModelType::Sonnet);
    assert_eq!(result[0].token_counts.input_tokens, 1000);

    assert_eq!(result[1].model_type, ModelType::Haiku);
    assert_eq!(result[1].token_counts.input_tokens, 2000);
}

#[tokio::test]
async fn test_parse_log_file_ignores_non_assistant_messages() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    let log_content = r#"{"type":"user","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"role":"user","content":"test"},"isMeta":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T11:00:00Z","toolUseResult":null,"thinkingMetadata":null,"isVisibleInTranscriptOnly":null,"isCompactSummary":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    let result = parse_log_file(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

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
    let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();

    let usages = vec![create_date_based_message(
        date,
        ModelType::Sonnet,
        "claude-sonnet-4".to_string(),
        TokenCounts::default(),
    )];

    let lines_changed = vec![(date, 100), (date, 50)];

    let mut unknown_models = HashSet::new();
    let mut total_unknown_tokens = TokenCounts::default();

    let result = aggregate_by_date(
        usages,
        lines_changed,
        &mut unknown_models,
        &mut total_unknown_tokens,
    );

    assert_eq!(result[&date].lines_changed, 150);
}

#[test]
fn test_aggregate_by_date_lines_changed_different_dates() {
    let date1 = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
    let date2 = NaiveDate::from_ymd_opt(2025, 10, 24).unwrap();

    let lines_changed = vec![(date1, 100), (date2, 200), (date1, 50)];

    let mut unknown_models = HashSet::new();
    let mut total_unknown_tokens = TokenCounts::default();

    let result = aggregate_by_date(
        Vec::new(),
        lines_changed,
        &mut unknown_models,
        &mut total_unknown_tokens,
    );

    assert_eq!(result[&date1].lines_changed, 150);
    assert_eq!(result[&date2].lines_changed, 200);
}

#[tokio::test]
async fn test_parse_lines_changed_with_edit_tool() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":[{"type":"tool_use","id":"tool_1","name":"Edit","input":{"file_path":"/test.rs","old_string":"line1\nline2","new_string":"line1\nmodified\nline3"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    let result = parse_lines_changed(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, NaiveDate::from_ymd_opt(2025, 10, 23).unwrap());
    assert_eq!(result[0].1, 3);
}

#[tokio::test]
async fn test_parse_lines_changed_with_write_tool() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":[{"type":"tool_use","id":"tool_1","name":"Write","input":{"file_path":"/test.rs","content":"line1\nline2\nline3\nline4"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    let result = parse_lines_changed(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, NaiveDate::from_ymd_opt(2025, 10, 23).unwrap());
    assert_eq!(result[0].1, 4);
}

#[tokio::test]
async fn test_parse_lines_changed_with_notebook_edit_tool() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":[{"type":"tool_use","id":"tool_1","name":"NotebookEdit","input":{"notebook_path":"/test.ipynb","new_source":"print('hello')\nprint('world')"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    let result = parse_lines_changed(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, NaiveDate::from_ymd_opt(2025, 10, 23).unwrap());
    assert_eq!(result[0].1, 2);
}

#[tokio::test]
async fn test_parse_lines_changed_multiple_tools_same_message() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":[{"type":"tool_use","id":"tool_1","name":"Edit","input":{"file_path":"/test.rs","old_string":"old","new_string":"new"}},{"type":"tool_use","id":"tool_2","name":"Write","input":{"file_path":"/test2.rs","content":"line1\nline2"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    let result = parse_lines_changed(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

    assert_eq!(result.len(), 2);
    assert_eq!(result[0].0, NaiveDate::from_ymd_opt(2025, 10, 23).unwrap());
    assert_eq!(result[1].0, NaiveDate::from_ymd_opt(2025, 10, 23).unwrap());
}

#[tokio::test]
async fn test_parse_lines_changed_empty_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("empty.jsonl");
    tokio::fs::write(&file_path, "").await.unwrap();

    let result = parse_lines_changed(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn test_parse_lines_changed_no_tool_uses() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"just text, no tools","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    let result = parse_lines_changed(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn test_parse_lines_changed_ignores_non_modifying_tools() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":[{"type":"tool_use","id":"tool_1","name":"Read","input":{"file_path":"/test.rs"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    let result = parse_lines_changed(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn test_parse_log_file_filters_synthetic_model() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"<synthetic>","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2000,"cache_creation_input_tokens":100,"cache_read_input_tokens":50,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":1000,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    let result = parse_log_file(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].model_string, "claude-sonnet-4");
    assert_eq!(result[0].token_counts.input_tokens, 2000);
}

#[tokio::test]
async fn test_parse_log_file_filters_synthetic_model_case_insensitive() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"<SYNTHETIC>","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"<Synthetic>","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1500,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":750,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    let result = parse_log_file(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

    assert!(result.is_empty());
}

#[tokio::test]
async fn test_parse_lines_changed_filters_synthetic_model() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"<synthetic>","container":null,"content":[{"type":"tool_use","id":"tool_1","name":"Edit","input":{"file_path":"/test.rs","old_string":"old","new_string":"new\ncode"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":[{"type":"tool_use","id":"tool_2","name":"Edit","input":{"file_path":"/test2.rs","old_string":"a","new_string":"b\nc"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":1000,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    let result = parse_lines_changed(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, NaiveDate::from_ymd_opt(2025, 10, 23).unwrap());
    assert_eq!(result[0].1, 3);
}

#[tokio::test]
async fn test_parse_lines_changed_filters_synthetic_model_case_insensitive() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"<SYNTHETIC>","container":null,"content":[{"type":"tool_use","id":"tool_1","name":"Edit","input":{"file_path":"/test.rs","old_string":"old","new_string":"new"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"<Synthetic>","container":null,"content":[{"type":"tool_use","id":"tool_2","name":"Write","input":{"file_path":"/test2.rs","content":"line1\nline2"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":1000,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    let result = parse_lines_changed(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

    assert!(result.is_empty());
}

#[tokio::test]
async fn test_parse_log_file_all_synthetic_entries() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"<synthetic>","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"<synthetic>","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":1000,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    let result = parse_log_file(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

    assert!(result.is_empty());
}

#[tokio::test]
async fn test_parse_lines_changed_all_synthetic_entries() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"<synthetic>","container":null,"content":[{"type":"tool_use","id":"tool_1","name":"Edit","input":{"file_path":"/test.rs","old_string":"old","new_string":"new"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"<synthetic>","container":null,"content":[{"type":"tool_use","id":"tool_2","name":"Write","input":{"file_path":"/test2.rs","content":"test"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":1000,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    let result = parse_lines_changed(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

    assert!(result.is_empty());
}

// ============================================================================
// Tests for streaming message deduplication
// ============================================================================
// Streaming API responses send multiple partial messages with the same requestId.
// These tests verify that we correctly deduplicate by keeping only the message
// with the highest output_tokens (the final, complete message).

#[tokio::test]
async fn test_parse_log_file_deduplicates_streaming_messages_by_request_id() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"partial","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":8,"cache_creation_input_tokens":17932,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":2,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"partial","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":8,"cache_creation_input_tokens":17932,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":2,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:01Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_3","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"partial","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":8,"cache_creation_input_tokens":17932,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":2,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000003","timestamp":"2025-10-23T12:00:02Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_4","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"final complete response","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":8,"cache_creation_input_tokens":17932,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":835,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000004","timestamp":"2025-10-23T12:00:03Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    let result = parse_log_file(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

    assert_eq!(
        result.len(),
        1,
        "Should only count the final message in streaming group, but got {} messages",
        result.len()
    );

    let msg = &result[0];
    assert_eq!(msg.model_type, ModelType::Sonnet);
    assert_eq!(msg.token_counts.input_tokens, 8);
    assert_eq!(
        msg.token_counts.output_tokens, 835,
        "Should have final output count"
    );
    assert_eq!(
        msg.token_counts.cache_write_tokens, 17932,
        "Should count cache_write only once"
    );
}

#[tokio::test]
async fn test_parse_log_file_by_session_deduplicates_streaming_messages() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    // Same test data as above
    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"partial","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":8,"cache_creation_input_tokens":17932,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":2,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"partial","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":8,"cache_creation_input_tokens":17932,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":2,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:01Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_3","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"partial","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":8,"cache_creation_input_tokens":17932,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":2,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000003","timestamp":"2025-10-23T12:00:02Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_4","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"final","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":8,"cache_creation_input_tokens":17932,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":835,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000004","timestamp":"2025-10-23T12:00:03Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    let result = parse_log_file_by_session(&file_path, &empty_filter())
        .await
        .unwrap();

    assert_eq!(
        result.len(),
        1,
        "Should only count the final message, but got {} messages",
        result.len()
    );

    let msg = &result[0];
    assert_eq!(msg.session_id, "session-1");
    assert_eq!(msg.token_counts.output_tokens, 835);
    assert_eq!(msg.token_counts.cache_write_tokens, 17932);
}

#[tokio::test]
async fn test_parse_log_file_handles_multiple_streaming_groups() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":8,"cache_creation_input_tokens":17932,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":2,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":8,"cache_creation_input_tokens":17932,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":2,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:01Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_3","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":8,"cache_creation_input_tokens":17932,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":835,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000003","timestamp":"2025-10-23T12:00:02Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_4","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":12,"cache_creation_input_tokens":5361,"cache_read_input_tokens":17932,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":2,"service_tier":null,"server_tool_use":null}},"requestId":"req-456","uuid":"00000000-0000-0000-0000-000000000004","timestamp":"2025-10-23T12:01:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_5","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":12,"cache_creation_input_tokens":5361,"cache_read_input_tokens":17932,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":867,"service_tier":null,"server_tool_use":null}},"requestId":"req-456","uuid":"00000000-0000-0000-0000-000000000005","timestamp":"2025-10-23T12:01:01Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    let result = parse_log_file(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

    assert_eq!(
        result.len(),
        2,
        "Should have 2 entries (one per request), but got {}",
        result.len()
    );

    // First request: final message has output_tokens=835
    assert_eq!(result[0].token_counts.output_tokens, 835);
    assert_eq!(result[0].token_counts.cache_write_tokens, 17932);

    // Second request: final message has output_tokens=867
    assert_eq!(result[1].token_counts.output_tokens, 867);
    assert_eq!(result[1].token_counts.cache_write_tokens, 5361);
    assert_eq!(result[1].token_counts.cache_read_tokens, 17932);
}

#[tokio::test]
async fn test_parse_log_file_preserves_non_streaming_messages() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    // Message without requestId should be preserved
    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    let result = parse_log_file(&file_path, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

    // Messages without requestId should be kept as-is
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].token_counts.input_tokens, 1000);
    assert_eq!(result[0].token_counts.output_tokens, 500);
}

#[test]
fn test_aggregate_by_date_with_deduplicated_streaming_data() {
    // This test shows the CORRECT cost when using deduplicated data
    let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();

    // Only the final message from a 4-message streaming group
    let usages = vec![create_date_based_message(
        date,
        ModelType::Sonnet,
        "claude-sonnet-4".to_string(),
        TokenCounts {
            input_tokens: 8,
            output_tokens: 835,
            cache_write_tokens: 17932,
            cache_read_tokens: 0,
        },
    )];

    let mut unknown_models = HashSet::new();
    let mut total_unknown_tokens = TokenCounts::default();

    let result = aggregate_by_date(
        usages,
        Vec::new(),
        &mut unknown_models,
        &mut total_unknown_tokens,
    );

    assert_eq!(result.len(), 1);
    let daily_usage = &result[&date];
    assert_eq!(daily_usage.sonnet_usage.cache_write_tokens, 17932);

    // Calculate cost (Sonnet pricing: cache_write = $3.75 per million)
    // 17932 tokens * $3.75 / 1M = $0.06725
    let daily_costs = daily_usage.calculate_costs();
    // This should be around $0.12 (input + output + cache_write)
    assert!(
        daily_costs.sonnet_costs.cache_write > 0.06 && daily_costs.sonnet_costs.cache_write < 0.07,
        "Cache write cost should be ~$0.067, got ${}",
        daily_costs.sonnet_costs.cache_write
    );
}

#[test]
fn test_aggregate_by_date_with_buggy_non_deduplicated_data() {
    // This test shows the INCORRECT cost when NOT deduplicating (current bug)
    let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();

    // All 4 messages from streaming group (buggy behavior)
    let usages = vec![
        create_date_based_message(
            date,
            ModelType::Sonnet,
            "claude-sonnet-4".to_string(),
            TokenCounts {
                input_tokens: 8,
                output_tokens: 2,
                cache_write_tokens: 17932,
                cache_read_tokens: 0,
            },
        ),
        create_date_based_message(
            date,
            ModelType::Sonnet,
            "claude-sonnet-4".to_string(),
            TokenCounts {
                input_tokens: 8,
                output_tokens: 2,
                cache_write_tokens: 17932,
                cache_read_tokens: 0,
            },
        ),
        create_date_based_message(
            date,
            ModelType::Sonnet,
            "claude-sonnet-4".to_string(),
            TokenCounts {
                input_tokens: 8,
                output_tokens: 2,
                cache_write_tokens: 17932,
                cache_read_tokens: 0,
            },
        ),
        create_date_based_message(
            date,
            ModelType::Sonnet,
            "claude-sonnet-4".to_string(),
            TokenCounts {
                input_tokens: 8,
                output_tokens: 835,
                cache_write_tokens: 17932,
                cache_read_tokens: 0,
            },
        ),
    ];

    let mut unknown_models = HashSet::new();
    let mut total_unknown_tokens = TokenCounts::default();

    let result = aggregate_by_date(
        usages,
        Vec::new(),
        &mut unknown_models,
        &mut total_unknown_tokens,
    );

    assert_eq!(result.len(), 1);
    let daily_usage = &result[&date];

    // BUG: cache_write counted 4 times!
    assert_eq!(
        daily_usage.sonnet_usage.cache_write_tokens,
        17932 * 4,
        "Bug: cache_write counted 4 times instead of once"
    );

    let daily_costs = daily_usage.calculate_costs();
    // With 4x counting: 71728 * $3.75 / 1M = $0.269
    assert!(
        daily_costs.sonnet_costs.cache_write > 0.26,
        "Buggy behavior: cache_write should be inflated to ~$0.27, got ${}",
        daily_costs.sonnet_costs.cache_write
    );
}

#[tokio::test]
async fn test_parse_log_file_handles_zero_output_tokens() {
    // Edge case: all messages in streaming group have zero output_tokens
    // Should keep the first message encountered
    let temp_dir = tempfile::tempdir().unwrap();
    let log_file = temp_dir.path().join("test.jsonl");

    // Create a log with streaming messages all having zero output_tokens
    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"response 1","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":10,"cache_creation_input_tokens":100,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":0,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"response 2","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":10,"cache_creation_input_tokens":100,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":0,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:01Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_3","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"response 3","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":10,"cache_creation_input_tokens":100,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":0,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000003","timestamp":"2025-10-23T12:00:02Z","isApiErrorMessage":null}"#;

    tokio::fs::write(&log_file, log_content).await.unwrap();

    let result = parse_log_file(&log_file, DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

    // Should have exactly 1 message (deduplicated from 3)
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

#[tokio::test]
async fn test_parse_log_file_filters_by_start_time() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    // Create log with messages at different times
    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":100,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-01-01T10:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":200,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-01-01T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_3","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":3000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":300,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000003","timestamp":"2025-01-01T14:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    // Filter: only messages after 11:00
    let filter = date_filter(Some("2025-01-01T11:00:00Z"), None);
    let result = parse_log_file(&file_path, DateTimezone::Utc, &filter)
        .await
        .unwrap();

    // Should only include messages at 12:00 and 14:00
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].token_counts.input_tokens, 2000);
    assert_eq!(result[1].token_counts.input_tokens, 3000);
}

#[tokio::test]
async fn test_parse_log_file_filters_by_end_time() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":100,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-01-01T10:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":200,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-01-01T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_3","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":3000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":300,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000003","timestamp":"2025-01-01T14:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    // Filter: only messages before 12:00 (exclusive end)
    let filter = date_filter(None, Some("2025-01-01T12:00:00Z"));
    let result = parse_log_file(&file_path, DateTimezone::Utc, &filter)
        .await
        .unwrap();

    // Should only include message at 10:00 (12:00 is excluded)
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].token_counts.input_tokens, 1000);
}

#[tokio::test]
async fn test_parse_log_file_filters_by_time_range() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":100,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-01-01T10:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":200,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-01-01T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_3","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":3000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":300,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000003","timestamp":"2025-01-01T14:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    // Filter: messages between 11:00 and 13:00
    let filter = date_filter(Some("2025-01-01T11:00:00Z"), Some("2025-01-01T13:00:00Z"));
    let result = parse_log_file(&file_path, DateTimezone::Utc, &filter)
        .await
        .unwrap();

    // Should only include message at 12:00
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].token_counts.input_tokens, 2000);
}

#[tokio::test]
async fn test_parse_log_file_filter_excludes_all_messages() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":100,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-01-01T12:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    // Filter that excludes all messages (time range in 2024)
    let filter = date_filter(Some("2024-01-01"), Some("2024-12-31"));
    let result = parse_log_file(&file_path, DateTimezone::Utc, &filter)
        .await
        .unwrap();

    // Should return empty vec
    assert!(result.is_empty());
}

#[tokio::test]
async fn test_parse_log_file_filter_with_date_only() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    // Messages throughout Jan 31
    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":100,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-01-31T00:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":200,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-01-31T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_3","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":3000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":300,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000003","timestamp":"2025-01-31T23:59:59Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_4","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":4000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":400,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000004","timestamp":"2025-02-01T00:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    // Filter: --end-time "2025-01-31" should include all of Jan 31 but not Feb 1
    let filter = date_filter(Some("2025-01-31"), Some("2025-01-31"));
    let result = parse_log_file(&file_path, DateTimezone::Utc, &filter)
        .await
        .unwrap();

    // Should include all 3 messages from Jan 31, but not Feb 1
    assert_eq!(result.len(), 3);
}

#[tokio::test]
async fn test_parse_log_file_includes_message_at_start_boundary() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    // Messages at 11:00, 12:00, and 13:00
    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":100,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-01-01T11:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":200,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-01-01T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_3","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":3000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":300,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000003","timestamp":"2025-01-01T13:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    // Filter: start at exactly 12:00 (should be inclusive)
    let filter = date_filter(Some("2025-01-01T12:00:00Z"), None);
    let result = parse_log_file(&file_path, DateTimezone::Utc, &filter)
        .await
        .unwrap();

    // Should include messages at 12:00 and 13:00 (start is inclusive)
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].token_counts.input_tokens, 2000);
    assert_eq!(result[1].token_counts.input_tokens, 3000);
}

#[tokio::test]
async fn test_parse_log_file_excludes_message_at_end_boundary() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    // Messages at 11:00, 12:00, and 13:00
    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":100,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-01-01T11:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":200,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-01-01T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_3","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":3000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":300,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000003","timestamp":"2025-01-01T13:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    // Filter: end at exactly 12:00 (should be exclusive)
    let filter = date_filter(None, Some("2025-01-01T12:00:00Z"));
    let result = parse_log_file(&file_path, DateTimezone::Utc, &filter)
        .await
        .unwrap();

    // Should include only message at 11:00 (end is exclusive)
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].token_counts.input_tokens, 1000);
}

#[tokio::test]
async fn test_parse_log_file_by_session_respects_filter() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":100,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-01-01T10:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":200,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-01-01T12:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    // Filter: only messages after 11:00
    let filter = date_filter(Some("2025-01-01T11:00:00Z"), None);
    let result = parse_log_file_by_session(&file_path, &filter)
        .await
        .unwrap();

    // Should only include message at 12:00
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].token_counts.input_tokens, 2000);
}

#[tokio::test]
async fn test_parse_lines_changed_respects_time_filter() {
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.jsonl");

    // Messages with tool uses at different times
    let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":[{"type":"tool_use","id":"tool_1","name":"Write","input":{"file_path":"/test/file1.txt","content":"line1\nline2\n"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":100,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":10,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-01-01T10:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":[{"type":"tool_use","id":"tool_2","name":"Write","input":{"file_path":"/test/file2.txt","content":"line1\nline2\nline3\n"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":100,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":10,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-01-01T12:00:00Z","isApiErrorMessage":null}"#;
    tokio::fs::write(&file_path, log_content).await.unwrap();

    // Filter: only messages after 11:00
    let filter = date_filter(Some("2025-01-01T11:00:00Z"), None);
    let result = parse_lines_changed(&file_path, DateTimezone::Utc, &filter)
        .await
        .unwrap();

    // Should only include lines from 12:00 message (3 lines)
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].1, 3); // 3 lines changed
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
    let shared_message = r#"{"type":"assistant","parentUuid":"bb0252ce-8926-4f5c-b616-fa5743f365de","isSidechain":false,"userType":"external","cwd":"/test","sessionId":"session-original","version":"2.0.32","gitBranch":"main","message":{"model":"claude-sonnet-4","id":"msg_01GRWfK8FJtwwQVzS6LypbBp","type":"message","role":"assistant","content":[{"type":"text","text":"I'll list the directories"}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":100,"cache_creation_input_tokens":5000,"cache_read_input_tokens":2000,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":"standard"}},"requestId":"req_011CUwGzDJueepzBaSgFrpQA","uuid":"70cd1811-0ecc-45b0-9f53-75bcb0758246","timestamp":"2025-11-08T23:18:07.276Z","isApiErrorMessage":null}"#;

    // Original session file
    let file1_path = temp_dir.path().join("session-original.jsonl");
    tokio::fs::write(&file1_path, shared_message).await.unwrap();

    // Forked session file - contains the exact same message
    // (in reality, sessionId might differ but all other fields including requestId are the same)
    let file2_path = temp_dir.path().join("session-forked.jsonl");
    tokio::fs::write(&file2_path, shared_message).await.unwrap();

    // Analyze the directory containing both files
    let result = analyze_directory(temp_dir.path(), DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

    assert_eq!(result.files_parsed, 2, "Should have parsed both files");

    // The critical assertion: tokens should be counted ONCE, not twice
    // If cross-file deduplication works correctly, we should see:
    //   input_tokens: 100 (not 200)
    //   output_tokens: 500 (not 1000)
    //   cache_write_tokens: 5000 (not 10000)
    //   cache_read_tokens: 2000 (not 4000)

    assert_eq!(result.daily_costs.len(), 1, "Should have one day of usage");
    let daily = &result.daily_costs[0];

    // If this test FAILS (values are doubled), there is NO cross-file deduplication
    // If this test PASSES (values are single), there IS cross-file deduplication
    // Use approximate equality for floating point comparisons
    let expected_input = 100.0 * 3.0 / 1_000_000.0; // $3.00 per MTok
    assert!(
        (daily.sonnet_costs.input - expected_input).abs() < 1e-10,
        "Input tokens should be counted once, not doubled. Expected ~{}, got {}",
        expected_input,
        daily.sonnet_costs.input
    );
    let expected_output = 500.0 * 15.0 / 1_000_000.0; // $15.00 per MTok
    assert!(
        (daily.sonnet_costs.output - expected_output).abs() < 1e-10,
        "Output tokens should be counted once, not doubled. Expected ~{}, got {}",
        expected_output,
        daily.sonnet_costs.output
    );
    let expected_cache_write = 5000.0 * 3.75 / 1_000_000.0; // $3.75 per MTok
    assert!(
        (daily.sonnet_costs.cache_write - expected_cache_write).abs() < 1e-10,
        "Cache write tokens should be counted once, not doubled. Expected ~{}, got {}",
        expected_cache_write,
        daily.sonnet_costs.cache_write
    );
    let expected_cache_read = 2000.0 * 0.30 / 1_000_000.0; // $0.30 per MTok
    assert!(
        (daily.sonnet_costs.cache_read - expected_cache_read).abs() < 1e-10,
        "Cache read tokens should be counted once, not doubled. Expected ~{}, got {}",
        expected_cache_read,
        daily.sonnet_costs.cache_read
    );
}

#[tokio::test]
async fn test_analyze_directory_keeps_message_with_more_tokens() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Simulate a forked conversation where the same requestId appears in two files
    // but with different output_tokens (one has partial streaming response, other has complete)

    // File 1: Partial streaming response (100 output tokens)
    let partial_message = r#"{"type":"assistant","parentUuid":"bb0252ce-8926-4f5c-b616-fa5743f365de","isSidechain":false,"userType":"external","cwd":"/test","sessionId":"session-1","version":"2.0.32","gitBranch":"main","message":{"model":"claude-sonnet-4","id":"msg_test","type":"message","role":"assistant","content":[{"type":"text","text":"Partial response"}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":50,"cache_creation_input_tokens":1000,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":100,"service_tier":"standard"}},"requestId":"req_test_different_tokens","uuid":"70cd1811-0ecc-45b0-9f53-75bcb0758246","timestamp":"2025-11-08T10:00:00.000Z","isApiErrorMessage":null}"#;

    // File 2: Complete streaming response (500 output tokens)
    let complete_message = r#"{"type":"assistant","parentUuid":"bb0252ce-8926-4f5c-b616-fa5743f365de","isSidechain":false,"userType":"external","cwd":"/test","sessionId":"session-2","version":"2.0.32","gitBranch":"main","message":{"model":"claude-sonnet-4","id":"msg_test","type":"message","role":"assistant","content":[{"type":"text","text":"Complete response"}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":50,"cache_creation_input_tokens":1000,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":"standard"}},"requestId":"req_test_different_tokens","uuid":"a1b2c3d4-e5f6-7890-abcd-ef1234567890","timestamp":"2025-11-08T10:00:00.000Z","isApiErrorMessage":null}"#;

    let file1_path = temp_dir.path().join("session-1.jsonl");
    tokio::fs::write(&file1_path, partial_message)
        .await
        .unwrap();

    let file2_path = temp_dir.path().join("session-2.jsonl");
    tokio::fs::write(&file2_path, complete_message)
        .await
        .unwrap();

    // Analyze the directory
    let result = analyze_directory(temp_dir.path(), DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

    assert_eq!(result.files_parsed, 2);
    assert_eq!(result.daily_costs.len(), 1);
    let daily = &result.daily_costs[0];

    // The critical assertion: Should keep the message with 500 tokens, not sum to 600
    // Expected cost based on 500 output tokens, not 600
    let expected_output = 500.0 * 15.0 / 1_000_000.0; // $15.00 per MTok
    assert!(
        (daily.sonnet_costs.output - expected_output).abs() < 1e-10,
        "Should keep message with MORE tokens (500), not sum both (600). Expected ~{}, got {}",
        expected_output,
        daily.sonnet_costs.output
    );

    // Input tokens should also be counted once (50 not 100)
    let expected_input = 50.0 * 3.0 / 1_000_000.0; // $3.00 per MTok
    assert!(
        (daily.sonnet_costs.input - expected_input).abs() < 1e-10,
        "Input tokens should be counted once. Expected ~{}, got {}",
        expected_input,
        daily.sonnet_costs.input
    );
}

#[tokio::test]
async fn test_analyze_directory_keeps_oldest_when_tokens_equal() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Simulate the rare case where the same requestId appears with equal tokens
    // but different timestamps (file modification, reprocessing, or clock skew)

    // File 1: Earlier timestamp (10:00)
    let earlier_message = r#"{"type":"assistant","parentUuid":"bb0252ce-8926-4f5c-b616-fa5743f365de","isSidechain":false,"userType":"external","cwd":"/test","sessionId":"session-1","version":"2.0.32","gitBranch":"main","message":{"model":"claude-sonnet-4","id":"msg_test","type":"message","role":"assistant","content":[{"type":"text","text":"Earlier message"}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":100,"cache_creation_input_tokens":2000,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":"standard"}},"requestId":"req_test_timestamp_tiebreak","uuid":"11111111-0ecc-45b0-9f53-75bcb0758246","timestamp":"2025-11-08T10:00:00.000Z","isApiErrorMessage":null}"#;

    // File 2: Later timestamp (12:00) with same tokens
    let later_message = r#"{"type":"assistant","parentUuid":"bb0252ce-8926-4f5c-b616-fa5743f365de","isSidechain":false,"userType":"external","cwd":"/test","sessionId":"session-2","version":"2.0.32","gitBranch":"main","message":{"model":"claude-sonnet-4","id":"msg_test","type":"message","role":"assistant","content":[{"type":"text","text":"Later message"}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":100,"cache_creation_input_tokens":2000,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":"standard"}},"requestId":"req_test_timestamp_tiebreak","uuid":"22222222-0ecc-45b0-9f53-75bcb0758246","timestamp":"2025-11-08T12:00:00.000Z","isApiErrorMessage":null}"#;

    let file1_path = temp_dir.path().join("session-1.jsonl");
    tokio::fs::write(&file1_path, earlier_message)
        .await
        .unwrap();

    let file2_path = temp_dir.path().join("session-2.jsonl");
    tokio::fs::write(&file2_path, later_message).await.unwrap();

    // Analyze the directory
    let result = analyze_directory(temp_dir.path(), DateTimezone::Utc, &empty_filter())
        .await
        .unwrap();

    assert_eq!(result.files_parsed, 2);
    assert_eq!(result.daily_costs.len(), 1);
    let daily = &result.daily_costs[0];

    // Verify tokens are counted once (not doubled)
    let expected_output = 500.0 * 15.0 / 1_000_000.0;
    assert!(
        (daily.sonnet_costs.output - expected_output).abs() < 1e-10,
        "With equal tokens, should keep OLDEST timestamp. Expected ~{}, got {}",
        expected_output,
        daily.sonnet_costs.output
    );

    // This test verifies the tie-breaking logic works correctly
    // We can't directly verify which timestamp was kept, but we verify
    // that deduplication occurred (tokens counted once, not twice)
}

#[tokio::test]
async fn test_analyze_directory_by_session_deduplicates_forked_conversation() {
    let temp_dir = tempfile::tempdir().unwrap();

    // Test that analyze_directory_by_session also handles cross-file deduplication
    // This mirrors the test for analyze_directory but uses session-based aggregation

    let shared_message = r#"{"type":"assistant","parentUuid":"bb0252ce-8926-4f5c-b616-fa5743f365de","isSidechain":false,"userType":"external","cwd":"/test","sessionId":"session-original","version":"2.0.32","gitBranch":"main","message":{"model":"claude-sonnet-4","id":"msg_test_session","type":"message","role":"assistant","content":[{"type":"text","text":"Test message"}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":200,"cache_creation_input_tokens":3000,"cache_read_input_tokens":1000,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":400,"service_tier":"standard"}},"requestId":"req_test_session","uuid":"33333333-0ecc-45b0-9f53-75bcb0758246","timestamp":"2025-11-08T15:00:00.000Z","isApiErrorMessage":null}"#;

    let file1_path = temp_dir.path().join("session-original.jsonl");
    tokio::fs::write(&file1_path, shared_message).await.unwrap();

    let file2_path = temp_dir.path().join("session-forked.jsonl");
    tokio::fs::write(&file2_path, shared_message).await.unwrap();

    // Analyze by session
    let result = analyze_directory_by_session(temp_dir.path(), &empty_filter())
        .await
        .unwrap();

    assert_eq!(result.files_parsed, 2);

    // Should have TWO sessions (session-original and session-forked)
    // But the duplicate message should be deduplicated across both files
    // Each session will show the message once, but globally it's counted once

    // Total tokens across all sessions should reflect single counting
    // We expect 2 sessions, each might have the message, but deduplication
    // should have occurred before session aggregation

    // The total cost should reflect the tokens being counted once globally
    let total_output_cost: f64 = result
        .session_costs
        .iter()
        .map(|s| s.sonnet_costs.output)
        .sum();

    let expected_output = 400.0 * 15.0 / 1_000_000.0; // $15.00 per MTok
    assert!(
        (total_output_cost - expected_output).abs() < 1e-10,
        "Session-based analysis should also deduplicate cross-file. Expected ~{}, got {}",
        expected_output,
        total_output_cost
    );
}
