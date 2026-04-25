mod analyzer;
#[cfg(test)]
mod analyzer_tests;
mod line_counter;
mod pricing;
mod time_filter;

use std::{collections::HashSet, path::Path};

use analyzer::{DailyCosts, SessionCosts};
use chrono::{DateTime, Local, Utc};
use pricing::{TokenCosts, TokenCounts};

// Re-export DateTimezone and TimeRangeFilter for use in main.rs
pub use analyzer::DateTimezone;
pub use time_filter::TimeRangeFilter;

// Type alias for cost components tuple
type CostComponents = (f64, f64, f64, f64); // (input, output, cache_write, cache_read)

/// Formats a currency amount with four decimal places (e.g. `$0.1234`).
fn fmt_money(amount: f64) -> String {
    format!("${:.4}", amount)
}

/// Formats the four per-category costs plus their subtotal.
///
/// Returns `(input, output, cache_write, cache_read, subtotal)` as pre-formatted strings.
fn fmt_cost_components(costs: CostComponents) -> (String, String, String, String, String) {
    let (input, output, cache_write, cache_read) = costs;
    (
        fmt_money(input),
        fmt_money(output),
        fmt_money(cache_write),
        fmt_money(cache_read),
        fmt_money(input + output + cache_write + cache_read),
    )
}

fn total_row_values(lines_changed: usize, total_cost: f64) -> (String, String) {
    (fmt_money(total_cost), lines_changed.to_string())
}

fn empty_cost_columns() -> (String, String, String, String) {
    (String::new(), String::new(), String::new(), String::new())
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

use tabled::{
    settings::{
        object::Rows,
        style::{HorizontalLine, Style},
        themes::Theme,
        Alignment, Modify, Width,
    },
    Table, Tabled,
};

/// Minimum terminal width for using word-wrapping instead of truncation.
/// On terminals wider than or equal to this, wrapping prevents cutting off currency values.
/// On narrower terminals, truncation keeps output compact.
const MIN_WIDTH_FOR_WRAPPING: usize = 100;

/// Represents a row in the cost breakdown table
#[derive(Tabled)]
struct CostRow {
    #[tabled(rename = "Date")]
    date: String,
    #[tabled(rename = "Model")]
    model: String,
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
    #[tabled(rename = "Lines")]
    lines: String,
}

impl CostRow {
    /// Creates a new cost row with pre-formatted currency strings.
    ///
    /// The subtotal is calculated at construction time since CostRow
    /// is immutable and used only for display purposes.
    fn new(
        date: &str,
        model: &str,
        input: f64,
        output: f64,
        cache_write: f64,
        cache_read: f64,
        lines: &str,
    ) -> Self {
        let (input, output, cache_write, cache_read, subtotal) =
            fmt_cost_components((input, output, cache_write, cache_read));
        Self {
            date: date.to_string(),
            model: model.to_string(),
            input,
            output,
            cache_write,
            cache_read,
            subtotal,
            lines: lines.to_string(),
        }
    }

    /// Creates a total row for displaying daily summary.
    ///
    /// Uses empty strings for individual cost columns because showing per-model
    /// costs in a total row would be misleading - users might confuse them for
    /// additional costs rather than components of the subtotal.
    fn new_total_row(lines_changed: usize, total_cost: f64) -> Self {
        let (subtotal, lines) = total_row_values(lines_changed, total_cost);
        let (input, output, cache_write, cache_read) = empty_cost_columns();
        Self {
            date: String::new(),
            model: "Total".to_string(),
            input,
            output,
            cache_write,
            cache_read,
            subtotal,
            lines,
        }
    }
}

/// Represents the grand total summary row
#[derive(Tabled)]
struct GrandTotalRow {
    #[tabled(rename = "Grand Total")]
    grand_total: String,
    #[tabled(rename = "Total Lines Changed")]
    total_lines_changed: String,
}

impl GrandTotalRow {
    /// Creates a new grand total row with formatted values.
    fn new(grand_total: f64, total_lines: usize) -> Self {
        Self {
            grand_total: fmt_money(grand_total),
            total_lines_changed: total_lines.to_string(),
        }
    }
}

/// Represents a row in the session cost breakdown table
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
    #[tabled(rename = "Lines")]
    lines: String,
}

