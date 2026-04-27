mod analyzer;
mod pricing;
mod time_filter;

use std::path::Path;

use chrono::{DateTime, Local, Utc};
use crossterm::terminal;
use tabled::{
    settings::{
        object::Rows,
        style::{HorizontalLine, Style},
        themes::Theme,
        Alignment, Modify, Width,
    },
    Table, Tabled,
};

use analyzer::{DailyCosts, SessionCosts};
use pricing::TokenCosts;

pub use analyzer::DateTimezone;
pub use time_filter::TimeRangeFilter;

/// `(input, output, cache_write, cache_read)` token-cost components, used as
/// a single bag of values when constructing per-model table rows so the four
/// always travel together and stay in the canonical column order.
type CostComponents = (f64, f64, f64, f64);

fn fmt_money(amount: f64) -> String {
    format!("${:.4}", amount)
}

/// Returns `value` on the first model row in a group and an empty string for
/// subsequent rows, so grouped tables only show identifying columns once.
fn grouped_label(first_row: bool, value: &str) -> &str {
    if first_row {
        value
    } else {
        ""
    }
}

/// Minimum terminal width for using word-wrapping instead of truncation.
/// On terminals wider than or equal to this, wrapping prevents cutting off currency values.
/// On narrower terminals, truncation keeps output compact.
const MIN_WIDTH_FOR_WRAPPING: usize = 100;

/// The five currency-formatted columns shared by every cost-table row.
///
/// Both `CostRow` and `SessionCostRow` embed this substruct via
/// `#[tabled(inline)]` so the per-token component columns and the trailing
/// `Subtotal` column appear once in the table layout and once in the
/// constructor logic, instead of being duplicated across each row type.
#[derive(Tabled)]
struct FormattedCostColumns {
    #[tabled(rename = "Input")]
    input: String,
    #[tabled(rename = "Output")]
    output: String,
    #[tabled(rename = "Cache Write")]
    cache_write: String,
    #[tabled(rename = "Cache Read")]
    cache_read: String,
    #[tabled(rename = "Subtotal")]
    subtotal: String,
}

impl FormattedCostColumns {
    fn from_components(components: CostComponents) -> Self {
        let (input, output, cache_write, cache_read) = components;
        Self {
            input: fmt_money(input),
            output: fmt_money(output),
            cache_write: fmt_money(cache_write),
            cache_read: fmt_money(cache_read),
            subtotal: fmt_money(input + output + cache_write + cache_read),
        }
    }

    /// Per-model component columns are intentionally blank in the total row:
    /// showing per-model values there would be misleading, since users could
    /// read them as additional costs rather than already-summed components
    /// of the subtotal.
    fn from_total(total_cost: f64) -> Self {
        Self {
            input: String::new(),
            output: String::new(),
            cache_write: String::new(),
            cache_read: String::new(),
            subtotal: fmt_money(total_cost),
        }
    }
}

#[derive(Tabled)]
struct CostRow {
    #[tabled(rename = "Date")]
    date: String,
    #[tabled(rename = "Model")]
    model: String,
    #[tabled(inline)]
    money: FormattedCostColumns,
}

impl CostRow {
    fn new(date: &str, model: &str, costs: CostComponents) -> Self {
        Self {
            date: date.to_string(),
            model: model.to_string(),
            money: FormattedCostColumns::from_components(costs),
        }
    }

    fn new_total_row(total_cost: f64) -> Self {
        Self {
            date: String::new(),
            model: "Total".to_string(),
            money: FormattedCostColumns::from_total(total_cost),
        }
    }
}

#[derive(Tabled)]
struct GrandTotalRow {
    #[tabled(rename = "Grand Total")]
    grand_total: String,
}

impl GrandTotalRow {
    fn new(grand_total: f64) -> Self {
        Self {
            grand_total: fmt_money(grand_total),
        }
    }
}

#[derive(Tabled)]
struct SessionCostRow {
    #[tabled(rename = "Session")]
    session: String,
    #[tabled(rename = "Time Range")]
    time_range: String,
    #[tabled(rename = "Duration")]
    duration: String,
    #[tabled(rename = "Model")]
    model: String,
    #[tabled(inline)]
    money: FormattedCostColumns,
}

impl SessionCostRow {
    fn new(
        session: &str,
        time_range: &str,
        duration: &str,
        model: &str,
        costs: CostComponents,
    ) -> Self {
        Self {
            session: session.to_string(),
            time_range: time_range.to_string(),
            duration: duration.to_string(),
            model: model.to_string(),
            money: FormattedCostColumns::from_components(costs),
        }
    }

