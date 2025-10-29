mod analyzer;
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
mod tests {
    use super::*;
    use crate::api_pricing::{
        analyzer::{AnalysisResult, DailyCosts},
        pricing::{TokenCosts, TokenCounts},
    };
    use chrono::NaiveDate;
    use std::collections::HashSet;

    #[test]
    fn test_display_analysis_summary_no_failures() {
        let result = AnalysisResult {
            daily_costs: vec![],
            unknown_models: HashSet::new(),
            total_unknown_tokens: TokenCounts::default(),
            files_parsed: 5,
            files_failed: 0,
        };

        display_analysis_summary(&result);
    }

    #[test]
    fn test_display_analysis_summary_with_failures() {
        let result = AnalysisResult {
            daily_costs: vec![],
            unknown_models: HashSet::new(),
            total_unknown_tokens: TokenCounts::default(),
            files_parsed: 3,
            files_failed: 2,
        };

        display_analysis_summary(&result);
    }

    #[test]
    fn test_display_costs_empty() {
        let daily_costs = vec![];
        display_costs(&daily_costs);
    }

    #[test]
    fn test_display_costs_single_day_sonnet_only() {
        let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
        let daily_costs = vec![DailyCosts {
            date,
            sonnet_costs: TokenCosts {
                input: 1.0,
                output: 2.0,
                cache_write: 0.5,
                cache_read: 0.25,
            },
            haiku_costs: TokenCosts::default(),
            opus_costs: TokenCosts::default(),
            lines_changed: 145,
        }];

        display_costs(&daily_costs);
    }

    #[test]
    fn test_display_costs_single_day_opus_only() {
        let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
        let daily_costs = vec![DailyCosts {
            date,
            opus_costs: TokenCosts {
                input: 15.0,
                output: 75.0,
                cache_write: 18.75,
                cache_read: 1.5,
            },
            sonnet_costs: TokenCosts::default(),
            haiku_costs: TokenCosts::default(),
            lines_changed: 0,
        }];

        display_costs(&daily_costs);
    }

    #[test]
    fn test_display_costs_single_day_both_models() {
        let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
        let daily_costs = vec![DailyCosts {
            date,
            sonnet_costs: TokenCosts {
                input: 1.0,
                output: 2.0,
                cache_write: 0.5,
                cache_read: 0.25,
            },
            haiku_costs: TokenCosts {
                input: 0.5,
                output: 1.0,
                cache_write: 0.25,
                cache_read: 0.1,
            },
            opus_costs: TokenCosts::default(),
            lines_changed: 0,
        }];

        display_costs(&daily_costs);
    }

    #[test]
    fn test_display_costs_multiple_days() {
        let daily_costs = vec![
            DailyCosts {
                date: NaiveDate::from_ymd_opt(2025, 10, 23).unwrap(),
                sonnet_costs: TokenCosts {
                    input: 1.0,
                    output: 2.0,
                    cache_write: 0.5,
                    cache_read: 0.25,
                },
                haiku_costs: TokenCosts::default(),
                opus_costs: TokenCosts::default(),
                lines_changed: 100,
            },
            DailyCosts {
                date: NaiveDate::from_ymd_opt(2025, 10, 24).unwrap(),
                sonnet_costs: TokenCosts::default(),
                haiku_costs: TokenCosts {
                    input: 0.5,
                    output: 1.0,
                    cache_write: 0.25,
                    cache_read: 0.1,
                },
                opus_costs: TokenCosts::default(),
                lines_changed: 50,
            },
        ];

        display_costs(&daily_costs);
    }

    #[test]
    fn test_display_costs_three_days() {
        // Tests num_separators = 2 match branch
        let daily_costs = vec![
            DailyCosts {
                date: NaiveDate::from_ymd_opt(2025, 10, 23).unwrap(),
                sonnet_costs: TokenCosts {
                    input: 1.0,
                    output: 1.0,
                    cache_write: 0.0,
                    cache_read: 0.0,
                },
                haiku_costs: TokenCosts::default(),
                opus_costs: TokenCosts::default(),
                lines_changed: 100,
            },
            DailyCosts {
                date: NaiveDate::from_ymd_opt(2025, 10, 24).unwrap(),
                haiku_costs: TokenCosts {
                    input: 0.5,
                    output: 0.5,
                    cache_write: 0.0,
                    cache_read: 0.0,
                },
                sonnet_costs: TokenCosts::default(),
                opus_costs: TokenCosts::default(),
                lines_changed: 50,
            },
            DailyCosts {
                date: NaiveDate::from_ymd_opt(2025, 10, 25).unwrap(),
                opus_costs: TokenCosts {
                    input: 2.0,
                    output: 2.0,
                    cache_write: 0.0,
                    cache_read: 0.0,
                },
                sonnet_costs: TokenCosts::default(),
                haiku_costs: TokenCosts::default(),
                lines_changed: 75,
            },
        ];

        display_costs(&daily_costs);
    }

