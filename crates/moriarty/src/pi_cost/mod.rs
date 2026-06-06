mod analyzer;
mod pricing;

use std::path::Path;

use tabled::Tabled;

use crate::cost_report::{
    build_grouped_rows, format_duration, format_session_id, format_time_range, grouped_label,
    push_nonzero_metric_rows, render_grouped_metrics, render_or_empty, render_stacked_charts,
    ChartBucket, ChartSegment, DateTimezone, FormattedMetricColumns, MetricComponents, MetricTotal,
    ReportMode, TimeRangeFilter,
};
use analyzer::{DailyMetrics, SessionMetrics};
use pi_logs::Provider;

#[derive(Tabled)]
struct PiMetricRow {
    #[tabled(rename = "Date")]
    date: String,
    #[tabled(rename = "Provider")]
    provider: String,
    #[tabled(rename = "Model")]
    model: String,
    #[tabled(inline)]
    metrics: FormattedMetricColumns,
}

impl PiMetricRow {
    fn new(date: &str, provider: &str, model: &str, metrics: impl Into<MetricComponents>) -> Self {
        Self {
            date: date.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            metrics: FormattedMetricColumns::from_metrics(metrics.into()),
        }
    }

    fn new_total_row(total: MetricTotal) -> Self {
        Self::new_labeled_total_row("", total)
    }

    fn new_labeled_total_row(date: &str, total: MetricTotal) -> Self {
        Self {
            date: date.to_string(),
            provider: String::new(),
            model: "Total".to_string(),
            metrics: FormattedMetricColumns::from_total(total),
        }
    }
}

#[derive(Tabled)]
struct PiSessionMetricRow {
    #[tabled(rename = "Session")]
    session: String,
    #[tabled(rename = "Time Range")]
    time_range: String,
    #[tabled(rename = "Duration")]
    duration: String,
    #[tabled(rename = "Provider")]
    provider: String,
    #[tabled(rename = "Model")]
    model: String,
    #[tabled(inline)]
    metrics: FormattedMetricColumns,
}

impl PiSessionMetricRow {
    fn new(
        session: &str,
        time_range: &str,
        duration: &str,
        provider: &str,
        model: &str,
        metrics: impl Into<MetricComponents>,
    ) -> Self {
        Self {
            session: session.to_string(),
            time_range: time_range.to_string(),
            duration: duration.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            metrics: FormattedMetricColumns::from_metrics(metrics.into()),
        }
    }

    fn new_total_row(total: MetricTotal) -> Self {
        Self::new_labeled_total_row("", "", "", total)
    }

    fn new_labeled_total_row(
        session: &str,
        time_range: &str,
        duration: &str,
        total: MetricTotal,
    ) -> Self {
        Self {
            session: session.to_string(),
            time_range: time_range.to_string(),
            duration: duration.to_string(),
            provider: String::new(),
            model: "Total".to_string(),
            metrics: FormattedMetricColumns::from_total(total),
        }
    }
}

fn provider_label(provider: Provider) -> &'static str {
    match provider {
        Provider::Anthropic => "Anthropic",
        Provider::OpenAi => "OpenAI",
        Provider::OpenAiCodex => "OpenAI Codex",
        Provider::OpenRouter => "OpenRouter",
        Provider::Faux => "Faux",
    }
}

fn daily_title(report_mode: ReportMode) -> &'static str {
    match report_mode {
        ReportMode::Cost => "Pi Cost Report",
        ReportMode::Tokens => "Pi Token Report",
    }
}

fn session_title(report_mode: ReportMode) -> &'static str {
    match report_mode {
        ReportMode::Cost => "Pi Cost Report by Conversation",
        ReportMode::Tokens => "Pi Token Report by Conversation",
    }
}

fn graph_title(report_mode: ReportMode, by_conversation: bool) -> &'static str {
    match (report_mode, by_conversation) {
        (ReportMode::Cost, false) => "Pi Cost Graphs",
        (ReportMode::Tokens, false) => "Pi Token Graphs",
        (ReportMode::Cost, true) => "Pi Cost Graphs by Conversation",
        (ReportMode::Tokens, true) => "Pi Token Graphs by Conversation",
    }
}