impl SessionCostRow {
    /// Creates a new session cost row with pre-formatted currency strings
    fn new(
        session: &str,
        time_range: &str,
        duration: &str,
        model: &str,
        costs: CostComponents,
        lines: &str,
    ) -> Self {
        let (input, output, cache_write, cache_read, subtotal) = fmt_cost_components(costs);
        Self {
            session: session.to_string(),
            time_range: time_range.to_string(),
            duration: duration.to_string(),
            model: model.to_string(),
            input,
            output,
            cache_write,
            cache_read,
            subtotal,
            lines: lines.to_string(),
        }
    }

    /// Creates a total row for displaying session summary
    fn new_total_row(lines_changed: usize, total_cost: f64) -> Self {
        let (subtotal, lines) = total_row_values(lines_changed, total_cost);
        let (input, output, cache_write, cache_read) = empty_cost_columns();
        Self {
            session: String::new(),
            time_range: String::new(),
            duration: String::new(),
            model: "Total".to_string(),
            input,
            output,
            cache_write,
            cache_read,
            subtotal,
            lines,
        }
    }
}

/// Get the terminal width, with a fallback to 80 columns
fn get_terminal_width() -> usize {
    crossterm::terminal::size()
        .map(|(cols, _)| cols as usize)
        .unwrap_or(80)
}

/// Get a divider string of the specified width
fn divider(width: usize) -> String {
    "=".repeat(width)
}

/// Apply width-based configuration to a table (wrap vs truncate)
fn apply_width_config(table: &mut Table, term_width: usize) {
    if term_width >= MIN_WIDTH_FOR_WRAPPING {
        table.with(Width::wrap(term_width).keep_words(true));
    } else {
        table.with(Width::truncate(term_width));
    }
}

/// Appends one row per non-zero-cost model in display order, using `make_row`
/// to populate the leading identifying columns only on the first emitted row.
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

/// Centralizes grand-total accumulation so daily and session displays stay in
/// lockstep when we add or remove per-group summary columns.
fn summed_totals<Item>(
    items: &[Item],
    total: impl Fn(&Item) -> f64,
    lines: impl Fn(&Item) -> usize,
) -> (f64, usize) {
    items
        .iter()
        .fold((0.0_f64, 0usize), |(cost, total_lines), item| {
            (cost + total(item), total_lines + lines(item))
        })
}

/// Build cost rows from daily costs data
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
                        model_costs.input,
                        model_costs.output,
                        model_costs.cache_write,
                        model_costs.cache_read,
                        "",
                    )
                },
            );
        },
        |rows, costs| rows.push(CostRow::new_total_row(costs.lines_changed, costs.total())),
    )
}

/// Create a table with group separators using Theme API.
///
/// `total_row_indices` marks the last row of each group; separators are inserted between groups.
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

/// Format session ID (truncate to first 8 characters)
fn format_session_id(session_id: &str) -> String {
    if session_id.len() > 8 {
        session_id[..8].to_string()
    } else {
        session_id.to_string()
    }
}

/// Format time range (start → end)
fn format_time_range(start: DateTime<Utc>, end: DateTime<Utc>) -> String {
    let start_local = start.with_timezone(&Local);
    let end_local = end.with_timezone(&Local);
    format!(
        "{} → {}",
        start_local.format("%Y-%m-%d %H:%M"),
        end_local.format("%H:%M")
    )
}

/// Format duration in a readable way
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

/// Build session cost rows from session costs data
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
                        (
                            model_costs.input,
                            model_costs.output,
                            model_costs.cache_write,
                            model_costs.cache_read,
                        ),
                        "",
                    )
                },
            );
        },
        |rows, costs| {
            rows.push(SessionCostRow::new_total_row(
                costs.lines_changed,
                costs.total(),
            ))
        },
    )
}

/// Render a grouped cost table: title banner, table body, grand-total summary.
///
/// `rows` and `total_row_indices` typically come from `build_cost_rows` or
/// `build_session_cost_rows`. `grand_total` and `total_lines` are the footer totals.
fn render_cost_report<T: Tabled>(
    title: &str,
    rows: &[T],
    total_row_indices: &[usize],
    grand_total: f64,
    total_lines: usize,
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

    display_grand_total(grand_total, total_lines);
}

