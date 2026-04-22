use super::*;
use crate::api_pricing::{
    analyzer::{AnalysisResult, DailyCosts},
    pricing::{ModelCostsMap, ModelType, TokenCosts, TokenCounts},
};
use chrono::NaiveDate;
use std::collections::HashSet;

/// Builder helper for creating DailyCosts with defaults
fn make_daily_costs(date: NaiveDate) -> DailyCosts {
    DailyCosts {
        date,
        per_model: ModelCostsMap::default(),
        lines_changed: 0,
    }
}

/// Short date constructor used throughout the display tests.
fn test_date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

/// Builder helper for creating DailyCosts with defaults for a specific date.
fn costs_on(year: i32, month: u32, day: u32) -> DailyCosts {
    make_daily_costs(test_date(year, month, day))
}

trait DailyCostsExt {
    fn with_sonnet(self, input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self;
    fn with_haiku(self, input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self;
    fn with_opus(self, input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self;
    fn with_opus4(self, input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self;
    fn with_lines(self, lines: usize) -> Self;
}

impl DailyCostsExt for DailyCosts {
    fn with_sonnet(mut self, input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self {
        self.per_model.set(
            ModelType::Sonnet,
            TokenCosts {
                input,
                output,
                cache_write,
                cache_read,
            },
        );
        self
    }
    fn with_haiku(mut self, input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self {
        self.per_model.set(
            ModelType::Haiku,
            TokenCosts {
                input,
                output,
                cache_write,
                cache_read,
            },
        );
        self
    }
    fn with_opus(mut self, input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self {
        self.per_model.set(
            ModelType::Opus,
            TokenCosts {
                input,
                output,
                cache_write,
                cache_read,
            },
        );
        self
    }
    fn with_opus4(mut self, input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self {
        self.per_model.set(
            ModelType::Opus4,
            TokenCosts {
                input,
                output,
                cache_write,
                cache_read,
            },
        );
        self
    }
    fn with_lines(mut self, lines: usize) -> Self {
        self.lines_changed = lines;
        self
    }
}

#[test]
fn test_display_analysis_summary_variants() {
    for result in [
        AnalysisResult {
            daily_costs: vec![],
            unknown_models: HashSet::new(),
            total_unknown_tokens: TokenCounts::default(),
            files_parsed: 5,
            files_failed: 0,
        },
        AnalysisResult {
            daily_costs: vec![],
            unknown_models: HashSet::new(),
            total_unknown_tokens: TokenCounts::default(),
            files_parsed: 3,
            files_failed: 2,
        },
    ] {
        display_parsing_summary(result.files_parsed, result.files_failed);
    }
}

#[test]
fn test_display_costs_smoke_variants() {
    let mut thirty_two_days: Vec<_> = (1..=31)
        .map(|day| costs_on(2025, 1, day).with_sonnet(1.0, 1.0, 0.0, 0.0).with_lines(10))
        .collect();
    thirty_two_days.push(costs_on(2025, 2, 1).with_sonnet(1.0, 1.0, 0.0, 0.0).with_lines(10));

    let cases = vec![
        ("empty", vec![]),
        (
            "two-day mixed models",
            vec![
                costs_on(2025, 10, 23)
                    .with_sonnet(1.0, 2.0, 0.5, 0.25)
                    .with_lines(145),
                costs_on(2025, 10, 24)
                    .with_haiku(0.5, 1.0, 0.25, 0.1)
                    .with_lines(50),
            ],
        ),
        // 3 days => num_separators = 2 branch in display_costs.
        (
            "three days",
            vec![
                costs_on(2025, 10, 23).with_sonnet(1.0, 1.0, 0.0, 0.0).with_lines(100),
                costs_on(2025, 10, 24).with_haiku(0.5, 0.5, 0.0, 0.0).with_lines(50),
                costs_on(2025, 10, 25).with_opus(2.0, 2.0, 0.0, 0.0).with_lines(75),
            ],
        ),
        (
            "multi-model days",
            vec![
                costs_on(2025, 10, 23)
                    .with_opus(1.0, 1.0, 0.0, 0.0)
                    .with_sonnet(2.0, 2.0, 0.0, 0.0)
                    .with_haiku(0.5, 0.5, 0.0, 0.0)
                    .with_lines(100),
                costs_on(2025, 10, 24).with_sonnet(1.0, 1.0, 0.0, 0.0).with_lines(50),
            ],
        ),
        // 10 days => num_separators = 9 branch near the end of the explicit match arms.
        (
            "ten days",
            (1..=10)
                .map(|day| costs_on(2025, 10, day).with_sonnet(1.0, 1.0, 0.0, 0.0).with_lines(10))
                .collect(),
        ),
        // 31 days => the last explicit match branch.
        (
            "thirty-one days",
            (1..=31)
                .map(|day| costs_on(2025, 10, day).with_sonnet(1.0, 1.0, 0.0, 0.0).with_lines(10))
                .collect(),
        ),
        // 32 days => fallback branch for values above the explicit match table.
        ("thirty-two days", thirty_two_days),
    ];

    for (label, daily_costs) in cases {
        let result = std::panic::catch_unwind(|| display_costs(&daily_costs));
        assert!(result.is_ok(), "display_costs panicked on case {label}");
    }
}

#[test]
fn test_display_costs_single_day_variants() {
    let cases = [
        (
            "sonnet-only",
            costs_on(2025, 10, 23)
                .with_sonnet(1.0, 2.0, 0.5, 0.25)
                .with_lines(145),
        ),
        (
            "opus-only",
            costs_on(2025, 10, 23).with_opus(15.0, 75.0, 18.75, 1.5),
        ),
        (
            "sonnet-haiku",
            costs_on(2025, 10, 23)
                .with_sonnet(1.0, 2.0, 0.5, 0.25)
                .with_haiku(0.5, 1.0, 0.25, 0.1),
        ),
    ];

    for (label, daily) in cases {
        let result = std::panic::catch_unwind(|| display_costs(&[daily]));
        assert!(result.is_ok(), "display_costs panicked on case {label}");
    }
}


#[test]
fn test_display_warnings_no_unknown_models() {
    display_unknown_model_warnings(&HashSet::new(), &TokenCounts::default());
}

#[test]
fn test_display_warnings_with_unknown_models() {
    let mut unknown_models = HashSet::new();
    unknown_models.insert("gpt-4".to_string());
    unknown_models.insert("gemini-pro".to_string());

    let total_unknown_tokens = TokenCounts::new(1000, 500, 100, 50);

    display_unknown_model_warnings(&unknown_models, &total_unknown_tokens);
}

#[test]
fn test_cost_row_variants() {
    let cases = [
        (
            "formats mixed costs",
            CostRow::new("2025-10-23", "Sonnet", 1.2345, 2.3456, 0.5, 0.25, "100"),
            ("2025-10-23", "Sonnet", "$1.2345", "$2.3456", "$0.5000", "$0.2500", "$4.3301", "100"),
        ),
        (
            "formats subtotal without lines",
            CostRow::new("2025-10-23", "Test", 1.0, 2.0, 0.5, 0.25, ""),
            ("2025-10-23", "Test", "$1.0000", "$2.0000", "$0.5000", "$0.2500", "$3.7500", ""),
        ),
        (
            "formats zero row",
            CostRow::new("", "Haiku", 0.0, 0.0, 0.0, 0.0, ""),
            ("", "Haiku", "$0.0000", "$0.0000", "$0.0000", "$0.0000", "$0.0000", ""),
        ),
    ];

    for (label, row, expected) in cases {
        assert_eq!(
            (
                row.date.as_str(),
                row.model.as_str(),
                row.input.as_str(),
                row.output.as_str(),
                row.cache_write.as_str(),
                row.cache_read.as_str(),
                row.subtotal.as_str(),
                row.lines.as_str(),
            ),
            expected,
            "case {label}",
        );
    }
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
fn test_cost_row_new_total_row_variants() {
    let cases = [
        ("zero", 0, 0.0, "$0.0000", "0"),
        ("large-values", 999999, 12345.6789, "$12345.6789", "999999"),
    ];

    for (label, lines, total, expected_subtotal, expected_lines) in cases {
        let row = CostRow::new_total_row(lines, total);
        assert_eq!(row.date, "", "case {label}");
        assert_eq!(row.model, "Total", "case {label}");
        assert_eq!(row.subtotal, expected_subtotal, "case {label}");
        assert_eq!(row.lines, expected_lines, "case {label}");
    }
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
fn test_grand_total_row_new_variants() {
    let cases = [
        ("zero", 0.0, 0, "$0.0000", "0"),
        ("large-values", 99999.9999, 999999, "$99999.9999", "999999"),
    ];

    for (label, grand_total, total_lines, expected_total, expected_lines) in cases {
        let row = GrandTotalRow::new(grand_total, total_lines);
        assert_eq!(row.grand_total, expected_total, "case {label}");
        assert_eq!(row.total_lines_changed, expected_lines, "case {label}");
    }
}

// Smoke tests for display_grand_total - verify the function doesn't panic
// with various input values. These tests don't assert on output format since
// that would require capturing stdout or refactoring the function signature.

#[test]
fn test_display_grand_total_variants() {
    for (grand_total, total_lines) in [(0.0, 0), (143.7082, 13010), (12345.6789, 999999), (0.0001, 1)] {
        display_grand_total(grand_total, total_lines);
    }
}

// Unit tests for helper functions extracted during refactoring

#[test]
fn test_build_cost_rows_variants() {
    let cases = vec![
        (
            "single-model day",
            vec![costs_on(2025, 10, 23).with_sonnet(1.0, 2.0, 0.0, 0.0).with_lines(100)],
            vec![1],
            vec![("2025-10-23", "Sonnet"), ("", "Total")],
        ),
        (
            "multi-model same day",
            vec![
                costs_on(2025, 10, 23)
                    .with_opus(1.0, 1.0, 0.0, 0.0)
                    .with_sonnet(2.0, 2.0, 0.0, 0.0)
                    .with_haiku(0.5, 0.5, 0.0, 0.0)
                    .with_lines(200),
            ],
            vec![3],
            vec![
                ("2025-10-23", "Opus"),
                ("", "Sonnet"),
                ("", "Haiku"),
                ("", "Total"),
            ],
        ),
        (
            "zero-cost day still gets total row",
            vec![costs_on(2025, 10, 23)],
            vec![0],
            vec![("", "Total")],
        ),
        (
            "multiple days",
            vec![
                costs_on(2025, 10, 23).with_sonnet(1.0, 1.0, 0.0, 0.0).with_lines(50),
                costs_on(2025, 10, 24).with_haiku(0.5, 0.5, 0.0, 0.0).with_lines(25),
            ],
            vec![1, 3],
            vec![
                ("2025-10-23", "Sonnet"),
                ("", "Total"),
                ("2025-10-24", "Haiku"),
                ("", "Total"),
            ],
        ),
        ("empty", vec![], vec![], vec![]),
        (
            "opus-haiku day",
            vec![
                costs_on(2025, 10, 23)
                    .with_opus(1.0, 1.0, 0.0, 0.0)
                    .with_haiku(0.5, 0.5, 0.0, 0.0)
                    .with_lines(100),
            ],
            vec![2],
            vec![("2025-10-23", "Opus"), ("", "Haiku"), ("", "Total")],
        ),
        (
            "sonnet-haiku day",
            vec![
                costs_on(2025, 10, 23)
                    .with_sonnet(1.0, 1.0, 0.0, 0.0)
                    .with_haiku(0.5, 0.5, 0.0, 0.0)
                    .with_lines(100),
            ],
            vec![2],
            vec![("2025-10-23", "Sonnet"), ("", "Haiku"), ("", "Total")],
        ),
    ];

    for (label, daily_costs, expected_total_row_indices, expected_rows) in cases {
        let (rows, total_row_indices) = build_cost_rows(&daily_costs);

        assert_eq!(total_row_indices, expected_total_row_indices, "case {label}");
        assert_eq!(rows.len(), expected_rows.len(), "case {label}");
        for (row, (expected_date, expected_model)) in rows.iter().zip(expected_rows) {
            assert_eq!(row.date, expected_date, "case {label}");
            assert_eq!(row.model, expected_model, "case {label}");
        }
    }
}

#[test]
fn test_create_grouped_table_variants() {
    let cases = vec![
        (
            "single group",
            vec![
                CostRow::new("2025-10-23", "Sonnet", 1.0, 1.0, 0.0, 0.0, ""),
                CostRow::new_total_row(100, 2.0),
            ],
            vec![1],
            vec!["Sonnet", "Total"],
            false,
        ),
        (
            "multiple groups with separator",
            vec![
                CostRow::new("2025-10-23", "Sonnet", 1.0, 1.0, 0.0, 0.0, ""),
                CostRow::new_total_row(100, 2.0),
                CostRow::new("2025-10-24", "Haiku", 0.5, 0.5, 0.0, 0.0, ""),
                CostRow::new_total_row(50, 1.0),
            ],
            vec![1, 3],
            vec!["2025-10-23", "2025-10-24", "Sonnet", "Haiku"],
            true,
        ),
        ("empty", vec![], vec![], vec![], false),
    ];

    for (label, rows, total_row_indices, expected_substrings, expect_separator) in cases {
        let output = create_grouped_table(&rows, &total_row_indices).to_string();
        assert!(!output.is_empty(), "case {label}");
        for expected in expected_substrings {
            assert!(output.contains(expected), "case {label}: missing {expected:?} in {output}");
        }
        if expect_separator {
            assert!(
                output.lines().any(|line| line.contains('┼')),
                "case {label}: expected to find day separator (with ┼ character) in table output"
            );
        }
    }
}

// Smoke tests for apply_width_config - verify function doesn't panic at
// various width boundaries. Actual wrapping/truncation behavior is tested
// by the tabled library and verified through integration tests.

#[test]
fn test_apply_width_config_variants() {
    let rows = vec![CostRow::new(
        "2025-10-23",
        "Sonnet",
        1.0,
        1.0,
        0.0,
        0.0,
        "100",
    )];

    for width in [99, 100, 101] {
        let mut table = Table::new(&rows);
        apply_width_config(&mut table, width);
        assert!(!table.to_string().is_empty());
    }
}

#[test]
fn test_iter_model_costs_returns_correct_order() {
    let costs = make_daily_costs(NaiveDate::from_ymd_opt(2025, 10, 23).unwrap())
        .with_opus(1.0, 1.0, 0.0, 0.0)
        .with_sonnet(2.0, 2.0, 0.0, 0.0)
        .with_haiku(0.5, 0.5, 0.0, 0.0)
        .with_lines(100);

    let models: Vec<&str> = costs
        .per_model
        .model_costs()
        .into_iter()
        .map(|(name, _)| name)
        .collect();

    assert_eq!(models, vec!["Opus 4", "Opus", "Sonnet", "Haiku"]);
}

#[test]
fn test_iter_model_costs_returns_correct_values() {
    let costs = make_daily_costs(NaiveDate::from_ymd_opt(2025, 10, 23).unwrap())
        .with_opus(15.0, 75.0, 0.0, 0.0)
        .with_sonnet(3.0, 15.0, 0.0, 0.0)
        .with_haiku(0.25, 1.25, 0.0, 0.0)
        .with_opus4(5.0, 25.0, 0.0, 0.0)
        .with_lines(100);

    let model_costs = costs.per_model.model_costs();

    assert_eq!(model_costs[0].0, "Opus 4");
    assert_eq!(model_costs[0].1.input, 5.0);

    assert_eq!(model_costs[1].0, "Opus");
    assert_eq!(model_costs[1].1.input, 15.0);

    assert_eq!(model_costs[2].0, "Sonnet");
    assert_eq!(model_costs[2].1.input, 3.0);

    assert_eq!(model_costs[3].0, "Haiku");
    assert_eq!(model_costs[3].1.input, 0.25);
}
