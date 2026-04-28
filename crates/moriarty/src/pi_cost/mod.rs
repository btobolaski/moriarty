mod analyzer;
mod pricing;

use std::path::Path;

use tabled::Tabled;

use crate::cost_report::{
    build_grouped_rows, format_duration, format_session_id, format_time_range, grouped_label,
    push_nonzero_cost_rows, render_grouped_costs, render_or_empty, CostComponents, DateTimezone,
    FormattedCostColumns, TimeRangeFilter,
};
use analyzer::{DailyCosts, SessionCosts};
use pi_logs::Provider;

#[derive(Tabled)]
struct PiCostRow {
    #[tabled(rename = "Date")]
    date: String,
    #[tabled(rename = "Provider")]
    provider: String,
    #[tabled(rename = "Model")]
    model: String,
    #[tabled(inline)]
    money: FormattedCostColumns,
}

impl PiCostRow {
    fn new(date: &str, provider: &str, model: &str, costs: CostComponents) -> Self {
        Self {
            date: date.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            money: FormattedCostColumns::from_components(costs),
        }
    }

    fn new_total_row(total_cost: f64) -> Self {
        Self::new_labeled_total_row("", total_cost)
    }

    fn new_labeled_total_row(date: &str, total_cost: f64) -> Self {
        Self {
            date: date.to_string(),
            provider: String::new(),
            model: "Total".to_string(),
            money: FormattedCostColumns::from_total(total_cost),
        }
    }
}

#[derive(Tabled)]
struct PiSessionCostRow {
    #[tabled(rename = "Session")]
    session: String,
    #[tabled(rename = "Time Range")]
    time_range: String,
    #[tabled(rename = "Duration")]
    duration: String,
    #[tabled(rename = "Provider")]
    provider: String,
    #[tabled(rename = "Model")]
    model: String,
    #[tabled(inline)]
    money: FormattedCostColumns,
}

impl PiSessionCostRow {
    fn new(
        session: &str,
        time_range: &str,
        duration: &str,
        provider: &str,
        model: &str,
        costs: CostComponents,
    ) -> Self {
        Self {
            session: session.to_string(),
            time_range: time_range.to_string(),
            duration: duration.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            money: FormattedCostColumns::from_components(costs),
        }
    }

    fn new_total_row(total_cost: f64) -> Self {
        Self::new_labeled_total_row("", "", "", total_cost)
    }

    fn new_labeled_total_row(
        session: &str,
        time_range: &str,
        duration: &str,
        total_cost: f64,
    ) -> Self {
        Self {
            session: session.to_string(),
            time_range: time_range.to_string(),
            duration: duration.to_string(),
            provider: String::new(),
            model: "Total".to_string(),
            money: FormattedCostColumns::from_total(total_cost),
        }
    }
}

fn provider_label(provider: Provider) -> &'static str {
    match provider {
        Provider::Anthropic => "Anthropic",
        Provider::OpenAi => "OpenAI",
    }
}

fn build_cost_rows(daily_costs: &[DailyCosts]) -> (Vec<PiCostRow>, Vec<usize>) {
    build_grouped_rows(
        daily_costs,
        |rows, costs| {
            let date_str = costs.date.to_string();
            push_nonzero_cost_rows(
                rows,
                costs.per_model.model_costs().map(|(model, costs)| {
                    (
                        (provider_label(model.provider), model.model.as_str()),
                        costs.as_components(),
                    )
                }),
                |first_row, (provider, model), components| {
                    PiCostRow::new(
                        grouped_label(first_row, &date_str),
                        provider,
                        model,
                        components,
                    )
                },
            );
        },
        |rows, costs, has_detail_rows| {
            rows.push(if has_detail_rows {
                PiCostRow::new_total_row(costs.total())
            } else {
                PiCostRow::new_labeled_total_row(&costs.date.to_string(), costs.total())
            })
        },
    )
}

fn build_session_cost_rows(
    session_costs: &[SessionCosts],
    timezone: DateTimezone,
) -> (Vec<PiSessionCostRow>, Vec<usize>) {
    build_grouped_rows(
        session_costs,
        |rows, costs| {
            let session_id = format_session_id(&costs.session_id);
            let time_range = format_time_range(timezone, costs.start_time, costs.end_time);
            let duration = format_duration(costs.duration_minutes());
            push_nonzero_cost_rows(
                rows,
                costs.per_model.model_costs().map(|(model, costs)| {
                    (
                        (provider_label(model.provider), model.model.as_str()),
                        costs.as_components(),
                    )
                }),
                |first_row, (provider, model), components| {
                    PiSessionCostRow::new(
                        grouped_label(first_row, &session_id),
                        grouped_label(first_row, &time_range),
                        grouped_label(first_row, &duration),
                        provider,
                        model,
                        components,
                    )
                },
            );
        },
        |rows, costs, has_detail_rows| {
            rows.push(if has_detail_rows {
                PiSessionCostRow::new_total_row(costs.total())
            } else {
                PiSessionCostRow::new_labeled_total_row(
                    &format_session_id(&costs.session_id),
                    &format_time_range(timezone, costs.start_time, costs.end_time),
                    &format_duration(costs.duration_minutes()),
                    costs.total(),
                )
            })
        },
    )
}