    fn new_total_row(total_cost: f64) -> Self {
        Self {
            session: String::new(),
            time_range: String::new(),
            duration: String::new(),
            model: "Total".to_string(),
            money: FormattedCostColumns::from_total(total_cost),
        }
    }
}

fn get_terminal_width() -> usize {
    terminal::size()
        .map(|(cols, _)| cols as usize)
        .unwrap_or(80)
}

fn divider(width: usize) -> String {
    "=".repeat(width)
}

/// On wide terminals (`>= MIN_WIDTH_FOR_WRAPPING`) word-wrap so currency
/// values stay legible; on narrower ones truncate to keep the layout from
/// collapsing. The threshold is the only reason this helper exists.
fn apply_width_config(table: &mut Table, term_width: usize) {
    if term_width >= MIN_WIDTH_FOR_WRAPPING {
        table.with(Width::wrap(term_width).keep_words(true));
    } else {
        table.with(Width::truncate(term_width));
    }
}

fn push_nonzero_model_rows<Row>(
    rows: &mut Vec<Row>,
    model_costs: [(&'static str, TokenCosts); 4],
    mut make_row: impl FnMut(bool, &'static str, TokenCosts) -> Row,
) {
    let mut first_row = true;

    for (model_name, costs) in model_costs {
        if costs.total() > 0.0 {
            rows.push(make_row(first_row, model_name, costs));
            first_row = false;
        }
    }
}

/// Returns the rendered rows plus the position of each group's terminal row,
/// which callers use to insert separators between groups without re-scanning.
fn build_grouped_rows<Item, Row>(
    items: &[Item],
    mut push_item_rows: impl FnMut(&mut Vec<Row>, &Item),
    mut push_total_row: impl FnMut(&mut Vec<Row>, &Item),
) -> (Vec<Row>, Vec<usize>) {
    let mut rows = Vec::new();
    let mut total_row_indices = Vec::new();

    for item in items {
        push_item_rows(&mut rows, item);
        push_total_row(&mut rows, item);
        total_row_indices.push(rows.len() - 1);
    }

    (rows, total_row_indices)
}

fn build_cost_rows(daily_costs: &[DailyCosts]) -> (Vec<CostRow>, Vec<usize>) {
    build_grouped_rows(
        daily_costs,
        |rows, costs| {
            let date_str = costs.date.to_string();
            push_nonzero_model_rows(
                rows,
                costs.per_model.model_costs(),
                |first_row, model_name, model_costs| {
                    CostRow::new(
                        grouped_label(first_row, &date_str),
                        model_name,
                        model_costs.as_components(),
                    )
                },
            );
        },
        |rows, costs| rows.push(CostRow::new_total_row(costs.total())),
    )
}

/// `Theme::insert_horizontal_line` is the only `tabled` mechanism that lets us
/// insert separators at arbitrary post-data row indices, so groups built from
/// heterogeneous data are stitched together with explicit separator indices
/// rather than a sentinel row variant in the underlying `Vec<T>`.
///
/// `total_row_indices` marks the last row of each group; separators are
/// inserted between groups.
fn create_grouped_table<T: Tabled>(rows: &[T], total_row_indices: &[usize]) -> Table {
    let mut table = Table::new(rows);

    let mut theme = Theme::from_style(Style::rounded());

    if total_row_indices.len() > 1 {
        let separator_line = HorizontalLine::full('─', '┼', '├', '┤');

        for &idx in &total_row_indices[..total_row_indices.len() - 1] {
            // +1 for header row, +1 to place separator after the total row
            theme.insert_horizontal_line(idx + 2, separator_line);
        }
    }

    table.with(theme);
    table.with(Modify::new(Rows::first()).with(Alignment::center()));

    table
}

/// 8 characters is enough to disambiguate Claude session UUIDs at a glance
/// while keeping the column narrow on small terminals.
fn format_session_id(session_id: &str) -> String {
    if session_id.len() > 8 {
        session_id[..8].to_string()
    } else {
        session_id.to_string()
    }
}

/// Renders the start in full `YYYY-MM-DD HH:MM` form but the end as `HH:MM`
/// only, since the date is almost always identical and the redundancy would
/// crowd the column.
fn format_time_range(start: DateTime<Utc>, end: DateTime<Utc>) -> String {
    let start_local = start.with_timezone(&Local);
    let end_local = end.with_timezone(&Local);
    format!(
        "{} → {}",
        start_local.format("%Y-%m-%d %H:%M"),
        end_local.format("%H:%M")
    )
}

fn format_duration(minutes: i64) -> String {
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

fn build_session_cost_rows(session_costs: &[SessionCosts]) -> (Vec<SessionCostRow>, Vec<usize>) {
    build_grouped_rows(
        session_costs,
        |rows, costs| {
            let session_id = format_session_id(&costs.session_id);
            let time_range = format_time_range(costs.start_time, costs.end_time);
            let duration = format_duration(costs.duration_minutes());
            push_nonzero_model_rows(
                rows,
                costs.per_model.model_costs(),
                |first_row, model_name, model_costs| {
                    SessionCostRow::new(
                        grouped_label(first_row, &session_id),
                        grouped_label(first_row, &time_range),
                        grouped_label(first_row, &duration),
                        model_name,
                        model_costs.as_components(),
                    )
                },
            );
        },
        |rows, costs| rows.push(SessionCostRow::new_total_row(costs.total())),
    )
}

/// `rows` and `total_row_indices` are produced together by
/// `build_cost_rows` / `build_session_cost_rows`; the indices are only valid
/// against the matching row vector, so the report renderer takes both at the
/// same call site to keep that invariant local.
fn render_cost_report<T: Tabled>(
    title: &str,
    rows: &[T],
    total_row_indices: &[usize],
    grand_total: f64,
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

    display_grand_total(grand_total);
}

fn display_grouped_costs<Item, Row: Tabled>(
    title: &str,
    items: &[Item],
    build_rows: impl Fn(&[Item]) -> (Vec<Row>, Vec<usize>),
    total: impl Fn(&Item) -> f64,
) {
    let (rows, total_row_indices) = build_rows(items);
    let grand_total: f64 = items.iter().map(total).sum();
    render_cost_report(title, &rows, &total_row_indices, grand_total);
}

fn display_session_costs(session_costs: &[SessionCosts]) {
    display_grouped_costs(
        "API Cost Report by Conversation",
        session_costs,
        build_session_cost_rows,
        SessionCosts::total,
    );
}

pub async fn run_by_session(dir: &Path, filter: &TimeRangeFilter) -> miette::Result<()> {
    let result = analyzer::analyze_directory_by_session(dir, filter).await?;
    render_or_empty(
        &result.session_costs,
        result.had_errors,
        display_session_costs,
    );
    Ok(())
}

pub async fn run(
    dir: &Path,
    timezone: DateTimezone,
    by_conversation: bool,
    filter: &TimeRangeFilter,
) -> miette::Result<()> {
    if by_conversation {
        return run_by_session(dir, filter).await;
    }

    let result = analyzer::analyze_directory(dir, timezone, filter).await?;
    render_or_empty(&result.daily_costs, result.had_errors, display_costs);
    Ok(())
}

/// Render `items` via `display` when non-empty, or a single "no usage data"
/// message when empty, then always emit the parser-failure warning.
///
/// The two report entry points must keep the empty-vs-nonempty branching and
/// the trailing `warn_if_incomplete` call in lockstep so that a partial parse
/// is surfaced even when the surviving rows were filtered out. Routing both
/// through one helper is the only way to keep that invariant from drifting.
fn render_or_empty<T>(items: &[T], had_errors: bool, display: impl FnOnce(&[T])) {
    if items.is_empty() {
        println!("\nNo usage data found.");
    } else {
        display(items);
    }
    warn_if_incomplete(had_errors);
}

/// Print a single user-facing warning when `cost_analyzer` reported that one
/// or more log files could not be read or parsed.
///
/// `cost_analyzer` already emits structured `tracing` events for each failing
/// file (see `crates/cost_analyzer/src/reader.rs`); main.rs routes those to
/// stderr. This helper exists only to make sure the operator notices that
/// totals may be incomplete even if the per-file logs scrolled past.
fn warn_if_incomplete(had_errors: bool) {
    if had_errors {
        eprintln!(
            "\nWarning: some log files could not be read or parsed; \
             totals may be incomplete. See the per-file errors above for details."
        );
    }
}

fn display_costs(daily_costs: &[DailyCosts]) {
    display_grouped_costs(
        "API Cost Report",
        daily_costs,
        build_cost_rows,
        DailyCosts::total,
    );
}

fn display_grand_total(grand_total: f64) {
    let term_width = get_terminal_width();
    println!("{}", divider(term_width));
    println!("Summary");
    println!("{}", divider(term_width));
    println!();

    let row = GrandTotalRow::new(grand_total);
    let mut table = Table::new(vec![row]);

    table.with(Style::rounded());
    table.with(Modify::new(Rows::first()).with(Alignment::center()));
    apply_width_config(&mut table, term_width);

    println!("{}", table);
    println!("{}", divider(term_width));
}

#[cfg(test)]
mod tests;
