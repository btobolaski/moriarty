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
