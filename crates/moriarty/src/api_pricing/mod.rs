mod analyzer;
mod pricing;

use std::path::Path;

use analyzer::{AnalysisResult, DailyCosts};

const REPORT_WIDTH: usize = 100;

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
    println!("\n{}", "=".repeat(REPORT_WIDTH));
    println!("File Parsing Summary");
    println!("{}", "=".repeat(REPORT_WIDTH));
    println!("  Successfully parsed: {} files", result.files_parsed);
    if result.files_failed > 0 {
        println!("  Failed to parse:     {} files", result.files_failed);
    }
    println!();
}

fn display_costs(daily_costs: &[DailyCosts]) {
    println!("{}", "=".repeat(REPORT_WIDTH));
    println!("API Cost Report");
    println!("{}", "=".repeat(REPORT_WIDTH));
    println!();

    let mut grand_total = 0.0;

    for costs in daily_costs {
        println!("Date: {}", costs.date);
        println!("{}", "-".repeat(REPORT_WIDTH));

        if costs.opus_costs.total() > 0.0 {
            println!(
                "  Opus:    Input: ${:>8.4}  Output: ${:>8.4}  Cache Write: ${:>8.4}  Cache Read: ${:>8.4}  Subtotal: ${:>8.4}",
                costs.opus_costs.input,
                costs.opus_costs.output,
                costs.opus_costs.cache_write,
                costs.opus_costs.cache_read,
                costs.opus_costs.total()
            );
        }

        // Sonnet costs
        if costs.sonnet_costs.total() > 0.0 {
            println!(
                "  Sonnet:  Input: ${:>8.4}  Output: ${:>8.4}  Cache Write: ${:>8.4}  Cache Read: ${:>8.4}  Subtotal: ${:>8.4}",
                costs.sonnet_costs.input,
                costs.sonnet_costs.output,
                costs.sonnet_costs.cache_write,
                costs.sonnet_costs.cache_read,
                costs.sonnet_costs.total()
            );
        }

        // Haiku costs
        if costs.haiku_costs.total() > 0.0 {
            println!(
                "  Haiku:   Input: ${:>8.4}  Output: ${:>8.4}  Cache Write: ${:>8.4}  Cache Read: ${:>8.4}  Subtotal: ${:>8.4}",
                costs.haiku_costs.input,
                costs.haiku_costs.output,
                costs.haiku_costs.cache_write,
                costs.haiku_costs.cache_read,
                costs.haiku_costs.total()
            );
        }

        let daily_total = costs.total();
        grand_total += daily_total;

        println!(
            "  {:<7}  {:<width$} Total: ${:>8.4}",
            "",
            "",
            daily_total,
            width = REPORT_WIDTH - 29
        );
        println!();
    }

    println!("{}", "=".repeat(REPORT_WIDTH));
    println!("Grand Total: ${:.4}", grand_total);
    println!("{}", "=".repeat(REPORT_WIDTH));
}

fn display_warnings(result: &AnalysisResult) {
    if !result.unknown_models.is_empty() {
        println!("\n{}", "=".repeat(REPORT_WIDTH));
        println!("WARNINGS");
        println!("{}", "=".repeat(REPORT_WIDTH));
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
        println!("{}", "=".repeat(REPORT_WIDTH));
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
}
