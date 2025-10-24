mod analyzer;
mod line_counter;
mod pricing;

use std::path::Path;

use analyzer::{AnalysisResult, DailyCosts};
use tabled::{
    settings::{object::Rows, style::Style, Alignment, Modify, Width},
    Table, Tabled,
};

/// Minimum terminal width for using word-wrapping instead of truncation.
/// On terminals wider than or equal to this, wrapping prevents cutting off currency values.
/// On narrower terminals, truncation keeps output compact.
const MIN_WIDTH_FOR_WRAPPING: usize = 100;

/// Represents a row in the cost breakdown table
#[derive(Tabled)]
struct CostRow {
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
}

impl CostRow {
    /// Creates a new cost row with pre-formatted currency strings.
    ///
    /// The subtotal is calculated at construction time since CostRow
    /// is immutable and used only for display purposes.
    fn new(model: &str, input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self {
        Self {
            model: model.to_string(),
            input: format!("${:.4}", input),
            output: format!("${:.4}", output),
            cache_write: format!("${:.4}", cache_write),
            cache_read: format!("${:.4}", cache_read),
            subtotal: format!("${:.4}", input + output + cache_write + cache_read),
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
pub async fn run(dir: &Path) -> miette::Result<()> {
    let result = analyzer::analyze_directory(dir).await?;

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

    for costs in daily_costs {
        println!("Date: {}", costs.date);
        println!();

        let mut rows = Vec::new();

        if costs.opus_costs.total() > 0.0 {
            rows.push(CostRow::new(
                "Opus",
                costs.opus_costs.input,
                costs.opus_costs.output,
                costs.opus_costs.cache_write,
                costs.opus_costs.cache_read,
            ));
        }

        if costs.sonnet_costs.total() > 0.0 {
            rows.push(CostRow::new(
                "Sonnet",
                costs.sonnet_costs.input,
                costs.sonnet_costs.output,
                costs.sonnet_costs.cache_write,
                costs.sonnet_costs.cache_read,
            ));
        }

        if costs.haiku_costs.total() > 0.0 {
            rows.push(CostRow::new(
                "Haiku",
                costs.haiku_costs.input,
                costs.haiku_costs.output,
                costs.haiku_costs.cache_write,
                costs.haiku_costs.cache_read,
            ));
        }

        let mut table = Table::new(&rows);
        table
            .with(Style::rounded())
            .with(Modify::new(Rows::first()).with(Alignment::center()));

        if term_width >= MIN_WIDTH_FOR_WRAPPING {
            table.with(Width::wrap(term_width).keep_words(true));
        } else {
            table.with(Width::truncate(term_width));
        }

        println!("{}", table);

        let daily_total = costs.total();
        grand_total += daily_total;
        total_lines += costs.lines_changed;

        println!(
            "Daily Total: ${:.4} | Lines Changed: {}",
            daily_total, costs.lines_changed
        );
        println!();
    }

    println!("{}", divider(term_width));
    println!("Grand Total: ${:.4}", grand_total);
    println!("Total Lines Changed: {}", total_lines);
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
        let row = CostRow::new("Sonnet", 1.2345, 2.3456, 0.5, 0.25);

        assert_eq!(row.model, "Sonnet");
        assert_eq!(row.input, "$1.2345");
        assert_eq!(row.output, "$2.3456");
        assert_eq!(row.cache_write, "$0.5000");
        assert_eq!(row.cache_read, "$0.2500");
        assert_eq!(row.subtotal, "$4.3301");
    }

    #[test]
    fn test_cost_row_new_calculates_subtotal() {
        let row = CostRow::new("Test", 1.0, 2.0, 0.5, 0.25);
        assert_eq!(row.subtotal, "$3.7500");
    }

    #[test]
    fn test_cost_row_new_handles_zero_costs() {
        let row = CostRow::new("Haiku", 0.0, 0.0, 0.0, 0.0);
        assert_eq!(row.subtotal, "$0.0000");
    }

    #[test]
    fn test_divider_generates_correct_length() {
        assert_eq!(divider(0), "");
        assert_eq!(divider(1), "=");
        assert_eq!(divider(5), "=====");
        assert_eq!(divider(100).len(), 100);
    }
}
