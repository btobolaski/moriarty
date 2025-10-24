use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
};

use chrono::NaiveDate;
use miette::IntoDiagnostic;
use tokio::fs;

use crate::logs::parser::{self, LogLine};

use super::pricing::{ModelType, TokenCosts, TokenCounts};

#[derive(Debug, Default)]
pub struct AnalysisResult {
    pub daily_costs: Vec<DailyCosts>,
    pub unknown_models: HashSet<String>,
    pub total_unknown_tokens: TokenCounts,
    pub files_parsed: usize,
    pub files_failed: usize,
}

#[derive(Debug)]
pub struct DailyUsage {
    pub date: NaiveDate,
    pub sonnet_usage: TokenCounts,
    pub haiku_usage: TokenCounts,
    pub opus_usage: TokenCounts,
    pub unknown_usage: TokenCounts,
}

impl DailyUsage {
    pub fn new(date: NaiveDate) -> Self {
        Self {
            date,
            sonnet_usage: TokenCounts::default(),
            haiku_usage: TokenCounts::default(),
            opus_usage: TokenCounts::default(),
            unknown_usage: TokenCounts::default(),
        }
    }

    pub fn add_usage(&mut self, model_type: ModelType, counts: TokenCounts) {
        match model_type {
            ModelType::Sonnet => self.sonnet_usage.add(&counts),
            ModelType::Haiku => self.haiku_usage.add(&counts),
            ModelType::Opus => self.opus_usage.add(&counts),
            ModelType::Unknown => self.unknown_usage.add(&counts),
        }
    }

    pub fn calculate_costs(&self) -> DailyCosts {
        let sonnet_costs = ModelType::Sonnet
            .pricing()
            .map(|p| p.calculate_cost(&self.sonnet_usage))
            .unwrap_or_default();

        let haiku_costs = ModelType::Haiku
            .pricing()
            .map(|p| p.calculate_cost(&self.haiku_usage))
            .unwrap_or_default();

        let opus_costs = ModelType::Opus
            .pricing()
            .map(|p| p.calculate_cost(&self.opus_usage))
            .unwrap_or_default();

        DailyCosts {
            date: self.date,
            sonnet_costs,
            haiku_costs,
            opus_costs,
        }
    }
}

#[derive(Debug)]
pub struct DailyCosts {
    pub date: NaiveDate,
    pub sonnet_costs: TokenCosts,
    pub haiku_costs: TokenCosts,
    pub opus_costs: TokenCosts,
}

impl DailyCosts {
    pub fn total(&self) -> f64 {
        self.sonnet_costs.total() + self.haiku_costs.total() + self.opus_costs.total()
    }
}

/// Recursively walk a directory and find all .jsonl files
pub async fn find_jsonl_files(dir: &Path) -> miette::Result<Vec<std::path::PathBuf>> {
    let mut jsonl_files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];

    while let Some(current_dir) = stack.pop() {
        let mut entries = fs::read_dir(&current_dir).await.into_diagnostic()?;

        while let Some(entry) = entries.next_entry().await.into_diagnostic()? {
            let path = entry.path();
            let metadata = entry.metadata().await.into_diagnostic()?;

            if metadata.is_dir() {
                stack.push(path);
            } else if metadata.is_file() {
                if let Some(extension) = path.extension() {
                    if extension == "jsonl" {
                        jsonl_files.push(path);
                    }
                }
            }
        }
    }

    Ok(jsonl_files)
}

/// Parse a log file and extract token usage
pub async fn parse_log_file(
    file: &Path,
) -> miette::Result<Vec<(NaiveDate, ModelType, String, TokenCounts)>> {
    let log_lines = parser::read_file(file).await?;
    let mut usages = Vec::new();

    for line in log_lines {
        if let LogLine::Assistant(assistant_line) = line {
            let date = assistant_line.timestamp.date_naive();
            let model_string = assistant_line.message.model.clone();
            let model_type = ModelType::from_model_string(&model_string);
            let usage = &assistant_line.message.usage;

            let counts = TokenCounts {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cache_write_tokens: usage.cache_creation_input_tokens,
                cache_read_tokens: usage.cache_read_input_tokens,
            };

            usages.push((date, model_type, model_string, counts));
        }
    }

    Ok(usages)
}