fn display_costs(daily_costs: &[DailyCosts]) {
    render_grouped_costs(
        "Pi Cost Report",
        daily_costs,
        build_cost_rows,
        DailyCosts::total,
    );
}

fn display_session_costs(session_costs: &[SessionCosts], timezone: DateTimezone) {
    render_grouped_costs(
        "Pi Cost Report by Conversation",
        session_costs,
        |items| build_session_cost_rows(items, timezone),
        SessionCosts::total,
    );
}

pub async fn run_by_session(
    dir: &Path,
    timezone: DateTimezone,
    filter: &TimeRangeFilter,
) -> miette::Result<()> {
    let result = analyzer::analyze_directory_by_session(dir, filter).await?;
    render_or_empty(&result.session_costs, result.had_errors, |items| {
        display_session_costs(items, timezone)
    });
    Ok(())
}

pub async fn run(
    dir: &Path,
    timezone: DateTimezone,
    by_conversation: bool,
    filter: &TimeRangeFilter,
) -> miette::Result<()> {
    if by_conversation {
        return run_by_session(dir, timezone, filter).await;
    }

    let result = analyzer::analyze_directory(dir, timezone, filter).await?;
    render_or_empty(&result.daily_costs, result.had_errors, display_costs);
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, TimeZone, Utc};

    use super::*;
    use crate::{
        cost_report::{fmt_money, FormattedCostColumns},
        pi_cost::{
            analyzer::{DailyCosts, SessionCosts},
            pricing::{PiModelCostsMap, TokenCosts},
        },
    };
    use cost_analyzer::PiModel;

    fn test_date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn costs_on(year: i32, month: u32, day: u32) -> DailyCosts {
        DailyCosts {
            date: test_date(year, month, day),
            per_model: PiModelCostsMap::default(),
        }
    }

    trait DailyCostsExt {
        fn with_model(
            self,
            provider: Provider,
            model: &str,
            input: f64,
            output: f64,
            cache_write: f64,
            cache_read: f64,
        ) -> Self;
    }

    impl DailyCostsExt for DailyCosts {
        fn with_model(
            mut self,
            provider: Provider,
            model: &str,
            input: f64,
            output: f64,
            cache_write: f64,
            cache_read: f64,
        ) -> Self {
            self.per_model.add(
                PiModel {
                    provider,
                    model: model.to_string(),
                },
                TokenCosts::new(input, output, cache_write, cache_read),
            );
            self
        }
    }

    fn session_costs_fixture(session_id: &str) -> SessionCosts {
        let start = Utc.with_ymd_and_hms(2025, 10, 23, 9, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2025, 10, 23, 10, 30, 0).unwrap();
        let mut per_model = PiModelCostsMap::default();
        per_model.add(
            PiModel {
                provider: Provider::Anthropic,
                model: "claude-sonnet-4-5".to_string(),
            },
            TokenCosts::new(1.0, 2.0, 0.0, 0.0),
        );
        SessionCosts {
            session_id: session_id.to_string(),
            start_time: start,
            end_time: end,
            per_model,
        }
    }

    fn assert_money_columns(money: &FormattedCostColumns, components: (f64, f64, f64, f64)) {
        let (input, output, cache_write, cache_read) = components;
        assert_eq!(money.input, fmt_money(input));
        assert_eq!(money.output, fmt_money(output));
        assert_eq!(money.cache_write, fmt_money(cache_write));
        assert_eq!(money.cache_read, fmt_money(cache_read));
        assert_eq!(
            money.subtotal,
            fmt_money(input + output + cache_write + cache_read)
        );
    }

    fn assert_blank_money_component_columns(money: &FormattedCostColumns) {
        assert_eq!(money.input, "");
        assert_eq!(money.output, "");
        assert_eq!(money.cache_write, "");
        assert_eq!(money.cache_read, "");
    }

    #[test]
    fn pi_cost_row_formats_provider_and_model_columns() {
        let row = PiCostRow::new(
            "2025-10-23",
            "Anthropic",
            "claude-sonnet-4-5",
            (1.25, 2.5, 0.5, 0.25),
        );

        assert_eq!(row.date, "2025-10-23");
        assert_eq!(row.provider, "Anthropic");
        assert_eq!(row.model, "claude-sonnet-4-5");
        assert_money_columns(&row.money, (1.25, 2.5, 0.5, 0.25));
    }

    #[test]
    fn pi_cost_row_total_uses_blank_component_columns() {
        let row = PiCostRow::new_total_row(7.5);

        assert_eq!(row.date, "");
        assert_eq!(row.provider, "");
        assert_eq!(row.model, "Total");
        assert_blank_money_component_columns(&row.money);
        assert_eq!(row.money.subtotal, "$7.5000");
    }

    #[test]
    fn build_cost_rows_preserves_provider_then_model_order() {
        let daily_costs = vec![costs_on(2025, 10, 23)
            .with_model(Provider::OpenAi, "gpt-5", 1.0, 0.0, 0.0, 0.0)
            .with_model(Provider::Anthropic, "claude-sonnet-4-5", 2.0, 0.0, 0.0, 0.0)
            .with_model(Provider::Anthropic, "claude-haiku-3-5", 0.5, 0.0, 0.0, 0.0)];

        let (rows, total_row_indices) = build_cost_rows(&daily_costs);

        assert_eq!(total_row_indices, vec![3]);
        assert_eq!(rows[0].provider, "Anthropic");
        assert_eq!(rows[0].model, "claude-haiku-3-5");
        assert_eq!(rows[1].provider, "Anthropic");
        assert_eq!(rows[1].model, "claude-sonnet-4-5");
        assert_eq!(rows[2].provider, "OpenAI");
        assert_eq!(rows[2].model, "gpt-5");
        assert_eq!(rows[3].model, "Total");
    }

    #[test]
    fn build_cost_rows_zero_cost_day_still_gets_labeled_total_row() {
        let (rows, total_row_indices) = build_cost_rows(&[costs_on(2025, 10, 23)]);

        assert_eq!(total_row_indices, vec![0]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].date, "2025-10-23");
        assert_eq!(rows[0].provider, "");
        assert_eq!(rows[0].model, "Total");
        assert_eq!(rows[0].money.subtotal, "$0.0000");
    }

    #[test]
    fn pi_session_cost_row_total_uses_blank_component_columns() {
        let row = PiSessionCostRow::new_total_row(4.0);

        assert_eq!(row.session, "");
        assert_eq!(row.time_range, "");
        assert_eq!(row.duration, "");
        assert_eq!(row.provider, "");
        assert_eq!(row.model, "Total");
        assert_blank_money_component_columns(&row.money);
        assert_eq!(row.money.subtotal, "$4.0000");
    }

    #[test]
    fn build_session_cost_rows_zero_cost_session_keeps_identifying_columns() {
        let session = SessionCosts {
            session_id: "ééééééééé-session".to_string(),
            start_time: Utc.with_ymd_and_hms(2025, 10, 23, 9, 0, 0).unwrap(),
            end_time: Utc.with_ymd_and_hms(2025, 10, 23, 9, 0, 0).unwrap(),
            per_model: PiModelCostsMap::default(),
        };

        let (rows, total_row_indices) = build_session_cost_rows(&[session], DateTimezone::Utc);

        assert_eq!(total_row_indices, vec![0]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].session, "éééééééé");
        assert_eq!(rows[0].time_range, "2025-10-23 09:00 → 09:00");
        assert_eq!(rows[0].duration, "0 min");
        assert_eq!(rows[0].provider, "");
        assert_eq!(rows[0].model, "Total");
    }

    #[test]
    fn build_session_cost_rows_only_first_row_repeats_identifying_columns() {
        let mut session = session_costs_fixture("019dc252-e50e-766c");
        session.per_model.add(
            PiModel {
                provider: Provider::OpenAi,
                model: "gpt-5".to_string(),
            },
            TokenCosts::new(0.5, 0.5, 0.0, 0.0),
        );

        let (rows, total_row_indices) = build_session_cost_rows(&[session], DateTimezone::Utc);

        assert_eq!(total_row_indices, vec![2]);
        assert_eq!(rows.len(), 3);

        assert_eq!(rows[0].session, "019dc252");
        assert!(!rows[0].time_range.is_empty());
        assert_eq!(rows[0].duration, "1 hr 30 min");

        assert_eq!(rows[1].session, "");
        assert_eq!(rows[1].time_range, "");
        assert_eq!(rows[1].duration, "");
        assert_eq!(rows[2].model, "Total");
    }
}