    #[test]
    fn test_display_costs_day_with_all_three_models() {
        // Verify separator placement when multiple models create multiple rows per day
        let daily_costs = vec![
            DailyCosts {
                date: NaiveDate::from_ymd_opt(2025, 10, 23).unwrap(),
                opus_costs: TokenCosts {
                    input: 1.0,
                    output: 1.0,
                    cache_write: 0.0,
                    cache_read: 0.0,
                },
                sonnet_costs: TokenCosts {
                    input: 2.0,
                    output: 2.0,
                    cache_write: 0.0,
                    cache_read: 0.0,
                },
                haiku_costs: TokenCosts {
                    input: 0.5,
                    output: 0.5,
                    cache_write: 0.0,
                    cache_read: 0.0,
                },
                lines_changed: 100,
            },
            DailyCosts {
                date: NaiveDate::from_ymd_opt(2025, 10, 24).unwrap(),
                sonnet_costs: TokenCosts {
                    input: 1.0,
                    output: 1.0,
                    cache_write: 0.0,
                    cache_read: 0.0,
                },
                haiku_costs: TokenCosts::default(),
                opus_costs: TokenCosts::default(),
                lines_changed: 50,
            },
        ];

        display_costs(&daily_costs);
    }

    #[test]
    fn test_display_costs_ten_days() {
        // Tests num_separators = 9 (near end of explicit match branches)
        let daily_costs: Vec<_> = (1..=10)
            .map(|day| DailyCosts {
                date: NaiveDate::from_ymd_opt(2025, 10, day).unwrap(),
                sonnet_costs: TokenCosts {
                    input: 1.0,
                    output: 1.0,
                    cache_write: 0.0,
                    cache_read: 0.0,
                },
                haiku_costs: TokenCosts::default(),
                opus_costs: TokenCosts::default(),
                lines_changed: 10,
            })
            .collect();

        display_costs(&daily_costs);
    }

    #[test]
    fn test_display_costs_thirty_one_days() {
        // Tests num_separators = 30 (last explicit match branch)
        let daily_costs: Vec<_> = (1..=31)
            .map(|day| DailyCosts {
                date: NaiveDate::from_ymd_opt(2025, 10, day).unwrap(),
                sonnet_costs: TokenCosts {
                    input: 1.0,
                    output: 1.0,
                    cache_write: 0.0,
                    cache_read: 0.0,
                },
                haiku_costs: TokenCosts::default(),
                opus_costs: TokenCosts::default(),
                lines_changed: 10,
            })
            .collect();

        display_costs(&daily_costs);
    }

    #[test]
    fn test_display_costs_thirty_two_days_uses_fallback() {
        // Tests fallback branch for >31 days
        // Use January (31 days) + 1 day from February
        let mut daily_costs: Vec<_> = (1..=31)
            .map(|day| DailyCosts {
                date: NaiveDate::from_ymd_opt(2025, 1, day).unwrap(),
                sonnet_costs: TokenCosts {
                    input: 1.0,
                    output: 1.0,
                    cache_write: 0.0,
                    cache_read: 0.0,
                },
                haiku_costs: TokenCosts::default(),
                opus_costs: TokenCosts::default(),
                lines_changed: 10,
            })
            .collect();

        // Add one more day from February to get 32 total days
        daily_costs.push(DailyCosts {
            date: NaiveDate::from_ymd_opt(2025, 2, 1).unwrap(),
            sonnet_costs: TokenCosts {
                input: 1.0,
                output: 1.0,
                cache_write: 0.0,
                cache_read: 0.0,
            },
            haiku_costs: TokenCosts::default(),
            opus_costs: TokenCosts::default(),
            lines_changed: 10,
        });

        display_costs(&daily_costs);
    }

    #[test]
    fn test_display_warnings_no_unknown_models() {
        let result = AnalysisResult {
            daily_costs: vec![],
            unknown_models: HashSet::new(),
            total_unknown_tokens: TokenCounts::default(),
            files_parsed: 0,
            files_failed: 0,
        };

        display_warnings(&result);
    }

