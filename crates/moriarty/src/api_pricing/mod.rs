mod analyzer;
mod line_counter;
mod pricing;

use std::path::Path;

use analyzer::{AnalysisResult, DailyCosts};

// Re-export DateTimezone for use in main.rs
pub use analyzer::DateTimezone;
use tabled::{
    settings::{
        object::Rows,
        style::{HorizontalLine, Style},
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

/// Run the API pricing analysis on a directory
pub async fn run(dir: &Path, timezone: DateTimezone) -> miette::Result<()> {
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

    let mut grand_total = 0.0;
    let mut total_lines = 0usize;
    let mut rows = Vec::new();
    let mut total_row_indices = Vec::new();

    for costs in daily_costs {
        let date_str = costs.date.to_string();

        // Show date only on the first model row to visually group all models
        // used on the same day, reducing table clutter. Subsequent model rows
        // for the same day will have empty date cells.
        let mut first_row = true;

        if costs.opus_costs.total() > 0.0 {
            rows.push(CostRow::new(
                if first_row { &date_str } else { "" },
                "Opus",
                costs.opus_costs.input,
                costs.opus_costs.output,
                costs.opus_costs.cache_write,
                costs.opus_costs.cache_read,
                "",
            ));
            first_row = false;
        }

        if costs.sonnet_costs.total() > 0.0 {
            rows.push(CostRow::new(
                if first_row { &date_str } else { "" },
                "Sonnet",
                costs.sonnet_costs.input,
                costs.sonnet_costs.output,
                costs.sonnet_costs.cache_write,
                costs.sonnet_costs.cache_read,
                "",
            ));
            first_row = false;
        }

        if costs.haiku_costs.total() > 0.0 {
            rows.push(CostRow::new(
                if first_row { &date_str } else { "" },
                "Haiku",
                costs.haiku_costs.input,
                costs.haiku_costs.output,
                costs.haiku_costs.cache_write,
                costs.haiku_costs.cache_read,
                "",
            ));
        }

        // Add total row for this day
        let daily_total = costs.total();
        grand_total += daily_total;
        total_lines += costs.lines_changed;

        rows.push(CostRow::new_total_row(costs.lines_changed, daily_total));
        total_row_indices.push(rows.len() - 1);
    }

    let mut table = Table::new(&rows);

    // Build horizontal separator lines after each day's total row (except the last one)
    // to visually separate different days while maintaining continuous vertical borders.
    //
    // The tabled crate's Style::horizontals() method requires compile-time constant array
    // sizes, preventing dynamic separator placement. This macro generates match arms for
    // common day counts (1-31 days, covering a full month of usage data).
    //
    // Limitation: For reports with >31 days, visual separators are not added. The table
    // remains functional but loses visual day grouping. This is an acceptable tradeoff
    // since most users analyze shorter time periods (weekly/monthly).
    let num_separators = total_row_indices.len().saturating_sub(1);

    macro_rules! apply_separators {
        ($table:expr, $indices:expr, $sep:expr, $count:expr) => {{
            // Build array by indexing into total_row_indices and offsetting by 2
            // (+1 for header row, +1 to place separator after the total row)
            let mut arr = [($indices[0] + 2, $sep); $count];
            for i in 0..$count {
                arr[i] = ($indices[i] + 2, $sep);
            }
            $table.with(Style::rounded().horizontals(arr))
        }};
    }

    if num_separators > 0 {
        // Use rounded style box-drawing characters for visual consistency
        let separator_line = HorizontalLine::full('─', '┼', '├', '┤');

        match num_separators {
            1 => {
                apply_separators!(table, total_row_indices, separator_line, 1);
            }
            2 => {
                apply_separators!(table, total_row_indices, separator_line, 2);
            }
            3 => {
                apply_separators!(table, total_row_indices, separator_line, 3);
            }
            4 => {
                apply_separators!(table, total_row_indices, separator_line, 4);
            }
            5 => {
                apply_separators!(table, total_row_indices, separator_line, 5);
            }
            6 => {
                apply_separators!(table, total_row_indices, separator_line, 6);
            }
            7 => {
                apply_separators!(table, total_row_indices, separator_line, 7);
            }
            8 => {
                apply_separators!(table, total_row_indices, separator_line, 8);
            }
            9 => {
                apply_separators!(table, total_row_indices, separator_line, 9);
            }
            10 => {
                apply_separators!(table, total_row_indices, separator_line, 10);
            }
            11 => {
                apply_separators!(table, total_row_indices, separator_line, 11);
            }
            12 => {
                apply_separators!(table, total_row_indices, separator_line, 12);
            }
            13 => {
                apply_separators!(table, total_row_indices, separator_line, 13);
            }
            14 => {
                apply_separators!(table, total_row_indices, separator_line, 14);
            }
            15 => {
                apply_separators!(table, total_row_indices, separator_line, 15);
            }
            16 => {
                apply_separators!(table, total_row_indices, separator_line, 16);
            }
            17 => {
                apply_separators!(table, total_row_indices, separator_line, 17);
            }
            18 => {
                apply_separators!(table, total_row_indices, separator_line, 18);
            }
            19 => {
                apply_separators!(table, total_row_indices, separator_line, 19);
            }
            20 => {
                apply_separators!(table, total_row_indices, separator_line, 20);
            }
            21 => {
                apply_separators!(table, total_row_indices, separator_line, 21);
            }
            22 => {
                apply_separators!(table, total_row_indices, separator_line, 22);
            }
            23 => {
                apply_separators!(table, total_row_indices, separator_line, 23);
            }
            24 => {
                apply_separators!(table, total_row_indices, separator_line, 24);
            }
            25 => {
                apply_separators!(table, total_row_indices, separator_line, 25);
            }
            26 => {
                apply_separators!(table, total_row_indices, separator_line, 26);
            }
            27 => {
                apply_separators!(table, total_row_indices, separator_line, 27);
            }
            28 => {
                apply_separators!(table, total_row_indices, separator_line, 28);
            }
            29 => {
                apply_separators!(table, total_row_indices, separator_line, 29);
            }
            30 => {
                apply_separators!(table, total_row_indices, separator_line, 30);
            }
            31 => {
                apply_separators!(table, total_row_indices, separator_line, 31);
            }
            _ => {
                // >31 days: use basic rounded style without separators
                table.with(Style::rounded());
            }
        }
    } else {
        table.with(Style::rounded());
    }

    table.with(Modify::new(Rows::first()).with(Alignment::center()));

    if term_width >= MIN_WIDTH_FOR_WRAPPING {
        table.with(Width::wrap(term_width).keep_words(true));
    } else {
        table.with(Width::truncate(term_width));
    }

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

    if term_width >= MIN_WIDTH_FOR_WRAPPING {
        table.with(Width::wrap(term_width).keep_words(true));
    } else {
        table.with(Width::truncate(term_width));
    }

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
}
