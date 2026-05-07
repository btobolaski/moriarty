use std::{collections::BTreeMap, path::Path};

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::prelude::ToPrimitive;

use super::pricing::{ModelMetricsMap, ModelType};
use crate::cost_report::{
    CostComponents, DateTimezone, MetricComponents, MetricTotal, ReportMode, TimeRangeFilter,
    TokenCounts,
};
use claude_logs::LogLine;
use cost_analyzer::{
    analyze_directory as cost_analyze_directory, AnalyzableLog, LineWithCost, LlmCost, TokenType,
};

/// `had_errors` is propagated from `cost_analyzer::analyze_directory` so the
/// report layer can warn that totals may be incomplete after per-file details
/// have already gone to tracing.
#[derive(Debug, Default)]
pub struct AnalysisResult {
    pub daily_metrics: Vec<DailyMetrics>,
    pub had_errors: bool,
}

/// Conversation-mode aggregation reuses the same partial-failure flag as the
/// daily report so callers do not need a separate warning path.
#[derive(Debug, Default)]
pub struct SessionAnalysisResult {
    pub session_metrics: Vec<SessionMetrics>,
    pub had_errors: bool,
}

#[derive(Debug)]
pub struct DailyMetrics {
    pub date: NaiveDate,
    pub per_model: ModelMetricsMap,
}

impl DailyMetrics {
    pub fn total(&self, report_mode: ReportMode) -> miette::Result<MetricTotal> {
        self.per_model.total(report_mode)
    }
}

/// `start_time` and `end_time` bracket only the kept billable lines, so the
/// rendered duration matches any caller-supplied time filter.
#[derive(Debug)]
pub struct SessionMetrics {
    pub session_id: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub per_model: ModelMetricsMap,
}

impl SessionMetrics {
    pub fn total(&self, report_mode: ReportMode) -> miette::Result<MetricTotal> {
        self.per_model.total(report_mode)
    }

    pub fn duration_minutes(&self) -> i64 {
        (self.end_time - self.start_time).num_minutes()
    }
}

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
    line: &LineWithCost<LogLine>,
    token_type: TokenType,
    component_name: &str,
) -> miette::Result<u64> {
    line.log.token_count(token_type).ok_or_else(|| {
        miette::miette!(
            "Claude billable line '{}' for model '{}' is missing {component_name} token usage.",
            line.id,
            line.model,
        )
    })
}

fn token_counts_from_line(line: &LineWithCost<LogLine>) -> miette::Result<TokenCounts> {
    Ok(TokenCounts::new(
        required_token_count(line, TokenType::Input, "input")?,
        required_token_count(line, TokenType::Output, "output")?,
        required_token_count(line, TokenType::CacheWrite, "cache-write")?,
        required_token_count(line, TokenType::CacheRead, "cache-read")?,
    ))
}

fn metric_components(
    line: &LineWithCost<LogLine>,
    report_mode: ReportMode,
) -> miette::Result<MetricComponents> {
    match report_mode {
        ReportMode::Cost => Ok(MetricComponents::Cost(cost_components_from_llm_cost(
            &line.cost,
        )?)),
        ReportMode::Tokens => Ok(MetricComponents::Tokens(token_counts_from_line(line)?)),
    }
}

/// Loads all dedup-resolved, time-filtered billable entries from `dir`.
///
/// Both `analyze_directory` and `analyze_directory_by_session` start by
/// fetching `cost_analyzer`'s deduplicated lines and pairing each surviving
/// line with its `(ModelType, MetricComponents)`. Sharing that prelude in one
/// helper keeps the two entry points from drifting in load semantics (e.g.,
/// dedup-then-filter ordering, future cost adjustments) and lets each entry
/// point own only its bucketing logic.
async fn load_billable_lines(
    dir: &Path,
    filter: &TimeRangeFilter,
    report_mode: ReportMode,
) -> miette::Result<(
    Vec<(LineWithCost<LogLine>, ModelType, MetricComponents)>,
    bool,
)> {
    let result = cost_analyze_directory::<LogLine>(dir.to_path_buf()).await;
    let mut entries = Vec::new();

    for line in result.lines {
        if let Some((model_type, metrics)) = billable_entry(&line, filter, report_mode)? {
            entries.push((line, model_type, metrics));
        }
    }

    Ok((entries, result.had_errors))
}