    #[test]
    fn test_display_warnings_with_unknown_models() {
        let mut unknown_models = HashSet::new();
        unknown_models.insert("claude-opus-4".to_string());
        unknown_models.insert("gpt-4".to_string());

        let result = AnalysisResult {
            daily_costs: vec![],
            unknown_models,
            total_unknown_tokens: TokenCounts {
                input_tokens: 1000,
                output_tokens: 500,
                cache_write_tokens: 100,
                cache_read_tokens: 50,
            },
            files_parsed: 0,
            files_failed: 0,
        };

        display_warnings(&result);
    }

    #[test]
    fn test_cost_row_new_formats_currency() {
        let row = CostRow::new("2025-10-23", "Sonnet", 1.2345, 2.3456, 0.5, 0.25, "100");

        assert_eq!(row.date, "2025-10-23");
        assert_eq!(row.model, "Sonnet");
        assert_eq!(row.input, "$1.2345");
        assert_eq!(row.output, "$2.3456");
        assert_eq!(row.cache_write, "$0.5000");
        assert_eq!(row.cache_read, "$0.2500");
        assert_eq!(row.subtotal, "$4.3301");
        assert_eq!(row.lines, "100");
    }

    #[test]
    fn test_cost_row_new_calculates_subtotal() {
        let row = CostRow::new("2025-10-23", "Test", 1.0, 2.0, 0.5, 0.25, "");
        assert_eq!(row.subtotal, "$3.7500");
    }

    #[test]
    fn test_cost_row_new_handles_zero_costs() {
        let row = CostRow::new("", "Haiku", 0.0, 0.0, 0.0, 0.0, "");
        assert_eq!(row.subtotal, "$0.0000");
    }

    #[test]
    fn test_cost_row_new_total_row_formats_correctly() {
        let row = CostRow::new_total_row(1234, 56.789);

        assert_eq!(row.date, "");
        assert_eq!(row.model, "Total");
        assert_eq!(row.input, "");
        assert_eq!(row.output, "");
        assert_eq!(row.cache_write, "");
        assert_eq!(row.cache_read, "");
        assert_eq!(row.subtotal, "$56.7890");
        assert_eq!(row.lines, "1234");
    }

    #[test]
    fn test_cost_row_new_total_row_zero_values() {
        let row = CostRow::new_total_row(0, 0.0);

        assert_eq!(row.date, "");
        assert_eq!(row.model, "Total");
        assert_eq!(row.subtotal, "$0.0000");
        assert_eq!(row.lines, "0");
    }

    #[test]
    fn test_cost_row_new_total_row_large_numbers() {
        let row = CostRow::new_total_row(999999, 12345.6789);

        assert_eq!(row.subtotal, "$12345.6789");
        assert_eq!(row.lines, "999999");
    }

    #[test]
    fn test_divider_generates_correct_length() {
        assert_eq!(divider(0), "");
        assert_eq!(divider(1), "=");
        assert_eq!(divider(5), "=====");
        assert_eq!(divider(100).len(), 100);
    }

    #[test]
    fn test_grand_total_row_new_formats_currency() {
        let row = GrandTotalRow::new(143.7082, 13010);

        assert_eq!(row.grand_total, "$143.7082");
        assert_eq!(row.total_lines_changed, "13010");
    }

    #[test]
    fn test_grand_total_row_new_handles_zero() {
        let row = GrandTotalRow::new(0.0, 0);

        assert_eq!(row.grand_total, "$0.0000");
        assert_eq!(row.total_lines_changed, "0");
    }

    #[test]
    fn test_grand_total_row_new_handles_large_numbers() {
        let row = GrandTotalRow::new(99999.9999, 999999);

        assert_eq!(row.grand_total, "$99999.9999");
        assert_eq!(row.total_lines_changed, "999999");
    }

    // Smoke tests for display_grand_total - verify the function doesn't panic
    // with various input values. These tests don't assert on output format since
    // that would require capturing stdout or refactoring the function signature.

    #[test]
    fn test_display_grand_total_zero_values() {
        display_grand_total(0.0, 0);
    }

    #[test]
    fn test_display_grand_total_typical_values() {
        display_grand_total(143.7082, 13010);
    }

    #[test]
    fn test_display_grand_total_large_values() {
        display_grand_total(12345.6789, 999999);
    }

    #[test]
    fn test_display_grand_total_small_values() {
        display_grand_total(0.0001, 1);
    }

    // Unit tests for helper functions extracted during refactoring

