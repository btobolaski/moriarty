use chrono::{NaiveDate, TimeZone, Utc};
use tabled::Table;

use claude_logs::{Model, ModelFamily};

use super::*;
use crate::{
    api_pricing::{
        analyzer::{DailyCosts, SessionCosts},
        pricing::ModelCostsMap,
    },
    cost_report::{
        apply_width_config, create_grouped_table, display_grand_total, divider, fmt_money,
        format_duration, format_session_id, format_time_range, ComponentTotals, DateTimezone,
        FormattedCostColumns, GrandTotalRow, MetricComponents, MetricTotal, ReportMode,
        TokenCounts,
    },
};

fn test_date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

// These builders keep tests on the same `ModelCostsMap::add` path that
// production aggregation uses instead of constructing per-model maps directly.
fn costs_on(year: i32, month: u32, day: u32) -> DailyCosts {
    DailyCosts {
        date: test_date(year, month, day),
        per_model: ModelCostsMap::default(),
    }
}

trait DailyCostsExt {
    fn with_sonnet(self, input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self;
    fn with_haiku(self, input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self;
    fn with_opus(self, input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self;
    fn with_opus4(self, input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self;
}

impl DailyCostsExt for DailyCosts {
    fn with_sonnet(mut self, input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self {
        self.per_model
            .add(
                Model::family(ModelFamily::Sonnet),
                ComponentTotals::new(input, output, cache_write, cache_read),
            )
            .unwrap();
        self
    }
    fn with_haiku(mut self, input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self {
        self.per_model
            .add(
                Model::family(ModelFamily::Haiku),
                ComponentTotals::new(input, output, cache_write, cache_read),
            )
            .unwrap();
        self
    }
    fn with_opus(mut self, input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self {
        self.per_model
            .add(
                Model::family(ModelFamily::Opus),
                ComponentTotals::new(input, output, cache_write, cache_read),
            )
            .unwrap();
        self
    }
    fn with_opus4(mut self, input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self {
        // Opus 4 is `ModelFamily::Opus` with a parsed major-4 version; the
        // raw id mirrors what production logs carry so this builder matches
        // the bucket key produced by `Model::from_model_string`.
        let model = Model::from_model_string("claude-opus-4-20250514").expect("fixture id parses");
        self.per_model
            .add(
                model,
                ComponentTotals::new(input, output, cache_write, cache_read),
            )
            .unwrap();
        self
    }
}

// Both daily and session rows embed the same `FormattedCostColumns`, so one
// helper keeps the formatting assertions aligned across the parallel tests.
fn assert_money_columns(money: &FormattedCostColumns, components: (f64, f64, f64, f64)) {
    let (e_input, e_output, e_cache_write, e_cache_read) = components;
    assert_eq!(money.input, fmt_money(e_input), "input column");
    assert_eq!(money.output, fmt_money(e_output), "output column");
    assert_eq!(
        money.cache_write,
        fmt_money(e_cache_write),
        "cache_write column"
    );
    assert_eq!(
        money.cache_read,
        fmt_money(e_cache_read),
        "cache_read column"
    );
    let subtotal_total = e_input + e_output + e_cache_write + e_cache_read;
    assert_eq!(money.subtotal, fmt_money(subtotal_total), "subtotal column");
}

// `Total` rows leave the component columns blank; callers assert the subtotal
// separately because it is the row's only meaningful value.
fn assert_blank_money_component_columns(money: &FormattedCostColumns) {
    assert_eq!(money.input, "", "input column");
    assert_eq!(money.output, "", "output column");
    assert_eq!(money.cache_write, "", "cache_write column");
    assert_eq!(money.cache_read, "", "cache_read column");
}

#[test]
fn display_costs_smoke_variants() {
    // Builds a 32-day fixture in two months to exercise the BTreeMap-driven
    // separator handling on a span larger than any single month.
    let mut thirty_two_days: Vec<_> = (1..=31)
        .map(|day| costs_on(2025, 1, day).with_sonnet(1.0, 1.0, 0.0, 0.0))
        .collect();
    thirty_two_days.push(costs_on(2025, 2, 1).with_sonnet(1.0, 1.0, 0.0, 0.0));

    let cases = vec![
        ("empty", vec![]),
        (
            "two-day mixed models",
            vec![
                costs_on(2025, 10, 23).with_sonnet(1.0, 2.0, 0.5, 0.25),
                costs_on(2025, 10, 24).with_haiku(0.5, 1.0, 0.25, 0.1),
            ],
        ),
        // Three days exercises the separator-between-groups branch.
        (
            "three days",
            vec![
                costs_on(2025, 10, 23).with_sonnet(1.0, 1.0, 0.0, 0.0),
                costs_on(2025, 10, 24).with_haiku(0.5, 0.5, 0.0, 0.0),
                costs_on(2025, 10, 25).with_opus(2.0, 2.0, 0.0, 0.0),
            ],
        ),
        (
            "multi-model days",
            vec![
                costs_on(2025, 10, 23)
                    .with_opus(1.0, 1.0, 0.0, 0.0)
                    .with_sonnet(2.0, 2.0, 0.0, 0.0)
                    .with_haiku(0.5, 0.5, 0.0, 0.0),
                costs_on(2025, 10, 24).with_sonnet(1.0, 1.0, 0.0, 0.0),
            ],
        ),
        (
            "ten days",
            (1..=10)
                .map(|day| costs_on(2025, 10, day).with_sonnet(1.0, 1.0, 0.0, 0.0))
                .collect(),
        ),
        (
            "thirty-one days",
            (1..=31)
                .map(|day| costs_on(2025, 10, day).with_sonnet(1.0, 1.0, 0.0, 0.0))
                .collect(),
        ),
        ("thirty-two days", thirty_two_days),
    ];

    for (label, daily_costs) in cases {
        match std::panic::catch_unwind(|| display_costs(&daily_costs, ReportMode::Cost)) {
            Ok(()) => {}
            Err(_) => panic!("display_costs panicked on case {label}"),
        }
    }
}

#[test]
fn display_costs_single_day_variants() {
    let cases = [
        (
            "sonnet-only",
            costs_on(2025, 10, 23).with_sonnet(1.0, 2.0, 0.5, 0.25),
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
        match std::panic::catch_unwind(|| display_costs(&[daily], ReportMode::Cost)) {
            Ok(()) => {}
            Err(_) => panic!("display_costs panicked on case {label}"),
        }
    }
}

#[test]
fn cost_row_formats_currency_columns() {
    let row = CostRow::new(
        "2025-10-23",
        "Sonnet",
        ComponentTotals::new(1.2345, 2.3456, 0.5, 0.25),
    );

    assert_eq!(row.date, "2025-10-23");
    assert_eq!(row.model, "Sonnet");
    assert_money_columns(&row.metrics, (1.2345, 2.3456, 0.5, 0.25));
}

#[test]
fn cost_row_formats_token_columns() {
    let row = CostRow::new(
        "2025-10-23",
        "Sonnet",
        MetricComponents::Tokens(TokenCounts::new(1_234, 5_678, 90, 12)),
    );

    assert_eq!(row.metrics.input, "1,234");
    assert_eq!(row.metrics.output, "5,678");
    assert_eq!(row.metrics.cache_write, "90");
    assert_eq!(row.metrics.cache_read, "12");
    assert_eq!(row.metrics.subtotal, "7,014");
}

#[test]
fn cost_row_formats_large_token_columns_exactly() {
    let row = CostRow::new(
        "2025-10-23",
        "Sonnet",
        MetricComponents::Tokens(TokenCounts::new(9_007_199_254_740_993, 8, 90, 12)),
    );

    assert_eq!(row.metrics.input, "9,007,199,254,740,993");
    assert_eq!(row.metrics.output, "8");
    assert_eq!(row.metrics.cache_write, "90");
    assert_eq!(row.metrics.cache_read, "12");
    assert_eq!(row.metrics.subtotal, "9,007,199,254,741,103");
}

#[test]
fn cost_row_zero_values_format_as_zero_currency() {
    let row = CostRow::new("", "Haiku", ComponentTotals::new(0.0, 0.0, 0.0, 0.0));

    assert_eq!(row.metrics.input, "$0.0000");
    assert_eq!(row.metrics.subtotal, "$0.0000");
}

#[test]
fn cost_row_total_row_uses_blank_component_columns() {
    let row = CostRow::new_total_row(MetricTotal::Cost(56.789));

    assert_eq!(row.date, "");
    assert_eq!(row.model, "Total");
    assert_blank_money_component_columns(&row.metrics);
    assert_eq!(row.metrics.subtotal, "$56.7890");
}

#[test]
fn cost_row_total_row_variants() {
    let cases = [
        ("zero", 0.0, "$0.0000"),
        ("large value", 12_345.678_9, "$12345.6789"),
    ];

    for (label, total, expected_subtotal) in cases {
        let row = CostRow::new_total_row(MetricTotal::Cost(total));
        assert_eq!(row.model, "Total", "case {label}");
        assert_eq!(row.metrics.subtotal, expected_subtotal, "case {label}");
    }
}

#[test]
fn divider_generates_correct_length() {
    assert_eq!(divider(0), "");
    assert_eq!(divider(1), "=");
    assert_eq!(divider(5), "=====");
    assert_eq!(divider(100).len(), 100);
}

#[test]
fn fmt_money_normalizes_negative_zero() {
    assert_eq!(fmt_money(-0.0), "$0.0000");
}

#[test]
fn grand_total_row_formats_currency() {
    let row = GrandTotalRow::new(ReportMode::Cost, 143.7082);

    assert_eq!(row.grand_total, "$143.7082");
}

#[test]
fn grand_total_row_formats_tokens() {
    let row = GrandTotalRow::new(ReportMode::Tokens, MetricTotal::Tokens(1_437_082));

    assert_eq!(row.grand_total, "1,437,082");
}

#[test]
fn grand_total_row_formats_large_tokens_exactly() {
    let row = GrandTotalRow::new(
        ReportMode::Tokens,
        MetricTotal::Tokens(9_007_199_254_741_103),
    );

    assert_eq!(row.grand_total, "9,007,199,254,741,103");
}

#[test]
fn grand_total_row_variants() {
    let cases = [
        ("zero", 0.0, "$0.0000"),
        ("large value", 99_999.999_9, "$99999.9999"),
    ];

    for (label, grand_total, expected) in cases {
        let row = GrandTotalRow::new(ReportMode::Cost, grand_total);
        assert_eq!(row.grand_total, expected, "case {label}");
    }
}

// Smoke tests for `display_grand_total`: the function writes to stdout, so we
// only verify that it does not panic across a representative range of inputs.
#[test]
fn display_grand_total_smoke_variants() {
    for grand_total in [0.0, 143.7082, 12_345.678_9, 0.0001] {
        display_grand_total(ReportMode::Cost, grand_total);
    }
}

#[test]
fn metric_total_checked_add_rejects_token_overflow() {
    let error = MetricTotal::Tokens(u128::MAX)
        .checked_add(MetricTotal::Tokens(1))
        .unwrap_err();

    assert!(error
        .to_string()
        .contains("grand token total exceeded u128"));
}

#[test]
fn build_cost_rows_variants() {
    type CostRowCase = (
        &'static str,
        Vec<DailyCosts>,
        Vec<usize>,
        Vec<(&'static str, &'static str)>,
    );
    let cases: Vec<CostRowCase> = vec![
        (
            "single-model day",
            vec![costs_on(2025, 10, 23).with_sonnet(1.0, 2.0, 0.0, 0.0)],
            vec![1],
            vec![("2025-10-23", "Sonnet"), ("", "Total")],
        ),
        (
            "multi-model same day",
            vec![costs_on(2025, 10, 23)
                .with_opus(1.0, 1.0, 0.0, 0.0)
                .with_sonnet(2.0, 2.0, 0.0, 0.0)
                .with_haiku(0.5, 0.5, 0.0, 0.0)],
            vec![3],
            vec![
                ("2025-10-23", "Opus"),
                ("", "Sonnet"),
                ("", "Haiku"),
                ("", "Total"),
            ],
        ),
        // A day with no nonzero per-model entries still emits one Total row
        // so the report shows a $0.0000 footer rather than dropping the day.
        (
            "zero-cost day still gets labeled total row",
            vec![costs_on(2025, 10, 23)],
            vec![0],
            vec![("2025-10-23", "Total")],
        ),
        (
            "multiple days",
            vec![
                costs_on(2025, 10, 23).with_sonnet(1.0, 1.0, 0.0, 0.0),
                costs_on(2025, 10, 24).with_haiku(0.5, 0.5, 0.0, 0.0),
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
    ];

    for (label, daily_costs, expected_total_row_indices, expected_rows) in cases {
        let (rows, total_row_indices) = build_cost_rows(&daily_costs, ReportMode::Cost);

        assert_eq!(
            total_row_indices, expected_total_row_indices,
            "case {label}"
        );
        assert_eq!(rows.len(), expected_rows.len(), "case {label}");
        for (row, (expected_date, expected_model)) in rows.iter().zip(expected_rows) {
            assert_eq!(row.date, expected_date, "case {label}");
            assert_eq!(row.model, expected_model, "case {label}");
        }
    }
}

#[test]
fn create_grouped_table_variants() {
    struct Case {
        label: &'static str,
        rows: Vec<CostRow>,
        total_row_indices: Vec<usize>,
        expected_substrings: Vec<&'static str>,
        expect_separator: bool,
    }

    let cases = vec![
        Case {
            label: "single group",
            rows: vec![
                CostRow::new(
                    "2025-10-23",
                    "Sonnet",
                    ComponentTotals::new(1.0, 1.0, 0.0, 0.0),
                ),
                CostRow::new_total_row(MetricTotal::Cost(2.0)),
            ],
            total_row_indices: vec![1],
            expected_substrings: vec!["Sonnet", "Total"],
            expect_separator: false,
        },
        Case {
            label: "multiple groups with separator",
            rows: vec![
                CostRow::new(
                    "2025-10-23",
                    "Sonnet",
                    ComponentTotals::new(1.0, 1.0, 0.0, 0.0),
                ),
                CostRow::new_total_row(MetricTotal::Cost(2.0)),
                CostRow::new(
                    "2025-10-24",
                    "Haiku",
                    ComponentTotals::new(0.5, 0.5, 0.0, 0.0),
                ),
                CostRow::new_total_row(MetricTotal::Cost(1.0)),
            ],
            total_row_indices: vec![1, 3],
            expected_substrings: vec!["2025-10-23", "2025-10-24", "Sonnet", "Haiku"],
            expect_separator: true,
        },
        Case {
            label: "empty",
            rows: vec![],
            total_row_indices: vec![],
            expected_substrings: vec![],
            expect_separator: false,
        },
    ];

    for case in cases {
        let output = create_grouped_table(&case.rows, &case.total_row_indices).to_string();
        assert!(!output.is_empty(), "case {}", case.label);
        for expected in case.expected_substrings {
            assert!(
                output.contains(expected),
                "case {}: missing {expected:?} in {output}",
                case.label
            );
        }
        if case.expect_separator {
            assert!(
                output.lines().any(|line| line.contains('┼')),
                "case {}: expected separator with ┼ character",
                case.label
            );
        }
    }
}

// Smoke tests for `apply_width_config`: actual wrapping/truncation is a
// `tabled` concern. We only verify the call does not panic at the wrap/truncate
// width boundary defined by `MIN_WIDTH_FOR_WRAPPING`.
#[test]
fn apply_width_config_handles_boundary_widths() {
    let rows = vec![CostRow::new(
        "2025-10-23",
        "Sonnet",
        ComponentTotals::new(1.0, 1.0, 0.0, 0.0),
    )];

    for width in [99, 100, 101] {
        let mut table = Table::new(&rows);
        apply_width_config(&mut table, width);
        assert!(!table.to_string().is_empty());
    }
}

#[test]
fn iter_model_costs_returns_display_order() {
    let costs = costs_on(2025, 10, 23)
        .with_opus(1.0, 1.0, 0.0, 0.0)
        .with_sonnet(2.0, 2.0, 0.0, 0.0)
        .with_haiku(0.5, 0.5, 0.0, 0.0);

    let models: Vec<String> = costs
        .per_model
        .model_costs()
        .into_iter()
        .map(|(name, _)| name)
        .collect();

    // Only populated families appear; absent buckets (Opus 4 here) are dropped
    // by the new dynamic-sized aggregator.
    assert_eq!(models, vec!["Opus", "Sonnet", "Haiku"]);
}

#[test]
fn format_session_id_truncates_to_first_eight_characters() {
    let cases = [
        ("019dc252-e50e-766c", "019dc252"),
        ("01234567", "01234567"),
        ("012345", "012345"),
        ("ééééééééé", "éééééééé"),
        ("", ""),
    ];

    for (input, expected) in cases {
        assert_eq!(format_session_id(input), expected, "input {input:?}");
    }
}

#[test]
fn format_duration_table_driven() {
    let cases = [
        (0_i64, "0 min"),
        (1, "1 min"),
        (59, "59 min"),
        (60, "1 hr"),
        (61, "1 hr 1 min"),
        (90, "1 hr 30 min"),
        (120, "2 hr"),
        (125, "2 hr 5 min"),
    ];

    for (minutes, expected) in cases {
        assert_eq!(format_duration(minutes), expected, "minutes {minutes}");
    }
}

#[test]
fn format_time_range_uses_requested_timezone_for_same_day_ranges() {
    let start = Utc.with_ymd_and_hms(2025, 1, 1, 23, 30, 0).unwrap();
    let end = Utc.with_ymd_and_hms(2025, 1, 2, 0, 15, 0).unwrap();

    assert_eq!(
        format_time_range(DateTimezone::Utc, start, end),
        "2025-01-01 23:30 → 2025-01-02 00:15"
    );

    let local = format_time_range(DateTimezone::Local, start, end);
    assert!(
        !local.is_empty(),
        "local formatting should still produce a time range"
    );
}

#[test]
fn format_time_range_shows_end_date_for_cross_day_ranges() {
    let start = Utc.with_ymd_and_hms(2025, 10, 23, 9, 0, 0).unwrap();
    let end = Utc.with_ymd_and_hms(2025, 10, 24, 10, 30, 0).unwrap();

    assert_eq!(
        format_time_range(DateTimezone::Utc, start, end),
        "2025-10-23 09:00 → 2025-10-24 10:30"
    );
}

#[test]
fn session_cost_row_formats_currency_columns() {
    let row = SessionCostRow::new(
        "019dc252",
        "2025-10-23 09:00 \u{2192} 10:30",
        "1 hr 30 min",
        "Sonnet",
        ComponentTotals::new(1.2345, 2.3456, 0.5, 0.25),
    );

    assert_eq!(row.session, "019dc252");
    assert_eq!(row.time_range, "2025-10-23 09:00 \u{2192} 10:30");
    assert_eq!(row.duration, "1 hr 30 min");
    assert_eq!(row.model, "Sonnet");
    assert_money_columns(&row.metrics, (1.2345, 2.3456, 0.5, 0.25));
}

#[test]
fn session_cost_row_total_uses_blank_component_columns() {
    let row = SessionCostRow::new_total_row(MetricTotal::Cost(7.5));

    assert_eq!(row.session, "");
    assert_eq!(row.time_range, "");
    assert_eq!(row.duration, "");
    assert_eq!(row.model, "Total");
    assert_blank_money_component_columns(&row.metrics);
    assert_eq!(row.metrics.subtotal, "$7.5000");
}

fn session_costs_fixture(session_id: &str) -> SessionCosts {
    let start = Utc.with_ymd_and_hms(2025, 10, 23, 9, 0, 0).unwrap();
    let end = Utc.with_ymd_and_hms(2025, 10, 23, 10, 30, 0).unwrap();
    let mut per_model = ModelCostsMap::default();
    per_model
        .add(
            Model::family(ModelFamily::Sonnet),
            ComponentTotals::new(1.0, 2.0, 0.0, 0.0),
        )
        .unwrap();
    SessionCosts {
        session_id: session_id.to_string(),
        start_time: start,
        end_time: end,
        per_model,
    }
}

#[test]
fn build_session_cost_rows_empty_input_returns_empty_rows() {
    let (rows, total_row_indices) =
        build_session_cost_rows(&[], DateTimezone::Utc, ReportMode::Cost);

    assert!(rows.is_empty());
    assert!(total_row_indices.is_empty());
}

#[test]
fn build_session_cost_rows_emits_per_model_row_then_total() {
    let session = session_costs_fixture("019dc252-e50e-766c");

    let (rows, total_row_indices) = build_session_cost_rows(
        std::slice::from_ref(&session),
        DateTimezone::Utc,
        ReportMode::Cost,
    );

    assert_eq!(total_row_indices, vec![1]);
    assert_eq!(rows.len(), 2);

    assert_eq!(rows[0].session, "019dc252");
    assert_eq!(rows[0].duration, "1 hr 30 min");
    assert_eq!(rows[0].model, "Sonnet");

    assert_eq!(rows[1].session, "");
    assert_eq!(rows[1].time_range, "");
    assert_eq!(rows[1].duration, "");
    assert_eq!(rows[1].model, "Total");
    assert_eq!(rows[1].metrics.subtotal, "$3.0000");
}

#[test]
fn build_session_cost_rows_zero_cost_session_keeps_identifying_columns() {
    let session = SessionCosts {
        session_id: "ééééééééé-session".to_string(),
        start_time: Utc.with_ymd_and_hms(2025, 10, 23, 9, 0, 0).unwrap(),
        end_time: Utc.with_ymd_and_hms(2025, 10, 23, 9, 0, 0).unwrap(),
        per_model: ModelCostsMap::default(),
    };

    let (rows, total_row_indices) =
        build_session_cost_rows(&[session], DateTimezone::Utc, ReportMode::Cost);

    assert_eq!(total_row_indices, vec![0]);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].session, "éééééééé");
    assert_eq!(rows[0].time_range, "2025-10-23 09:00 → 09:00");
    assert_eq!(rows[0].duration, "0 min");
    assert_eq!(rows[0].model, "Total");
}

#[test]
fn build_session_cost_rows_inserts_separator_indices_per_session() {
    let sessions = vec![
        session_costs_fixture("aaaaaaaa-aaaa"),
        session_costs_fixture("bbbbbbbb-bbbb"),
    ];

    let (rows, total_row_indices) =
        build_session_cost_rows(&sessions, DateTimezone::Utc, ReportMode::Cost);

    assert_eq!(total_row_indices, vec![1, 3]);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].session, "aaaaaaaa");
    assert_eq!(rows[2].session, "bbbbbbbb");
}

#[test]
fn iter_model_costs_returns_per_bucket_values() {
    let costs = costs_on(2025, 10, 23)
        .with_opus(15.0, 75.0, 0.0, 0.0)
        .with_sonnet(3.0, 15.0, 0.0, 0.0)
        .with_haiku(0.25, 1.25, 0.0, 0.0)
        .with_opus4(5.0, 25.0, 0.0, 0.0);

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

#[test]
fn collect_model_aggregates_total_equals_grand_total() {
    let daily_costs = vec![
        costs_on(2025, 10, 23)
            .with_sonnet(1.0, 2.0, 0.0, 0.0)
            .with_haiku(0.5, 1.0, 0.0, 0.0),
        costs_on(2025, 10, 24)
            .with_opus(15.0, 75.0, 0.0, 0.0)
            .with_sonnet(3.0, 15.0, 0.0, 0.0)
            .with_opus4(5.0, 25.0, 0.0, 0.0),
    ];

    let grand_total = daily_costs
        .iter()
        .fold(MetricTotal::Cost(0.0), |acc, item| {
            acc.checked_add(item.total(ReportMode::Cost).unwrap())
                .unwrap()
        });

    let models = collect_model_aggregates(&daily_costs);
    let model_grand = models
        .iter()
        .map(|(_, m)| m.total())
        .fold(MetricTotal::Cost(0.0), |acc, t| acc.checked_add(t).unwrap());

    assert_eq!(grand_total, model_grand);
    assert_eq!(grand_total, MetricTotal::Cost(142.5));
}

#[test]
fn collect_session_model_aggregates_total_equals_grand_total() {
    let mut per_model_a = ModelCostsMap::default();
    per_model_a
        .add(
            Model::family(ModelFamily::Sonnet),
            MetricComponents::Cost(ComponentTotals::new(1.0, 2.0, 0.0, 0.0)),
        )
        .unwrap();
    per_model_a
        .add(
            Model::family(ModelFamily::Haiku),
            MetricComponents::Cost(ComponentTotals::new(0.5, 1.0, 0.0, 0.0)),
        )
        .unwrap();

    let mut per_model_b = ModelCostsMap::default();
    per_model_b
        .add(
            Model::family(ModelFamily::Opus),
            MetricComponents::Cost(ComponentTotals::new(15.0, 75.0, 0.0, 0.0)),
        )
        .unwrap();

    let sessions = vec![
        SessionCosts {
            session_id: "session-a".to_string(),
            start_time: Utc.with_ymd_and_hms(2025, 10, 23, 9, 0, 0).unwrap(),
            end_time: Utc.with_ymd_and_hms(2025, 10, 23, 10, 0, 0).unwrap(),
            per_model: per_model_a,
        },
        SessionCosts {
            session_id: "session-b".to_string(),
            start_time: Utc.with_ymd_and_hms(2025, 10, 24, 9, 0, 0).unwrap(),
            end_time: Utc.with_ymd_and_hms(2025, 10, 24, 10, 0, 0).unwrap(),
            per_model: per_model_b,
        },
    ];

    let grand_total = sessions.iter().fold(MetricTotal::Cost(0.0), |acc, item| {
        acc.checked_add(item.total(ReportMode::Cost).unwrap())
            .unwrap()
    });

    let models = collect_session_model_aggregates(&sessions);
    let model_grand = models
        .iter()
        .map(|(_, m)| m.total())
        .fold(MetricTotal::Cost(0.0), |acc, t| acc.checked_add(t).unwrap());

    assert_eq!(grand_total, model_grand);
    assert_eq!(grand_total, MetricTotal::Cost(94.5));
}

#[test]
fn collect_model_aggregates_preserves_family_then_version_desc_order() {
    let daily_costs = vec![costs_on(2025, 10, 23)
        .with_haiku(0.25, 1.25, 0.0, 0.0)
        .with_sonnet(3.0, 15.0, 0.0, 0.0)
        .with_opus(15.0, 75.0, 0.0, 0.0)
        .with_opus4(5.0, 25.0, 0.0, 0.0)];

    let models = collect_model_aggregates(&daily_costs);
    let labels: Vec<&str> = models.iter().map(|(name, _)| name.as_str()).collect();

    assert_eq!(labels, vec!["Opus 4", "Opus", "Sonnet", "Haiku"]);
}

#[test]
fn display_costs_with_summary_smoke_variants() {
    let cases = vec![
        (
            "single model single day",
            vec![costs_on(2025, 10, 23).with_sonnet(1.0, 2.0, 0.0, 0.0)],
        ),
        (
            "multiple models multiple days",
            vec![
                costs_on(2025, 10, 23)
                    .with_opus(15.0, 75.0, 0.0, 0.0)
                    .with_sonnet(3.0, 15.0, 0.0, 0.0)
                    .with_haiku(0.25, 1.25, 0.0, 0.0)
                    .with_opus4(5.0, 25.0, 0.0, 0.0),
                costs_on(2025, 10, 24).with_sonnet(1.0, 1.0, 0.0, 0.0),
            ],
        ),
    ];

    for (label, daily_costs) in cases {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            display_costs(&daily_costs, ReportMode::Cost)
        }));
        if result.is_err() {
            panic!("display_costs panicked on case {label}");
        }
    }
}

#[test]
fn display_costs_with_summary_token_mode_smoke() {
    let mut map = ModelCostsMap::default();
    let opus4 = Model::from_model_string("claude-opus-4-20250514").unwrap();
    map.add(
        opus4,
        MetricComponents::Tokens(TokenCounts::new(1_000, 500, 100, 50)),
    )
    .unwrap();
    let token_daily = vec![DailyCosts {
        date: test_date(2025, 10, 23),
        per_model: map,
    }];

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        display_costs(&token_daily, ReportMode::Tokens)
    }));
    if result.is_err() {
        panic!("token mode display panicked");
    }
}
