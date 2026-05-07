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

#[derive(Tabled)]
struct ApiMetricRow {
    #[tabled(rename = "Date")]
    date: String,
    #[tabled(rename = "Model")]
    model: String,
    #[tabled(inline)]
    metrics: FormattedMetricColumns,
}

impl ApiMetricRow {
    fn new(date: &str, model: &str, metrics: impl Into<MetricComponents>) -> Self {
        Self {
            date: date.to_string(),
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
            model: "Total".to_string(),
            metrics: FormattedMetricColumns::from_total(total),
        }
    }
}

#[derive(Tabled)]
struct ApiSessionMetricRow {
    #[tabled(rename = "Session")]
    session: String,
    #[tabled(rename = "Time Range")]
    time_range: String,
    #[tabled(rename = "Duration")]
    duration: String,
    #[tabled(rename = "Model")]
    model: String,
    #[tabled(inline)]
    metrics: FormattedMetricColumns,
}

impl ApiSessionMetricRow {
    fn new(
        session: &str,
        time_range: &str,
        duration: &str,
        model: &str,
        metrics: impl Into<MetricComponents>,
    ) -> Self {
        Self {
            session: session.to_string(),
            time_range: time_range.to_string(),
            duration: duration.to_string(),
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
            model: "Total".to_string(),
            metrics: FormattedMetricColumns::from_total(total),
        }
    }
}

fn daily_title(report_mode: ReportMode) -> &'static str {
    match report_mode {
        ReportMode::Cost => "API Cost Report",
        ReportMode::Tokens => "API Token Report",
    }
}

fn session_title(report_mode: ReportMode) -> &'static str {
    match report_mode {
        ReportMode::Cost => "API Cost Report by Conversation",
        ReportMode::Tokens => "API Token Report by Conversation",
    }
}

fn graph_title(report_mode: ReportMode, by_conversation: bool) -> &'static str {
    match (report_mode, by_conversation) {
        (ReportMode::Cost, false) => "API Cost Graphs",
        (ReportMode::Tokens, false) => "API Token Graphs",
        (ReportMode::Cost, true) => "API Cost Graphs by Conversation",
        (ReportMode::Tokens, true) => "API Token Graphs by Conversation",
    }
}

fn time_series_chart_title(report_mode: ReportMode, by_conversation: bool) -> &'static str {
    match (report_mode, by_conversation) {
        (ReportMode::Cost, false) => "Daily total cost by model",
        (ReportMode::Tokens, false) => "Daily total tokens by model",
        (ReportMode::Cost, true) => "Conversation total cost by model",
        (ReportMode::Tokens, true) => "Conversation total tokens by model",
    }
}

fn share_chart_title(report_mode: ReportMode) -> &'static str {
    match report_mode {
        ReportMode::Cost => "Cost share by model",
        ReportMode::Tokens => "Token share by model",
    }
}

fn build_daily_chart_buckets(
    daily_metrics: &[DailyMetrics],
    report_mode: ReportMode,
) -> Vec<ChartBucket> {
    daily_metrics
        .iter()
        .map(|metrics| ChartBucket {
            label: metrics.date.to_string(),
            segments: metrics
                .per_model
                .model_metrics(report_mode)
                .into_iter()
                .map(|(model_name, metric_components)| ChartSegment {
                    label: model_name.to_string(),
                    total: metric_components.total(),
                })
                .collect(),
        })
        .collect()
}

fn build_session_chart_buckets(
    session_metrics: &[SessionMetrics],
    report_mode: ReportMode,
) -> Vec<ChartBucket> {
    session_metrics
        .iter()
        .map(|metrics| ChartBucket {
            label: format_session_id(&metrics.session_id),
            segments: metrics
                .per_model
                .model_metrics(report_mode)
                .into_iter()
                .map(|(model_name, metric_components)| ChartSegment {
                    label: model_name.to_string(),
                    total: metric_components.total(),
                })
                .collect(),
        })
        .collect()
}

fn build_daily_rows(
    daily_metrics: &[DailyMetrics],
    report_mode: ReportMode,
) -> miette::Result<(Vec<ApiMetricRow>, Vec<usize>)> {
    build_grouped_rows(
        daily_metrics,
        |rows, metrics| {
            let date_str = metrics.date.to_string();
            push_nonzero_metric_rows(
                rows,
                metrics.per_model.model_metrics(report_mode).into_iter(),
                |first_row, model_name, metric_components| {
                    ApiMetricRow::new(
                        grouped_label(first_row, &date_str),
                        model_name,
                        metric_components,
                    )
                },
            );
            Ok(())
        },
        |rows, metrics, has_detail_rows| {
            rows.push(if has_detail_rows {
                ApiMetricRow::new_total_row(metrics.total(report_mode)?)
            } else {
                ApiMetricRow::new_labeled_total_row(
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
) -> miette::Result<(Vec<ApiSessionMetricRow>, Vec<usize>)> {
    build_grouped_rows(
        session_metrics,
        |rows, metrics| {
            let session_id = format_session_id(&metrics.session_id);
            let time_range = format_time_range(timezone, metrics.start_time, metrics.end_time);
            let duration = format_duration(metrics.duration_minutes());
            push_nonzero_metric_rows(
                rows,
                metrics.per_model.model_metrics(report_mode).into_iter(),
                |first_row, model_name, metric_components| {
                    ApiSessionMetricRow::new(
                        grouped_label(first_row, &session_id),
                        grouped_label(first_row, &time_range),
                        grouped_label(first_row, &duration),
                        model_name,
                        metric_components,
                    )
                },
            );
            Ok(())
        },
        |rows, metrics, has_detail_rows| {
            rows.push(if has_detail_rows {
                ApiSessionMetricRow::new_total_row(metrics.total(report_mode)?)
            } else {
                ApiSessionMetricRow::new_labeled_total_row(
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
        &build_daily_chart_buckets(daily_metrics, report_mode),
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
        &build_session_chart_buckets(session_metrics, report_mode),
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
type CostRow = ApiMetricRow;
#[cfg(test)]
type SessionCostRow = ApiSessionMetricRow;

#[cfg(test)]
fn build_cost_rows(
    daily_costs: &[DailyMetrics],
    report_mode: ReportMode,
) -> (Vec<CostRow>, Vec<usize>) {
    build_daily_rows(daily_costs, report_mode).expect("build daily rows")
}

#[cfg(test)]
fn build_session_cost_rows(
    session_costs: &[SessionMetrics],
    timezone: DateTimezone,
    report_mode: ReportMode,
) -> (Vec<SessionCostRow>, Vec<usize>) {
    build_session_rows(session_costs, timezone, report_mode).expect("build session rows")
}

#[cfg(test)]
fn display_costs(daily_costs: &[DailyMetrics], report_mode: ReportMode) {
    display_daily_metrics(daily_costs, report_mode).expect("display daily metrics")
}

#[cfg(test)]
mod tests;