/// Centralizing the post-dedup filter check + model classification + metric
/// extraction keeps both aggregation entry points aligned if the shared prelude
/// ever gains another step.
fn billable_entry(
    line: &LineWithCost<LogLine>,
    filter: &TimeRangeFilter,
    report_mode: ReportMode,
) -> miette::Result<Option<(ModelType, MetricComponents)>> {
    if !filter.contains(&line.timestamp) {
        return Ok(None);
    }
    Ok(Some((
        ModelType::from_model_string(&line.model),
        metric_components(line, report_mode)?,
    )))
}

pub async fn analyze_directory(
    dir: &Path,
    timezone: DateTimezone,
    filter: &TimeRangeFilter,
    report_mode: ReportMode,
) -> miette::Result<AnalysisResult> {
    let (entries, had_errors) = load_billable_lines(dir, filter, report_mode).await?;

    let mut buckets: BTreeMap<NaiveDate, ModelMetricsMap> = BTreeMap::new();

    for (line, model_type, metrics) in entries {
        let date = timezone.to_date(&line.timestamp);
        buckets.entry(date).or_default().add(model_type, metrics)?;
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
    let (entries, had_errors) = load_billable_lines(dir, filter, report_mode).await?;
    session_analysis_from_entries(entries, had_errors)
}

fn session_analysis_from_entries(
    entries: Vec<(LineWithCost<LogLine>, ModelType, MetricComponents)>,
    had_errors: bool,
) -> miette::Result<SessionAnalysisResult> {
    struct SessionAccumulator {
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        per_model: ModelMetricsMap,
    }

    let mut buckets: BTreeMap<String, SessionAccumulator> = BTreeMap::new();

    for (line, model_type, metrics) in entries {
        let session_id = line.session_id.clone().ok_or_else(|| {
            miette::miette!(
                "Claude billable line '{}' for model '{}' is missing a session id. \
                 Conversation reports require session metadata on every billable assistant line.",
                line.id,
                line.model,
            )
        })?;

        let acc = buckets
            .entry(session_id)
            .or_insert_with(|| SessionAccumulator {
                start_time: line.timestamp,
                end_time: line.timestamp,
                per_model: ModelMetricsMap::default(),
            });

        if line.timestamp < acc.start_time {
            acc.start_time = line.timestamp;
        }
        if line.timestamp > acc.end_time {
            acc.end_time = line.timestamp;
        }
        acc.per_model.add(model_type, metrics)?;
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
pub type DailyCosts = DailyMetrics;
#[cfg(test)]
pub type SessionCosts = SessionMetrics;
#[cfg(test)]
pub type ModelCostsMap = ModelMetricsMap;

#[cfg(test)]
fn component_totals_from_llm_cost(cost: &LlmCost) -> CostComponents {
    cost_components_from_llm_cost(cost).expect("cost components convert")
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use chrono::TimeZone;
    use rust_decimal::Decimal;
    use serde_json::{json, Value};
    use tempfile::TempDir;

    use super::*;

    /// Pricing for `claude-sonnet-4-20250514` at one million tokens of each
    /// kind, used to check that aggregation sums `LineWithCost.cost` directly
    /// rather than re-pricing token counts. Mirrors the constants in
    /// `cost_analyzer::ClaudeModelPricing::SONNET`.
    const SONNET_INPUT_PER_MILLION: f64 = 3.0;
    const SONNET_OUTPUT_PER_MILLION: f64 = 15.0;
    const SONNET_CACHE_WRITE_PER_MILLION: f64 = 3.75;
    const SONNET_CACHE_READ_PER_MILLION: f64 = 0.30;

    /// Pricing for `claude-3-haiku-20240307`.
    const HAIKU_INPUT_PER_MILLION: f64 = 1.0;
    const HAIKU_OUTPUT_PER_MILLION: f64 = 5.0;

    fn write_log(dir: &Path, name: &str, lines: &[Value]) {
        let path = dir.join(name);
        let body: String = lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(path, body).expect("write fixture log");
    }

    fn unrestricted_filter() -> TimeRangeFilter {
        TimeRangeFilter::new(None, None).expect("unrestricted filter")
    }

    /// Filter that brackets the single calendar day 2026-04-16, used by the
    /// daily and session `*_applies_time_filter_after_dedup` tests.
    ///
    /// Both bounds use `2026-04-16` because `parse_datetime_for_end` advances
    /// date-only inputs to 00:00 of the *next* day for exclusive-end
    /// semantics, so this argument pair produces the half-open window
    /// `[2026-04-16 00:00 UTC, 2026-04-17 00:00 UTC)`.
    fn april_16_only_filter() -> TimeRangeFilter {
        TimeRangeFilter::new(
            Some("2026-04-16".to_string()),
            Some("2026-04-16".to_string()),
        )
        .expect("filter parses")
    }

    /// Writes a two-line Sonnet fixture: an out-of-window line on 2026-04-15
    /// (`req-out`, 1M input tokens) followed by an in-window line on
    /// 2026-04-16 (`req-in`, 1M output tokens). The session ids are caller-
    /// supplied so the same fixture can drive both the daily test (single
    /// shared session) and the session test (two distinct sessions).
    fn write_filter_window_fixture(
        dir: &Path,
        file_name: &str,
        session_out: &str,
        session_in: &str,
    ) {
        write_log(
            dir,
            file_name,
            &[
                assistant_line(
                    session_out,
                    timestamp(2026, 4, 15, 12, 0),
                    "claude-sonnet-4-20250514",
                    "req-out",
                    usage_json(1_000_000, 0, 0, 0),
                ),
                assistant_line(
                    session_in,
                    timestamp(2026, 4, 16, 12, 0),
                    "claude-sonnet-4-20250514",
                    "req-in",
                    usage_json(0, 1_000_000, 0, 0),
                ),
            ],
        );
    }

    /// Asserts that only the in-window line's output cost survived the filter:
    /// Sonnet output equals `SONNET_OUTPUT_PER_MILLION` and Sonnet input is 0.
    fn assert_only_in_window_sonnet_output(per_model: &ModelCostsMap) {
        let costs = per_model.get(ModelType::Sonnet);
        assert!(
            (costs.output - SONNET_OUTPUT_PER_MILLION).abs() < 1e-9,
            "in-window Sonnet output cost should match SONNET_OUTPUT_PER_MILLION, got {}",
            costs.output,
        );
        // Exact 0.0 is safe here without an epsilon: when the only kept line
        // contributes output tokens, `costs.input` is the unmodified default
        // from `ComponentTotals::default()` rather than the result of any
        // floating-point accumulation.
        assert_eq!(
            costs.input, 0.0,
            "out-of-window Sonnet input cost must not contribute to the bucket",
        );
    }

    fn timestamp(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .expect("valid timestamp")
    }

    fn usage_json(
        input_tokens: usize,
        output_tokens: usize,
        cache_write_tokens: usize,
        cache_read_tokens: usize,
    ) -> Value {
        json!({
            "input_tokens": input_tokens,
            "cache_creation_input_tokens": cache_write_tokens,
            "cache_read_input_tokens": cache_read_tokens,
            "cache_creation": {
                "ephemeral_5m_input_tokens": 0,
                "ephemeral_1h_input_tokens": 0,
            },
            "output_tokens": output_tokens,
            "service_tier": null,
            "server_tool_use": null,
            "inference_geo": null,
            "iterations": null,
            "speed": null,
        })
    }

    /// Fixed UUID used for every fixture line. Dedup in `cost_analyzer` keys on
    /// `request_id -> message.id -> uuid` (in that order); every fixture sets a
    /// unique `request_id`, so the `uuid` field is only required to satisfy
    /// `claude_logs`'s strict deserialization and never participates in dedup.
    const FIXTURE_UUID: &str = "00000000-0000-4000-8000-000000000000";

    /// Builds an Assistant log line that satisfies
    /// `claude_logs::AssistantLogLine`'s `deny_unknown_fields`. Designed to
    /// match the shape produced by `cost_analyzer::test_support::claude_assistant_json`
    /// so the same `cost_analyzer` parser path is exercised here.
    fn assistant_line(
        session_id: &str,
        ts: DateTime<Utc>,
        model: &str,
        request_id: &str,
        usage: Value,
    ) -> Value {
        json!({
            "parentUuid": null,
            "isSidechain": false,
            "agentId": null,
            "userType": "external",
            "cwd": "/tmp/moriarty-test",
            "sessionId": session_id,
            "version": "2.1.104",
            "gitBranch": "main",
            "slug": null,
            "type": "assistant",
            "message": {
                "id": format!("msg-{request_id}"),
                "type": "message",
                "role": "assistant",
                "model": model,
                "container": null,
                "content": [{"type": "text", "text": "hello"}],
                "stop_reason": "end_turn",
                "stop_sequence": null,
                "stop_details": null,
                "usage": usage,
                "context_management": null,
            },
            "requestId": request_id,
            "uuid": FIXTURE_UUID,
            "timestamp": ts.to_rfc3339(),
            "isApiErrorMessage": null,
            "error": null,
            "entrypoint": null,
        })
    }

    #[test]
    fn date_timezone_utc_to_date_uses_utc_calendar_day() {
        let ts = timestamp(2026, 4, 16, 23, 59);

        assert_eq!(
            DateTimezone::Utc.to_date(&ts),
            NaiveDate::from_ymd_opt(2026, 4, 16).unwrap()
        );
    }

    #[test]
    fn component_totals_from_llm_cost_handles_zero_and_nonzero_components() {
        let zero = LlmCost {
            input: Decimal::ZERO,
            output: Decimal::ZERO,
            cache_write: Decimal::ZERO,
            cache_read: Decimal::ZERO,
        };
        let zero_costs = component_totals_from_llm_cost(&zero);
        assert_eq!(zero_costs.input, 0.0);
        assert_eq!(zero_costs.output, 0.0);
        assert_eq!(zero_costs.cache_write, 0.0);
        assert_eq!(zero_costs.cache_read, 0.0);

        let cost = LlmCost {
            input: Decimal::new(150, 2),      // 1.50
            output: Decimal::new(275, 2),     // 2.75
            cache_write: Decimal::new(50, 2), // 0.50
            cache_read: Decimal::new(125, 3), // 0.125
        };
        let converted = component_totals_from_llm_cost(&cost);
        assert!((converted.input - 1.50).abs() < 1e-9);
        assert!((converted.output - 2.75).abs() < 1e-9);
        assert!((converted.cache_write - 0.50).abs() < 1e-9);
        assert!((converted.cache_read - 0.125).abs() < 1e-9);
    }

    #[test]
    fn session_analysis_from_entries_errors_when_session_id_is_missing() {
        let mut line = LineWithCost::<LogLine>::parse(
            &assistant_line(
                "session-a",
                timestamp(2026, 4, 16, 9, 0),
                "claude-sonnet-4-20250514",
                "req-missing-session",
                usage_json(1_000_000, 0, 0, 0),
            )
            .to_string(),
        )
        .unwrap()
        .unwrap();
        let model_type = ModelType::from_model_string(&line.model);
        let costs = component_totals_from_llm_cost(&line.cost);
        line.session_id = None;

        let error = session_analysis_from_entries(
            vec![(line, model_type, MetricComponents::Cost(costs))],
            false,
        )
        .unwrap_err();

        assert!(
            error.to_string().contains("missing a session id"),
            "unexpected error: {error}"
        );
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
    async fn analyze_directory_buckets_lines_by_utc_date_and_sums_costs_per_model() {
        let dir = TempDir::new().unwrap();
        let session = "019dc252-e50e-766c-8182-d654b46881af";
        // Two Sonnet calls on 2026-04-16 (one in each file, no duplicate IDs)
        // and one Haiku call on 2026-04-17.
        write_log(
            dir.path(),
            "a.jsonl",
            &[assistant_line(
                session,
                timestamp(2026, 4, 16, 1, 0),
                "claude-sonnet-4-20250514",
                "req-sonnet-1",
                usage_json(1_000_000, 0, 0, 0),
            )],
        );
        write_log(
            dir.path(),
            "b.jsonl",
            &[
                assistant_line(
                    session,
                    timestamp(2026, 4, 16, 2, 0),
                    "claude-sonnet-4-20250514",
                    "req-sonnet-2",
                    usage_json(0, 1_000_000, 0, 0),
                ),
                assistant_line(
                    session,
                    timestamp(2026, 4, 17, 5, 0),
                    "claude-3-haiku-20240307",
                    "req-haiku-1",
                    usage_json(1_000_000, 1_000_000, 0, 0),
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
        let sonnet_day_1 = day_1.per_model.get(ModelType::Sonnet);
        assert!((sonnet_day_1.input - SONNET_INPUT_PER_MILLION).abs() < 1e-9);
        assert!((sonnet_day_1.output - SONNET_OUTPUT_PER_MILLION).abs() < 1e-9);
        assert_eq!(day_1.per_model.get(ModelType::Haiku).total(), 0.0);

        let day_2 = &result.daily_metrics[1];
        assert_eq!(day_2.date, NaiveDate::from_ymd_opt(2026, 4, 17).unwrap());
        let haiku_day_2 = day_2.per_model.get(ModelType::Haiku);
        assert!((haiku_day_2.input - HAIKU_INPUT_PER_MILLION).abs() < 1e-9);
        assert!((haiku_day_2.output - HAIKU_OUTPUT_PER_MILLION).abs() < 1e-9);
    }

    #[tokio::test]
    async fn analyze_directory_tokens_uses_raw_usage_counts() {
        let dir = TempDir::new().unwrap();
        let session = "019dc252-e50e-766c-8182-d654b46881af";
        write_log(
            dir.path(),
            "tokens.jsonl",
            &[
                assistant_line(
                    session,
                    timestamp(2026, 4, 16, 1, 0),
                    "claude-sonnet-4-20250514",
                    "req-token-1",
                    usage_json(1_234, 5_678, 90, 12),
                ),
                assistant_line(
                    session,
                    timestamp(2026, 4, 16, 2, 0),
                    "claude-sonnet-4-20250514",
                    "req-token-2",
                    usage_json(10, 20, 30, 40),
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
            .get_tokens(ModelType::Sonnet);
        assert_eq!(costs.input, 1_244);
        assert_eq!(costs.output, 5_698);
        assert_eq!(costs.cache_write, 120);
        assert_eq!(costs.cache_read, 52);
        assert_eq!(costs.total(), 7_114);
    }

    #[tokio::test]
    async fn analyze_directory_applies_time_filter_after_dedup() {
        let dir = TempDir::new().unwrap();
        let session = "019dc252-e50e-766c-8182-d654b46881af";
        write_filter_window_fixture(dir.path(), "all.jsonl", session, session);

        let result = analyze_directory(
            dir.path(),
            DateTimezone::Utc,
            &april_16_only_filter(),
            ReportMode::Cost,
        )
        .await
        .unwrap();

        assert_eq!(result.daily_metrics.len(), 1);
        let kept = &result.daily_metrics[0];
        assert_eq!(kept.date, NaiveDate::from_ymd_opt(2026, 4, 16).unwrap());
        // Only the in-window line should contribute its output cost; the
        // out-of-window line's input cost must not appear.
        assert_only_in_window_sonnet_output(&kept.per_model);
    }

    /// Locks in the dedup-then-filter ordering: when two lines share a
    /// `request_id`, `cost_analyzer` collapses them to the higher-cost
    /// winner *before* `moriarty` applies its time filter. If the order
    /// were reversed, the in-window line would survive the filter alone
    /// and produce a non-empty result here.
    #[tokio::test]
    async fn analyze_directory_dedup_runs_before_time_filter() {
        let dir = TempDir::new().unwrap();
        let session = "019dc252-e50e-766c-8182-d654b46881af";
        let shared_request_id = "req-shared";
        write_log(
            dir.path(),
            "shared.jsonl",
            &[
                // Out-of-window line: 1M output tokens (~$15 Sonnet),
                // higher than the in-window line's input cost, so dedup
                // keeps THIS line.
                assistant_line(
                    session,
                    timestamp(2026, 4, 15, 12, 0),
                    "claude-sonnet-4-20250514",
                    shared_request_id,
                    usage_json(0, 1_000_000, 0, 0),
                ),
                // In-window line: 1M input tokens (~$3 Sonnet), lower
                // cost, so dedup discards it.
                assistant_line(
                    session,
                    timestamp(2026, 4, 16, 12, 0),
                    "claude-sonnet-4-20250514",
                    shared_request_id,
                    usage_json(1_000_000, 0, 0, 0),
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

        assert!(!result.had_errors);
        assert!(
            result.daily_metrics.is_empty(),
            "dedup must keep the higher-cost out-of-window line and let the time filter \
             discard it; if filtering ran first, the in-window line would survive dedup \
             as the only candidate. Got dates: {:?}",
            result
                .daily_metrics
                .iter()
                .map(|d| d.date)
                .collect::<Vec<_>>(),
        );
    }

    #[tokio::test]
    async fn analyze_directory_by_session_groups_by_session_and_tracks_time_range() {
        let dir = TempDir::new().unwrap();
        let session_a = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let session_b = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
        write_log(
            dir.path(),
            "sessions.jsonl",
            &[
                // Session B starts later but its first kept line appears first
                // in the file; sorting should still place A before B.
                assistant_line(
                    session_b,
                    timestamp(2026, 4, 16, 12, 0),
                    "claude-sonnet-4-20250514",
                    "req-b1",
                    usage_json(1_000_000, 0, 0, 0),
                ),
                assistant_line(
                    session_a,
                    timestamp(2026, 4, 16, 9, 0),
                    "claude-sonnet-4-20250514",
                    "req-a1",
                    usage_json(1_000_000, 0, 0, 0),
                ),
                assistant_line(
                    session_a,
                    timestamp(2026, 4, 16, 10, 30),
                    "claude-sonnet-4-20250514",
                    "req-a2",
                    usage_json(0, 1_000_000, 0, 0),
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
        let costs = first.per_model.get(ModelType::Sonnet);
        assert!((costs.input - SONNET_INPUT_PER_MILLION).abs() < 1e-9);
        assert!((costs.output - SONNET_OUTPUT_PER_MILLION).abs() < 1e-9);

        let second = &result.session_metrics[1];
        assert_eq!(second.session_id, session_b);
        assert_eq!(second.start_time, timestamp(2026, 4, 16, 12, 0));
        assert_eq!(second.end_time, timestamp(2026, 4, 16, 12, 0));
    }

    #[tokio::test]
    async fn analyze_directory_by_session_tokens_group_by_session_and_preserve_time_range() {
        let dir = TempDir::new().unwrap();
        let session = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        write_log(
            dir.path(),
            "session-tokens.jsonl",
            &[
                assistant_line(
                    session,
                    timestamp(2026, 4, 16, 9, 0),
                    "claude-sonnet-4-20250514",
                    "req-token-a1",
                    usage_json(1_234, 5_678, 90, 12),
                ),
                assistant_line(
                    session,
                    timestamp(2026, 4, 16, 10, 30),
                    "claude-sonnet-4-20250514",
                    "req-token-a2",
                    usage_json(10, 20, 30, 40),
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
        let costs = session_metrics.per_model.get_tokens(ModelType::Sonnet);
        assert_eq!(costs.input, 1_244);
        assert_eq!(costs.output, 5_698);
        assert_eq!(costs.cache_write, 120);
        assert_eq!(costs.cache_read, 52);
        assert_eq!(costs.total(), 7_114);
    }

    #[tokio::test]
    async fn analyze_directory_by_session_applies_time_filter_after_dedup() {
        let dir = TempDir::new().unwrap();
        let session_in = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let session_out = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
        write_filter_window_fixture(dir.path(), "sessions.jsonl", session_out, session_in);

        let result =
            analyze_directory_by_session(dir.path(), &april_16_only_filter(), ReportMode::Cost)
                .await
                .unwrap();

        assert_eq!(result.session_metrics.len(), 1);
        assert!(!result.had_errors);
        let kept = &result.session_metrics[0];
        assert_eq!(kept.session_id, session_in);
        assert_only_in_window_sonnet_output(&kept.per_model);
    }

    #[tokio::test]
    async fn aggregation_sums_llm_cost_directly_including_cache_components() {
        // One million tokens of each kind on a single Sonnet call should
        // produce exactly the published per-million prices, summed by
        // accumulating `LineWithCost.cost` rather than re-running pricing.
        let dir = TempDir::new().unwrap();
        write_log(
            dir.path(),
            "cache.jsonl",
            &[assistant_line(
                "019dc252-e50e-766c-8182-d654b46881af",
                timestamp(2026, 4, 16, 0, 0),
                "claude-sonnet-4-20250514",
                "req-cache",
                usage_json(1_000_000, 1_000_000, 1_000_000, 1_000_000),
            )],
        );

        let result = analyze_directory(
            dir.path(),
            DateTimezone::Utc,
            &unrestricted_filter(),
            ReportMode::Cost,
        )
        .await
        .unwrap();

        let costs = result.daily_metrics[0].per_model.get(ModelType::Sonnet);
        assert!((costs.input - SONNET_INPUT_PER_MILLION).abs() < 1e-9);
        assert!((costs.output - SONNET_OUTPUT_PER_MILLION).abs() < 1e-9);
        assert!((costs.cache_write - SONNET_CACHE_WRITE_PER_MILLION).abs() < 1e-9);
        assert!((costs.cache_read - SONNET_CACHE_READ_PER_MILLION).abs() < 1e-9);
    }
}