fn time_series_chart_title(report_mode: ReportMode, by_conversation: bool) -> &'static str {
    match (report_mode, by_conversation) {
        (ReportMode::Cost, false) => "Daily total cost by provider/model",
        (ReportMode::Tokens, false) => "Daily total tokens by provider/model",
        (ReportMode::Cost, true) => "Conversation total cost by provider/model",
        (ReportMode::Tokens, true) => "Conversation total tokens by provider/model",
    }
}

fn share_chart_title(report_mode: ReportMode) -> &'static str {
    match report_mode {
        ReportMode::Cost => "Cost share by provider/model",
        ReportMode::Tokens => "Token share by provider/model",
    }
}

fn segment_label(provider: Provider, model: &str) -> String {
    format!("{} / {}", provider_label(provider), model)
}

fn build_daily_chart_buckets(daily_metrics: &[DailyMetrics]) -> Vec<ChartBucket> {
    daily_metrics
        .iter()
        .map(|metrics| ChartBucket {
            label: metrics.date.to_string(),
            segments: metrics
                .per_model
                .model_metrics()
                .map(|(model, metric_components)| ChartSegment {
                    label: segment_label(model.provider, &model.model),
                    total: metric_components.total(),
                })
                .collect(),
        })
        .collect()
}

fn build_session_chart_buckets(session_metrics: &[SessionMetrics]) -> Vec<ChartBucket> {
    session_metrics
        .iter()
        .map(|metrics| ChartBucket {
            label: format_session_id(&metrics.session_id),
            segments: metrics
                .per_model
                .model_metrics()
                .map(|(model, metric_components)| ChartSegment {
                    label: segment_label(model.provider, &model.model),
                    total: metric_components.total(),
                })
                .collect(),
        })
        .collect()
}

fn build_daily_rows(
    daily_metrics: &[DailyMetrics],
    report_mode: ReportMode,
) -> miette::Result<(Vec<PiMetricRow>, Vec<usize>)> {
    build_grouped_rows(
        daily_metrics,
        |rows, metrics| {
            let date_str = metrics.date.to_string();
            push_nonzero_metric_rows(
                rows,
                metrics
                    .per_model
                    .model_metrics()
                    .map(|(model, metric_components)| {
                        (
                            (provider_label(model.provider), model.model.as_str()),
                            *metric_components,
                        )
                    }),
                |first_row, (provider, model), metric_components| {
                    PiMetricRow::new(
                        grouped_label(first_row, &date_str),
                        provider,
                        model,
                        metric_components,
                    )
                },
            );
            Ok(())
        },
        |rows, metrics, has_detail_rows| {
            rows.push(if has_detail_rows {
                PiMetricRow::new_total_row(metrics.total(report_mode)?)
            } else {
                PiMetricRow::new_labeled_total_row(
                    &metrics.date.to_string(),
                    metrics.total(report_mode)?,
                )
            });
            Ok(())
        },
    )
}

fn build_session_rows(
    session_metrics: &[SessionMetrics],
    timezone: DateTimezone,
    report_mode: ReportMode,
) -> miette::Result<(Vec<PiSessionMetricRow>, Vec<usize>)> {
    build_grouped_rows(
        session_metrics,
        |rows, metrics| {
            let session_id = format_session_id(&metrics.session_id);
            let time_range = format_time_range(timezone, metrics.start_time, metrics.end_time);
            let duration = format_duration(metrics.duration_minutes());
            push_nonzero_metric_rows(
                rows,
                metrics
                    .per_model
                    .model_metrics()
                    .map(|(model, metric_components)| {
                        (
                            (provider_label(model.provider), model.model.as_str()),
                            *metric_components,
                        )
                    }),
                |first_row, (provider, model), metric_components| {
                    PiSessionMetricRow::new(
                        grouped_label(first_row, &session_id),
                        grouped_label(first_row, &time_range),
                        grouped_label(first_row, &duration),
                        provider,
                        model,
                        metric_components,
                    )
                },
            );
            Ok(())
        },
        |rows, metrics, has_detail_rows| {
            rows.push(if has_detail_rows {
                PiSessionMetricRow::new_total_row(metrics.total(report_mode)?)
            } else {
                PiSessionMetricRow::new_labeled_total_row(
                    &format_session_id(&metrics.session_id),
                    &format_time_range(timezone, metrics.start_time, metrics.end_time),
                    &format_duration(metrics.duration_minutes()),
                    metrics.total(report_mode)?,
                )
            });
            Ok(())
        },
    )
}

