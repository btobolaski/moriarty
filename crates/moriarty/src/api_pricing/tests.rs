use chrono::{NaiveDate, TimeZone, Utc};

use super::*;
use crate::api_pricing::{
    analyzer::{DailyCosts, SessionCosts},
    pricing::{ModelCostsMap, ModelType, TokenCosts},
};

/// Short date constructor used throughout the display tests.
fn test_date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

/// Build a `DailyCosts` with the given date and an empty cost map.
///
/// The builder helpers below add per-model costs through `ModelCostsMap::add`.
/// Because each builder targets a distinct `ModelType` bucket and the map
/// starts empty, `add` is equivalent to a wholesale set in this context
/// while keeping the production aggregation path the only public API.
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
        self.per_model.add(
            ModelType::Sonnet,
            TokenCosts::new(input, output, cache_write, cache_read),
        );
        self
    }
    fn with_haiku(mut self, input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self {
        self.per_model.add(
            ModelType::Haiku,
            TokenCosts::new(input, output, cache_write, cache_read),
        );
        self
    }
    fn with_opus(mut self, input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self {
        self.per_model.add(
            ModelType::Opus,
            TokenCosts::new(input, output, cache_write, cache_read),
        );
        self
    }
    fn with_opus4(mut self, input: f64, output: f64, cache_write: f64, cache_read: f64) -> Self {
        self.per_model.add(
            ModelType::Opus4,
            TokenCosts::new(input, output, cache_write, cache_read),
        );
        self
    }
}

/// Assert the four currency columns and the subtotal of a row match the
/// expected token-cost components.
///
/// Takes the embedded `FormattedCostColumns` substruct shared by both `CostRow`
/// and `SessionCostRow`, so parallel daily/session formatting tests can
/// collapse to a single helper call instead of repeating five `assert_eq!`
/// lines apiece.
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

/// Assert that the four token-cost component columns are blank, as on the
/// trailing `Total` row in both the daily and session cost tables. The
/// `subtotal` column is intentionally NOT checked here — callers assert it
/// directly because it carries the row's only meaningful value.
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
        match std::panic::catch_unwind(|| display_costs(&daily_costs)) {
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
        match std::panic::catch_unwind(|| display_costs(&[daily])) {
            Ok(()) => {}
            Err(_) => panic!("display_costs panicked on case {label}"),
        }
    }
}

#[test]
fn cost_row_formats_currency_columns() {
    let row = CostRow::new("2025-10-23", "Sonnet", (1.2345, 2.3456, 0.5, 0.25));

    assert_eq!(row.date, "2025-10-23");
    assert_eq!(row.model, "Sonnet");
    assert_money_columns(&row.money, (1.2345, 2.3456, 0.5, 0.25));
}

#[test]
fn cost_row_zero_values_format_as_zero_currency() {
    let row = CostRow::new("", "Haiku", (0.0, 0.0, 0.0, 0.0));

    assert_eq!(row.money.input, "$0.0000");
    assert_eq!(row.money.subtotal, "$0.0000");
}

#[test]
fn cost_row_total_row_uses_blank_component_columns() {
    let row = CostRow::new_total_row(56.789);

    assert_eq!(row.date, "");
    assert_eq!(row.model, "Total");
    assert_blank_money_component_columns(&row.money);
    assert_eq!(row.money.subtotal, "$56.7890");
}