    #[test]
    fn test_build_cost_rows_single_model_per_day() {
        let daily_costs = vec![DailyCosts {
            date: NaiveDate::from_ymd_opt(2025, 10, 23).unwrap(),
            sonnet_costs: TokenCosts {
                input: 1.0,
                output: 2.0,
                cache_write: 0.0,
                cache_read: 0.0,
            },
            haiku_costs: TokenCosts::default(),
            opus_costs: TokenCosts::default(),
            lines_changed: 100,
        }];

        let (rows, total_row_indices) = build_cost_rows(&daily_costs);

        assert_eq!(rows.len(), 2); // 1 model row + 1 total row
        assert_eq!(total_row_indices, vec![1]);
        assert_eq!(rows[0].date, "2025-10-23");
        assert_eq!(rows[0].model, "Sonnet");
        assert_eq!(rows[1].model, "Total");
        assert_eq!(rows[1].date, ""); // Total row has empty date
    }

    #[test]
    fn test_build_cost_rows_multiple_models_same_day() {
        let daily_costs = vec![DailyCosts {
            date: NaiveDate::from_ymd_opt(2025, 10, 23).unwrap(),
            opus_costs: TokenCosts {
                input: 1.0,
                output: 1.0,
                cache_write: 0.0,
                cache_read: 0.0,
            },
            sonnet_costs: TokenCosts {
                input: 2.0,
                output: 2.0,
                cache_write: 0.0,
                cache_read: 0.0,
            },
            haiku_costs: TokenCosts {
                input: 0.5,
                output: 0.5,
                cache_write: 0.0,
                cache_read: 0.0,
            },
            lines_changed: 200,
        }];

        let (rows, total_row_indices) = build_cost_rows(&daily_costs);

        // Should have: Opus (with date), Sonnet (no date), Haiku (no date), Total
        assert_eq!(rows.len(), 4);
        assert_eq!(total_row_indices, vec![3]); // Total row is at index 3
        assert_eq!(rows[0].date, "2025-10-23"); // First row shows date
        assert_eq!(rows[0].model, "Opus");
        assert_eq!(rows[1].date, ""); // Subsequent rows empty date
        assert_eq!(rows[1].model, "Sonnet");
        assert_eq!(rows[2].date, "");
        assert_eq!(rows[2].model, "Haiku");
        assert_eq!(rows[3].model, "Total");
    }

    #[test]
    fn test_build_cost_rows_only_zero_cost_models() {
        let daily_costs = vec![DailyCosts {
            date: NaiveDate::from_ymd_opt(2025, 10, 23).unwrap(),
            opus_costs: TokenCosts::default(),
            sonnet_costs: TokenCosts::default(),
            haiku_costs: TokenCosts::default(),
            lines_changed: 0,
        }];

        let (rows, total_row_indices) = build_cost_rows(&daily_costs);

        // Should only have total row (no model rows since all costs are 0)
        assert_eq!(rows.len(), 1);
        assert_eq!(total_row_indices, vec![0]);
        assert_eq!(rows[0].model, "Total");
    }

    #[test]
    fn test_build_cost_rows_multiple_days() {
        let daily_costs = vec![
            DailyCosts {
                date: NaiveDate::from_ymd_opt(2025, 10, 23).unwrap(),
                sonnet_costs: TokenCosts {
                    input: 1.0,
                    output: 1.0,
                    cache_write: 0.0,
                    cache_read: 0.0,
                },
                opus_costs: TokenCosts::default(),
                haiku_costs: TokenCosts::default(),
                lines_changed: 50,
            },
            DailyCosts {
                date: NaiveDate::from_ymd_opt(2025, 10, 24).unwrap(),
                haiku_costs: TokenCosts {
                    input: 0.5,
                    output: 0.5,
                    cache_write: 0.0,
                    cache_read: 0.0,
                },
                sonnet_costs: TokenCosts::default(),
                opus_costs: TokenCosts::default(),
                lines_changed: 25,
            },
        ];

        let (rows, total_row_indices) = build_cost_rows(&daily_costs);

        // Day 1: Sonnet + Total, Day 2: Haiku + Total
        assert_eq!(rows.len(), 4);
        assert_eq!(total_row_indices, vec![1, 3]);
        assert_eq!(rows[0].date, "2025-10-23");
        assert_eq!(rows[2].date, "2025-10-24");
    }

    #[test]
    fn test_build_cost_rows_empty() {
        let daily_costs = vec![];
        let (rows, total_row_indices) = build_cost_rows(&daily_costs);

        assert!(rows.is_empty());
        assert!(total_row_indices.is_empty());
    }