fn display_daily_metrics(
    daily_metrics: &[DailyMetrics],
    report_mode: ReportMode,
) -> miette::Result<()> {
    render_grouped_metrics(
        daily_title(report_mode),
        daily_metrics,
        report_mode,
        |items| build_daily_rows(items, report_mode),
        DailyMetrics::total,
    )
}

fn display_session_metrics(
    session_metrics: &[SessionMetrics],
    timezone: DateTimezone,
    report_mode: ReportMode,
) -> miette::Result<()> {
    render_grouped_metrics(
        session_title(report_mode),
        session_metrics,
        report_mode,
        |items| build_session_rows(items, timezone, report_mode),
        SessionMetrics::total,
    )
}

fn display_daily_graphs(
    daily_metrics: &[DailyMetrics],
    report_mode: ReportMode,
) -> miette::Result<()> {
    render_stacked_charts(
        graph_title(report_mode, false),
        time_series_chart_title(report_mode, false),
        share_chart_title(report_mode),
        &build_daily_chart_buckets(daily_metrics),
        report_mode,
    )
}

fn display_session_graphs(
    session_metrics: &[SessionMetrics],
    report_mode: ReportMode,
) -> miette::Result<()> {
    render_stacked_charts(
        graph_title(report_mode, true),
        time_series_chart_title(report_mode, true),
        share_chart_title(report_mode),
        &build_session_chart_buckets(session_metrics),
        report_mode,
    )
}

pub async fn run_by_session(
    dir: &Path,
    timezone: DateTimezone,
    filter: &TimeRangeFilter,
    report_mode: ReportMode,
) -> miette::Result<()> {
    let result = analyzer::analyze_directory_by_session(dir, filter, report_mode).await?;
    render_or_empty(&result.session_metrics, result.had_errors, |items| {
        display_session_metrics(items, timezone, report_mode)
    })
}

pub async fn run_graphs_by_session(
    dir: &Path,
    filter: &TimeRangeFilter,
    report_mode: ReportMode,
) -> miette::Result<()> {
    let result = analyzer::analyze_directory_by_session(dir, filter, report_mode).await?;
    render_or_empty(&result.session_metrics, result.had_errors, |items| {
        display_session_graphs(items, report_mode)
    })
}

pub async fn run(
    dir: &Path,
    timezone: DateTimezone,
    by_conversation: bool,
    filter: &TimeRangeFilter,
    report_mode: ReportMode,
) -> miette::Result<()> {
    if by_conversation {
        return run_by_session(dir, timezone, filter, report_mode).await;
    }

    let result = analyzer::analyze_directory(dir, timezone, filter, report_mode).await?;
    render_or_empty(&result.daily_metrics, result.had_errors, |items| {
        display_daily_metrics(items, report_mode)
    })
}

pub async fn run_graphs(
    dir: &Path,
    timezone: DateTimezone,
    by_conversation: bool,
    filter: &TimeRangeFilter,
    report_mode: ReportMode,
) -> miette::Result<()> {
    if by_conversation {
        return run_graphs_by_session(dir, filter, report_mode).await;
    }

    let result = analyzer::analyze_directory(dir, timezone, filter, report_mode).await?;
    render_or_empty(&result.daily_metrics, result.had_errors, |items| {
        display_daily_graphs(items, report_mode)
    })
}

#[cfg(test)]
type PiCostRow = PiMetricRow;
#[cfg(test)]
type PiSessionCostRow = PiSessionMetricRow;

#[cfg(test)]
fn build_cost_rows(
    daily_costs: &[DailyMetrics],
    report_mode: ReportMode,
) -> (Vec<PiCostRow>, Vec<usize>) {
    build_daily_rows(daily_costs, report_mode).expect("build daily rows")
}