/// Aggregate usage data by date
pub fn aggregate_by_date(
    usages: Vec<(NaiveDate, ModelType, String, TokenCounts)>,
    unknown_models: &mut HashSet<String>,
    total_unknown_tokens: &mut TokenCounts,
) -> BTreeMap<NaiveDate, DailyUsage> {
    let mut daily_usage: BTreeMap<NaiveDate, DailyUsage> = BTreeMap::new();

    for (date, model_type, model_string, counts) in usages {
        // Track unknown models
        if model_type == ModelType::Unknown {
            unknown_models.insert(model_string);
            total_unknown_tokens.add(&counts);
        }

        daily_usage
            .entry(date)
            .or_insert_with(|| DailyUsage::new(date))
            .add_usage(model_type, counts);
    }

    daily_usage
}

/// Analyze all log files in a directory and return daily costs
pub async fn analyze_directory(dir: &Path) -> miette::Result<AnalysisResult> {
    let jsonl_files = find_jsonl_files(dir).await?;

    if jsonl_files.is_empty() {
        eprintln!("Warning: No .jsonl files found in directory");
        return Ok(AnalysisResult::default());
    }

    println!("Found {} log files to analyze", jsonl_files.len());

    let mut all_usages = Vec::new();
    let mut files_parsed = 0;
    let mut files_failed = 0;

    for file in &jsonl_files {
        match parse_log_file(file).await {
            Ok(usages) => {
                all_usages.extend(usages);
                files_parsed += 1;
            }
            Err(e) => {
                eprintln!("Warning: Failed to parse {:?}: {}", file, e);
                files_failed += 1;
            }
        }
    }

    let mut unknown_models = HashSet::new();
    let mut total_unknown_tokens = TokenCounts::default();

    let daily_usage = aggregate_by_date(all_usages, &mut unknown_models, &mut total_unknown_tokens);
    let daily_costs: Vec<DailyCosts> = daily_usage
        .into_values()
        .map(|usage| usage.calculate_costs())
        .collect();

    Ok(AnalysisResult {
        daily_costs,
        unknown_models,
        total_unknown_tokens,
        files_parsed,
        files_failed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn test_daily_usage_new() {
        let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
        let usage = DailyUsage::new(date);

        assert_eq!(usage.date, date);
        assert_eq!(usage.sonnet_usage.input_tokens, 0);
        assert_eq!(usage.haiku_usage.input_tokens, 0);
        assert_eq!(usage.opus_usage.input_tokens, 0);
        assert_eq!(usage.unknown_usage.input_tokens, 0);
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
        };

        assert!((costs.total() - 5.6).abs() < 1e-10);
    }

    #[test]
    fn test_aggregate_by_date_empty() {
        let usages = Vec::new();
        let mut unknown_models = HashSet::new();
        let mut total_unknown_tokens = TokenCounts::default();

        let result = aggregate_by_date(usages, &mut unknown_models, &mut total_unknown_tokens);

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

        let usages = vec![(
            date,
            ModelType::Sonnet,
            "claude-sonnet-4".to_string(),
            counts,
        )];
        let mut unknown_models = HashSet::new();
        let mut total_unknown_tokens = TokenCounts::default();

        let result = aggregate_by_date(usages, &mut unknown_models, &mut total_unknown_tokens);

        assert_eq!(result.len(), 1);
        assert!(result.contains_key(&date));
        assert_eq!(result[&date].sonnet_usage.input_tokens, 1000);
    }

    #[test]
    fn test_aggregate_by_date_multiple_dates() {
        let date1 = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
        let date2 = NaiveDate::from_ymd_opt(2025, 10, 24).unwrap();

        let usages = vec![
            (
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
            (
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

        let result = aggregate_by_date(usages, &mut unknown_models, &mut total_unknown_tokens);

        assert_eq!(result.len(), 2);
        assert_eq!(result[&date1].sonnet_usage.input_tokens, 1000);
        assert_eq!(result[&date2].haiku_usage.input_tokens, 2000);
    }

    #[test]
    fn test_aggregate_by_date_same_date_accumulates() {
        let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();

        let usages = vec![
            (
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
            (
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

        let result = aggregate_by_date(usages, &mut unknown_models, &mut total_unknown_tokens);

        assert_eq!(result.len(), 1);
        assert_eq!(result[&date].sonnet_usage.input_tokens, 1500);
        assert_eq!(result[&date].sonnet_usage.output_tokens, 750);
    }

    #[test]
    fn test_aggregate_by_date_tracks_unknown_models() {
        let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();

        let usages = vec![
            (
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
            (
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

        let result = aggregate_by_date(usages, &mut unknown_models, &mut total_unknown_tokens);

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
            (
                date1,
                ModelType::Sonnet,
                "claude-sonnet-4".to_string(),
                TokenCounts::default(),
            ),
            (
                date2,
                ModelType::Sonnet,
                "claude-sonnet-4".to_string(),
                TokenCounts::default(),
            ),
            (
                date3,
                ModelType::Sonnet,
                "claude-sonnet-4".to_string(),
                TokenCounts::default(),
            ),
        ];
        let mut unknown_models = HashSet::new();
        let mut total_unknown_tokens = TokenCounts::default();

        let result = aggregate_by_date(usages, &mut unknown_models, &mut total_unknown_tokens);
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

        let result = parse_log_file(&file_path).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_parse_log_file_extracts_usage_correctly() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.jsonl");

        let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4-20250514","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":100,"cache_read_input_tokens":50,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
        tokio::fs::write(&file_path, log_content).await.unwrap();

        let result = parse_log_file(&file_path).await.unwrap();

        assert_eq!(result.len(), 1);
        let (date, model_type, model_string, counts) = &result[0];

        assert_eq!(date, &NaiveDate::from_ymd_opt(2025, 10, 23).unwrap());
        assert_eq!(model_type, &ModelType::Sonnet);
        assert_eq!(model_string, "claude-sonnet-4-20250514");
        assert_eq!(counts.input_tokens, 1000);
        assert_eq!(counts.output_tokens, 500);
        assert_eq!(counts.cache_write_tokens, 100);
        assert_eq!(counts.cache_read_tokens, 50);
    }

    #[tokio::test]
    async fn test_parse_log_file_handles_multiple_assistant_messages() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.jsonl");

        let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-haiku-3","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":1000,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-24T12:00:00Z","isApiErrorMessage":null}"#;
        tokio::fs::write(&file_path, log_content).await.unwrap();

        let result = parse_log_file(&file_path).await.unwrap();

        assert_eq!(result.len(), 2);

        assert_eq!(result[0].1, ModelType::Sonnet);
        assert_eq!(result[0].3.input_tokens, 1000);

        assert_eq!(result[1].1, ModelType::Haiku);
        assert_eq!(result[1].3.input_tokens, 2000);
    }

    #[tokio::test]
    async fn test_parse_log_file_ignores_non_assistant_messages() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.jsonl");

        let log_content = r#"{"type":"user","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"role":"user","content":"test"},"isMeta":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T11:00:00Z","toolUseResult":null,"thinkingMetadata":null,"isVisibleInTranscriptOnly":null,"isCompactSummary":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
        tokio::fs::write(&file_path, log_content).await.unwrap();

        let result = parse_log_file(&file_path).await.unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, ModelType::Sonnet);
    }

    #[tokio::test]
    async fn test_analyze_directory_empty() {
        let temp_dir = tempfile::tempdir().unwrap();
        let result = analyze_directory(temp_dir.path()).await.unwrap();

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

        let result = analyze_directory(temp_dir.path()).await.unwrap();
        assert!(result.daily_costs.is_empty());
        assert_eq!(result.files_parsed, 0);
        assert_eq!(result.files_failed, 1);
    }
}
