mod analyzer;
#[cfg(test)]
mod analyzer_tests;
mod line_counter;
mod pricing;

use std::path::Path;

use analyzer::{AnalysisResult, DailyCosts, SessionAnalysisResult, SessionCosts};
use chrono::{DateTime, Local, Utc};
use pricing::TokenCosts;

// Re-export DateTimezone for use in main.rs
pub use analyzer::DateTimezone;

// Type alias for cost components tuple
type CostComponents = (f64, f64, f64, f64); // (input, output, cache_write, cache_read)

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
        Self {
            date: date.to_string(),
            model: model.to_string(),
            input: format!("${:.4}", input),
            output: format!("${:.4}", output),
            cache_write: format!("${:.4}", cache_write),
            cache_read: format!("${:.4}", cache_read),
            subtotal: format!("${:.4}", input + output + cache_write + cache_read),
            lines: lines.to_string(),
        }
    }

    /// Creates a total row for displaying daily summary.
    ///
    /// Uses empty strings for individual cost columns because showing per-model
    /// costs in a total row would be misleading - users might confuse them for
    /// additional costs rather than components of the subtotal.
    fn new_total_row(lines_changed: usize, total_cost: f64) -> Self {
        Self {
            date: String::new(),
            model: "Total".to_string(),
            input: String::new(),
            output: String::new(),
            cache_write: String::new(),
            cache_read: String::new(),
            subtotal: format!("${:.4}", total_cost),
            lines: lines_changed.to_string(),
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
            grand_total: format!("${:.4}", grand_total),
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
        let (input, output, cache_write, cache_read) = costs;
        Self {
            session: session.to_string(),
            time_range: time_range.to_string(),
            duration: duration.to_string(),
            model: model.to_string(),
            input: format!("${:.4}", input),
            output: format!("${:.4}", output),
            cache_write: format!("${:.4}", cache_write),
            cache_read: format!("${:.4}", cache_read),
            subtotal: format!("${:.4}", input + output + cache_write + cache_read),
            lines: lines.to_string(),
        }
    }

    /// Creates a total row for displaying session summary
    fn new_total_row(lines_changed: usize, total_cost: f64) -> Self {
        Self {
            session: String::new(),
            time_range: String::new(),
            duration: String::new(),
            model: "Total".to_string(),
            input: String::new(),
            output: String::new(),
            cache_write: String::new(),
            cache_read: String::new(),
            subtotal: format!("${:.4}", total_cost),
            lines: lines_changed.to_string(),
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

/// Returns model costs in display order: Opus, Sonnet, Haiku
fn iter_model_costs(costs: &DailyCosts) -> impl Iterator<Item = (&'static str, &TokenCosts)> {
    [
        ("Opus", &costs.opus_costs),
        ("Sonnet", &costs.sonnet_costs),
        ("Haiku", &costs.haiku_costs),
    ]
    .into_iter()
}

/// Build cost rows from daily costs data
fn build_cost_rows(daily_costs: &[DailyCosts]) -> (Vec<CostRow>, Vec<usize>) {
    let mut rows = Vec::new();
    let mut total_row_indices = Vec::new();

    for costs in daily_costs {
        let date_str = costs.date.to_string();
        let mut first_row = true;

        // Iterate over models in priority order
        for (model_name, model_costs) in iter_model_costs(costs) {
            if model_costs.total() > 0.0 {
                rows.push(CostRow::new(
                    if first_row { &date_str } else { "" },
                    model_name,
                    model_costs.input,
                    model_costs.output,
                    model_costs.cache_write,
                    model_costs.cache_read,
                    "",
                ));
                first_row = false;
            }
        }

        // Add total row for this day
        rows.push(CostRow::new_total_row(costs.lines_changed, costs.total()));
        total_row_indices.push(rows.len() - 1);
    }

    (rows, total_row_indices)
}

/// Create a table with day separators using Theme API
fn create_styled_table(rows: &[CostRow], total_row_indices: &[usize]) -> Table {
    let mut table = Table::new(rows);

    // Apply base style
    let mut theme = Theme::from_style(Style::rounded());

    // Add horizontal separators after each day (except the last)
    if total_row_indices.len() > 1 {
        let separator_line = HorizontalLine::full('─', '┼', '├', '┤');

        // Build separators dynamically - works for ANY number of days
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

/// Returns model costs in display order: Opus, Sonnet, Haiku
fn iter_session_model_costs(
    costs: &SessionCosts,
) -> impl Iterator<Item = (&'static str, &TokenCosts)> {
    [
        ("Opus", &costs.opus_costs),
        ("Sonnet", &costs.sonnet_costs),
        ("Haiku", &costs.haiku_costs),
    ]
    .into_iter()
}

/// Build session cost rows from session costs data
fn build_session_cost_rows(session_costs: &[SessionCosts]) -> (Vec<SessionCostRow>, Vec<usize>) {
    let mut rows = Vec::new();
    let mut total_row_indices = Vec::new();

    for costs in session_costs {
        let session_id = format_session_id(&costs.session_id);
        let time_range = format_time_range(costs.start_time, costs.end_time);
        let duration = format_duration(costs.duration_minutes());
        let mut first_row = true;

        // Iterate over models in priority order
        for (model_name, model_costs) in iter_session_model_costs(costs) {
            if model_costs.total() > 0.0 {
                rows.push(SessionCostRow::new(
                    if first_row { &session_id } else { "" },
                    if first_row { &time_range } else { "" },
                    if first_row { &duration } else { "" },
                    model_name,
                    (
                        model_costs.input,
                        model_costs.output,
                        model_costs.cache_write,
                        model_costs.cache_read,
                    ),
                    "",
                ));
                first_row = false;
            }
        }

        // Add total row for this session
        rows.push(SessionCostRow::new_total_row(
            costs.lines_changed,
            costs.total(),
        ));
        total_row_indices.push(rows.len() - 1);
    }

    (rows, total_row_indices)
}

/// Display session costs
fn display_session_costs(session_costs: &[SessionCosts]) {
    let term_width = get_terminal_width();
    println!("{}", divider(term_width));
    println!("API Cost Report by Conversation");
    println!("{}", divider(term_width));
    println!();

    // Build rows and track totals
    let mut grand_total = 0.0;
    let mut total_lines = 0usize;

    let (rows, total_row_indices) = build_session_cost_rows(session_costs);

    // Calculate grand totals
    for costs in session_costs {
        grand_total += costs.total();
        total_lines += costs.lines_changed;
    }

    // Create and configure table
    let mut table = create_styled_table_sessions(&rows, &total_row_indices);
    apply_width_config(&mut table, term_width);

    println!("{}", table);
    println!();

    display_grand_total(grand_total, total_lines);
}

/// Create a table with session separators using Theme API
fn create_styled_table_sessions(rows: &[SessionCostRow], total_row_indices: &[usize]) -> Table {
    let mut table = Table::new(rows);

    // Apply base style
    let mut theme = Theme::from_style(Style::rounded());

    // Add horizontal separators after each session (except the last)
    if total_row_indices.len() > 1 {
        let separator_line = HorizontalLine::full('─', '┼', '├', '┤');

        // Build separators dynamically - works for ANY number of sessions
        for &idx in &total_row_indices[..total_row_indices.len() - 1] {
            // +1 for header row, +1 to place separator after the total row
            theme.insert_horizontal_line(idx + 2, separator_line);
        }
    }

    table.with(theme);
    table.with(Modify::new(Rows::first()).with(Alignment::center()));

    table
}

fn display_session_analysis_summary(result: &SessionAnalysisResult) {
    let term_width = get_terminal_width();
    println!("\n{}", divider(term_width));
    println!("File Parsing Summary");
    println!("{}", divider(term_width));
    println!("  Successfully parsed: {} files", result.files_parsed);
    if result.files_failed > 0 {
        println!("  Failed to parse:     {} files", result.files_failed);
    }
    println!();
}

fn display_session_warnings(result: &SessionAnalysisResult) {
    if !result.unknown_models.is_empty() {
        let term_width = get_terminal_width();
        println!("\n{}", divider(term_width));
        println!("WARNINGS");
        println!("{}", divider(term_width));
        println!(
            "\nUnknown models detected ({} unique):",
            result.unknown_models.len()
        );

        let mut models: Vec<_> = result.unknown_models.iter().collect();
        models.sort();

        for model in models {
            println!("  - {}", model);
        }

        let total_tokens = result.total_unknown_tokens.input_tokens
            + result.total_unknown_tokens.output_tokens
            + result.total_unknown_tokens.cache_write_tokens
            + result.total_unknown_tokens.cache_read_tokens;

        println!("\nTotal tokens from unknown models: {}", total_tokens);
        println!(
            "  Input:       {}",
            result.total_unknown_tokens.input_tokens
        );
        println!(
            "  Output:      {}",
            result.total_unknown_tokens.output_tokens
        );
        println!(
            "  Cache Write: {}",
            result.total_unknown_tokens.cache_write_tokens
        );
        println!(
            "  Cache Read:  {}",
            result.total_unknown_tokens.cache_read_tokens
        );
        println!("\n⚠️  These tokens are NOT included in the cost calculations above.");
        println!("{}", divider(term_width));
    }
}

/// Run the API pricing analysis by session
pub async fn run_by_session(dir: &Path) -> miette::Result<()> {
    let result = analyzer::analyze_directory_by_session(dir).await?;

    display_session_analysis_summary(&result);

    if result.session_costs.is_empty() {
        println!("\nNo usage data found.");
        return Ok(());
    }

    display_session_costs(&result.session_costs);
    display_session_warnings(&result);

    Ok(())
}

/// Run the API pricing analysis on a directory
pub async fn run(dir: &Path, timezone: DateTimezone, by_conversation: bool) -> miette::Result<()> {
    if by_conversation {
        return run_by_session(dir).await;
    }

    let result = analyzer::analyze_directory(dir, timezone).await?;

    display_analysis_summary(&result);

    if result.daily_costs.is_empty() {
        println!("\nNo usage data found.");
        return Ok(());
    }

    display_costs(&result.daily_costs);
    display_warnings(&result);

    Ok(())
}

fn display_analysis_summary(result: &AnalysisResult) {
    let term_width = get_terminal_width();
    println!("\n{}", divider(term_width));
    println!("File Parsing Summary");
    println!("{}", divider(term_width));
    println!("  Successfully parsed: {} files", result.files_parsed);
    if result.files_failed > 0 {
        println!("  Failed to parse:     {} files", result.files_failed);
    }
    println!();
}

fn display_costs(daily_costs: &[DailyCosts]) {
    let term_width = get_terminal_width();
    println!("{}", divider(term_width));
    println!("API Cost Report");
    println!("{}", divider(term_width));
    println!();

    // Build rows and track totals
    let mut grand_total = 0.0;
    let mut total_lines = 0usize;

    let (rows, total_row_indices) = build_cost_rows(daily_costs);

    // Calculate grand totals
    for costs in daily_costs {
        grand_total += costs.total();
        total_lines += costs.lines_changed;
    }

    // Create and configure table
    let mut table = create_styled_table(&rows, &total_row_indices);
    apply_width_config(&mut table, term_width);

    println!("{}", table);
    println!();

    display_grand_total(grand_total, total_lines);
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

fn display_warnings(result: &AnalysisResult) {
    if !result.unknown_models.is_empty() {
        let term_width = get_terminal_width();
        println!("\n{}", divider(term_width));
        println!("WARNINGS");
        println!("{}", divider(term_width));
        println!(
            "\nUnknown models detected ({} unique):",
            result.unknown_models.len()
        );

        let mut models: Vec<_> = result.unknown_models.iter().collect();
        models.sort();

        for model in models {
            println!("  - {}", model);
        }

        let total_tokens = result.total_unknown_tokens.input_tokens
            + result.total_unknown_tokens.output_tokens
            + result.total_unknown_tokens.cache_write_tokens
            + result.total_unknown_tokens.cache_read_tokens;

        println!("\nTotal tokens from unknown models: {}", total_tokens);
        println!(
            "  Input:       {}",
            result.total_unknown_tokens.input_tokens
        );
        println!(
            "  Output:      {}",
            result.total_unknown_tokens.output_tokens
        );
        println!(
            "  Cache Write: {}",
            result.total_unknown_tokens.cache_write_tokens
        );
        println!(
            "  Cache Read:  {}",
            result.total_unknown_tokens.cache_read_tokens
        );
        println!("\n⚠️  These tokens are NOT included in the cost calculations above.");
        println!("{}", divider(term_width));
    }
}

#[cfg(test)]
mod tests;
