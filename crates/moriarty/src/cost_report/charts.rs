use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    env,
    io::{self, IsTerminal},
};

use crossterm::{
    style::{Color, Stylize},
    terminal,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::render::{MetricTotal, ReportMode, divider, fmt_money, fmt_tokens};

const DEFAULT_TOP_N: usize = 5;
const DEFAULT_TIME_BAR_WIDTH: usize = 48;
const DEFAULT_SHARE_BAR_WIDTH: usize = 40;
const MIN_TIME_BAR_WIDTH: usize = 8;
const MIN_SHARE_BAR_WIDTH: usize = 12;

const MARKER_DISPLAY_WIDTH: usize = 1;
const GLYPHS: [char; 6] = ['█', '▓', '▒', '░', '▇', '▆'];
const COLORS: [Color; 6] = [
    Color::Blue,
    Color::Magenta,
    Color::Cyan,
    Color::Green,
    Color::Yellow,
    Color::Red,
];

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ChartBucket {
    pub(crate) label: String,
    pub(crate) segments: Vec<ChartSegment>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ChartSegment {
    pub(crate) label: String,
    pub(crate) total: MetricTotal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChartColorMode {
    Auto,
    #[cfg_attr(not(test), allow(dead_code))]
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChartRenderOptions {
    pub(crate) top_n: usize,
    pub(crate) color_mode: ChartColorMode,
    pub(crate) terminal_width: Option<usize>,
}

impl Default for ChartRenderOptions {
    fn default() -> Self {
        Self {
            top_n: DEFAULT_TOP_N,
            color_mode: ChartColorMode::Auto,
            terminal_width: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SegmentId {
    Label(String),
    Overflow,
}

#[derive(Debug, Clone, PartialEq)]
struct OrderedSegment {
    id: SegmentId,
    label: String,
    total: MetricTotal,
    glyph: char,
    color: Option<Color>,
}

#[derive(Debug, Clone, PartialEq)]
struct NormalizedBucket {
    label: String,
    total: MetricTotal,
    segment_totals: Vec<MetricTotal>,
}

#[derive(Debug, Clone, PartialEq)]
struct SelectedChart {
    buckets: Vec<NormalizedBucket>,
    segments: Vec<OrderedSegment>,
    grand_total: MetricTotal,
}

pub(crate) fn render_stacked_charts(
    title: &str,
    time_series_title: &str,
    share_title: &str,
    buckets: &[ChartBucket],
    report_mode: ReportMode,
) -> miette::Result<()> {
    print!(
        "{}",
        build_stacked_charts_output(
            title,
            time_series_title,
            share_title,
            buckets,
            report_mode,
            ChartRenderOptions::default(),
        )?
    );
    Ok(())
}

fn build_stacked_charts_output(
    title: &str,
    time_series_title: &str,
    share_title: &str,
    buckets: &[ChartBucket],
    report_mode: ReportMode,
    options: ChartRenderOptions,
) -> miette::Result<String> {
    let selected = select_segments(buckets, report_mode, options.top_n)?;
    let term_width = options.terminal_width.unwrap_or_else(get_terminal_width);
    let use_color = use_ansi_color(options.color_mode);

    let mut lines = vec![
        divider(term_width),
        title.to_string(),
        divider(term_width),
        String::new(),
    ];

    lines.extend(render_time_series_chart_lines(
        time_series_title,
        &selected,
        term_width,
        use_color,
    ));
    lines.push(String::new());
    lines.extend(render_share_chart_lines(
        share_title,
        &selected,
        term_width,
        use_color,
    ));
    lines.push(String::new());
    lines.push(format!(
        "Grand Total: {}",
        format_metric_total(selected.grand_total)
    ));
    lines.push(divider(term_width));

    Ok(lines.join("\n") + "\n")
}

fn select_segments(
    buckets: &[ChartBucket],
    report_mode: ReportMode,
    top_n: usize,
) -> miette::Result<SelectedChart> {
    let mut aggregate_totals: HashMap<String, (MetricTotal, usize)> = HashMap::new();
    let mut normalized_buckets = Vec::with_capacity(buckets.len());
    let mut grand_total = MetricTotal::zero(report_mode);
    let mut next_index = 0usize;

    for bucket in buckets {
        let mut bucket_totals: HashMap<String, MetricTotal> = HashMap::new();
        let mut bucket_total = MetricTotal::zero(report_mode);

        for segment in &bucket.segments {
            bucket_total = bucket_total.checked_add(segment.total)?;

            if is_zero_total(segment.total) {
                continue;
            }

            let entry = bucket_totals
                .entry(segment.label.clone())
                .or_insert_with(|| MetricTotal::zero(report_mode));
            *entry = entry.checked_add(segment.total)?;

            let entry = aggregate_totals
                .entry(segment.label.clone())
                .or_insert_with(|| {
                    let index = next_index;
                    next_index += 1;
                    (MetricTotal::zero(report_mode), index)
                });
            entry.0 = entry.0.checked_add(segment.total)?;
        }

        grand_total = grand_total.checked_add(bucket_total)?;
        normalized_buckets.push((bucket.label.clone(), bucket_total, bucket_totals));
    }

    let mut ordered_segments: Vec<_> = aggregate_totals
        .into_iter()
        .map(|(label, (total, index))| (label, total, index))
        .collect();
    ordered_segments.sort_by(|left, right| {
        compare_totals_desc(left.1, right.1).then_with(|| left.2.cmp(&right.2))
    });

    let visible_segment_count = ordered_segments.len().min(top_n.max(1));
    let mut visible_segments = Vec::with_capacity(visible_segment_count + 1);

    for (idx, (label, total, _)) in ordered_segments
        .iter()
        .take(visible_segment_count)
        .enumerate()
    {
        visible_segments.push(OrderedSegment {
            id: SegmentId::Label(label.clone()),
            label: label.clone(),
            total: *total,
            glyph: GLYPHS[idx % GLYPHS.len()],
            color: Some(COLORS[idx % COLORS.len()]),
        });
    }

    let overflow_total = ordered_segments
        .iter()
        .skip(visible_segment_count)
        .try_fold(MetricTotal::zero(report_mode), |acc, (_, total, _)| {
            acc.checked_add(*total)
        })?;
    let has_other = !is_zero_total(overflow_total);

    if has_other {
        let index = visible_segments.len();
        visible_segments.push(OrderedSegment {
            id: SegmentId::Overflow,
            label: unique_overflow_label(&visible_segments),
            total: overflow_total,
            glyph: GLYPHS[index % GLYPHS.len()],
            color: Some(Color::DarkGrey),
        });
    }

    let visible_labels: Vec<_> = ordered_segments
        .iter()
        .take(visible_segment_count)
        .map(|(label, _, _)| label.clone())
        .collect();

    let buckets = normalized_buckets
        .into_iter()
        .map(|(label, total, bucket_totals)| {
            let mut segment_totals = Vec::with_capacity(visible_segments.len());

            for segment in &visible_segments {
                match &segment.id {
                    SegmentId::Label(visible_label) => segment_totals.push(
                        bucket_totals
                            .get(visible_label)
                            .copied()
                            .unwrap_or_else(|| MetricTotal::zero(report_mode)),
                    ),
                    SegmentId::Overflow => {
                        let other_total = bucket_totals
                            .iter()
                            .filter(|(segment_label, _)| !visible_labels.contains(segment_label))
                            .try_fold(MetricTotal::zero(report_mode), |acc, (_, total)| {
                                acc.checked_add(*total)
                            })?;
                        segment_totals.push(other_total);
                    }
                }
            }

            Ok(NormalizedBucket {
                label,
                total,
                segment_totals,
            })
        })
        .collect::<miette::Result<Vec<_>>>()?;

    Ok(SelectedChart {
        buckets,
        segments: visible_segments,
        grand_total,
    })
}

fn render_time_series_chart_lines(
    title: &str,
    chart: &SelectedChart,
    term_width: usize,
    use_color: bool,
) -> Vec<String> {
    let mut lines = vec![title.to_string(), String::new()];
    lines.extend(render_legend_lines(&chart.segments, term_width, use_color));

    let max_label_width = chart
        .buckets
        .iter()
        .map(|bucket| display_width(&bucket.label))
        .max()
        .unwrap_or(0);
    let total_width = chart
        .buckets
        .iter()
        .map(|bucket| display_width(&format_metric_total(bucket.total)))
        .max()
        .unwrap_or_else(|| display_width(&format_metric_total(chart.grand_total)));
    let (label_width, bar_width) = time_chart_layout(term_width, max_label_width, total_width);
    let max_total = chart
        .buckets
        .iter()
        .map(|bucket| bucket.total)
        .max_by(|left, right| compare_totals_desc(*right, *left));

    if chart.buckets.is_empty() {
        lines.push("No chart buckets to render.".to_string());
        return lines;
    }

    for bucket in &chart.buckets {
        let label = truncate_to_width(&bucket.label, label_width);
        let total = format_metric_total(bucket.total);

        if bar_width == 0 {
            lines.push(format!(
                "{}  {}",
                pad_right(&label, label_width),
                pad_left(&total, total_width),
            ));
            continue;
        }

        let scaled_total_width = match max_total {
            Some(max_total) => scaled_bar_width(bucket.total, max_total, bar_width),
            None => 0,
        };
        let segment_widths = allocate_segment_widths(&bucket.segment_totals, scaled_total_width);
        let bar = render_bar(&chart.segments, &segment_widths, use_color);
        let filled_width = segment_widths.iter().sum::<usize>();
        let bar_padding = " ".repeat(bar_width.saturating_sub(filled_width));

        lines.push(format!(
            "{}  {}{}  {}",
            pad_right(&label, label_width),
            bar,
            bar_padding,
            pad_left(&total, total_width),
        ));
    }

    lines
}

fn render_share_chart_lines(
    title: &str,
    chart: &SelectedChart,
    term_width: usize,
    use_color: bool,
) -> Vec<String> {
    let mut lines = vec![title.to_string(), String::new()];

    if chart.segments.is_empty() {
        lines.push("No non-zero model/provider totals.".to_string());
        return lines;
    }

    let value_width = chart
        .segments
        .iter()
        .map(|segment| display_width(&format_metric_total(segment.total)))
        .max()
        .unwrap_or(0);
    let bar_width = share_bar_width(term_width);
    let segment_totals: Vec<_> = chart.segments.iter().map(|segment| segment.total).collect();
    let bar = render_bar(
        &chart.segments,
        &allocate_segment_widths(&segment_totals, bar_width),
        use_color,
    );
    let filled_width = allocate_segment_widths(&segment_totals, bar_width)
        .iter()
        .sum::<usize>();
    lines.push(format!(
        "[{}{}]",
        bar,
        " ".repeat(bar_width.saturating_sub(filled_width))
    ));
    lines.push(String::new());

    for segment in &chart.segments {
        lines.push(render_share_detail_line(
            segment,
            chart.grand_total,
            term_width,
            value_width,
            use_color,
        ));
    }

    lines
}

fn render_share_detail_line(
    segment: &OrderedSegment,
    grand_total: MetricTotal,
    term_width: usize,
    value_width: usize,
    use_color: bool,
) -> String {
    let marker = render_marker(segment, use_color);
    let value = format_metric_total(segment.total);
    let percent = format_share_percent(segment.total, grand_total);

    let full_fixed_width =
        1 + MARKER_DISPLAY_WIDTH + 1 + 2 + display_width(&percent) + 2 + value_width;
    if full_fixed_width <= term_width {
        let label_width = term_width.saturating_sub(full_fixed_width);
        let label = truncate_to_width(&segment.label, label_width);
        return format!(
            " {} {}  {}  {}",
            marker,
            pad_right(&label, label_width),
            percent,
            pad_left(&value, value_width),
        );
    }

    let percent_fixed_width = 1 + MARKER_DISPLAY_WIDTH + 1 + 2 + display_width(&percent);
    if percent_fixed_width <= term_width {
        let label_width = term_width.saturating_sub(percent_fixed_width);
        let label = truncate_to_width(&segment.label, label_width);
        return format!(
            " {} {}  {}",
            marker,
            pad_right(&label, label_width),
            percent
        );
    }

    let value_fixed_width = 1 + MARKER_DISPLAY_WIDTH + 1 + 2 + display_width(&value);
    if value_fixed_width <= term_width {
        let label_width = term_width.saturating_sub(value_fixed_width);
        let label = truncate_to_width(&segment.label, label_width);
        return format!(" {} {}  {}", marker, pad_right(&label, label_width), value);
    }

    let label_fixed_width = 1 + MARKER_DISPLAY_WIDTH + 1;
    if label_fixed_width <= term_width {
        let label = truncate_to_width(&segment.label, term_width.saturating_sub(label_fixed_width));
        return if label.is_empty() {
            format!(" {}", marker)
        } else {
            format!(" {} {}", marker, label)
        };
    }

    truncate_to_width(&marker, term_width)
}

fn format_share_percent(segment_total: MetricTotal, grand_total: MetricTotal) -> String {
    let percent = if is_zero_total(grand_total) {
        0.0
    } else {
        (numeric_total(segment_total) / numeric_total(grand_total)) * 100.0
    };

    format!("{:>5.1}%", percent)
}

fn render_legend_lines(
    segments: &[OrderedSegment],
    term_width: usize,
    use_color: bool,
) -> Vec<String> {
    if segments.is_empty() {
        return vec![truncate_to_width("Legend: none", term_width), String::new()];
    }

    let prefix = "Legend: ";
    let prefix_width = display_width(prefix);
    if term_width < prefix_width + MARKER_DISPLAY_WIDTH {
        let mut lines = vec![truncate_to_width("Legend:", term_width)];
        for segment in segments {
            let (item, _) = render_legend_item(
                segment,
                term_width.saturating_sub(MARKER_DISPLAY_WIDTH + 1),
                use_color,
            );
            lines.push(item);
        }
        lines.push(String::new());
        return lines;
    }

    let mut lines = Vec::new();
    let indent = " ".repeat(prefix.len());
    let mut current_line = prefix.to_string();
    let mut current_width = prefix_width;

    for segment in segments {
        let separator_width = if current_width == prefix_width { 0 } else { 2 };
        let available_width = term_width.saturating_sub(current_width + separator_width);

        if current_width > prefix_width && available_width < MARKER_DISPLAY_WIDTH {
            lines.push(current_line);
            current_line = indent.clone();
            current_width = prefix_width;
        }

        // Prefer vertical space over truncation when a complete legend
        // item can still fit on its own line.
        if current_width > prefix_width {
            let full_item_width = MARKER_DISPLAY_WIDTH + 1 + display_width(&segment.label);
            let fits_on_current = current_width + 2 + full_item_width <= term_width;
            let fits_on_fresh = prefix_width + full_item_width <= term_width;
            if !fits_on_current && fits_on_fresh {
                lines.push(current_line);
                current_line = indent.clone();
                current_width = prefix_width;
            }
        }

        let separator_width = if current_width == prefix_width { 0 } else { 2 };
        let available_width = term_width.saturating_sub(current_width + separator_width);
        let label_width = available_width.saturating_sub(MARKER_DISPLAY_WIDTH + 1);
        let (item, item_width) = render_legend_item(segment, label_width, use_color);

        if current_width > prefix_width {
            current_line.push_str("  ");
            current_width += 2;
        }
        current_line.push_str(&item);
        current_width += item_width;
    }

    lines.push(current_line);
    lines.push(String::new());
    lines
}

fn render_legend_item(
    segment: &OrderedSegment,
    label_width: usize,
    use_color: bool,
) -> (String, usize) {
    let marker = render_marker(segment, use_color);
    let label = truncate_to_width(&segment.label, label_width);

    if label.is_empty() {
        (marker, MARKER_DISPLAY_WIDTH)
    } else {
        (
            format!("{} {}", marker, label),
            MARKER_DISPLAY_WIDTH + 1 + display_width(&label),
        )
    }
}

fn render_bar(segments: &[OrderedSegment], widths: &[usize], use_color: bool) -> String {
    let mut bar = String::new();

    for (segment, width) in segments.iter().zip(widths.iter().copied()) {
        if width == 0 {
            continue;
        }

        let chunk = segment.glyph.to_string().repeat(width);
        if use_color {
            if let Some(color) = segment.color {
                bar.push_str(&format!("{}", chunk.with(color)));
            } else {
                bar.push_str(&chunk);
            }
        } else {
            bar.push_str(&chunk);
        }
    }

    bar
}

fn render_marker(segment: &OrderedSegment, use_color: bool) -> String {
    let marker = segment.glyph.to_string();
    if use_color && let Some(color) = segment.color {
        return format!("{}", marker.with(color));
    }
    marker
}

fn allocate_segment_widths(segment_totals: &[MetricTotal], target_width: usize) -> Vec<usize> {
    if target_width == 0 || segment_totals.is_empty() {
        return vec![0; segment_totals.len()];
    }

    let sum: f64 = segment_totals
        .iter()
        .map(|total| numeric_total(*total))
        .sum();
    if sum <= 0.0 {
        return vec![0; segment_totals.len()];
    }

    let mut widths = Vec::with_capacity(segment_totals.len());
    let mut remainders = Vec::with_capacity(segment_totals.len());
    let mut assigned = 0usize;

    for (idx, total) in segment_totals.iter().enumerate() {
        let exact = (numeric_total(*total) / sum) * target_width as f64;
        let floor = exact.floor() as usize;
        widths.push(floor);
        remainders.push((idx, exact - floor as f64));
        assigned += floor;
    }

    remainders.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });

    for (idx, _) in remainders
        .into_iter()
        .take(target_width.saturating_sub(assigned))
    {
        widths[idx] += 1;
    }

    widths
}

fn scaled_bar_width(total: MetricTotal, max_total: MetricTotal, max_width: usize) -> usize {
    if max_width == 0 || is_zero_total(total) || is_zero_total(max_total) {
        return 0;
    }

    let scaled =
        ((numeric_total(total) / numeric_total(max_total)) * max_width as f64).round() as usize;
    scaled.clamp(1, max_width)
}

fn time_chart_layout(
    term_width: usize,
    max_label_width: usize,
    total_width: usize,
) -> (usize, usize) {
    let available = term_width.saturating_sub(total_width + 4);
    if available == 0 {
        return (0, 0);
    }

    let preferred_bar_width = DEFAULT_TIME_BAR_WIDTH.min(available.saturating_sub(1));
    let mut label_width = max_label_width;
    let min_bar_width = MIN_TIME_BAR_WIDTH.min(available);

    if label_width + preferred_bar_width > available {
        label_width = label_width.min(available.saturating_sub(min_bar_width));
    }

    if label_width == 0 && max_label_width > 0 && available > 1 {
        label_width = 1;
    }

    let mut bar_width = available.saturating_sub(label_width);
    if bar_width == 0 && available > 0 {
        label_width = label_width.saturating_sub(1);
        bar_width = 1;
    }

    (label_width, bar_width)
}

fn share_bar_width(term_width: usize) -> usize {
    term_width.saturating_sub(2).clamp(
        MIN_SHARE_BAR_WIDTH.min(term_width.saturating_sub(2)),
        DEFAULT_SHARE_BAR_WIDTH,
    )
}

fn unique_overflow_label(visible_segments: &[OrderedSegment]) -> String {
    let used_labels: HashSet<_> = visible_segments
        .iter()
        .map(|segment| segment.label.as_str())
        .collect();

    if !used_labels.contains("Other") {
        return "Other".to_string();
    }

    let mut index = 1usize;
    loop {
        let candidate = if index == 1 {
            "Other (grouped)".to_string()
        } else {
            format!("Other (grouped {index})")
        };
        if !used_labels.contains(candidate.as_str()) {
            return candidate;
        }
        index += 1;
    }
}

fn compare_totals_desc(left: MetricTotal, right: MetricTotal) -> Ordering {
    match (left, right) {
        (MetricTotal::Cost(left), MetricTotal::Cost(right)) => right.total_cmp(&left),
        (MetricTotal::Tokens(left), MetricTotal::Tokens(right)) => right.cmp(&left),
        (MetricTotal::Cost(_), MetricTotal::Tokens(_)) => Ordering::Equal,
        (MetricTotal::Tokens(_), MetricTotal::Cost(_)) => Ordering::Equal,
    }
}

fn numeric_total(total: MetricTotal) -> f64 {
    match total {
        MetricTotal::Cost(amount) => amount,
        MetricTotal::Tokens(amount) => amount as f64,
    }
}

fn is_zero_total(total: MetricTotal) -> bool {
    match total {
        MetricTotal::Cost(amount) => amount == 0.0,
        MetricTotal::Tokens(amount) => amount == 0,
    }
}

fn format_metric_total(total: MetricTotal) -> String {
    match total {
        MetricTotal::Cost(amount) => fmt_money(amount),
        MetricTotal::Tokens(amount) => fmt_tokens(amount),
    }
}

fn get_terminal_width() -> usize {
    terminal::size()
        .map(|(cols, _)| cols as usize)
        .unwrap_or(80)
}

fn use_ansi_color(color_mode: ChartColorMode) -> bool {
    match color_mode {
        ChartColorMode::Never => false,
        ChartColorMode::Auto => env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal(),
    }
}

fn truncate_to_width(value: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }

    if display_width(value) <= max_width {
        return value.to_string();
    }

    if max_width == 1 {
        return "…".to_string();
    }

    let mut truncated = String::new();
    let mut width = 0usize;

    for ch in value.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width + 1 > max_width {
            break;
        }
        truncated.push(ch);
        width += ch_width;
    }

    truncated.push('…');
    truncated
}

fn pad_right(value: &str, width: usize) -> String {
    format!(
        "{}{}",
        value,
        " ".repeat(width.saturating_sub(display_width(value)))
    )
}

fn pad_left(value: &str, width: usize) -> String {
    format!(
        "{}{}",
        " ".repeat(width.saturating_sub(display_width(value))),
        value
    )
}

fn display_width(value: &str) -> usize {
    UnicodeWidthStr::width(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cost(amount: f64) -> MetricTotal {
        MetricTotal::Cost(amount)
    }

    fn tokens(amount: u128) -> MetricTotal {
        MetricTotal::Tokens(amount)
    }

    fn bucket(label: &str, segments: &[(&str, MetricTotal)]) -> ChartBucket {
        ChartBucket {
            label: label.to_string(),
            segments: segments
                .iter()
                .map(|(label, total)| ChartSegment {
                    label: (*label).to_string(),
                    total: *total,
                })
                .collect(),
        }
    }

    #[test]
    fn select_segments_keeps_top_five_and_groups_other() {
        let buckets = vec![bucket(
            "2026-05-01",
            &[
                ("Alpha", cost(10.0)),
                ("Beta", cost(9.0)),
                ("Gamma", cost(8.0)),
                ("Delta", cost(7.0)),
                ("Epsilon", cost(6.0)),
                ("Zeta", cost(5.0)),
                ("Eta", cost(4.0)),
            ],
        )];

        let selected = select_segments(&buckets, ReportMode::Cost, 5).unwrap();

        assert_eq!(
            selected
                .segments
                .iter()
                .map(|segment| segment.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Alpha", "Beta", "Gamma", "Delta", "Epsilon", "Other"]
        );
        assert_eq!(selected.segments[5].total, cost(9.0));
        assert_eq!(selected.buckets[0].segment_totals[5], cost(9.0));
    }

    #[test]
    fn allocate_segment_widths_uses_largest_remainder_rounding() {
        let widths = allocate_segment_widths(&[tokens(5), tokens(3), tokens(2)], 7);

        assert_eq!(widths, vec![4, 2, 1]);
        assert_eq!(widths.iter().sum::<usize>(), 7);
    }

    #[test]
    fn build_output_aligns_total_column_for_time_series_rows() {
        let output = build_stacked_charts_output(
            "API Cost Graphs",
            "Daily total cost by model",
            "Cost share by model",
            &[
                bucket(
                    "2026-05-01",
                    &[("Sonnet", cost(12.5)), ("Opus", cost(3.25))],
                ),
                bucket("2026-05-02", &[("Sonnet", cost(4.0))]),
            ],
            ReportMode::Cost,
            ChartRenderOptions {
                terminal_width: Some(72),
                color_mode: ChartColorMode::Never,
                ..ChartRenderOptions::default()
            },
        )
        .unwrap();

        let data_lines: Vec<_> = output
            .lines()
            .filter(|line| line.starts_with("2026-05"))
            .collect();
        assert_eq!(data_lines.len(), 2);

        let line_widths: Vec<_> = data_lines.iter().map(|line| display_width(line)).collect();
        assert_eq!(line_widths[0], line_widths[1]);
    }

    #[test]
    fn build_output_handles_all_zero_buckets() {
        let output = build_stacked_charts_output(
            "API Token Graphs",
            "Daily total tokens by model",
            "Token share by model",
            &[bucket(
                "2026-05-01",
                &[("Sonnet", tokens(0)), ("Opus", tokens(0))],
            )],
            ReportMode::Tokens,
            ChartRenderOptions {
                terminal_width: Some(60),
                color_mode: ChartColorMode::Never,
                ..ChartRenderOptions::default()
            },
        )
        .unwrap();

        assert!(output.contains("Legend: none"));
        assert!(output.contains("No non-zero model/provider totals."));
        assert!(output.contains("Grand Total: 0"));
    }

    #[test]
    fn build_output_truncates_long_labels_in_narrow_terminals() {
        let output = build_stacked_charts_output(
            "Pi Cost Graphs",
            "Daily total cost by provider/model",
            "Cost share by provider/model",
            &[bucket(
                "very-long-session-label-that-needs-truncation",
                &[("Anthropic / claude-sonnet-4-5", cost(10.0))],
            )],
            ReportMode::Cost,
            ChartRenderOptions {
                terminal_width: Some(40),
                color_mode: ChartColorMode::Never,
                ..ChartRenderOptions::default()
            },
        )
        .unwrap();

        assert!(output.contains("very-long-session"));
        assert!(output.contains('…'));
        assert!(output.contains("Anthropic /"));
    }

    #[test]
    fn build_output_truncates_long_legend_items_in_narrow_terminals() {
        let output = build_stacked_charts_output(
            "Graphs",
            "Daily totals",
            "Share",
            &[bucket(
                "2026-05-01",
                &[(
                    "Anthropic / claude-opus-4-20250514-extra-long-model-name",
                    cost(10.0),
                )],
            )],
            ReportMode::Cost,
            ChartRenderOptions {
                terminal_width: Some(40),
                color_mode: ChartColorMode::Never,
                ..ChartRenderOptions::default()
            },
        )
        .unwrap();

        for line in output
            .lines()
            .filter(|line| line.starts_with("Legend:") || line.starts_with("        "))
        {
            assert!(display_width(line) <= 40, "legend line too wide: {line:?}");
        }
        assert!(output.contains('…'));
    }

    #[test]
    fn build_output_keeps_legend_within_very_narrow_widths() {
        let output = build_stacked_charts_output(
            "Graphs",
            "Daily totals",
            "Share",
            &[bucket("2026-05-01", &[("Sonnet", cost(10.0))])],
            ReportMode::Cost,
            ChartRenderOptions {
                terminal_width: Some(8),
                color_mode: ChartColorMode::Never,
                ..ChartRenderOptions::default()
            },
        )
        .unwrap();

        for line in output
            .lines()
            .filter(|line| line.starts_with("Legend") || line == &"█")
        {
            assert!(display_width(line) <= 8, "legend line too wide: {line:?}");
        }
    }

    #[test]
    fn build_output_uses_display_width_for_wide_labels() {
        let output = build_stacked_charts_output(
            "Graphs",
            "Daily totals",
            "Share",
            &[
                bucket("表表表", &[("Sonnet", cost(12.5))]),
                bucket("plain", &[("Sonnet", cost(4.0))]),
            ],
            ReportMode::Cost,
            ChartRenderOptions {
                terminal_width: Some(48),
                color_mode: ChartColorMode::Never,
                ..ChartRenderOptions::default()
            },
        )
        .unwrap();

        let data_lines: Vec<_> = output
            .lines()
            .filter(|line| line.starts_with("表表表") || line.starts_with("plain"))
            .collect();
        assert_eq!(data_lines.len(), 2);
        assert_eq!(display_width(data_lines[0]), display_width(data_lines[1]));
    }

    #[test]
    fn select_segments_avoids_other_label_collisions() {
        let selected = select_segments(
            &[bucket(
                "2026-05-01",
                &[
                    ("Other", tokens(10)),
                    ("Other (grouped)", tokens(9)),
                    ("Alpha", tokens(4)),
                ],
            )],
            ReportMode::Tokens,
            2,
        )
        .unwrap();

        assert_eq!(selected.segments[0].label, "Other");
        assert_eq!(selected.segments[1].label, "Other (grouped)");
        assert_eq!(selected.segments[2].label, "Other (grouped 2)");
        assert_eq!(selected.buckets[0].segment_totals[2], tokens(4));
    }

    #[test]
    fn select_segments_sums_duplicate_labels_within_bucket() {
        let selected = select_segments(
            &[bucket(
                "2026-05-01",
                &[("Sonnet", tokens(10)), ("Sonnet", tokens(5))],
            )],
            ReportMode::Tokens,
            5,
        )
        .unwrap();

        assert_eq!(selected.segments[0].label, "Sonnet");
        assert_eq!(selected.segments[0].total, tokens(15));
        assert_eq!(selected.buckets[0].segment_totals[0], tokens(15));
    }

    #[test]
    fn select_segments_preserves_first_seen_order_for_ties() {
        let selected = select_segments(
            &[bucket(
                "2026-05-01",
                &[("Beta", tokens(10)), ("Alpha", tokens(10))],
            )],
            ReportMode::Tokens,
            5,
        )
        .unwrap();

        assert_eq!(
            selected
                .segments
                .iter()
                .map(|segment| segment.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Beta", "Alpha"]
        );
    }

    #[test]
    fn build_output_handles_tiny_terminal_width_without_panicking() {
        let output = build_stacked_charts_output(
            "API Token Graphs",
            "Daily total tokens by model",
            "Token share by model",
            &[bucket("2026-05-01", &[("Sonnet", tokens(10))])],
            ReportMode::Tokens,
            ChartRenderOptions {
                terminal_width: Some(6),
                color_mode: ChartColorMode::Never,
                ..ChartRenderOptions::default()
            },
        )
        .unwrap();

        assert!(output.contains("Grand Total: 10"));
    }

    #[test]
    fn build_output_keeps_share_rows_within_narrow_widths() {
        let output = build_stacked_charts_output(
            "Graphs",
            "Daily totals",
            "Share",
            &[bucket(
                "2026-05-01",
                &[("Anthropic / claude-sonnet-4-5", tokens(12_345_678))],
            )],
            ReportMode::Tokens,
            ChartRenderOptions {
                terminal_width: Some(18),
                color_mode: ChartColorMode::Never,
                ..ChartRenderOptions::default()
            },
        )
        .unwrap();

        for line in output.lines().filter(|line| line.starts_with(" █")) {
            assert!(display_width(line) <= 18, "share row too wide: {line:?}");
        }
    }

    #[test]
    fn selected_segments_assign_stable_glyphs() {
        let selected = select_segments(
            &[
                bucket("2026-05-01", &[("Alpha", tokens(10)), ("Beta", tokens(5))]),
                bucket("2026-05-02", &[("Alpha", tokens(2)), ("Beta", tokens(1))]),
            ],
            ReportMode::Tokens,
            5,
        )
        .unwrap();

        assert_eq!(selected.segments[0].glyph, '█');
        assert_eq!(selected.segments[1].glyph, '▓');
    }

    #[test]
    fn legend_wraps_item_when_full_label_fits_on_fresh_line() {
        let lines = render_legend_lines(
            &[
                OrderedSegment {
                    id: SegmentId::Label("Alpha".to_string()),
                    label: "Alpha".to_string(),
                    total: cost(10.0),
                    glyph: '█',
                    color: None,
                },
                OrderedSegment {
                    id: SegmentId::Label("Beta".to_string()),
                    label: "Beta".to_string(),
                    total: cost(9.0),
                    glyph: '▓',
                    color: None,
                },
            ],
            20,
            false,
        );

        // "Legend: █ Alpha" is 15 wide, "        ▓ Beta" is 14 wide
        assert_eq!(lines[0], "Legend: █ Alpha");
        assert_eq!(lines[1], "        ▓ Beta");
        assert!(!lines[1].contains('…'));
    }

    #[test]
    fn legend_does_not_wrap_when_full_label_would_not_fit_on_fresh_line() {
        let lines = render_legend_lines(
            &[
                OrderedSegment {
                    id: SegmentId::Label("Alpha".to_string()),
                    label: "Alpha".to_string(),
                    total: cost(10.0),
                    glyph: '█',
                    color: None,
                },
                OrderedSegment {
                    id: SegmentId::Label("VeryLongSegment".to_string()),
                    label: "VeryLongSegment".to_string(),
                    total: cost(9.0),
                    glyph: '▓',
                    color: None,
                },
            ],
            24,
            false,
        );

        // "Legend: █ Alpha  ▓ Very…" = exactly 24 wide, truncated on same line
        assert_eq!(lines[0], "Legend: █ Alpha  ▓ Very…");
        assert_eq!(display_width(&lines[0]), 24);
        assert_eq!(lines[1], "");
    }

    #[test]
    fn share_detail_line_uses_full_available_label_width() {
        let long_label = "Anthropic / claude-opus-4-20250514-extra-long-model-name";

        let output = build_stacked_charts_output(
            "Graphs",
            "Daily totals",
            "Share",
            &[bucket("2026-05-01", &[(long_label, cost(10.0))])],
            ReportMode::Cost,
            ChartRenderOptions {
                terminal_width: Some(120),
                color_mode: ChartColorMode::Never,
                ..ChartRenderOptions::default()
            },
        )
        .unwrap();

        let share_line = output
            .lines()
            .find(|line| line.starts_with(" █ "))
            .expect("share detail line should be rendered");

        assert!(share_line.contains(long_label));
        assert!(!share_line.contains('…'));
        assert!(display_width(share_line) <= 120);
    }
}
