use std::{collections::BTreeMap, path::Path};

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::prelude::ToPrimitive;

use super::pricing::PiModelMetricsMap;
use crate::cost_report::{
    CostComponents, DateTimezone, MetricComponents, MetricTotal, ReportMode, TimeRangeFilter,
    TokenCounts,
};
use cost_analyzer::{
    analyze_directory as cost_analyze_directory, AnalyzableLog, LineWithCost, LlmCost, TokenType,
};
use pi_logs::PiLogLine;

#[derive(Debug, Default)]
pub struct AnalysisResult {
    pub daily_metrics: Vec<DailyMetrics>,
    pub had_errors: bool,
}

#[derive(Debug, Default)]
pub struct SessionAnalysisResult {
    pub session_metrics: Vec<SessionMetrics>,
    pub had_errors: bool,
}

#[derive(Debug)]
pub struct DailyMetrics {
    pub date: NaiveDate,
    pub per_model: PiModelMetricsMap,
}

impl DailyMetrics {
    pub fn total(&self, report_mode: ReportMode) -> miette::Result<MetricTotal> {
        self.per_model.total(report_mode)
    }
}

#[derive(Debug)]
pub struct SessionMetrics {
    pub session_id: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub per_model: PiModelMetricsMap,
}

impl SessionMetrics {
    pub fn total(&self, report_mode: ReportMode) -> miette::Result<MetricTotal> {
        self.per_model.total(report_mode)
    }

    pub fn duration_minutes(&self) -> i64 {
        (self.end_time - self.start_time).num_minutes()
    }
}

/// Pi logs already carry money totals, so cost mode only needs to bridge
/// `cost_analyzer`'s Decimal values into the table layer's `f64` helpers.
fn cost_components_from_llm_cost(cost: &LlmCost) -> miette::Result<CostComponents> {
    Ok(CostComponents::new(
        cost.input
            .to_f64()
            .ok_or_else(|| miette::miette!("input cost could not be represented as f64"))?,
        cost.output
            .to_f64()
            .ok_or_else(|| miette::miette!("output cost could not be represented as f64"))?,
        cost.cache_write
            .to_f64()
            .ok_or_else(|| miette::miette!("cache-write cost could not be represented as f64"))?,
        cost.cache_read
            .to_f64()
            .ok_or_else(|| miette::miette!("cache-read cost could not be represented as f64"))?,
    ))
}

fn required_token_count(
    line: &LineWithCost<PiLogLine>,
    token_type: TokenType,
    component_name: &str,
) -> miette::Result<u64> {
    line.log.token_count(token_type).ok_or_else(|| {
        miette::miette!(
            "Pi billable line '{}' for model '{}' is missing {component_name} token usage.",
            line.id,
            line.model.model,
        )
    })
}

fn token_counts_from_line(line: &LineWithCost<PiLogLine>) -> miette::Result<TokenCounts> {
    Ok(TokenCounts::new(
        required_token_count(line, TokenType::Input, "input")?,
        required_token_count(line, TokenType::Output, "output")?,
        required_token_count(line, TokenType::CacheWrite, "cache-write")?,
        required_token_count(line, TokenType::CacheRead, "cache-read")?,
    ))
}

fn metric_components(
    line: &LineWithCost<PiLogLine>,
    report_mode: ReportMode,
) -> miette::Result<MetricComponents> {
    match report_mode {
        ReportMode::Cost => Ok(MetricComponents::Cost(cost_components_from_llm_cost(
            &line.cost,
        )?)),
        ReportMode::Tokens => Ok(MetricComponents::Tokens(token_counts_from_line(line)?)),
    }
}

async fn load_billable_lines(
    dir: &Path,
    filter: &TimeRangeFilter,
    report_mode: ReportMode,
) -> miette::Result<(Vec<(LineWithCost<PiLogLine>, MetricComponents)>, bool)> {
    let result = cost_analyze_directory::<PiLogLine>(dir.to_path_buf()).await;
    let mut entries = Vec::new();

    for line in result.lines {
        if let Some(metrics) = billable_entry(&line, filter, report_mode)? {
            entries.push((line, metrics));
        }
    }

    Ok((entries, result.had_errors))
}

