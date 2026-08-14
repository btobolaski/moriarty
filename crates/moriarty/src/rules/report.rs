//! Human-readable effectiveness summaries over completed PreToolUse hook records.

// standard library
use std::{collections::BTreeMap, path::PathBuf};

// 3rd party crates
use miette::Result;
use tabled::Tabled;

// local / workspace deps
use crate::{
    cost_report::{DateTimezone, TimeRangeFilter, parse_timezone, print_grouped_report},
    hooks::{
        report::{HashSkipStats, OutcomeRecord, fold_outcomes},
        result::PreToolResult,
    },
};

const UNKNOWN_DIRECTORY: &str = "Unknown";

pub(super) async fn run(
    dir: Option<PathBuf>,
    start_time: Option<String>,
    end_time: Option<String>,
    timezone: String,
    directories: bool,
    current_rules: bool,
) -> Result<()> {
    let timezone = parse_timezone(&timezone)?;
    let filter = TimeRangeFilter::new(start_time, end_time, timezone)?;
    let LoadedReport {
        report,
        skipped,
        active_hash,
    } = load_report(dir, &filter, timezone, directories, current_rules).await?;

    println!("{}", scope_note(active_hash.as_deref(), &skipped));
    if report.groups.is_empty() {
        println!("\nNo matching completed PreToolUse records found.");
        return Ok(());
    }

    println!();
    render(&report, directories);
    Ok(())
}

struct LoadedReport {
    report: EffectivenessReport,
    skipped: HashSkipStats,
    active_hash: Option<String>,
}

async fn load_report(
    dir: Option<PathBuf>,
    filter: &TimeRangeFilter,
    timezone: DateTimezone,
    directories: bool,
    current_rules: bool,
) -> Result<LoadedReport> {
    let (hash_filter, active_hash) = super::resolve_hash_filter(!current_rules, None).await?;
    let (value, skipped) = fold_outcomes(
        dir,
        filter,
        &hash_filter,
        EffectivenessAccumulator::default(),
        |report, record| report.record(&record, timezone, directories),
    )
    .await?;
    Ok(LoadedReport {
        report: value.finish(),
        skipped,
        active_hash,
    })
}