    #[test]
    fn test_build_cost_rows_filters_zero_cost_models() {
        let daily_costs = vec![DailyCosts {
            date: NaiveDate::from_ymd_opt(2025, 10, 23).unwrap(),
            opus_costs: TokenCosts {
                input: 1.0,
                output: 1.0,
                cache_write: 0.0,
                cache_read: 0.0,
            },
            sonnet_costs: TokenCosts::default(), // Zero - should be filtered out
            haiku_costs: TokenCosts {
                input: 0.5,
                output: 0.5,
                cache_write: 0.0,
                cache_read: 0.0,
            },
            lines_changed: 100,
        }];

        let (rows, total_row_indices) = build_cost_rows(&daily_costs);

        // Should have Opus, Haiku, Total (NOT Sonnet since it has zero costs)
        assert_eq!(rows.len(), 3);
        assert_eq!(total_row_indices, vec![2]);
        assert_eq!(rows[0].model, "Opus");
        assert_eq!(rows[1].model, "Haiku"); // Sonnet skipped!
        assert_eq!(rows[2].model, "Total");
    }

    #[test]
    fn test_build_cost_rows_date_only_on_first_nonzero_model() {
        let daily_costs = vec![DailyCosts {
            date: NaiveDate::from_ymd_opt(2025, 10, 23).unwrap(),
            opus_costs: TokenCosts::default(), // Zero - will be skipped
            sonnet_costs: TokenCosts {
                input: 1.0,
                output: 1.0,
                cache_write: 0.0,
                cache_read: 0.0,
            },
            haiku_costs: TokenCosts {
                input: 0.5,
                output: 0.5,
                cache_write: 0.0,
                cache_read: 0.0,
            },
            lines_changed: 100,
        }];

        let (rows, _) = build_cost_rows(&daily_costs);

        // Opus is skipped, so Sonnet is first row and should show the date
        assert_eq!(rows[0].date, "2025-10-23"); // Date on first non-zero (Sonnet)
        assert_eq!(rows[0].model, "Sonnet");
        assert_eq!(rows[1].date, ""); // Empty on second (Haiku)
        assert_eq!(rows[1].model, "Haiku");
        assert_eq!(rows[2].date, ""); // Empty on total
        assert_eq!(rows[2].model, "Total");
    }

    #[test]
    fn test_create_styled_table_no_separators_single_day() {
        let rows = vec![
            CostRow::new("2025-10-23", "Sonnet", 1.0, 1.0, 0.0, 0.0, ""),
            CostRow::new_total_row(100, 2.0),
        ];
        let total_row_indices = vec![1];

        let table = create_styled_table(&rows, &total_row_indices);

        // Verify it doesn't panic and can be converted to string
        let output = table.to_string();
        assert!(!output.is_empty());
        assert!(output.contains("Sonnet"));
        assert!(output.contains("Total"));
    }

    #[test]
    fn test_create_styled_table_with_separators_multiple_days() {
        let rows = vec![
            CostRow::new("2025-10-23", "Sonnet", 1.0, 1.0, 0.0, 0.0, ""),
            CostRow::new_total_row(100, 2.0),
            CostRow::new("2025-10-24", "Haiku", 0.5, 0.5, 0.0, 0.0, ""),
            CostRow::new_total_row(50, 1.0),
        ];
        let total_row_indices = vec![1, 3];

        let table = create_styled_table(&rows, &total_row_indices);
        let output = table.to_string();

        assert!(!output.is_empty());
        assert!(output.contains("2025-10-23"));
        assert!(output.contains("2025-10-24"));
    }

    #[test]
    fn test_create_styled_table_empty_rows() {
        let rows: Vec<CostRow> = vec![];
        let total_row_indices: Vec<usize> = vec![];

        let table = create_styled_table(&rows, &total_row_indices);
        let output = table.to_string();

        // Should at least have headers
        assert!(!output.is_empty());
    }