fn billable_entry(
    line: &LineWithCost<PiLogLine>,
    filter: &TimeRangeFilter,
    report_mode: ReportMode,
) -> miette::Result<Option<MetricComponents>> {
    if !filter.contains(&line.timestamp) {
        return Ok(None);
    }
    Ok(Some(metric_components(line, report_mode)?))
}

pub async fn analyze_directory(
    dir: &Path,
    timezone: DateTimezone,
    filter: &TimeRangeFilter,
    report_mode: ReportMode,
) -> miette::Result<AnalysisResult> {
    let (entries, had_errors) = load_billable_lines(dir, filter, report_mode).await?;

    let mut buckets: BTreeMap<NaiveDate, PiModelMetricsMap> = BTreeMap::new();

    for (line, metrics) in entries {
        let date = timezone.to_date(&line.timestamp);
        buckets
            .entry(date)
            .or_default()
            .add(line.model.clone(), metrics)?;
    }

    let daily_metrics = buckets
        .into_iter()
        .map(|(date, per_model)| DailyMetrics { date, per_model })
        .collect();

    Ok(AnalysisResult {
        daily_metrics,
        had_errors,
    })
}

pub async fn analyze_directory_by_session(
    dir: &Path,
    filter: &TimeRangeFilter,
    report_mode: ReportMode,
) -> miette::Result<SessionAnalysisResult> {
    struct SessionAccumulator {
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        per_model: PiModelMetricsMap,
    }

    let (entries, had_errors) = load_billable_lines(dir, filter, report_mode).await?;
    let mut buckets: BTreeMap<String, SessionAccumulator> = BTreeMap::new();

    for (line, metrics) in entries {
        let session_id = line.session_id.clone().ok_or_else(|| {
            miette::miette!(
                "Pi billable line '{}' for model '{}' is missing a session id. \
                 Re-run after fixing the sessions directory so each billable line follows a session header.",
                line.id,
                line.model.model,
            )
        })?;

        let acc = buckets
            .entry(session_id)
            .or_insert_with(|| SessionAccumulator {
                start_time: line.timestamp,
                end_time: line.timestamp,
                per_model: PiModelMetricsMap::default(),
            });

        if line.timestamp < acc.start_time {
            acc.start_time = line.timestamp;
        }
        if line.timestamp > acc.end_time {
            acc.end_time = line.timestamp;
        }
        acc.per_model.add(line.model.clone(), metrics)?;
    }

    let mut session_metrics: Vec<SessionMetrics> = buckets
        .into_iter()
        .map(|(session_id, acc)| SessionMetrics {
            session_id,
            start_time: acc.start_time,
            end_time: acc.end_time,
            per_model: acc.per_model,
        })
        .collect();

    session_metrics.sort_by_key(|session| session.start_time);

    Ok(SessionAnalysisResult {
        session_metrics,
        had_errors,
    })
}