#[test]
fn cost_row_total_row_variants() {
    let cases = [
        ("zero", 0.0, "$0.0000"),
        ("large value", 12_345.6789, "$12345.6789"),
    ];

    for (label, total, expected_subtotal) in cases {
        let row = CostRow::new_total_row(total);
        assert_eq!(row.model, "Total", "case {label}");
        assert_eq!(row.money.subtotal, expected_subtotal, "case {label}");
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
fn grand_total_row_formats_currency() {
    let row = GrandTotalRow::new(143.7082);

    assert_eq!(row.grand_total, "$143.7082");
}

#[test]
fn grand_total_row_variants() {
    let cases = [
        ("zero", 0.0, "$0.0000"),
        ("large value", 99_999.9999, "$99999.9999"),
    ];

    for (label, grand_total, expected) in cases {
        let row = GrandTotalRow::new(grand_total);
        assert_eq!(row.grand_total, expected, "case {label}");
    }
}

// Smoke tests for `display_grand_total`: the function writes to stdout, so we
// only verify that it does not panic across a representative range of inputs.
#[test]
fn display_grand_total_smoke_variants() {
    for grand_total in [0.0, 143.7082, 12_345.6789, 0.0001] {
        display_grand_total(grand_total);
    }
}

#[test]
fn build_cost_rows_variants() {
    let cases: Vec<(_, Vec<DailyCosts>, Vec<usize>, Vec<(&str, &str)>)> = vec![
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
            "zero-cost day still gets total row",
            vec![costs_on(2025, 10, 23)],
            vec![0],
            vec![("", "Total")],
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
        let (rows, total_row_indices) = build_cost_rows(&daily_costs);

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
                CostRow::new("2025-10-23", "Sonnet", (1.0, 1.0, 0.0, 0.0)),
                CostRow::new_total_row(2.0),
            ],
            total_row_indices: vec![1],
            expected_substrings: vec!["Sonnet", "Total"],
            expect_separator: false,
        },
        Case {
            label: "multiple groups with separator",
            rows: vec![
                CostRow::new("2025-10-23", "Sonnet", (1.0, 1.0, 0.0, 0.0)),
                CostRow::new_total_row(2.0),
                CostRow::new("2025-10-24", "Haiku", (0.5, 0.5, 0.0, 0.0)),
                CostRow::new_total_row(1.0),
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
    let rows = vec![CostRow::new("2025-10-23", "Sonnet", (1.0, 1.0, 0.0, 0.0))];

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

    let models: Vec<&str> = costs
        .per_model
        .model_costs()
        .into_iter()
        .map(|(name, _)| name)
        .collect();

    assert_eq!(models, vec!["Opus 4", "Opus", "Sonnet", "Haiku"]);
}

#[test]
fn format_session_id_truncates_to_first_eight_characters() {
    let cases = [
        ("019dc252-e50e-766c", "019dc252"),
        ("01234567", "01234567"),
        ("012345", "012345"),
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
fn session_cost_row_formats_currency_columns() {
    let row = SessionCostRow::new(
        "019dc252",
        "2025-10-23 09:00 \u{2192} 10:30",
        "1 hr 30 min",
        "Sonnet",
        (1.2345, 2.3456, 0.5, 0.25),
    );

    assert_eq!(row.session, "019dc252");
    assert_eq!(row.time_range, "2025-10-23 09:00 \u{2192} 10:30");
    assert_eq!(row.duration, "1 hr 30 min");
    assert_eq!(row.model, "Sonnet");
    assert_money_columns(&row.money, (1.2345, 2.3456, 0.5, 0.25));
}

#[test]
fn session_cost_row_total_uses_blank_component_columns() {
    let row = SessionCostRow::new_total_row(7.5);

    assert_eq!(row.session, "");
    assert_eq!(row.time_range, "");
    assert_eq!(row.duration, "");
    assert_eq!(row.model, "Total");
    assert_blank_money_component_columns(&row.money);
    assert_eq!(row.money.subtotal, "$7.5000");
}

/// Build a `SessionCosts` for a single Sonnet entry with a fixed time range.
fn session_costs_fixture(session_id: &str) -> SessionCosts {
    let start = Utc.with_ymd_and_hms(2025, 10, 23, 9, 0, 0).unwrap();
    let end = Utc.with_ymd_and_hms(2025, 10, 23, 10, 30, 0).unwrap();
    let mut per_model = ModelCostsMap::default();
    per_model.add(ModelType::Sonnet, TokenCosts::new(1.0, 2.0, 0.0, 0.0));
    SessionCosts {
        session_id: session_id.to_string(),
        start_time: start,
        end_time: end,
        per_model,
    }
}

#[test]
fn build_session_cost_rows_empty_input_returns_empty_rows() {
    let (rows, total_row_indices) = build_session_cost_rows(&[]);

    assert!(rows.is_empty());
    assert!(total_row_indices.is_empty());
}

#[test]
fn build_session_cost_rows_emits_per_model_row_then_total() {
    let session = session_costs_fixture("019dc252-e50e-766c");

    let (rows, total_row_indices) = build_session_cost_rows(std::slice::from_ref(&session));

    assert_eq!(total_row_indices, vec![1]);
    assert_eq!(rows.len(), 2);

    // First row identifies the session and model; the duration string is
    // produced by `format_duration` and must round-trip via the row.
    assert_eq!(rows[0].session, "019dc252");
    assert_eq!(rows[0].duration, "1 hr 30 min");
    assert_eq!(rows[0].model, "Sonnet");

    // Total row uses the `grouped_label` blanking pattern: the leading
    // identifier columns must be empty so the table footer reads as
    // "        Total  $...".
    assert_eq!(rows[1].session, "");
    assert_eq!(rows[1].time_range, "");
    assert_eq!(rows[1].duration, "");
    assert_eq!(rows[1].model, "Total");
    assert_eq!(rows[1].money.subtotal, "$3.0000");
}

#[test]
fn build_session_cost_rows_inserts_separator_indices_per_session() {
    let sessions = vec![
        session_costs_fixture("aaaaaaaa-aaaa"),
        session_costs_fixture("bbbbbbbb-bbbb"),
    ];

    let (rows, total_row_indices) = build_session_cost_rows(&sessions);

    // Each fixture contributes one model row + one total row, so the total
    // rows live at indices 1 and 3 in row order.
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
