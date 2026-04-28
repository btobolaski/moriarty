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

#[derive(Tabled)]
struct CostRow {
    #[tabled(rename = "Date")]
    date: String,
    #[tabled(rename = "Model")]
    model: String,
    #[tabled(inline)]
    money: FormattedCostColumns,
}

impl CostRow {
    fn new(date: &str, model: &str, costs: CostComponents) -> Self {
        Self {
            date: date.to_string(),
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
            model: "Total".to_string(),
            money: FormattedCostColumns::from_total(total_cost),
        }
    }
}

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
    #[tabled(inline)]
    money: FormattedCostColumns,
}

impl SessionCostRow {
    fn new(
        session: &str,
        time_range: &str,
        duration: &str,
        model: &str,
        costs: CostComponents,
    ) -> Self {
        Self {
            session: session.to_string(),
            time_range: time_range.to_string(),
            duration: duration.to_string(),
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
            model: "Total".to_string(),
            money: FormattedCostColumns::from_total(total_cost),
        }
    }
}

fn build_cost_rows(daily_costs: &[DailyCosts]) -> (Vec<CostRow>, Vec<usize>) {
    build_grouped_rows(
        daily_costs,
        |rows, costs| {
            let date_str = costs.date.to_string();
            push_nonzero_cost_rows(
                rows,
                costs
                    .per_model
                    .model_costs()
                    .into_iter()
                    .map(|(name, costs)| (name, costs.as_components())),
                |first_row, model_name, components| {
                    CostRow::new(grouped_label(first_row, &date_str), model_name, components)
                },
            );
        },
        |rows, costs, has_detail_rows| {
            rows.push(if has_detail_rows {
                CostRow::new_total_row(costs.total())
            } else {
                CostRow::new_labeled_total_row(&costs.date.to_string(), costs.total())
            })
        },
    )
}

fn build_session_cost_rows(
    session_costs: &[SessionCosts],
    timezone: DateTimezone,
) -> (Vec<SessionCostRow>, Vec<usize>) {
    build_grouped_rows(
        session_costs,
        |rows, costs| {
            let session_id = format_session_id(&costs.session_id);
            let time_range = format_time_range(timezone, costs.start_time, costs.end_time);
            let duration = format_duration(costs.duration_minutes());
            push_nonzero_cost_rows(
                rows,
                costs
                    .per_model
                    .model_costs()
                    .into_iter()
                    .map(|(name, costs)| (name, costs.as_components())),
                |first_row, model_name, components| {
                    SessionCostRow::new(
                        grouped_label(first_row, &session_id),
                        grouped_label(first_row, &time_range),
                        grouped_label(first_row, &duration),
                        model_name,
                        components,
                    )
                },
            );
        },
        |rows, costs, has_detail_rows| {
            rows.push(if has_detail_rows {
                SessionCostRow::new_total_row(costs.total())
            } else {
                SessionCostRow::new_labeled_total_row(
                    &format_session_id(&costs.session_id),
                    &format_time_range(timezone, costs.start_time, costs.end_time),
                    &format_duration(costs.duration_minutes()),
                    costs.total(),
                )
            })
        },
    )
}

fn display_session_costs(session_costs: &[SessionCosts], timezone: DateTimezone) {
    render_grouped_costs(
        "API Cost Report by Conversation",
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

fn display_costs(daily_costs: &[DailyCosts]) {
    render_grouped_costs(
        "API Cost Report",
        daily_costs,
        build_cost_rows,
        DailyCosts::total,
    );
}

#[cfg(test)]
mod tests;