#[cfg(test)]
pub(crate) type DailyCosts = DailyMetrics;
#[cfg(test)]
pub(crate) type SessionCosts = SessionMetrics;
#[cfg(test)]
pub(crate) type ComponentTotals = CostComponents;

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use chrono::TimeZone;
    use rust_decimal::Decimal;
    use serde_json::{json, Value};
    use tempfile::TempDir;

    use super::*;
    use pi_logs::Provider;

    fn write_log(dir: &Path, name: &str, lines: &[Value]) {
        let path = dir.join(name);
        let body = lines
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(path, body).expect("write fixture log");
    }

    fn unrestricted_filter() -> TimeRangeFilter {
        TimeRangeFilter::new(None, None).expect("unrestricted filter")
    }

    fn april_16_only_filter() -> TimeRangeFilter {
        TimeRangeFilter::new(
            Some("2026-04-16".to_string()),
            Some("2026-04-16".to_string()),
        )
        .expect("filter parses")
    }

    fn timestamp(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .expect("valid timestamp")
    }

    fn session_line(session_id: &str, timestamp: DateTime<Utc>) -> Value {
        json!({
            "type": "session",
            "version": 1,
            "id": session_id,
            "timestamp": timestamp.to_rfc3339(),
            "cwd": "/tmp/moriarty-test"
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn assistant_line(
        id: &str,
        timestamp: DateTime<Utc>,
        provider: &str,
        api: &str,
        model: &str,
        input: &str,
        output: &str,
        cache_write: &str,
        cache_read: &str,
    ) -> Value {
        assistant_line_with_tokens(
            id,
            timestamp,
            provider,
            api,
            model,
            input,
            output,
            cache_write,
            cache_read,
            10,
            5,
            1,
            2,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn assistant_line_with_tokens(
        id: &str,
        timestamp: DateTime<Utc>,
        provider: &str,
        api: &str,
        model: &str,
        input: &str,
        output: &str,
        cache_write: &str,
        cache_read: &str,
        input_tokens: i64,
        output_tokens: i64,
        cache_write_tokens: i64,
        cache_read_tokens: i64,
    ) -> Value {
        let total = Decimal::from_str_exact(input).unwrap()
            + Decimal::from_str_exact(output).unwrap()
            + Decimal::from_str_exact(cache_write).unwrap()
            + Decimal::from_str_exact(cache_read).unwrap();
        json!({
            "type": "message",
            "id": id,
            "parentId": "u1",
            "timestamp": timestamp.to_rfc3339(),
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": "hello"}],
                "api": api,
                "provider": provider,
                "model": model,
                "usage": {
                    "input": input_tokens,
                    "output": output_tokens,
                    "cacheRead": cache_read_tokens,
                    "cacheWrite": cache_write_tokens,
                    "totalTokens": input_tokens + output_tokens + cache_write_tokens + cache_read_tokens,
                    "cost": {
                        "input": input,
                        "output": output,
                        "cacheRead": cache_read,
                        "cacheWrite": cache_write,
                        "total": total.to_string()
                    }
                },
                "stopReason": "stop",
                "timestamp": 1_700_000_000
            }
        })
    }

    fn anthropic_line(
        id: &str,
        timestamp: DateTime<Utc>,
        model: &str,
        input: &str,
        output: &str,
        cache_write: &str,
        cache_read: &str,
    ) -> Value {
        assistant_line(
            id,
            timestamp,
            "anthropic",
            "anthropic-messages",
            model,
            input,
            output,
            cache_write,
            cache_read,
        )
    }

    fn openai_line(
        id: &str,
        timestamp: DateTime<Utc>,
        model: &str,
        input: &str,
        output: &str,
        cache_write: &str,
        cache_read: &str,
    ) -> Value {
        assistant_line(
            id,
            timestamp,
            "openai",
            "openai-responses",
            model,
            input,
            output,
            cache_write,
            cache_read,
        )
    }

    fn model_cost<'a>(
        daily: &'a DailyCosts,
        provider: Provider,
        model: &str,
    ) -> &'a ComponentTotals {
        daily
            .per_model
            .model_costs()
            .find_map(|(pi_model, costs)| {
                (pi_model.provider == provider && pi_model.model == model).then_some(costs)
            })
            .expect("model bucket present")
    }

    fn session_model_cost<'a>(
        session: &'a SessionCosts,
        provider: Provider,
        model: &str,
    ) -> &'a ComponentTotals {
        session
            .per_model
            .model_costs()
            .find_map(|(pi_model, costs)| {
                (pi_model.provider == provider && pi_model.model == model).then_some(costs)
            })
            .expect("model bucket present")
    }

    #[tokio::test]
    async fn analyze_directory_returns_empty_for_directory_with_no_logs() {
        let dir = TempDir::new().unwrap();

        let result = analyze_directory(
            dir.path(),
            DateTimezone::Utc,
            &unrestricted_filter(),
            ReportMode::Cost,
        )
        .await
        .unwrap();

        assert!(result.daily_metrics.is_empty());
        assert!(!result.had_errors);
    }

    #[tokio::test]
    async fn analyze_directory_propagates_partial_failures() {
        let dir = TempDir::new().unwrap();
        let session = "019dc252-e50e-766c-8182-d654b46881b0";
        write_log(
            dir.path(),
            "valid.jsonl",
            &[
                session_line(session, timestamp(2026, 4, 16, 0, 0)),
                anthropic_line(
                    "anthropic-1",
                    timestamp(2026, 4, 16, 9, 0),
                    "claude-sonnet-4-5",
                    "1.0",
                    "2.0",
                    "0",
                    "0",
                ),
            ],
        );
        fs::write(dir.path().join("invalid.jsonl"), "not json at all").unwrap();

        let result = analyze_directory(
            dir.path(),
            DateTimezone::Utc,
            &unrestricted_filter(),
            ReportMode::Cost,
        )
        .await
        .unwrap();

        assert_eq!(result.daily_metrics.len(), 1);
        assert!(result.had_errors);
    }

    #[tokio::test]
    async fn analyze_directory_buckets_lines_by_date_and_provider_model() {
        let dir = TempDir::new().unwrap();
        let session = "019dc252-e50e-766c-8182-d654b46881b0";
        write_log(
            dir.path(),
            "day-one.jsonl",
            &[
                session_line(session, timestamp(2026, 4, 16, 0, 0)),
                anthropic_line(
                    "anthropic-1",
                    timestamp(2026, 4, 16, 9, 0),
                    "claude-sonnet-4-5",
                    "1.5",
                    "2.5",
                    "0",
                    "0",
                ),
                openai_line(
                    "openai-1",
                    timestamp(2026, 4, 16, 10, 0),
                    "gpt-5",
                    "0.5",
                    "1.0",
                    "0",
                    "0",
                ),
            ],
        );
        write_log(
            dir.path(),
            "day-two.jsonl",
            &[
                session_line(session, timestamp(2026, 4, 17, 0, 0)),
                anthropic_line(
                    "anthropic-2",
                    timestamp(2026, 4, 17, 12, 0),
                    "claude-haiku-3-5",
                    "0.25",
                    "0.75",
                    "0",
                    "0",
                ),
            ],
        );

        let result = analyze_directory(
            dir.path(),
            DateTimezone::Utc,
            &unrestricted_filter(),
            ReportMode::Cost,
        )
        .await
        .unwrap();

        assert_eq!(result.daily_metrics.len(), 2);
        assert!(!result.had_errors);

        let day_1 = &result.daily_metrics[0];
        assert_eq!(day_1.date, NaiveDate::from_ymd_opt(2026, 4, 16).unwrap());
        let sonnet = model_cost(day_1, Provider::Anthropic, "claude-sonnet-4-5");
        assert!((sonnet.input - 1.5).abs() < 1e-9);
        assert!((sonnet.output - 2.5).abs() < 1e-9);
        let gpt = model_cost(day_1, Provider::OpenAi, "gpt-5");
        assert!((gpt.input - 0.5).abs() < 1e-9);
        assert!((gpt.output - 1.0).abs() < 1e-9);

        let day_2 = &result.daily_metrics[1];
        assert_eq!(day_2.date, NaiveDate::from_ymd_opt(2026, 4, 17).unwrap());
        let haiku = model_cost(day_2, Provider::Anthropic, "claude-haiku-3-5");
        assert!((haiku.input - 0.25).abs() < 1e-9);
        assert!((haiku.output - 0.75).abs() < 1e-9);
    }

    #[tokio::test]
    async fn analyze_directory_by_session_groups_by_session_and_tracks_time_range() {
        let dir = TempDir::new().unwrap();
        let session_a = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let session_b = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
        write_log(
            dir.path(),
            "session-b.jsonl",
            &[
                session_line(session_b, timestamp(2026, 4, 16, 11, 0)),
                openai_line(
                    "b-1",
                    timestamp(2026, 4, 16, 12, 0),
                    "gpt-5",
                    "1.0",
                    "0.0",
                    "0",
                    "0",
                ),
            ],
        );
        write_log(
            dir.path(),
            "session-a.jsonl",
            &[
                session_line(session_a, timestamp(2026, 4, 16, 8, 30)),
                anthropic_line(
                    "a-1",
                    timestamp(2026, 4, 16, 9, 0),
                    "claude-sonnet-4-5",
                    "1.0",
                    "0.0",
                    "0",
                    "0",
                ),
                anthropic_line(
                    "a-2",
                    timestamp(2026, 4, 16, 10, 30),
                    "claude-sonnet-4-5",
                    "0.0",
                    "2.0",
                    "0",
                    "0",
                ),
            ],
        );

        let result =
            analyze_directory_by_session(dir.path(), &unrestricted_filter(), ReportMode::Cost)
                .await
                .unwrap();

        assert_eq!(result.session_metrics.len(), 2);
        assert!(!result.had_errors);

        let first = &result.session_metrics[0];
        assert_eq!(first.session_id, session_a);
        assert_eq!(first.start_time, timestamp(2026, 4, 16, 9, 0));
        assert_eq!(first.end_time, timestamp(2026, 4, 16, 10, 30));
        assert_eq!(first.duration_minutes(), 90);
        assert_eq!(first.per_model.len(), 1);

        let second = &result.session_metrics[1];
        assert_eq!(second.session_id, session_b);
        assert_eq!(second.start_time, timestamp(2026, 4, 16, 12, 0));
        assert_eq!(second.end_time, timestamp(2026, 4, 16, 12, 0));
        assert_eq!(second.per_model.len(), 1);
    }

    #[tokio::test]
    async fn analyze_directory_by_session_merges_same_session_across_files() {
        let dir = TempDir::new().unwrap();
        let session = "019dc252-e50e-766c-8182-d654b46881b0";
        write_log(
            dir.path(),
            "session-part-a.jsonl",
            &[
                session_line(session, timestamp(2026, 4, 16, 8, 0)),
                anthropic_line(
                    "a-1",
                    timestamp(2026, 4, 16, 9, 0),
                    "claude-sonnet-4-5",
                    "1.0",
                    "0.0",
                    "0",
                    "0",
                ),
            ],
        );
        write_log(
            dir.path(),
            "session-part-b.jsonl",
            &[
                session_line(session, timestamp(2026, 4, 16, 11, 0)),
                openai_line(
                    "b-1",
                    timestamp(2026, 4, 16, 12, 30),
                    "gpt-5",
                    "0.5",
                    "1.5",
                    "0",
                    "0",
                ),
            ],
        );

        let result =
            analyze_directory_by_session(dir.path(), &unrestricted_filter(), ReportMode::Cost)
                .await
                .unwrap();

        assert_eq!(result.session_metrics.len(), 1);
        let merged = &result.session_metrics[0];
        assert_eq!(merged.session_id, session);
        assert_eq!(merged.start_time, timestamp(2026, 4, 16, 9, 0));
        assert_eq!(merged.end_time, timestamp(2026, 4, 16, 12, 30));
        assert_eq!(merged.per_model.len(), 2);

        let sonnet = session_model_cost(merged, Provider::Anthropic, "claude-sonnet-4-5");
        assert!((sonnet.input - 1.0).abs() < 1e-9);
        let gpt = session_model_cost(merged, Provider::OpenAi, "gpt-5");
        assert!((gpt.input - 0.5).abs() < 1e-9);
        assert!((gpt.output - 1.5).abs() < 1e-9);
    }

    #[tokio::test]
    async fn analyze_directory_tokens_use_raw_usage_counts() {
        let dir = TempDir::new().unwrap();
        let session = "019dc252-e50e-766c-8182-d654b46881b0";
        write_log(
            dir.path(),
            "tokens.jsonl",
            &[
                session_line(session, timestamp(2026, 4, 16, 0, 0)),
                assistant_line_with_tokens(
                    "anthropic-1",
                    timestamp(2026, 4, 16, 9, 0),
                    "anthropic",
                    "anthropic-messages",
                    "claude-sonnet-4-5",
                    "1.0",
                    "2.0",
                    "0",
                    "0",
                    1_234,
                    5_678,
                    90,
                    12,
                ),
            ],
        );

        let result = analyze_directory(
            dir.path(),
            DateTimezone::Utc,
            &unrestricted_filter(),
            ReportMode::Tokens,
        )
        .await
        .unwrap();

        let costs = result.daily_metrics[0]
            .per_model
            .model_metrics()
            .find_map(|(pi_model, metrics)| {
                (pi_model.provider == Provider::Anthropic && pi_model.model == "claude-sonnet-4-5")
                    .then_some(metrics)
            })
            .expect("model bucket present");
        let MetricComponents::Tokens(costs) = costs else {
            panic!("expected token metrics")
        };
        assert_eq!(costs.input, 1_234);
        assert_eq!(costs.output, 5_678);
        assert_eq!(costs.cache_write, 90);
        assert_eq!(costs.cache_read, 12);
        assert_eq!(costs.total(), 7_014);
    }

    #[tokio::test]
    async fn analyze_directory_by_session_tokens_group_by_session_and_preserve_time_range() {
        let dir = TempDir::new().unwrap();
        let session = "019dc252-e50e-766c-8182-d654b46881b0";
        write_log(
            dir.path(),
            "session-tokens.jsonl",
            &[
                session_line(session, timestamp(2026, 4, 16, 0, 0)),
                assistant_line_with_tokens(
                    "anthropic-1",
                    timestamp(2026, 4, 16, 9, 0),
                    "anthropic",
                    "anthropic-messages",
                    "claude-sonnet-4-5",
                    "1.0",
                    "2.0",
                    "0",
                    "0",
                    1_234,
                    5_678,
                    90,
                    12,
                ),
                assistant_line_with_tokens(
                    "anthropic-2",
                    timestamp(2026, 4, 16, 10, 30),
                    "anthropic",
                    "anthropic-messages",
                    "claude-sonnet-4-5",
                    "0.5",
                    "1.5",
                    "0",
                    "0",
                    10,
                    20,
                    30,
                    40,
                ),
            ],
        );

        let result =
            analyze_directory_by_session(dir.path(), &unrestricted_filter(), ReportMode::Tokens)
                .await
                .unwrap();

        assert_eq!(result.session_metrics.len(), 1);
        let session_metrics = &result.session_metrics[0];
        assert_eq!(session_metrics.session_id, session);
        assert_eq!(session_metrics.start_time, timestamp(2026, 4, 16, 9, 0));
        assert_eq!(session_metrics.end_time, timestamp(2026, 4, 16, 10, 30));
        assert_eq!(session_metrics.duration_minutes(), 90);
        let costs = session_metrics
            .per_model
            .model_metrics()
            .find_map(|(pi_model, metrics)| {
                (pi_model.provider == Provider::Anthropic && pi_model.model == "claude-sonnet-4-5")
                    .then_some(metrics)
            })
            .expect("model bucket present");
        let MetricComponents::Tokens(costs) = costs else {
            panic!("expected token metrics")
        };
        assert_eq!(costs.input, 1_244);
        assert_eq!(costs.output, 5_698);
        assert_eq!(costs.cache_write, 120);
        assert_eq!(costs.cache_read, 52);
        assert_eq!(costs.total(), 7_114);
    }

    #[tokio::test]
    async fn analyze_directory_applies_time_filter_after_dedup() {
        let dir = TempDir::new().unwrap();
        let session = "019dc252-e50e-766c-8182-d654b46881b0";
        let shared_id = "shared-response";
        write_log(
            dir.path(),
            "out-of-window.jsonl",
            &[
                session_line(session, timestamp(2026, 4, 15, 0, 0)),
                anthropic_line(
                    shared_id,
                    timestamp(2026, 4, 15, 12, 0),
                    "claude-sonnet-4-5",
                    "0.0",
                    "5.0",
                    "0",
                    "0",
                ),
            ],
        );
        write_log(
            dir.path(),
            "in-window.jsonl",
            &[
                session_line(session, timestamp(2026, 4, 16, 0, 0)),
                anthropic_line(
                    shared_id,
                    timestamp(2026, 4, 16, 12, 0),
                    "claude-sonnet-4-5",
                    "1.0",
                    "0.0",
                    "0",
                    "0",
                ),
            ],
        );

        let result = analyze_directory(
            dir.path(),
            DateTimezone::Utc,
            &april_16_only_filter(),
            ReportMode::Cost,
        )
        .await
        .unwrap();

        assert!(result.daily_metrics.is_empty());
        assert!(!result.had_errors);
    }

    #[tokio::test]
    async fn analyze_directory_by_session_applies_time_filter_after_dedup() {
        let dir = TempDir::new().unwrap();
        let session = "019dc252-e50e-766c-8182-d654b46881b0";
        let shared_id = "shared-response";
        write_log(
            dir.path(),
            "out-of-window.jsonl",
            &[
                session_line(session, timestamp(2026, 4, 15, 0, 0)),
                anthropic_line(
                    shared_id,
                    timestamp(2026, 4, 15, 12, 0),
                    "claude-sonnet-4-5",
                    "0.0",
                    "5.0",
                    "0",
                    "0",
                ),
            ],
        );
        write_log(
            dir.path(),
            "in-window.jsonl",
            &[
                session_line(session, timestamp(2026, 4, 16, 0, 0)),
                anthropic_line(
                    shared_id,
                    timestamp(2026, 4, 16, 12, 0),
                    "claude-sonnet-4-5",
                    "1.0",
                    "0.0",
                    "0",
                    "0",
                ),
            ],
        );

        let result =
            analyze_directory_by_session(dir.path(), &april_16_only_filter(), ReportMode::Cost)
                .await
                .unwrap();

        assert!(result.session_metrics.is_empty());
        assert!(!result.had_errors);
    }

    #[tokio::test]
    async fn analyze_directory_keeps_in_window_lines_when_filtered() {
        let dir = TempDir::new().unwrap();
        let session = "019dc252-e50e-766c-8182-d654b46881b0";
        write_log(
            dir.path(),
            "filtered.jsonl",
            &[
                session_line(session, timestamp(2026, 4, 16, 0, 0)),
                anthropic_line(
                    "out-of-window",
                    timestamp(2026, 4, 15, 11, 0),
                    "claude-sonnet-4-5",
                    "1.0",
                    "0.0",
                    "0",
                    "0",
                ),
                openai_line(
                    "in-window",
                    timestamp(2026, 4, 16, 14, 0),
                    "gpt-5",
                    "0.5",
                    "1.5",
                    "0",
                    "0",
                ),
            ],
        );

        let result = analyze_directory(
            dir.path(),
            DateTimezone::Utc,
            &april_16_only_filter(),
            ReportMode::Cost,
        )
        .await
        .unwrap();

        assert_eq!(result.daily_metrics.len(), 1);
        let day = &result.daily_metrics[0];
        assert_eq!(day.date, NaiveDate::from_ymd_opt(2026, 4, 16).unwrap());
        let kept = model_cost(day, Provider::OpenAi, "gpt-5");
        assert!((kept.input - 0.5).abs() < 1e-9);
        assert!((kept.output - 1.5).abs() < 1e-9);
        assert_eq!(day.per_model.len(), 1);
    }

    #[tokio::test]
    async fn analyze_directory_by_session_keeps_in_window_lines_when_filtered() {
        let dir = TempDir::new().unwrap();
        let session = "019dc252-e50e-766c-8182-d654b46881b0";
        write_log(
            dir.path(),
            "filtered-session.jsonl",
            &[
                session_line(session, timestamp(2026, 4, 16, 0, 0)),
                anthropic_line(
                    "out-of-window",
                    timestamp(2026, 4, 15, 11, 0),
                    "claude-sonnet-4-5",
                    "1.0",
                    "0.0",
                    "0",
                    "0",
                ),
                openai_line(
                    "in-window",
                    timestamp(2026, 4, 16, 14, 0),
                    "gpt-5",
                    "0.5",
                    "1.5",
                    "0",
                    "0",
                ),
            ],
        );

        let result =
            analyze_directory_by_session(dir.path(), &april_16_only_filter(), ReportMode::Cost)
                .await
                .unwrap();

        assert_eq!(result.session_metrics.len(), 1);
        let kept_session = &result.session_metrics[0];
        assert_eq!(kept_session.start_time, timestamp(2026, 4, 16, 14, 0));
        assert_eq!(kept_session.end_time, timestamp(2026, 4, 16, 14, 0));
        let kept = session_model_cost(kept_session, Provider::OpenAi, "gpt-5");
        assert!((kept.input - 0.5).abs() < 1e-9);
        assert!((kept.output - 1.5).abs() < 1e-9);
        assert_eq!(kept_session.per_model.len(), 1);
    }

    #[tokio::test]
    async fn analyze_directory_by_session_propagates_partial_failures() {
        let dir = TempDir::new().unwrap();
        let session = "019dc252-e50e-766c-8182-d654b46881b0";
        write_log(
            dir.path(),
            "valid.jsonl",
            &[
                session_line(session, timestamp(2026, 4, 16, 0, 0)),
                anthropic_line(
                    "anthropic-1",
                    timestamp(2026, 4, 16, 9, 0),
                    "claude-sonnet-4-5",
                    "1.0",
                    "2.0",
                    "0",
                    "0",
                ),
            ],
        );
        fs::write(dir.path().join("invalid.jsonl"), "not json at all").unwrap();

        let result =
            analyze_directory_by_session(dir.path(), &unrestricted_filter(), ReportMode::Cost)
                .await
                .unwrap();

        assert_eq!(result.session_metrics.len(), 1);
        assert!(result.had_errors);
    }

    #[tokio::test]
    async fn analyze_directory_sums_cache_components_without_repricing() {
        let dir = TempDir::new().unwrap();
        let session = "019dc252-e50e-766c-8182-d654b46881b0";
        write_log(
            dir.path(),
            "cache.jsonl",
            &[
                session_line(session, timestamp(2026, 4, 16, 0, 0)),
                anthropic_line(
                    "cache-line",
                    timestamp(2026, 4, 16, 1, 0),
                    "claude-sonnet-4-5",
                    "1.0",
                    "2.0",
                    "3.0",
                    "4.0",
                ),
            ],
        );

        let result = analyze_directory(
            dir.path(),
            DateTimezone::Utc,
            &unrestricted_filter(),
            ReportMode::Cost,
        )
        .await
        .unwrap();

        let costs = model_cost(
            &result.daily_metrics[0],
            Provider::Anthropic,
            "claude-sonnet-4-5",
        );
        assert!((costs.input - 1.0).abs() < 1e-9);
        assert!((costs.output - 2.0).abs() < 1e-9);
        assert!((costs.cache_write - 3.0).abs() < 1e-9);
        assert!((costs.cache_read - 4.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn analyze_directory_accumulates_multiple_providers_and_models_within_one_bucket() {
        let dir = TempDir::new().unwrap();
        let session = "019dc252-e50e-766c-8182-d654b46881b0";
        write_log(
            dir.path(),
            "mixed.jsonl",
            &[
                session_line(session, timestamp(2026, 4, 16, 0, 0)),
                anthropic_line(
                    "anthropic-sonnet",
                    timestamp(2026, 4, 16, 9, 0),
                    "claude-sonnet-4-5",
                    "1.0",
                    "0.0",
                    "0",
                    "0",
                ),
                anthropic_line(
                    "anthropic-haiku",
                    timestamp(2026, 4, 16, 9, 30),
                    "claude-haiku-3-5",
                    "0.5",
                    "0.0",
                    "0",
                    "0",
                ),
                openai_line(
                    "openai-gpt",
                    timestamp(2026, 4, 16, 10, 0),
                    "gpt-5",
                    "2.0",
                    "0.0",
                    "0",
                    "0",
                ),
            ],
        );

        let result = analyze_directory(
            dir.path(),
            DateTimezone::Utc,
            &unrestricted_filter(),
            ReportMode::Cost,
        )
        .await
        .unwrap();

        let bucket = &result.daily_metrics[0];
        assert_eq!(bucket.per_model.len(), 3);
    }

    #[tokio::test]
    async fn analyze_directory_by_session_errors_when_a_kept_line_has_no_session_id() {
        let dir = TempDir::new().unwrap();
        write_log(
            dir.path(),
            "missing-session.jsonl",
            &[anthropic_line(
                "no-session",
                timestamp(2026, 4, 16, 9, 0),
                "claude-sonnet-4-5",
                "1.0",
                "0.0",
                "0",
                "0",
            )],
        );

        let error =
            analyze_directory_by_session(dir.path(), &unrestricted_filter(), ReportMode::Cost)
                .await
                .unwrap_err();

        assert!(
            error.to_string().contains("missing a session id"),
            "unexpected error: {error}"
        );
    }
}
