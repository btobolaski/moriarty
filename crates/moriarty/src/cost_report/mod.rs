mod charts;
mod render;
mod series;
mod time_filter;

pub(crate) use charts::{ChartBucket, ChartSegment, render_stacked_charts};
pub(crate) use render::{
    CostComponents, FormattedMetricColumns, MetricComponents, MetricTotal, ReportMode,
    ReportTotals, TokenCounts, build_grouped_rows, display_summary, format_duration,
    format_session_id, format_time_range, grouped_label, print_grouped_report,
    push_nonzero_metric_rows, render_or_empty, sum_report_totals,
};
pub(crate) use series::{DatedSegments, SessionSegments};

#[cfg(test)]
pub(crate) use render::{
    GrandTotalRow, MetricPayload, apply_width_config, create_grouped_table, display_grand_total,
    divider, fmt_money,
};
#[cfg(test)]
pub(crate) type ComponentTotals = CostComponents;
#[cfg(test)]
pub(crate) type FormattedCostColumns = FormattedMetricColumns;
pub(crate) use time_filter::{DateTimezone, TimeRangeFilter, parse_timezone};