    #[test]
    fn test_create_styled_table_separator_placement() {
        let rows = vec![
            CostRow::new("2025-10-23", "Sonnet", 1.0, 1.0, 0.0, 0.0, ""),
            CostRow::new_total_row(100, 2.0),
            CostRow::new("2025-10-24", "Haiku", 0.5, 0.5, 0.0, 0.0, ""),
            CostRow::new_total_row(50, 1.0),
        ];
        let total_row_indices = vec![1, 3];

        let table = create_styled_table(&rows, &total_row_indices);
        let output = table.to_string();
        let lines: Vec<&str> = output.lines().collect();

        // Verify separators appear in the output by checking for lines with
        // the junction character (┼) which appears in day separators
        let has_day_separator = lines.iter().any(|line| line.contains('┼'));

        assert!(
            has_day_separator,
            "Expected to find day separator (with ┼ character) in table output"
        );

        // Verify the dates appear in the output
        assert!(output.contains("2025-10-23"));
        assert!(output.contains("2025-10-24"));

        // Verify both models appear
        assert!(output.contains("Sonnet"));
        assert!(output.contains("Haiku"));
    }

    // Smoke tests for apply_width_config - verify function doesn't panic at
    // various width boundaries. Actual wrapping/truncation behavior is tested
    // by the tabled library and verified through integration tests.

    #[test]
    fn test_apply_width_config_wraps_at_threshold() {
        let rows = vec![CostRow::new(
            "2025-10-23",
            "Sonnet",
            1.0,
            1.0,
            0.0,
            0.0,
            "100",
        )];
        let mut table = Table::new(&rows);

        apply_width_config(&mut table, 100); // Exactly at MIN_WIDTH_FOR_WRAPPING

        // Smoke test: verify it doesn't panic
        let output = table.to_string();
        assert!(!output.is_empty());
    }

    #[test]
    fn test_apply_width_config_truncates_below_threshold() {
        let rows = vec![CostRow::new(
            "2025-10-23",
            "Sonnet",
            1.0,
            1.0,
            0.0,
            0.0,
            "100",
        )];
        let mut table = Table::new(&rows);

        apply_width_config(&mut table, 99); // Just below MIN_WIDTH_FOR_WRAPPING

        // Smoke test: verify it doesn't panic
        let output = table.to_string();
        assert!(!output.is_empty());
    }

    #[test]
    fn test_apply_width_config_wraps_above_threshold() {
        let rows = vec![CostRow::new(
            "2025-10-23",
            "Sonnet",
            1.0,
            1.0,
            0.0,
            0.0,
            "100",
        )];
        let mut table = Table::new(&rows);

        apply_width_config(&mut table, 101); // Just above MIN_WIDTH_FOR_WRAPPING

        // Smoke test: verify it doesn't panic
        let output = table.to_string();
        assert!(!output.is_empty());
    }

    #[test]
    fn test_iter_model_costs_returns_correct_order() {
        let costs = DailyCosts {
            date: NaiveDate::from_ymd_opt(2025, 10, 23).unwrap(),
            opus_costs: TokenCosts {
                input: 1.0,
                output: 1.0,
                cache_write: 0.0,
                cache_read: 0.0,
            },
            sonnet_costs: TokenCosts {
                input: 2.0,
                output: 2.0,
                cache_write: 0.0,
                cache_read: 0.0,
            },
            haiku_costs: TokenCosts {
                input: 0.5,
                output: 0.5,
                cache_write: 0.0,
                cache_read: 0.0,
            },
            lines_changed: 100,
        };

        let models: Vec<&str> = iter_model_costs(&costs).map(|(name, _)| name).collect();

        assert_eq!(models, vec!["Opus", "Sonnet", "Haiku"]);
    }

    #[test]
    fn test_iter_model_costs_returns_correct_references() {
        let costs = DailyCosts {
            date: NaiveDate::from_ymd_opt(2025, 10, 23).unwrap(),
            opus_costs: TokenCosts {
                input: 15.0,
                output: 75.0,
                cache_write: 0.0,
                cache_read: 0.0,
            },
            sonnet_costs: TokenCosts {
                input: 3.0,
                output: 15.0,
                cache_write: 0.0,
                cache_read: 0.0,
            },
            haiku_costs: TokenCosts {
                input: 0.25,
                output: 1.25,
                cache_write: 0.0,
                cache_read: 0.0,
            },
            lines_changed: 100,
        };

        let mut iter = iter_model_costs(&costs);

        let (name, token_costs) = iter.next().unwrap();
        assert_eq!(name, "Opus");
        assert_eq!(token_costs.input, 15.0);

        let (name, token_costs) = iter.next().unwrap();
        assert_eq!(name, "Sonnet");
        assert_eq!(token_costs.input, 3.0);

        let (name, token_costs) = iter.next().unwrap();
        assert_eq!(name, "Haiku");
        assert_eq!(token_costs.input, 0.25);

        assert!(iter.next().is_none());
    }
}