fn scope_note(active_hash: Option<&str>, skipped: &HashSkipStats) -> String {
    match active_hash {
        Some(hash) => format!(
            "Rule set: current ({hash}); {} (omit --current-rules to include them).",
            super::rules_hash_skip_note(skipped)
        ),
        None => "Rule set: all recorded history; no hash filter applied.".to_string(),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct OutcomeCounts {
    allowed: u64,
    denied: u64,
    asked: u64,
    modified: u64,
    passthrough: u64,
}

impl OutcomeCounts {
    fn record(&mut self, result: PreToolResult) {
        match result {
            PreToolResult::Allow => self.allowed += 1,
            PreToolResult::Deny => self.denied += 1,
            PreToolResult::Ask => self.asked += 1,
            PreToolResult::Modify => self.modified += 1,
            PreToolResult::Passthrough => self.passthrough += 1,
        }
    }

    fn total(self) -> u64 {
        self.allowed + self.denied + self.asked + self.modified + self.passthrough
    }
}

#[derive(Debug, PartialEq, Eq)]
struct OutcomeGroup {
    label: String,
    counts: OutcomeCounts,
}

#[derive(Debug, PartialEq, Eq)]
struct EffectivenessReport {
    groups: Vec<OutcomeGroup>,
    grand_total: OutcomeCounts,
}

#[derive(Default)]
struct EffectivenessAccumulator {
    groups: BTreeMap<String, OutcomeCounts>,
    grand_total: OutcomeCounts,
}

impl EffectivenessAccumulator {
    fn record(&mut self, record: &OutcomeRecord, timezone: DateTimezone, directories: bool) {
        let label = if directories {
            record
                .cwd
                .as_deref()
                .filter(|cwd| !cwd.is_empty())
                .unwrap_or(UNKNOWN_DIRECTORY)
                .to_string()
        } else {
            timezone.to_date(&record.timestamp).to_string()
        };
        self.groups.entry(label).or_default().record(record.result);
        self.grand_total.record(record.result);
    }

    fn finish(self) -> EffectivenessReport {
        let groups = self
            .groups
            .into_iter()
            .map(|(label, counts)| OutcomeGroup { label, counts })
            .collect();
        EffectivenessReport {
            groups,
            grand_total: self.grand_total,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Tabled)]
struct FormattedOutcomeColumns {
    #[tabled(rename = "Allowed")]
    allowed: String,
    #[tabled(rename = "Denied")]
    denied: String,
    #[tabled(rename = "Asked")]
    asked: String,
    #[tabled(rename = "Modified")]
    modified: String,
    #[tabled(rename = "Passthrough")]
    passthrough: String,
    #[tabled(rename = "Total Calls")]
    total_calls: u64,
}

impl From<OutcomeCounts> for FormattedOutcomeColumns {
    fn from(counts: OutcomeCounts) -> Self {
        let total = counts.total();
        Self {
            allowed: format_count(counts.allowed, total),
            denied: format_count(counts.denied, total),
            asked: format_count(counts.asked, total),
            modified: format_count(counts.modified, total),
            passthrough: format_count(counts.passthrough, total),
            total_calls: total,
        }
    }
}

fn format_count(count: u64, total: u64) -> String {
    let percentage = if total == 0 {
        0.0
    } else {
        count as f64 * 100.0 / total as f64
    };
    format!("{count} ({percentage:.1}%)")
}

#[derive(Tabled)]
struct DailyRow {
    #[tabled(rename = "Date")]
    label: String,
    #[tabled(inline)]
    outcomes: FormattedOutcomeColumns,
}

#[derive(Tabled)]
struct DirectoryRow {
    #[tabled(rename = "Directory")]
    label: String,
    #[tabled(inline)]
    outcomes: FormattedOutcomeColumns,
}

fn formatted_rows(report: &EffectivenessReport) -> Vec<(String, FormattedOutcomeColumns)> {
    report
        .groups
        .iter()
        .map(|group| (group.label.clone(), group.counts.into()))
        .chain(std::iter::once((
            "Grand Total".to_string(),
            report.grand_total.into(),
        )))
        .collect()
}

fn render(report: &EffectivenessReport, directories: bool) {
    if report.groups.is_empty() {
        return;
    }

    let formatted = formatted_rows(report);
    // `print_grouped_report` inserts separators after every index except the last. Treating the
    // final bucket and footer as the two totals yields one separator immediately above the footer.
    let total_row_indices = vec![report.groups.len() - 1, report.groups.len()];

    if directories {
        let rows = formatted
            .into_iter()
            .map(|(label, outcomes)| DirectoryRow { label, outcomes })
            .collect::<Vec<_>>();
        print_grouped_report(
            "Rules Effectiveness by Directory",
            &rows,
            &total_row_indices,
        );
    } else {
        let rows = formatted
            .into_iter()
            .map(|(label, outcomes)| DailyRow { label, outcomes })
            .collect::<Vec<_>>();
        print_grouped_report("Rules Effectiveness by Day", &rows, &total_row_indices);
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::*;
    use crate::{
        test_helpers::{TestEnvVarGuard, setup_isolated_xdg_config},
        user_config::load_user_config,
    };

    fn outcome(timestamp: &str, cwd: Option<&str>, result: PreToolResult) -> OutcomeRecord {
        OutcomeRecord {
            timestamp: timestamp.parse::<DateTime<Utc>>().unwrap(),
            cwd: cwd.map(str::to_string),
            result,
        }
    }

    fn aggregate(
        records: &[OutcomeRecord],
        timezone: DateTimezone,
        directories: bool,
    ) -> EffectivenessReport {
        let mut report = EffectivenessAccumulator::default();
        for record in records {
            report.record(record, timezone, directories);
        }
        report.finish()
    }

    #[test]
    fn daily_groups_are_ascending_and_include_every_outcome() {
        let records = vec![
            outcome("2026-06-04T00:00:00Z", None, PreToolResult::Passthrough),
            outcome("2026-06-03T05:00:00Z", None, PreToolResult::Allow),
            outcome("2026-06-03T06:00:00Z", None, PreToolResult::Deny),
            outcome("2026-06-03T07:00:00Z", None, PreToolResult::Ask),
            outcome("2026-06-03T08:00:00Z", None, PreToolResult::Modify),
        ];

        let report = aggregate(&records, DateTimezone::Utc, false);

        assert_eq!(
            report.groups,
            vec![
                OutcomeGroup {
                    label: "2026-06-03".to_string(),
                    counts: OutcomeCounts {
                        allowed: 1,
                        denied: 1,
                        asked: 1,
                        modified: 1,
                        passthrough: 0,
                    },
                },
                OutcomeGroup {
                    label: "2026-06-04".to_string(),
                    counts: OutcomeCounts {
                        passthrough: 1,
                        ..OutcomeCounts::default()
                    },
                },
            ]
        );
        assert_eq!(
            report.grand_total,
            OutcomeCounts {
                allowed: 1,
                denied: 1,
                asked: 1,
                modified: 1,
                passthrough: 1,
            }
        );
    }

    #[test]
    fn daily_groups_use_the_selected_timezone_at_a_date_boundary() {
        let _timezone = TestEnvVarGuard::set("TZ", "Australia/Sydney");
        let record = outcome("2026-06-03T23:30:00Z", None, PreToolResult::Allow);

        let local = aggregate(std::slice::from_ref(&record), DateTimezone::Local, false);
        let utc = aggregate(std::slice::from_ref(&record), DateTimezone::Utc, false);

        assert_eq!(local.groups[0].label, "2026-06-04");
        assert_eq!(utc.groups[0].label, "2026-06-03");
    }

    #[test]
    fn directory_groups_are_lexical_and_legacy_cwds_are_unknown() {
        let records = vec![
            outcome(
                "2026-06-03T01:00:00Z",
                Some("/work/z"),
                PreToolResult::Allow,
            ),
            outcome("2026-06-03T02:00:00Z", None, PreToolResult::Ask),
            outcome("2026-06-03T03:00:00Z", Some(""), PreToolResult::Deny),
            outcome(
                "2026-06-03T04:00:00Z",
                Some("/work/a"),
                PreToolResult::Modify,
            ),
        ];

        let report = aggregate(&records, DateTimezone::Utc, true);

        assert_eq!(
            report
                .groups
                .iter()
                .map(|group| group.label.as_str())
                .collect::<Vec<_>>(),
            vec!["/work/a", "/work/z", UNKNOWN_DIRECTORY]
        );
        assert_eq!(report.groups[2].counts.total(), 2);
    }

    #[test]
    fn percentages_round_to_one_decimal_and_handle_zero_totals() {
        let formatted = FormattedOutcomeColumns::from(OutcomeCounts {
            allowed: 2,
            asked: 1,
            ..OutcomeCounts::default()
        });

        assert_eq!(formatted.allowed, "2 (66.7%)");
        assert_eq!(formatted.asked, "1 (33.3%)");
        assert_eq!(formatted.denied, "0 (0.0%)");
        assert_eq!(formatted.total_calls, 3);

        let zero = FormattedOutcomeColumns::from(OutcomeCounts::default());
        assert_eq!(zero.allowed, "0 (0.0%)");
        assert_eq!(zero.passthrough, "0 (0.0%)");
        assert_eq!(zero.total_calls, 0);
    }

    #[test]
    fn formatted_rows_append_a_reconciled_grand_total() {
        let report = EffectivenessReport {
            groups: vec![OutcomeGroup {
                label: "2026-06-03".to_string(),
                counts: OutcomeCounts {
                    allowed: 3,
                    denied: 1,
                    ..OutcomeCounts::default()
                },
            }],
            grand_total: OutcomeCounts {
                allowed: 3,
                denied: 1,
                ..OutcomeCounts::default()
            },
        };

        let rows = formatted_rows(&report);

        assert_eq!(rows.last().unwrap().0, "Grand Total");
        assert_eq!(rows.last().unwrap().1.total_calls, 4);
        assert_eq!(rows.last().unwrap().1.allowed, "3 (75.0%)");
    }

    #[tokio::test]
    async fn current_rules_scope_uses_the_installed_config_hash() {
        let _config = setup_isolated_xdg_config();
        let active_hash = load_user_config().await.unwrap().effective_hash();
        let dir = tempfile::tempdir().unwrap();
        let line = |hash: Option<&str>, result: &str| {
            serde_json::json!({
                "timestamp": "2026-06-03T01:00:00Z",
                "fields": {
                    "message": "PreToolUse hook completed",
                    "tool_name": "Bash",
                    "tool_args": "{}",
                    "cwd": "/work",
                    "rules_hash": hash,
                    "result": result
                }
            })
            .to_string()
        };
        let lines = [
            line(Some(&active_hash), "allow"),
            line(Some("sha256:other"), "deny"),
            line(None, "ask"),
        ];
        tokio::fs::write(
            dir.path().join("hooks.log.2026-06-03"),
            format!("{}\n", lines.join("\n")),
        )
        .await
        .unwrap();
        let filter = TimeRangeFilter::new(None, None, DateTimezone::Utc).unwrap();

        let LoadedReport {
            report,
            skipped,
            active_hash: reported_hash,
        } = load_report(
            Some(dir.path().to_path_buf()),
            &filter,
            DateTimezone::Utc,
            false,
            true,
        )
        .await
        .unwrap();

        assert_eq!(reported_hash.as_deref(), Some(active_hash.as_str()));
        assert_eq!(report.groups.len(), 1);
        assert_eq!(report.groups[0].counts.allowed, 1);
        assert_eq!(
            skipped,
            HashSkipStats {
                other_rules: 1,
                no_hash: 1,
            }
        );
    }

    #[tokio::test]
    async fn run_accepts_an_empty_explicit_log_directory() {
        let dir = tempfile::tempdir().unwrap();

        run(
            Some(dir.path().to_path_buf()),
            None,
            None,
            "utc".to_string(),
            false,
            false,
        )
        .await
        .unwrap();
    }

    #[test]
    fn current_rules_scope_note_includes_hash_and_skip_counts() {
        let note = scope_note(
            Some("sha256:current"),
            &HashSkipStats {
                other_rules: 3,
                no_hash: 2,
            },
        );
        assert!(note.contains("current (sha256:current)"));
        assert!(note.contains("skipped 3 record(s)"));
        assert!(note.contains("2 without a recorded rule hash"));
    }

    #[test]
    fn rendering_grouping_modes_and_empty_data_does_not_panic() {
        let records = vec![outcome(
            "2026-06-03T01:00:00Z",
            Some("/work"),
            PreToolResult::Allow,
        )];

        render(&aggregate(&records, DateTimezone::Utc, false), false);
        render(&aggregate(&records, DateTimezone::Utc, true), true);
        render(&aggregate(&[], DateTimezone::Utc, false), false);
    }
}