#[cfg(test)]
fn build_session_cost_rows(
    session_costs: &[SessionMetrics],
    timezone: DateTimezone,
    report_mode: ReportMode,
) -> (Vec<PiSessionCostRow>, Vec<usize>) {
    build_session_rows(session_costs, timezone, report_mode).expect("build session rows")
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, TimeZone, Utc};

    use super::*;
    use crate::{
        cost_report::{
            fmt_money, ComponentTotals, FormattedCostColumns, MetricComponents, ReportMode,
            TokenCounts,
        },
        pi_cost::{
            analyzer::{DailyCosts, SessionCosts},
            pricing::PiModelCostsMap,
        },
    };
    use cost_analyzer::PiModel;

    fn test_date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn costs_on(year: i32, month: u32, day: u32) -> DailyCosts {
        DailyCosts {
            date: test_date(year, month, day),
            per_model: PiModelCostsMap::default(),
        }
    }

    trait DailyCostsExt {
        fn with_model(
            self,
            provider: Provider,
            model: &str,
            input: f64,
            output: f64,
            cache_write: f64,
            cache_read: f64,
        ) -> Self;
    }

    impl DailyCostsExt for DailyCosts {
        fn with_model(
            mut self,
            provider: Provider,
            model: &str,
            input: f64,
            output: f64,
            cache_write: f64,
            cache_read: f64,
        ) -> Self {
            self.per_model
                .add(
                    PiModel {
                        provider,
                        model: model.to_string(),
                    },
                    ComponentTotals::new(input, output, cache_write, cache_read),
                )
                .unwrap();
            self
        }
    }

    fn session_costs_fixture(session_id: &str) -> SessionCosts {
        let start = Utc.with_ymd_and_hms(2025, 10, 23, 9, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2025, 10, 23, 10, 30, 0).unwrap();
        let mut per_model = PiModelCostsMap::default();
        per_model
            .add(
                PiModel {
                    provider: Provider::Anthropic,
                    model: "claude-sonnet-4-5".to_string(),
                },
                ComponentTotals::new(1.0, 2.0, 0.0, 0.0),
            )
            .unwrap();
        SessionCosts {
            session_id: session_id.to_string(),
            start_time: start,
            end_time: end,
            per_model,
        }
    }

    fn assert_money_columns(money: &FormattedCostColumns, components: (f64, f64, f64, f64)) {
        let (input, output, cache_write, cache_read) = components;
        assert_eq!(money.input, fmt_money(input));
        assert_eq!(money.output, fmt_money(output));
        assert_eq!(money.cache_write, fmt_money(cache_write));
        assert_eq!(money.cache_read, fmt_money(cache_read));
        assert_eq!(
            money.subtotal,
            fmt_money(input + output + cache_write + cache_read)
        );
    }

    fn assert_blank_money_component_columns(money: &FormattedCostColumns) {
        assert_eq!(money.input, "");
        assert_eq!(money.output, "");
        assert_eq!(money.cache_write, "");
        assert_eq!(money.cache_read, "");
    }

    #[test]
    fn pi_cost_row_formats_provider_and_model_columns() {
        let row = PiCostRow::new(
            "2025-10-23",
            "Anthropic",
            "claude-sonnet-4-5",
            ComponentTotals::new(1.25, 2.5, 0.5, 0.25),
        );

        assert_eq!(row.date, "2025-10-23");
        assert_eq!(row.provider, "Anthropic");
        assert_eq!(row.model, "claude-sonnet-4-5");
        assert_money_columns(&row.metrics, (1.25, 2.5, 0.5, 0.25));
    }

    #[test]
    fn pi_cost_row_total_uses_blank_component_columns() {
        let row = PiCostRow::new_total_row(MetricTotal::Cost(7.5));

        assert_eq!(row.date, "");
        assert_eq!(row.provider, "");
        assert_eq!(row.model, "Total");
        assert_blank_money_component_columns(&row.metrics);
        assert_eq!(row.metrics.subtotal, "$7.5000");
    }

    #[test]
    fn pi_cost_row_formats_token_columns() {
        let row = PiCostRow::new(
            "2025-10-23",
            "Anthropic",
            "claude-sonnet-4-5",
            MetricComponents::Tokens(TokenCounts::new(1_234, 5_678, 90, 12)),
        );

        assert_eq!(row.metrics.input, "1,234");
        assert_eq!(row.metrics.output, "5,678");
        assert_eq!(row.metrics.cache_write, "90");
        assert_eq!(row.metrics.cache_read, "12");
        assert_eq!(row.metrics.subtotal, "7,014");
    }

    #[test]
    fn build_cost_rows_preserves_provider_then_model_order() {
        let daily_costs = vec![costs_on(2025, 10, 23)
            .with_model(Provider::OpenAi, "gpt-5", 1.0, 0.0, 0.0, 0.0)
            .with_model(Provider::Anthropic, "claude-sonnet-4-5", 2.0, 0.0, 0.0, 0.0)
            .with_model(Provider::Anthropic, "claude-haiku-3-5", 0.5, 0.0, 0.0, 0.0)];

        let (rows, total_row_indices) = build_cost_rows(&daily_costs, ReportMode::Cost);

        assert_eq!(total_row_indices, vec![3]);
        assert_eq!(rows[0].provider, "Anthropic");
        assert_eq!(rows[0].model, "claude-haiku-3-5");
        assert_eq!(rows[1].provider, "Anthropic");
        assert_eq!(rows[1].model, "claude-sonnet-4-5");
        assert_eq!(rows[2].provider, "OpenAI");
        assert_eq!(rows[2].model, "gpt-5");
        assert_eq!(rows[3].model, "Total");
    }

    #[test]
    fn build_cost_rows_zero_cost_day_still_gets_labeled_total_row() {
        let (rows, total_row_indices) =
            build_cost_rows(&[costs_on(2025, 10, 23)], ReportMode::Cost);

        assert_eq!(total_row_indices, vec![0]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].date, "2025-10-23");
        assert_eq!(rows[0].provider, "");
        assert_eq!(rows[0].model, "Total");
        assert_eq!(rows[0].metrics.subtotal, "$0.0000");
    }

    #[test]
    fn pi_session_cost_row_total_uses_blank_component_columns() {
        let row = PiSessionCostRow::new_total_row(MetricTotal::Cost(4.0));

        assert_eq!(row.session, "");
        assert_eq!(row.time_range, "");
        assert_eq!(row.duration, "");
        assert_eq!(row.provider, "");
        assert_eq!(row.model, "Total");
        assert_blank_money_component_columns(&row.metrics);
        assert_eq!(row.metrics.subtotal, "$4.0000");
    }

    #[test]
    fn build_session_cost_rows_zero_cost_session_keeps_identifying_columns() {
        let session = SessionCosts {
            session_id: "ééééééééé-session".to_string(),
            start_time: Utc.with_ymd_and_hms(2025, 10, 23, 9, 0, 0).unwrap(),
            end_time: Utc.with_ymd_and_hms(2025, 10, 23, 9, 0, 0).unwrap(),
            per_model: PiModelCostsMap::default(),
        };

        let (rows, total_row_indices) =
            build_session_cost_rows(&[session], DateTimezone::Utc, ReportMode::Cost);

        assert_eq!(total_row_indices, vec![0]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].session, "éééééééé");
        assert_eq!(rows[0].time_range, "2025-10-23 09:00 → 09:00");
        assert_eq!(rows[0].duration, "0 min");
        assert_eq!(rows[0].provider, "");
        assert_eq!(rows[0].model, "Total");
    }

    #[test]
    fn build_session_cost_rows_only_first_row_repeats_identifying_columns() {
        let mut session = session_costs_fixture("019dc252-e50e-766c");
        session
            .per_model
            .add(
                PiModel {
                    provider: Provider::OpenAi,
                    model: "gpt-5".to_string(),
                },
                ComponentTotals::new(0.5, 0.5, 0.0, 0.0),
            )
            .unwrap();

        let (rows, total_row_indices) =
            build_session_cost_rows(&[session], DateTimezone::Utc, ReportMode::Cost);

        assert_eq!(total_row_indices, vec![2]);
        assert_eq!(rows.len(), 3);

        assert_eq!(rows[0].session, "019dc252");
        assert!(!rows[0].time_range.is_empty());
        assert_eq!(rows[0].duration, "1 hr 30 min");

        assert_eq!(rows[1].session, "");
        assert_eq!(rows[1].time_range, "");
        assert_eq!(rows[1].duration, "");
        assert_eq!(rows[2].model, "Total");
    }
}
