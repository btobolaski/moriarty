use chrono::{DateTime, Local, Utc};
use crossterm::terminal;
use tabled::{
    settings::{
        object::Rows,
        style::{HorizontalLine, Style},
        themes::Theme,
        Alignment, Modify, Width,
    },
    Table, Tabled,
};

use super::time_filter::DateTimezone;

pub(crate) type CostComponents = (f64, f64, f64, f64);

pub(crate) fn fmt_money(amount: f64) -> String {
    let normalized = if amount == 0.0 { 0.0 } else { amount };
    format!("${normalized:.4}")
}

pub(crate) fn grouped_label(first_row: bool, value: &str) -> &str {
    if first_row {
        value
    } else {
        ""
    }
}

const MIN_WIDTH_FOR_WRAPPING: usize = 100;

#[derive(Tabled)]
pub(crate) struct FormattedCostColumns {
    #[tabled(rename = "Input")]
    pub(crate) input: String,
    #[tabled(rename = "Output")]
    pub(crate) output: String,
    #[tabled(rename = "Cache Write")]
    pub(crate) cache_write: String,
    #[tabled(rename = "Cache Read")]
    pub(crate) cache_read: String,
    #[tabled(rename = "Subtotal")]
    pub(crate) subtotal: String,
}

impl FormattedCostColumns {
    pub(crate) fn from_components(components: CostComponents) -> Self {
        let (input, output, cache_write, cache_read) = components;
        Self {
            input: fmt_money(input),
            output: fmt_money(output),
            cache_write: fmt_money(cache_write),
            cache_read: fmt_money(cache_read),
            subtotal: fmt_money(input + output + cache_write + cache_read),
        }
    }

    /// Leaving the per-component cells blank prevents the footer from looking
    /// like another model row whose subtotal should be added again.
    pub(crate) fn from_total(total_cost: f64) -> Self {
        Self {
            input: String::new(),
            output: String::new(),
            cache_write: String::new(),
            cache_read: String::new(),
            subtotal: fmt_money(total_cost),
        }
    }
}

#[derive(Tabled)]
pub(crate) struct GrandTotalRow {
    #[tabled(rename = "Grand Total")]
    pub(crate) grand_total: String,
}

impl GrandTotalRow {
    pub(crate) fn new(grand_total: f64) -> Self {
        Self {
            grand_total: fmt_money(grand_total),
        }
    }
}

fn get_terminal_width() -> usize {
    terminal::size()
        .map(|(cols, _)| cols as usize)
        .unwrap_or(80)
}

pub(crate) fn divider(width: usize) -> String {
    "=".repeat(width)
}

/// The wrap/truncate split keeps wide terminals readable without letting
/// narrow terminals explode horizontally.
pub(crate) fn apply_width_config(table: &mut Table, term_width: usize) {
    if term_width >= MIN_WIDTH_FOR_WRAPPING {
        table.with(Width::wrap(term_width).keep_words(true));
    } else {
        table.with(Width::truncate(term_width));
    }
}

pub(crate) fn push_nonzero_cost_rows<Row, Key, Items>(
    rows: &mut Vec<Row>,
    items: Items,
    mut make_row: impl FnMut(bool, Key, CostComponents) -> Row,
) where
    Items: IntoIterator<Item = (Key, CostComponents)>,
{
    let mut first_row = true;

    for (key, components) in items {
        let subtotal = components.0 + components.1 + components.2 + components.3;
        if subtotal > 0.0 {
            rows.push(make_row(first_row, key, components));
            first_row = false;
        }
    }
}

/// Rows and separator indices are produced together so callers cannot
/// accidentally render indices against a different row vector.
pub(crate) fn build_grouped_rows<Item, Row>(
    items: &[Item],
    mut push_item_rows: impl FnMut(&mut Vec<Row>, &Item),
    mut push_total_row: impl FnMut(&mut Vec<Row>, &Item, bool),
) -> (Vec<Row>, Vec<usize>) {
    let mut rows = Vec::new();
    let mut total_row_indices = Vec::new();

    for item in items {
        let rows_before_group = rows.len();
        push_item_rows(&mut rows, item);
        let has_detail_rows = rows.len() > rows_before_group;
        push_total_row(&mut rows, item, has_detail_rows);
        total_row_indices.push(rows.len() - 1);
    }

    (rows, total_row_indices)
}

