use std::{collections::BTreeMap, future::Future, path::Path};

use chrono::NaiveDate;

use crate::{
    api_pricing,
    cost_report::{
        ChartBucket, ChartSegment, DateTimezone, DatedSegments, ReportMode, SessionSegments,
        TimeRangeFilter, format_session_id, render_or_empty, render_stacked_charts,
    },
    pi_cost,
};

const CLAUDE_SOURCE: &str = "Claude Code";
const PI_SOURCE: &str = "pi";

/// Dollar totals mix pi's recorded prices with Claude's local pricing tables;
/// token mode is homogeneous because both sources report the same unit.
pub(crate) async fn run_graphs(
    claude_dir: Option<&Path>,
    pi_dir: Option<&Path>,
    timezone: DateTimezone,
    by_conversation: bool,
    filter: &TimeRangeFilter,
    report_mode: ReportMode,
) -> miette::Result<()> {
    if claude_dir.is_none() && pi_dir.is_none() {
        return Err(miette::miette!(
            "No log directories to analyze; pass --claude-dir and/or --pi-dir"
        ));
    }

    let (buckets, had_errors) = if by_conversation {
        let ((claude, claude_errors), (pi, pi_errors)) = tokio::try_join!(
            optional_series(claude_dir.map(|dir| api_pricing::session_chart_series(
                dir,
                filter,
                report_mode
            ))),
            optional_series(pi_dir.map(|dir| pi_cost::session_chart_series(
                dir,
                filter,
                report_mode
            )))
        )?;
        (merge_sessions(claude, pi), claude_errors || pi_errors)
    } else {
        let ((claude, claude_errors), (pi, pi_errors)) = tokio::try_join!(
            optional_series(claude_dir.map(|dir| {
                api_pricing::daily_chart_series(dir, timezone, filter, report_mode)
            })),
            optional_series(
                pi_dir
                    .map(|dir| { pi_cost::daily_chart_series(dir, timezone, filter, report_mode) })
            )
        )?;
        (merge_daily(claude, pi), claude_errors || pi_errors)
    };
    let (title, series_title, share_title) = titles(report_mode, by_conversation);

    render_or_empty(&buckets, had_errors, |items| {
        render_stacked_charts(title, series_title, share_title, items, report_mode)
    })
}

async fn optional_series<T>(
    future: Option<impl Future<Output = miette::Result<(Vec<T>, bool)>>>,
) -> miette::Result<(Vec<T>, bool)> {
    match future {
        Some(future) => future.await,
        None => Ok((Vec::new(), false)),
    }
}

/// The renderer preserves input buckets, so sources must share one bucket per date.
fn merge_daily(claude: Vec<DatedSegments>, pi: Vec<DatedSegments>) -> Vec<ChartBucket> {
    let mut by_date: BTreeMap<NaiveDate, Vec<ChartSegment>> = BTreeMap::new();
    for (source, series) in [(CLAUDE_SOURCE, claude), (PI_SOURCE, pi)] {
        for item in series {
            by_date
                .entry(item.date)
                .or_default()
                .extend(prefixed(source, item.segments));
        }
    }
    by_date
        .into_iter()
        .map(|(date, segments)| ChartBucket {
            label: date.to_string(),
            segments,
        })
        .collect()
}

/// Full ids break equal-time ties before labels are shortened for display.
fn merge_sessions(claude: Vec<SessionSegments>, pi: Vec<SessionSegments>) -> Vec<ChartBucket> {
    let mut sessions: Vec<_> = claude
        .into_iter()
        .map(|mut item| {
            item.segments = prefixed(CLAUDE_SOURCE, item.segments).collect();
            item
        })
        .chain(pi.into_iter().map(|mut item| {
            item.segments = prefixed(PI_SOURCE, item.segments).collect();
            item
        }))
        .collect();
    sessions.sort_by(|left, right| {
        (left.start_time, left.session_id.as_str())
            .cmp(&(right.start_time, right.session_id.as_str()))
    });
    sessions
        .into_iter()
        .map(|item| ChartBucket {
            label: format_session_id(&item.session_id),
            segments: item.segments,
        })
        .collect()
}

fn prefixed(source: &str, segments: Vec<ChartSegment>) -> impl Iterator<Item = ChartSegment> + '_ {
    segments.into_iter().map(move |segment| ChartSegment {
        label: format!("{source} / {}", segment.label),
        total: segment.total,
    })
}

fn titles(
    report_mode: ReportMode,
    by_conversation: bool,
) -> (&'static str, &'static str, &'static str) {
    match (report_mode, by_conversation) {
        (ReportMode::Cost, false) => (
            "Combined Cost Graphs",
            "Daily total cost by source/model",
            "Cost share by source/model",
        ),
        (ReportMode::Tokens, false) => (
            "Combined Token Graphs",
            "Daily total tokens by source/model",
            "Token share by source/model",
        ),
        (ReportMode::Cost, true) => (
            "Combined Cost Graphs by Conversation",
            "Conversation total cost by source/model",
            "Cost share by source/model",
        ),
        (ReportMode::Tokens, true) => (
            "Combined Token Graphs by Conversation",
            "Conversation total tokens by source/model",
            "Token share by source/model",
        ),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::cost_report::MetricTotal;

    fn segment(label: &str, total: u128) -> ChartSegment {
        ChartSegment {
            label: label.into(),
            total: MetricTotal::Tokens(total),
        }
    }

    #[test]
    fn daily_sources_merge_and_receive_prefixes() {
        let date = NaiveDate::from_ymd_opt(2026, 4, 16).unwrap();
        let buckets = merge_daily(
            vec![DatedSegments {
                date,
                segments: vec![segment("Sonnet 4", 11)],
            }],
            vec![DatedSegments {
                date,
                segments: vec![segment("Anthropic / claude-sonnet-4-5", 22)],
            }],
        );

        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].label, "2026-04-16");
        assert_eq!(
            buckets[0].segments[0],
            segment("Claude Code / Sonnet 4", 11)
        );
        assert_eq!(
            buckets[0].segments[1],
            segment("pi / Anthropic / claude-sonnet-4-5", 22)
        );
    }

    #[test]
    fn sessions_interleave_by_time_then_full_id() {
        let start_time = Utc.with_ymd_and_hms(2026, 4, 16, 10, 0, 0).unwrap();
        let buckets = merge_sessions(
            vec![SessionSegments {
                session_id: "zzzzzzzz-claude".into(),
                start_time,
                segments: vec![segment("Sonnet 4", 2)],
            }],
            vec![
                SessionSegments {
                    session_id: "later000-pi".into(),
                    start_time: start_time + chrono::Duration::hours(1),
                    segments: vec![segment("OpenAI / gpt-5", 3)],
                },
                SessionSegments {
                    session_id: "aaaaaaaa-pi".into(),
                    start_time,
                    segments: vec![segment("Anthropic / claude-sonnet-4-5", 1)],
                },
            ],
        );

        assert_eq!(
            buckets
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            ["aaaaaaaa", "zzzzzzzz", "later000"]
        );
    }
}
