use chrono::{DateTime, Local, Utc};
use crossterm::terminal;
use miette::miette;
use tabled::{
    settings::{
        object::Rows,
        style::{HorizontalLine, Style},
        themes::Theme,
        Alignment, Modify, Width,
    },
    Table, Tabled,
};

use super::time_filter::DateTimezone;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct CostComponents {
    pub(crate) input: f64,
    pub(crate) output: f64,
    pub(crate) cache_write: f64,
    pub(crate) cache_read: f64,
}

impl CostComponents {
    pub(crate) fn new(input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self {
        Self {
            input,
            output,
            cache_write,
            cache_read,
        }
    }

    pub(crate) fn total(&self) -> f64 {
        self.input + self.output + self.cache_write + self.cache_read
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TokenCounts {
    pub(crate) input: u64,
    pub(crate) output: u64,
    pub(crate) cache_write: u64,
    pub(crate) cache_read: u64,
}

impl TokenCounts {
    pub(crate) fn new(input: u64, output: u64, cache_write: u64, cache_read: u64) -> Self {
        Self {
            input,
            output,
            cache_write,
            cache_read,
        }
    }

    pub(crate) fn total(&self) -> u128 {
        self.input as u128
            + self.output as u128
            + self.cache_write as u128
            + self.cache_read as u128
    }

    fn checked_add_assign(&mut self, other: Self) -> miette::Result<()> {
        self.input = self
            .input
            .checked_add(other.input)
            .ok_or_else(|| miette!("token input total exceeded u64"))?;
        self.output = self
            .output
            .checked_add(other.output)
            .ok_or_else(|| miette!("token output total exceeded u64"))?;
        self.cache_write = self
            .cache_write
            .checked_add(other.cache_write)
            .ok_or_else(|| miette!("token cache-write total exceeded u64"))?;
        self.cache_read = self
            .cache_read
            .checked_add(other.cache_read)
            .ok_or_else(|| miette!("token cache-read total exceeded u64"))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum MetricComponents {
    Cost(CostComponents),
    Tokens(TokenCounts),
}

impl From<CostComponents> for MetricComponents {
    fn from(value: CostComponents) -> Self {
        Self::Cost(value)
    }
}

impl From<TokenCounts> for MetricComponents {
    fn from(value: TokenCounts) -> Self {
        Self::Tokens(value)
    }
}

impl MetricComponents {
    #[cfg(test)]
    pub(crate) fn zero(report_mode: ReportMode) -> Self {
        match report_mode {
            ReportMode::Cost => Self::Cost(CostComponents::default()),
            ReportMode::Tokens => Self::Tokens(TokenCounts::default()),
        }
    }

    pub(crate) fn is_zero(&self) -> bool {
        match self {
            Self::Cost(costs) => costs.total() == 0.0,
            Self::Tokens(counts) => counts.total() == 0,
        }
    }

    pub(crate) fn total(&self) -> MetricTotal {
        match self {
            Self::Cost(costs) => MetricTotal::Cost(costs.total()),
            Self::Tokens(counts) => MetricTotal::Tokens(counts.total()),
        }
    }

    pub(crate) fn checked_add_assign(&mut self, other: Self) -> miette::Result<()> {
        match (self, other) {
            (Self::Cost(current), Self::Cost(other)) => {
                current.input += other.input;
                current.output += other.output;
                current.cache_write += other.cache_write;
                current.cache_read += other.cache_read;
                Ok(())
            }
            (Self::Tokens(current), Self::Tokens(other)) => current.checked_add_assign(other),
            (Self::Cost(_), Self::Tokens(_)) => Err(miette!(
                "attempted to add token metrics into a cost accumulator"
            )),
            (Self::Tokens(_), Self::Cost(_)) => Err(miette!(
                "attempted to add cost metrics into a token accumulator"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum MetricTotal {
    Cost(f64),
    Tokens(u128),
}

impl MetricTotal {
    pub(crate) fn zero(report_mode: ReportMode) -> Self {
        match report_mode {
            ReportMode::Cost => Self::Cost(0.0),
            ReportMode::Tokens => Self::Tokens(0),
        }
    }

    pub(crate) fn report_mode(self) -> ReportMode {
        match self {
            Self::Cost(_) => ReportMode::Cost,
            Self::Tokens(_) => ReportMode::Tokens,
        }
    }

    pub(crate) fn checked_add(self, other: Self) -> miette::Result<Self> {
        match (self, other) {
            (Self::Cost(left), Self::Cost(right)) => Ok(Self::Cost(left + right)),
            (Self::Tokens(left), Self::Tokens(right)) => {
                Ok(Self::Tokens(left.checked_add(right).ok_or_else(|| {
                    miette!("grand token total exceeded u128")
                })?))
            }
            (Self::Cost(_), Self::Tokens(_)) => Err(miette!(
                "attempted to add token totals into a cost grand total"
            )),
            (Self::Tokens(_), Self::Cost(_)) => Err(miette!(
                "attempted to add cost totals into a token grand total"
            )),
        }
    }

    fn format(self) -> String {
        match self {
            Self::Cost(amount) => fmt_money(amount),
            Self::Tokens(amount) => fmt_tokens(amount),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReportMode {
    Cost,
    Tokens,
}

pub(crate) fn fmt_money(amount: f64) -> String {
    let normalized = if amount == 0.0 { 0.0 } else { amount };
    format!("${normalized:.4}")
}

pub(crate) fn fmt_tokens(amount: u128) -> String {
    format_integer_with_separators(amount)
}

fn format_integer_with_separators(value: u128) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);

    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(ch);
    }

    grouped
}

pub(crate) fn grouped_label(first_row: bool, value: &str) -> &str {
    if first_row {
        value
    } else {
        ""
    }
}

const MIN_WIDTH_FOR_WRAPPING: usize = 100;

#[derive(Tabled)]
pub(crate) struct FormattedMetricColumns {
    #[tabled(rename = "Input")]
    pub(crate) input: String,
    #[tabled(rename = "Output")]
    pub(crate) output: String,
    #[tabled(rename = "Cache Write")]
    pub(crate) cache_write: String,
    #[tabled(rename = "Cache Read")]
    pub(crate) cache_read: String,
    #[tabled(rename = "Subtotal")]
    pub(crate) subtotal: String,
}

impl FormattedMetricColumns {
    pub(crate) fn from_metrics(metrics: MetricComponents) -> Self {
        match metrics {
            MetricComponents::Cost(costs) => Self {
                input: fmt_money(costs.input),
                output: fmt_money(costs.output),
                cache_write: fmt_money(costs.cache_write),
                cache_read: fmt_money(costs.cache_read),
                subtotal: fmt_money(costs.total()),
            },
            MetricComponents::Tokens(counts) => Self {
                input: fmt_tokens(counts.input as u128),
                output: fmt_tokens(counts.output as u128),
                cache_write: fmt_tokens(counts.cache_write as u128),
                cache_read: fmt_tokens(counts.cache_read as u128),
                subtotal: fmt_tokens(counts.total()),
            },
        }
    }

    /// Leaving the per-component cells blank prevents the footer from looking
    /// like another model row whose subtotal should be added again.
    pub(crate) fn from_total(total: MetricTotal) -> Self {
        Self {
            input: String::new(),
            output: String::new(),
            cache_write: String::new(),
            cache_read: String::new(),
            subtotal: total.format(),
        }
    }
}

pub(crate) trait IntoMetricTotalForMode {
    fn into_metric_total(self, report_mode: ReportMode) -> MetricTotal;
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

#[cfg(test)]
impl IntoMetricTotalForMode for u64 {
    fn into_metric_total(self, report_mode: ReportMode) -> MetricTotal {
        match report_mode {
            ReportMode::Cost => MetricTotal::Cost(self as f64),
            ReportMode::Tokens => MetricTotal::Tokens(self as u128),
        }
    }
}

#[cfg(test)]
impl IntoMetricTotalForMode for u128 {
    fn into_metric_total(self, report_mode: ReportMode) -> MetricTotal {
        match report_mode {
            ReportMode::Cost => MetricTotal::Cost(self as f64),
            ReportMode::Tokens => MetricTotal::Tokens(self),
        }
    }
}

#[derive(Tabled)]
pub(crate) struct GrandTotalRow {
    #[tabled(rename = "Grand Total")]
    pub(crate) grand_total: String,
}

impl GrandTotalRow {
    pub(crate) fn new(report_mode: ReportMode, grand_total: impl IntoMetricTotalForMode) -> Self {
        Self {
            grand_total: grand_total.into_metric_total(report_mode).format(),
        }
    }
}

fn get_terminal_width() -> usize {
    terminal::size()
        .map(|(cols, _)| cols as usize)
        .unwrap_or(80)
}

pub(crate) fn divider(width: usize) -> String {
    "=".repeat(width)
}

/// The wrap/truncate split keeps wide terminals readable without letting
/// narrow terminals explode horizontally.
pub(crate) fn apply_width_config(table: &mut Table, term_width: usize) {
    if term_width >= MIN_WIDTH_FOR_WRAPPING {
        table.with(Width::wrap(term_width).keep_words(true));
    } else {
        table.with(Width::truncate(term_width));
    }
}

pub(crate) fn push_nonzero_metric_rows<Row, Key, Items>(
    rows: &mut Vec<Row>,
    items: Items,
    mut make_row: impl FnMut(bool, Key, MetricComponents) -> Row,
) where
    Items: IntoIterator<Item = (Key, MetricComponents)>,
{
    let mut first_row = true;

    for (key, metrics) in items {
        if !metrics.is_zero() {
            rows.push(make_row(first_row, key, metrics));
            first_row = false;
        }
    }
}

/// Rows and separator indices are produced together so callers cannot
/// accidentally render indices against a different row vector.
pub(crate) fn build_grouped_rows<Item, Row>(
    items: &[Item],
    mut push_item_rows: impl FnMut(&mut Vec<Row>, &Item) -> miette::Result<()>,
    mut push_total_row: impl FnMut(&mut Vec<Row>, &Item, bool) -> miette::Result<()>,
) -> miette::Result<(Vec<Row>, Vec<usize>)> {
    let mut rows = Vec::new();
    let mut total_row_indices = Vec::new();

    for item in items {
        let rows_before_group = rows.len();
        push_item_rows(&mut rows, item)?;
        let has_detail_rows = rows.len() > rows_before_group;
        push_total_row(&mut rows, item, has_detail_rows)?;
        total_row_indices.push(rows.len() - 1);
    }

    Ok((rows, total_row_indices))
}

pub(crate) fn create_grouped_table<T: Tabled>(rows: &[T], total_row_indices: &[usize]) -> Table {
    let mut table = Table::new(rows);
    let mut theme = Theme::from_style(Style::rounded());

    if total_row_indices.len() > 1 {
        let separator_line = HorizontalLine::full('─', '┼', '├', '┤');

        for &idx in &total_row_indices[..total_row_indices.len() - 1] {
            theme.insert_horizontal_line(idx + 2, separator_line);
        }
    }

    table.with(theme);
    table.with(Modify::new(Rows::first()).with(Alignment::center()));
    table
}

pub(crate) fn format_session_id(session_id: &str) -> String {
    let truncated: String = session_id.chars().take(8).collect();
    if truncated.is_empty() {
        session_id.to_string()
    } else {
        truncated
    }
}

/// Conversation reports use the caller-selected timezone so date bucketing and
/// rendered session ranges stay consistent.
pub(crate) fn format_time_range(
    timezone: DateTimezone,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> String {
    match timezone {
        DateTimezone::Local => format_time_range_in_zone(
            start
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
            end.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
            start.with_timezone(&Local).date_naive() == end.with_timezone(&Local).date_naive(),
            end.with_timezone(&Local).format("%H:%M").to_string(),
        ),
        DateTimezone::Utc => format_time_range_in_zone(
            start.format("%Y-%m-%d %H:%M").to_string(),
            end.format("%Y-%m-%d %H:%M").to_string(),
            start.date_naive() == end.date_naive(),
            end.format("%H:%M").to_string(),
        ),
    }
}

fn format_time_range_in_zone(
    start_full: String,
    end_full: String,
    same_day: bool,
    end_short: String,
) -> String {
    if same_day {
        format!("{start_full} → {end_short}")
    } else {
        format!("{start_full} → {end_full}")
    }
}

pub(crate) fn format_duration(minutes: i64) -> String {
    if minutes < 60 {
        format!("{} min", minutes)
    } else {
        let hours = minutes / 60;
        let mins = minutes % 60;
        if mins == 0 {
            format!("{} hr", hours)
        } else {
            format!("{} hr {} min", hours, mins)
        }
    }
}

pub(crate) fn render_metric_report<T: Tabled>(
    title: &str,
    rows: &[T],
    total_row_indices: &[usize],
    grand_total: MetricTotal,
) {
    let term_width = get_terminal_width();
    println!("{}", divider(term_width));
    println!("{}", title);
    println!("{}", divider(term_width));
    println!();

    let mut table = create_grouped_table(rows, total_row_indices);
    apply_width_config(&mut table, term_width);

    println!("{}", table);
    println!();

    display_grand_total(grand_total.report_mode(), grand_total);
}

pub(crate) fn render_grouped_metrics<Item, Row: Tabled>(
    title: &str,
    items: &[Item],
    report_mode: ReportMode,
    build_rows: impl Fn(&[Item]) -> miette::Result<(Vec<Row>, Vec<usize>)>,
    total: impl Fn(&Item, ReportMode) -> miette::Result<MetricTotal>,
) -> miette::Result<()> {
    let (rows, total_row_indices) = build_rows(items)?;
    let grand_total = items
        .iter()
        .try_fold(MetricTotal::zero(report_mode), |acc, item| {
            acc.checked_add(total(item, report_mode)?)
        })?;
    render_metric_report(title, &rows, &total_row_indices, grand_total);
    Ok(())
}

pub(crate) fn render_or_empty<T>(
    items: &[T],
    had_errors: bool,
    display: impl FnOnce(&[T]) -> miette::Result<()>,
) -> miette::Result<()> {
    if items.is_empty() {
        println!("\nNo usage data found.");
    } else {
        display(items)?;
    }
    warn_if_incomplete(had_errors);
    Ok(())
}

/// The detailed per-file parse errors already went to tracing; this summary is
/// only here so operators do not miss that totals may be partial.
pub(crate) fn warn_if_incomplete(had_errors: bool) {
    if had_errors {
        eprintln!(
            "\nWarning: some log files could not be read or parsed; \
             totals may be incomplete. See the per-file errors above for details."
        );
    }
}

pub(crate) fn display_grand_total(
    report_mode: ReportMode,
    grand_total: impl IntoMetricTotalForMode,
) {
    let term_width = get_terminal_width();
    println!("{}", divider(term_width));
    println!("Summary");
    println!("{}", divider(term_width));
    println!();

    let row = GrandTotalRow::new(report_mode, grand_total);
    let mut table = Table::new(vec![row]);

    table.with(Style::rounded());
    table.with(Modify::new(Rows::first()).with(Alignment::center()));
    apply_width_config(&mut table, term_width);

    println!("{}", table);
    println!("{}", divider(term_width));
}
