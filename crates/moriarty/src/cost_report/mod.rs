mod render;
mod time_filter;

pub(crate) use render::{
    build_grouped_rows, format_duration, format_session_id, format_time_range, grouped_label,
    push_nonzero_metric_rows, render_grouped_metrics, render_or_empty, CostComponents,
    FormattedMetricColumns, MetricComponents, MetricTotal, ReportMode, TokenCounts,
};

#[cfg(test)]
pub(crate) use render::{
    apply_width_config, create_grouped_table, display_grand_total, divider, fmt_money,
    GrandTotalRow,
};
#[cfg(test)]
pub(crate) type ComponentTotals = CostComponents;
#[cfg(test)]
pub(crate) type FormattedCostColumns = FormattedMetricColumns;
pub(crate) use time_filter::{DateTimezone, TimeRangeFilter};