fn display_grouped_costs<Item, Row: Tabled>(
    title: &str,
    items: &[Item],
    build_rows: impl Fn(&[Item]) -> (Vec<Row>, Vec<usize>),
    total: impl Fn(&Item) -> f64,
    lines: impl Fn(&Item) -> usize,
) {
    let (rows, total_row_indices) = build_rows(items);
    let (grand_total, total_lines) = summed_totals(items, total, lines);
    render_cost_report(title, &rows, &total_row_indices, grand_total, total_lines);
}

/// Display session costs
fn display_session_costs(session_costs: &[SessionCosts]) {
    display_grouped_costs(
        "API Cost Report by Conversation",
        session_costs,
        build_session_cost_rows,
        SessionCosts::total,
        |costs| costs.lines_changed,
    );
}

/// Print the file-parsing summary (count of successes and failures) to stdout.
fn display_parsing_summary(files_parsed: usize, files_failed: usize) {
    let term_width = get_terminal_width();
    println!("\n{}", divider(term_width));
    println!("File Parsing Summary");
    println!("{}", divider(term_width));
    println!("  Successfully parsed: {} files", files_parsed);
    if files_failed > 0 {
        println!("  Failed to parse:     {} files", files_failed);
    }
    println!();
}

/// Print a WARNINGS section listing unknown models and their token totals, if any.
fn display_unknown_model_warnings(
    unknown_models: &HashSet<String>,
    total_unknown_tokens: &TokenCounts,
) {
    if !unknown_models.is_empty() {
        let term_width = get_terminal_width();
        println!("\n{}", divider(term_width));
        println!("WARNINGS");
        println!("{}", divider(term_width));
        println!(
            "\nUnknown models detected ({} unique):",
            unknown_models.len()
        );

        let mut models: Vec<_> = unknown_models.iter().collect();
        models.sort();

        for model in models {
            println!("  - {}", model);
        }

        let total_tokens = total_unknown_tokens.input_tokens
            + total_unknown_tokens.output_tokens
            + total_unknown_tokens.cache_write_tokens
            + total_unknown_tokens.cache_read_tokens;

        println!("\nTotal tokens from unknown models: {}", total_tokens);
        println!("  Input:       {}", total_unknown_tokens.input_tokens);
        println!("  Output:      {}", total_unknown_tokens.output_tokens);
        println!("  Cache Write: {}", total_unknown_tokens.cache_write_tokens);
        println!("  Cache Read:  {}", total_unknown_tokens.cache_read_tokens);
        println!("\n⚠️  These tokens are NOT included in the cost calculations above.");
        println!("{}", divider(term_width));
    }
}

/// Run the API pricing analysis by session
pub async fn run_by_session(dir: &Path, filter: &TimeRangeFilter) -> miette::Result<()> {
    let result = analyzer::analyze_directory_by_session(dir, filter).await?;

    display_parsing_summary(result.files_parsed, result.files_failed);

    if result.session_costs.is_empty() {
        println!("\nNo usage data found.");
        return Ok(());
    }

    display_session_costs(&result.session_costs);
    display_unknown_model_warnings(&result.unknown_models, &result.total_unknown_tokens);

    Ok(())
}

/// Run the API pricing analysis on a directory
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

    display_parsing_summary(result.files_parsed, result.files_failed);

    if result.daily_costs.is_empty() {
        println!("\nNo usage data found.");
        return Ok(());
    }

    display_costs(&result.daily_costs);
    display_unknown_model_warnings(&result.unknown_models, &result.total_unknown_tokens);

    Ok(())
}

fn display_costs(daily_costs: &[DailyCosts]) {
    display_grouped_costs(
        "API Cost Report",
        daily_costs,
        build_cost_rows,
        DailyCosts::total,
        |costs| costs.lines_changed,
    );
}

fn display_grand_total(grand_total: f64, total_lines: usize) {
    let term_width = get_terminal_width();
    println!("{}", divider(term_width));
    println!("Summary");
    println!("{}", divider(term_width));
    println!();

    let row = GrandTotalRow::new(grand_total, total_lines);
    let mut table = Table::new(vec![row]);

    table.with(Style::rounded());
    table.with(Modify::new(Rows::first()).with(Alignment::center()));
    apply_width_config(&mut table, term_width);

    println!("{}", table);
    println!("{}", divider(term_width));
}

#[cfg(test)]
mod tests;
