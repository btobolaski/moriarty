mod analyzer;
mod pricing;

use std::path::Path;

use tabled::Tabled;

use crate::cost_report::{
    build_grouped_rows, format_duration, format_session_id, format_time_range, grouped_label,
    push_nonzero_metric_rows, render_grouped_metrics, render_or_empty, DateTimezone,
    FormattedMetricColumns, MetricComponents, MetricTotal, ReportMode, TimeRangeFilter,
};
#[cfg(test)]
use crate::cost_report::{CostComponents, TokenCounts};
use analyzer::{DailyMetrics, SessionMetrics};

trait IntoMetricComponentsForMode {
    fn into_metric_components(self, report_mode: ReportMode) -> MetricComponents;
}

trait IntoMetricTotalForMode {
    fn into_metric_total(self, report_mode: ReportMode) -> MetricTotal;
}

impl IntoMetricComponentsForMode for MetricComponents {
    fn into_metric_components(self, _report_mode: ReportMode) -> MetricComponents {
        self
    }
}

#[cfg(test)]
impl IntoMetricComponentsForMode for CostComponents {
    fn into_metric_components(self, report_mode: ReportMode) -> MetricComponents {
        match report_mode {
            ReportMode::Cost => MetricComponents::Cost(self),
            ReportMode::Tokens => MetricComponents::Tokens(TokenCounts::new(
                self.input.round() as u64,
                self.output.round() as u64,
                self.cache_write.round() as u64,
                self.cache_read.round() as u64,
            )),
        }
    }
}

impl IntoMetricTotalForMode for MetricTotal {
    fn into_metric_total(self, _report_mode: ReportMode) -> MetricTotal {
        self
    }
}

#[cfg(test)]
impl IntoMetricTotalForMode for f64 {
    fn into_metric_total(self, report_mode: ReportMode) -> MetricTotal {
        match report_mode {
            ReportMode::Cost => MetricTotal::Cost(self),
            ReportMode::Tokens => MetricTotal::Tokens(self.round() as u128),
        }
    }
}

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
    fn new(
        date: &str,
        model: &str,
        metrics: impl IntoMetricComponentsForMode,
        report_mode: ReportMode,
    ) -> Self {
        Self {
            date: date.to_string(),
            model: model.to_string(),
            metrics: FormattedMetricColumns::from_metrics(
                metrics.into_metric_components(report_mode),
            ),
        }
    }

    fn new_total_row(total: impl IntoMetricTotalForMode, report_mode: ReportMode) -> Self {
        Self::new_labeled_total_row("", total, report_mode)
    }

    fn new_labeled_total_row(
        date: &str,
        total: impl IntoMetricTotalForMode,
        report_mode: ReportMode,
    ) -> Self {
        Self {
            date: date.to_string(),
            model: "Total".to_string(),
            metrics: FormattedMetricColumns::from_total(total.into_metric_total(report_mode)),
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
        metrics: impl IntoMetricComponentsForMode,
        report_mode: ReportMode,
    ) -> Self {
        Self {
            session: session.to_string(),
            time_range: time_range.to_string(),
            duration: duration.to_string(),
            model: model.to_string(),
            metrics: FormattedMetricColumns::from_metrics(
                metrics.into_metric_components(report_mode),
            ),
        }
    }

    fn new_total_row(total: impl IntoMetricTotalForMode, report_mode: ReportMode) -> Self {
        Self::new_labeled_total_row("", "", "", total, report_mode)
    }

    fn new_labeled_total_row(
        session: &str,
        time_range: &str,
        duration: &str,
        total: impl IntoMetricTotalForMode,
        report_mode: ReportMode,
    ) -> Self {
        Self {
            session: session.to_string(),
            time_range: time_range.to_string(),
            duration: duration.to_string(),
            model: "Total".to_string(),
            metrics: FormattedMetricColumns::from_total(total.into_metric_total(report_mode)),
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
                        report_mode,
                    )
                },
            );
            Ok(())
        },
        |rows, metrics, has_detail_rows| {
            rows.push(if has_detail_rows {
                ApiMetricRow::new_total_row(metrics.total(report_mode)?, report_mode)
            } else {
                ApiMetricRow::new_labeled_total_row(
                    &metrics.date.to_string(),
                    metrics.total(report_mode)?,
                    report_mode,
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
                        report_mode,
                    )
                },
            );
            Ok(())
        },
        |rows, metrics, has_detail_rows| {
            rows.push(if has_detail_rows {
                ApiSessionMetricRow::new_total_row(metrics.total(report_mode)?, report_mode)
            } else {
                ApiSessionMetricRow::new_labeled_total_row(
                    &format_session_id(&metrics.session_id),
                    &format_time_range(timezone, metrics.start_time, metrics.end_time),
                    &format_duration(metrics.duration_minutes()),
                    metrics.total(report_mode)?,
                    report_mode,
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
