use std::{collections::BTreeMap, path::Path};

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::prelude::ToPrimitive;

use super::pricing::{PiModelCostsMap, TokenCosts};
use crate::cost_report::{DateTimezone, TimeRangeFilter};
use cost_analyzer::{analyze_directory as cost_analyze_directory, LineWithCost, LlmCost};
use pi_logs::PiLogLine;

#[derive(Debug, Default)]
pub struct AnalysisResult {
    pub daily_costs: Vec<DailyCosts>,
    pub had_errors: bool,
}

#[derive(Debug, Default)]
pub struct SessionAnalysisResult {
    pub session_costs: Vec<SessionCosts>,
    pub had_errors: bool,
}

#[derive(Debug)]
pub struct DailyCosts {
    pub date: NaiveDate,
    pub per_model: PiModelCostsMap,
}

impl DailyCosts {
    pub fn total(&self) -> f64 {
        self.per_model.total()
    }
}

#[derive(Debug)]
pub struct SessionCosts {
    pub session_id: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub per_model: PiModelCostsMap,
}

impl SessionCosts {
    pub fn total(&self) -> f64 {
        self.per_model.total()
    }

    pub fn duration_minutes(&self) -> i64 {
        (self.end_time - self.start_time).num_minutes()
    }
}

/// Pi logs already carry money totals, so the Moriarty-side conversion only
/// bridges `cost_analyzer`'s Decimal values into the table layer's `f64`
/// formatting helpers.
fn token_costs_from_llm_cost(cost: &LlmCost) -> TokenCosts {
    TokenCosts::new(
        cost.input.to_f64().unwrap_or(0.0),
        cost.output.to_f64().unwrap_or(0.0),
        cost.cache_write.to_f64().unwrap_or(0.0),
        cost.cache_read.to_f64().unwrap_or(0.0),
    )
}

async fn load_billable_lines(
    dir: &Path,
    filter: &TimeRangeFilter,
) -> (Vec<(LineWithCost<PiLogLine>, TokenCosts)>, bool) {
    let result = cost_analyze_directory::<PiLogLine>(dir.to_path_buf()).await;
    let entries = result
        .lines
        .into_iter()
        .filter_map(|line| billable_entry(&line, filter).map(|costs| (line, costs)))
        .collect();
    (entries, result.had_errors)
}

fn billable_entry(line: &LineWithCost<PiLogLine>, filter: &TimeRangeFilter) -> Option<TokenCosts> {
    if !filter.contains(&line.timestamp) {
        return None;
    }
    Some(token_costs_from_llm_cost(&line.cost))
}

pub async fn analyze_directory(
    dir: &Path,
    timezone: DateTimezone,
    filter: &TimeRangeFilter,
) -> miette::Result<AnalysisResult> {
    let (entries, had_errors) = load_billable_lines(dir, filter).await;

    let mut buckets: BTreeMap<NaiveDate, PiModelCostsMap> = BTreeMap::new();

    for (line, costs) in entries {
        let date = timezone.to_date(&line.timestamp);
        buckets
            .entry(date)
            .or_default()
            .add(line.model.clone(), costs);
    }

    let daily_costs = buckets
        .into_iter()
        .map(|(date, per_model)| DailyCosts { date, per_model })
        .collect();

    Ok(AnalysisResult {
        daily_costs,
        had_errors,
    })
}

