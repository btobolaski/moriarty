use chrono::{DateTime, Local, Utc};
use crossterm::terminal;
use miette::miette;
use tabled::{
    Table, Tabled,
    settings::{
        Alignment, Modify, Width,
        object::Rows,
        style::{HorizontalLine, Style},
        themes::Theme,
    },
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
pub(crate) enum MetricPayload {
    Cost(CostComponents),
    Tokens(TokenCounts),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MetricComponents {
    pub(crate) payload: MetricPayload,
    // Payload conversions stay neutral because Pi compactions are billable but not agent turns.
    agent_turns: u64,
}

impl From<CostComponents> for MetricComponents {
    fn from(value: CostComponents) -> Self {
        Self {
            payload: MetricPayload::Cost(value),
            agent_turns: 0,
        }
    }
}

impl From<TokenCounts> for MetricComponents {
    fn from(value: TokenCounts) -> Self {
        Self {
            payload: MetricPayload::Tokens(value),
            agent_turns: 0,
        }
    }
}

impl MetricComponents {
    #[cfg(test)]
    pub(crate) fn zero(report_mode: ReportMode) -> Self {
        match report_mode {
            ReportMode::Cost => CostComponents::default().into(),
            ReportMode::Tokens => TokenCounts::default().into(),
        }
    }

    pub(crate) fn with_agent_turns(mut self, agent_turns: u64) -> Self {
        self.agent_turns = agent_turns;
        self
    }

    pub(crate) fn agent_turns(&self) -> u64 {
        self.agent_turns
    }

    pub(crate) fn is_zero(&self) -> bool {
        self.agent_turns == 0
            && match self.payload {
                MetricPayload::Cost(costs) => costs.total() == 0.0,
                MetricPayload::Tokens(counts) => counts.total() == 0,
            }
    }

    pub(crate) fn total(&self) -> MetricTotal {
        match self.payload {
            MetricPayload::Cost(costs) => MetricTotal::Cost(costs.total()),
            MetricPayload::Tokens(counts) => MetricTotal::Tokens(counts.total()),
        }
    }

    pub(crate) fn try_add_assign(&mut self, other: Self) -> miette::Result<()> {
        match (&mut self.payload, other.payload) {
            (MetricPayload::Cost(current), MetricPayload::Cost(other)) => {
                current.input += other.input;
                current.output += other.output;
                current.cache_write += other.cache_write;
                current.cache_read += other.cache_read;
            }
            (MetricPayload::Tokens(current), MetricPayload::Tokens(other)) => {
                current.checked_add_assign(other)?;
            }
            (MetricPayload::Cost(_), MetricPayload::Tokens(_)) => {
                return Err(miette!(
                    "attempted to add token metrics into a cost accumulator"
                ));
            }
            (MetricPayload::Tokens(_), MetricPayload::Cost(_)) => {
                return Err(miette!(
                    "attempted to add cost metrics into a token accumulator"
                ));
            }
        }

        self.agent_turns += other.agent_turns;
        Ok(())
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ReportTotals {
    pub(crate) total: MetricTotal,
    pub(crate) agent_turns: u64,
}

impl ReportTotals {
    pub(crate) fn new(total: MetricTotal, agent_turns: u64) -> Self {
        Self { total, agent_turns }
    }

    pub(crate) fn zero(report_mode: ReportMode) -> Self {
        Self::new(MetricTotal::zero(report_mode), 0)
    }

    pub(crate) fn try_add(self, other: Self) -> miette::Result<Self> {
        Ok(Self {
            total: self.total.checked_add(other.total)?,
            agent_turns: self.agent_turns + other.agent_turns,
        })
    }
}

pub(crate) fn sum_report_totals<'a, Item: 'a>(
    report_mode: ReportMode,
    items: impl IntoIterator<Item = &'a Item>,
    item_totals: impl Fn(&Item) -> miette::Result<ReportTotals>,
) -> miette::Result<ReportTotals> {
    items
        .into_iter()
        .try_fold(ReportTotals::zero(report_mode), |totals, item| {
            totals.try_add(item_totals(item)?)
        })
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
    if first_row { value } else { "" }
}

const MIN_WIDTH_FOR_WRAPPING: usize = 100;

#[derive(Tabled)]
pub(crate) struct FormattedMetricColumns {
    #[tabled(rename = "Agent Turns")]
    pub(crate) agent_turns: String,
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
        let agent_turns = fmt_tokens(metrics.agent_turns() as u128);
        match metrics.payload {
            MetricPayload::Cost(costs) => Self {
                agent_turns,
                input: fmt_money(costs.input),
                output: fmt_money(costs.output),
                cache_write: fmt_money(costs.cache_write),
                cache_read: fmt_money(costs.cache_read),
                subtotal: fmt_money(costs.total()),
            },
            MetricPayload::Tokens(counts) => Self {
                agent_turns,
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
    pub(crate) fn from_total(totals: ReportTotals) -> Self {
        Self {
            agent_turns: fmt_tokens(totals.agent_turns as u128),
            input: String::new(),
            output: String::new(),
            cache_write: String::new(),
            cache_read: String::new(),
            subtotal: totals.total.format(),
        }
    }
}

#[derive(Tabled)]
pub(crate) struct GrandTotalRow {
    #[tabled(rename = "Agent Turns")]
    pub(crate) agent_turns: String,
    #[tabled(rename = "Grand Total")]
    pub(crate) grand_total: String,
}

impl GrandTotalRow {
    pub(crate) fn new(totals: ReportTotals) -> Self {
        Self {
            agent_turns: fmt_tokens(totals.agent_turns as u128),
            grand_total: totals.total.format(),
        }
    }
}

#[derive(Tabled)]
pub(crate) struct ProviderSummaryRow {
    #[tabled(rename = "Provider")]
    provider: String,
    #[tabled(inline)]
    metrics: FormattedMetricColumns,
}

#[derive(Tabled)]
pub(crate) struct ModelSummaryRow {
    #[tabled(rename = "Model")]
    model: String,
    #[tabled(inline)]
    metrics: FormattedMetricColumns,
}

pub(crate) fn get_terminal_width() -> usize {
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

pub(crate) fn print_grouped_report<T: Tabled>(
    title: &str,
    rows: &[T],
    total_row_indices: &[usize],
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

pub(crate) fn display_summary(
    report_mode: ReportMode,
    providers: Option<&[(String, MetricComponents)]>,
    models: &[(String, MetricComponents)],
    grand_totals: ReportTotals,
) {
    let term_width = get_terminal_width();
    println!("{}", divider(term_width));
    println!("Summary");
    println!("{}", divider(term_width));
    println!();

    if let Some(providers) = providers {
        render_summary_table(
            report_mode,
            providers,
            |provider, metrics| ProviderSummaryRow {
                provider,
                metrics: FormattedMetricColumns::from_metrics(metrics),
            },
            |totals| ProviderSummaryRow {
                provider: "Total".to_string(),
                metrics: FormattedMetricColumns::from_total(totals),
            },
            term_width,
        );
        println!();
    }

    render_summary_table(
        report_mode,
        models,
        |model, metrics| ModelSummaryRow {
            model,
            metrics: FormattedMetricColumns::from_metrics(metrics),
        },
        |totals| ModelSummaryRow {
            model: "Total".to_string(),
            metrics: FormattedMetricColumns::from_total(totals),
        },
        term_width,
    );
    println!();

    let row = GrandTotalRow::new(grand_totals);
    let mut table = Table::new(vec![row]);
    table.with(Style::rounded());
    table.with(Modify::new(Rows::first()).with(Alignment::center()));
    apply_width_config(&mut table, term_width);
    println!("{}", table);
    println!("{}", divider(term_width));
}

fn render_summary_table<Row: Tabled>(
    report_mode: ReportMode,
    items: &[(String, MetricComponents)],
    into_row: impl Fn(String, MetricComponents) -> Row,
    into_total_row: impl Fn(ReportTotals) -> Row,
    term_width: usize,
) {
    let mut rows: Vec<Row> = Vec::new();
    let mut totals = ReportTotals::zero(report_mode);

    for (key, metrics) in items.iter() {
        if !metrics.is_zero() {
            totals = totals
                .try_add(ReportTotals::new(metrics.total(), metrics.agent_turns()))
                .expect("summary table totals overflow");
            rows.push(into_row(key.clone(), *metrics));
        }
    }

    rows.push(into_total_row(totals));

    let mut table = Table::new(rows);
    table.with(Style::rounded());
    table.with(Modify::new(Rows::first()).with(Alignment::center()));
    apply_width_config(&mut table, term_width);
    println!("{}", table);
}

#[cfg(test)]
pub(crate) fn display_grand_total(grand_totals: ReportTotals) {
    let term_width = get_terminal_width();
    println!("{}", divider(term_width));
    println!("Summary");
    println!("{}", divider(term_width));
    println!();

    let row = GrandTotalRow::new(grand_totals);
    let mut table = Table::new(vec![row]);

    table.with(Style::rounded());
    table.with(Modify::new(Rows::first()).with(Alignment::center()));
    apply_width_config(&mut table, term_width);

    println!("{}", table);
    println!("{}", divider(term_width));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cost_metrics(
        input: f64,
        output: f64,
        cache_write: f64,
        cache_read: f64,
    ) -> MetricComponents {
        CostComponents::new(input, output, cache_write, cache_read).into()
    }

    fn token_metrics(
        input: u64,
        output: u64,
        cache_write: u64,
        cache_read: u64,
    ) -> MetricComponents {
        TokenCounts::new(input, output, cache_write, cache_read).into()
    }

    #[test]
    fn formatted_columns_include_agent_turns() {
        let detail = FormattedMetricColumns::from_metrics(
            cost_metrics(1.0, 2.0, 3.0, 4.0).with_agent_turns(12_345),
        );
        assert_eq!(detail.agent_turns, "12,345");

        let total =
            FormattedMetricColumns::from_total(ReportTotals::new(MetricTotal::Cost(10.0), 12_345));
        assert_eq!(total.agent_turns, "12,345");
        assert_eq!(total.input, "");
        assert_eq!(total.output, "");
        assert_eq!(total.cache_write, "");
        assert_eq!(total.cache_read, "");
        assert_eq!(total.subtotal, "$10.0000");
    }

    #[test]
    fn metric_components_add_turns_for_both_payload_modes() {
        let mut costs = cost_metrics(1.0, 0.0, 0.0, 0.0).with_agent_turns(2);
        costs
            .try_add_assign(cost_metrics(2.0, 0.0, 0.0, 0.0).with_agent_turns(3))
            .unwrap();
        assert_eq!(costs.agent_turns(), 5);

        let mut tokens = token_metrics(1, 0, 0, 0).with_agent_turns(4);
        tokens
            .try_add_assign(token_metrics(2, 0, 0, 0).with_agent_turns(5))
            .unwrap();
        assert_eq!(tokens.agent_turns(), 9);
    }

    #[test]
    fn sum_report_totals_handles_empty_sum_and_mode_mismatch() {
        let empty: [ReportTotals; 0] = [];
        assert_eq!(
            sum_report_totals(ReportMode::Cost, &empty, |totals| Ok(*totals)).unwrap(),
            ReportTotals::zero(ReportMode::Cost)
        );

        let items = [
            ReportTotals::new(MetricTotal::Cost(1.5), 2),
            ReportTotals::new(MetricTotal::Cost(2.5), 3),
        ];
        assert_eq!(
            sum_report_totals(ReportMode::Cost, &items, |totals| Ok(*totals)).unwrap(),
            ReportTotals::new(MetricTotal::Cost(4.0), 5)
        );

        let error =
            sum_report_totals(ReportMode::Tokens, &items[..1], |totals| Ok(*totals)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("attempted to add cost totals into a token grand total")
        );
    }

    #[test]
    fn metric_components_with_turn_is_not_zero_when_payload_is_zero() {
        assert!(
            !cost_metrics(0.0, 0.0, 0.0, 0.0)
                .with_agent_turns(1)
                .is_zero()
        );
        assert!(!token_metrics(0, 0, 0, 0).with_agent_turns(1).is_zero());
    }

    #[test]
    fn display_summary_cost_mode_does_not_panic() {
        let providers = vec![
            ("Anthropic".to_string(), cost_metrics(1.0, 2.0, 0.0, 0.0)),
            ("OpenAI".to_string(), cost_metrics(0.5, 1.0, 0.0, 0.0)),
        ];
        let models = vec![
            (
                "claude-sonnet-4-5".to_string(),
                cost_metrics(1.0, 2.0, 0.0, 0.0),
            ),
            ("gpt-5".to_string(), cost_metrics(0.5, 1.0, 0.0, 0.0)),
        ];

        display_summary(
            ReportMode::Cost,
            Some(&providers),
            &models,
            ReportTotals::new(MetricTotal::Cost(4.5), 0),
        );
    }

    #[test]
    fn display_summary_token_mode_does_not_panic() {
        let providers = vec![("Anthropic".to_string(), token_metrics(1_000, 500, 100, 50))];
        let models = vec![(
            "claude-sonnet-4-5".to_string(),
            token_metrics(1_000, 500, 100, 50),
        )];

        display_summary(
            ReportMode::Tokens,
            Some(&providers),
            &models,
            ReportTotals::new(MetricTotal::Tokens(1_650), 0),
        );
    }

    #[test]
    fn display_summary_no_providers_does_not_panic() {
        let models = vec![("Sonnet".to_string(), cost_metrics(1.0, 2.0, 0.0, 0.0))];

        display_summary(
            ReportMode::Cost,
            None,
            &models,
            ReportTotals::new(MetricTotal::Cost(3.0), 0),
        );
    }

    #[test]
    fn display_summary_empty_models_does_not_panic() {
        display_summary(
            ReportMode::Cost,
            None,
            &[],
            ReportTotals::new(MetricTotal::Cost(0.0), 0),
        );
    }

    #[test]
    fn display_summary_handles_zero_payload_rows() {
        let models = vec![
            ("Zero".to_string(), cost_metrics(0.0, 0.0, 0.0, 0.0)),
            (
                "Turn only".to_string(),
                cost_metrics(0.0, 0.0, 0.0, 0.0).with_agent_turns(1),
            ),
        ];
        display_summary(
            ReportMode::Cost,
            None,
            &models,
            ReportTotals::new(MetricTotal::Cost(0.0), 1),
        );
    }
}