pub(crate) fn create_grouped_table<T: Tabled>(rows: &[T], total_row_indices: &[usize]) -> Table {
    let mut table = Table::new(rows);
    let mut theme = Theme::from_style(Style::rounded());

    if total_row_indices.len() > 1 {
        let separator_line = HorizontalLine::full('─', '┼', '├', '┤');

        for &idx in &total_row_indices[..total_row_indices.len() - 1] {
            theme.insert_horizontal_line(idx + 2, separator_line);
        }
    }

    table.with(theme);
    table.with(Modify::new(Rows::first()).with(Alignment::center()));
    table
}

pub(crate) fn format_session_id(session_id: &str) -> String {
    let truncated: String = session_id.chars().take(8).collect();
    if truncated.is_empty() {
        session_id.to_string()
    } else {
        truncated
    }
}

/// Conversation reports use the caller-selected timezone so date bucketing and
/// rendered session ranges stay consistent.
pub(crate) fn format_time_range(
    timezone: DateTimezone,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> String {
    match timezone {
        DateTimezone::Local => format_time_range_in_zone(
            start
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
            end.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
            start.with_timezone(&Local).date_naive() == end.with_timezone(&Local).date_naive(),
            end.with_timezone(&Local).format("%H:%M").to_string(),
        ),
        DateTimezone::Utc => format_time_range_in_zone(
            start.format("%Y-%m-%d %H:%M").to_string(),
            end.format("%Y-%m-%d %H:%M").to_string(),
            start.date_naive() == end.date_naive(),
            end.format("%H:%M").to_string(),
        ),
    }
}

fn format_time_range_in_zone(
    start_full: String,
    end_full: String,
    same_day: bool,
    end_short: String,
) -> String {
    if same_day {
        format!("{start_full} → {end_short}")
    } else {
        format!("{start_full} → {end_full}")
    }
}

pub(crate) fn format_duration(minutes: i64) -> String {
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

pub(crate) fn render_cost_report<T: Tabled>(
    title: &str,
    rows: &[T],
    total_row_indices: &[usize],
    grand_total: f64,
) {
    let term_width = get_terminal_width();
    println!("{}", divider(term_width));
    println!("{}", title);
    println!("{}", divider(term_width));
    println!();

    let mut table = create_grouped_table(rows, total_row_indices);
    apply_width_config(&mut table, term_width);

    println!("{}", table);
    println!();

    display_grand_total(grand_total);
}

pub(crate) fn render_grouped_costs<Item, Row: Tabled>(
    title: &str,
    items: &[Item],
    build_rows: impl Fn(&[Item]) -> (Vec<Row>, Vec<usize>),
    total: impl Fn(&Item) -> f64,
) {
    let (rows, total_row_indices) = build_rows(items);
    let grand_total: f64 = items.iter().map(total).sum();
    render_cost_report(title, &rows, &total_row_indices, grand_total);
}

pub(crate) fn render_or_empty<T>(items: &[T], had_errors: bool, display: impl FnOnce(&[T])) {
    if items.is_empty() {
        println!("\nNo usage data found.");
    } else {
        display(items);
    }
    warn_if_incomplete(had_errors);
}

/// The detailed per-file parse errors already went to tracing; this summary is
/// only here so operators do not miss that totals may be partial.
pub(crate) fn warn_if_incomplete(had_errors: bool) {
    if had_errors {
        eprintln!(
            "\nWarning: some log files could not be read or parsed; \
             totals may be incomplete. See the per-file errors above for details."
        );
    }
}

pub(crate) fn display_grand_total(grand_total: f64) {
    let term_width = get_terminal_width();
    println!("{}", divider(term_width));
    println!("Summary");
    println!("{}", divider(term_width));
    println!();

    let row = GrandTotalRow::new(grand_total);
    let mut table = Table::new(vec![row]);

    table.with(Style::rounded());
    table.with(Modify::new(Rows::first()).with(Alignment::center()));
    apply_width_config(&mut table, term_width);

    println!("{}", table);
    println!("{}", divider(term_width));
}