pub async fn analyze_directory_by_session(
    dir: &Path,
    filter: &TimeRangeFilter,
) -> miette::Result<SessionAnalysisResult> {
    struct SessionAccumulator {
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
        per_model: PiModelCostsMap,
    }

    let (entries, had_errors) = load_billable_lines(dir, filter).await;
    let mut buckets: BTreeMap<String, SessionAccumulator> = BTreeMap::new();

    for (line, costs) in entries {
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
                per_model: PiModelCostsMap::default(),
            });

        if line.timestamp < acc.start_time {
            acc.start_time = line.timestamp;
        }
        if line.timestamp > acc.end_time {
            acc.end_time = line.timestamp;
        }
        acc.per_model.add(line.model.clone(), costs);
    }

    let mut session_costs: Vec<SessionCosts> = buckets
        .into_iter()
        .map(|(session_id, acc)| SessionCosts {
            session_id,
            start_time: acc.start_time,
            end_time: acc.end_time,
            per_model: acc.per_model,
        })
        .collect();

    session_costs.sort_by_key(|session| session.start_time);

    Ok(SessionAnalysisResult {
        session_costs,
        had_errors,
    })
}

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
                    "input": 10,
                    "output": 5,
                    "cacheRead": 2,
                    "cacheWrite": 1,
                    "totalTokens": 18,
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

    fn model_cost<'a>(daily: &'a DailyCosts, provider: Provider, model: &str) -> &'a TokenCosts {
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
    ) -> &'a TokenCosts {
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

        let result = analyze_directory(dir.path(), DateTimezone::Utc, &unrestricted_filter())
            .await
            .unwrap();

        assert!(result.daily_costs.is_empty());
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

        let result = analyze_directory(dir.path(), DateTimezone::Utc, &unrestricted_filter())
            .await
            .unwrap();

        assert_eq!(result.daily_costs.len(), 1);
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

        let result = analyze_directory(dir.path(), DateTimezone::Utc, &unrestricted_filter())
            .await
            .unwrap();

        assert_eq!(result.daily_costs.len(), 2);
        assert!(!result.had_errors);

        let day_1 = &result.daily_costs[0];
        assert_eq!(day_1.date, NaiveDate::from_ymd_opt(2026, 4, 16).unwrap());
        let sonnet = model_cost(day_1, Provider::Anthropic, "claude-sonnet-4-5");
        assert!((sonnet.input - 1.5).abs() < 1e-9);
        assert!((sonnet.output - 2.5).abs() < 1e-9);
        let gpt = model_cost(day_1, Provider::OpenAi, "gpt-5");
        assert!((gpt.input - 0.5).abs() < 1e-9);
        assert!((gpt.output - 1.0).abs() < 1e-9);

        let day_2 = &result.daily_costs[1];
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

        let result = analyze_directory_by_session(dir.path(), &unrestricted_filter())
            .await
            .unwrap();

        assert_eq!(result.session_costs.len(), 2);
        assert!(!result.had_errors);

        let first = &result.session_costs[0];
        assert_eq!(first.session_id, session_a);
        assert_eq!(first.start_time, timestamp(2026, 4, 16, 9, 0));
        assert_eq!(first.end_time, timestamp(2026, 4, 16, 10, 30));
        assert_eq!(first.duration_minutes(), 90);
        assert_eq!(first.per_model.len(), 1);

        let second = &result.session_costs[1];
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

        let result = analyze_directory_by_session(dir.path(), &unrestricted_filter())
            .await
            .unwrap();

        assert_eq!(result.session_costs.len(), 1);
        let merged = &result.session_costs[0];
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

        let result = analyze_directory(dir.path(), DateTimezone::Utc, &april_16_only_filter())
            .await
            .unwrap();

        assert!(result.daily_costs.is_empty());
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

        let result = analyze_directory_by_session(dir.path(), &april_16_only_filter())
            .await
            .unwrap();

        assert!(result.session_costs.is_empty());
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

        let result = analyze_directory(dir.path(), DateTimezone::Utc, &april_16_only_filter())
            .await
            .unwrap();

        assert_eq!(result.daily_costs.len(), 1);
        let day = &result.daily_costs[0];
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

        let result = analyze_directory_by_session(dir.path(), &april_16_only_filter())
            .await
            .unwrap();

        assert_eq!(result.session_costs.len(), 1);
        let kept_session = &result.session_costs[0];
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

        let result = analyze_directory_by_session(dir.path(), &unrestricted_filter())
            .await
            .unwrap();

        assert_eq!(result.session_costs.len(), 1);
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

        let result = analyze_directory(dir.path(), DateTimezone::Utc, &unrestricted_filter())
            .await
            .unwrap();

        let costs = model_cost(
            &result.daily_costs[0],
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

        let result = analyze_directory(dir.path(), DateTimezone::Utc, &unrestricted_filter())
            .await
            .unwrap();

        let bucket = &result.daily_costs[0];
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

        let error = analyze_directory_by_session(dir.path(), &unrestricted_filter())
            .await
            .unwrap_err();

        assert!(
            error.to_string().contains("missing a session id"),
            "unexpected error: {error}"
        );
    }
}
