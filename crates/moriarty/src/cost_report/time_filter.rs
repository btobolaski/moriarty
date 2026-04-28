use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, Utc};
use miette::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateTimezone {
    Local,
    Utc,
}

impl DateTimezone {
    pub fn to_date(self, timestamp: &DateTime<Utc>) -> NaiveDate {
        match self {
            Self::Local => timestamp.with_timezone(&Local).date_naive(),
            Self::Utc => timestamp.date_naive(),
        }
    }
}

/// Half-open time filter shared by Claude and pi cost reports.
#[derive(Debug, Clone)]
pub struct TimeRangeFilter {
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
}

impl TimeRangeFilter {
    /// Date-only inputs map to whole-day bounds so callers can filter by day
    /// without having to spell out the exclusive end timestamp themselves.
    pub fn new(start: Option<String>, end: Option<String>) -> Result<Self> {
        let start_dt = start.map(|s| parse_datetime_for_start(&s)).transpose()?;
        let end_dt = end.map(|s| parse_datetime_for_end(&s)).transpose()?;

        if let (Some(start), Some(end)) = (start_dt, end_dt) {
            if start >= end {
                return Err(miette::miette!(
                    "Start time ({}) must be before end time ({})",
                    start,
                    end
                ));
            }
        }

        Ok(Self {
            start: start_dt,
            end: end_dt,
        })
    }

    pub fn contains(&self, timestamp: &DateTime<Utc>) -> bool {
        if let Some(start) = self.start {
            if timestamp < &start {
                return false;
            }
        }
        if let Some(end) = self.end {
            if timestamp >= &end {
                return false;
            }
        }
        true
    }

    pub fn is_unrestricted(&self) -> bool {
        self.start.is_none() && self.end.is_none()
    }
}

fn try_parse_rfc3339(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn try_parse_naive_datetime(s: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
        .ok()
        .map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc))
}

fn date_to_midnight_utc(date: NaiveDate) -> DateTime<Utc> {
    let datetime = date
        .and_hms_opt(0, 0, 0)
        .expect("00:00:00 is always a valid time");
    DateTime::from_naive_utc_and_offset(datetime, Utc)
}

fn invalid_datetime_error(s: &str) -> miette::Report {
    miette::miette!(
        "Invalid datetime format: '{}'. Expected ISO 8601 (e.g., '2025-01-01T00:00:00Z') or date (e.g., '2025-01-01')",
        s
    )
}

fn parse_datetime_with(
    s: &str,
    date_to_dt: impl FnOnce(NaiveDate) -> DateTime<Utc>,
) -> Result<DateTime<Utc>> {
    if let Some(dt) = try_parse_rfc3339(s) {
        return Ok(dt);
    }
    if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(date_to_dt(date));
    }
    if let Some(dt) = try_parse_naive_datetime(s) {
        return Ok(dt);
    }
    Err(invalid_datetime_error(s))
}

fn parse_datetime_for_start(s: &str) -> Result<DateTime<Utc>> {
    parse_datetime_with(s, date_to_midnight_utc)
}

/// Date-only end bounds advance to the next midnight so whole-day filters keep
/// the same inclusive-start / exclusive-end contract as timestamp inputs.
fn parse_datetime_for_end(s: &str) -> Result<DateTime<Utc>> {
    parse_datetime_with(s, |date| {
        let next_day = date
            .succ_opt()
            .expect("Date overflow only occurs beyond year 262000, unreachable for API logs");
        date_to_midnight_utc(next_day)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_datetime_iso8601_for_start() {
        let dt = parse_datetime_for_start("2025-01-01T12:00:00Z").unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-01-01T12:00:00+00:00");
    }

    #[test]
    fn test_parse_datetime_iso8601_with_offset() {
        let dt = parse_datetime_for_start("2025-01-01T12:00:00-05:00").unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-01-01T17:00:00+00:00");
    }

    #[test]
    fn test_parse_datetime_date_only_for_start() {
        let dt = parse_datetime_for_start("2025-01-01").unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-01-01T00:00:00+00:00");
    }

    #[test]
    fn test_parse_datetime_date_only_for_end() {
        let dt = parse_datetime_for_end("2025-01-01").unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-01-02T00:00:00+00:00");
    }

    #[test]
    fn test_parse_datetime_without_timezone() {
        let dt = parse_datetime_for_start("2025-01-01T12:00:00").unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-01-01T12:00:00+00:00");
    }

    #[test]
    fn test_parse_datetime_invalid() {
        let _err =
            parse_datetime_for_start("invalid").expect_err("Should fail with invalid datetime");
    }

    #[test]
    fn test_time_range_filter_contains() {
        let filter = TimeRangeFilter {
            start: Some(parse_datetime_for_start("2025-01-01T00:00:00Z").unwrap()),
            end: Some(parse_datetime_for_start("2025-02-01T00:00:00Z").unwrap()),
        };

        assert!(filter.contains(&parse_datetime_for_start("2025-01-01T00:00:00Z").unwrap()));
        assert!(!filter.contains(&parse_datetime_for_start("2024-12-31T23:59:59Z").unwrap()));
        assert!(filter.contains(&parse_datetime_for_start("2025-01-15T12:00:00Z").unwrap()));
        assert!(filter.contains(&parse_datetime_for_start("2025-01-31T23:59:59Z").unwrap()));
        assert!(!filter.contains(&parse_datetime_for_start("2025-02-01T00:00:00Z").unwrap()));
        assert!(!filter.contains(&parse_datetime_for_start("2025-02-01T00:00:01Z").unwrap()));
    }

    #[test]
    fn test_time_range_filter_start_only() {
        let filter = TimeRangeFilter {
            start: Some(parse_datetime_for_start("2025-01-01T00:00:00Z").unwrap()),
            end: None,
        };

        assert!(filter.contains(&parse_datetime_for_start("2025-01-01T00:00:00Z").unwrap()));
        assert!(filter.contains(&parse_datetime_for_start("2030-01-01T00:00:00Z").unwrap()));
        assert!(!filter.contains(&parse_datetime_for_start("2024-12-31T23:59:59Z").unwrap()));
    }

    #[test]
    fn test_time_range_filter_end_only() {
        let filter = TimeRangeFilter {
            start: None,
            end: Some(parse_datetime_for_start("2025-02-01T00:00:00Z").unwrap()),
        };

        assert!(filter.contains(&parse_datetime_for_start("2025-01-15T12:00:00Z").unwrap()));
        assert!(filter.contains(&parse_datetime_for_start("2020-01-01T00:00:00Z").unwrap()));
        assert!(filter.contains(&parse_datetime_for_start("2025-01-31T23:59:59Z").unwrap()));
        assert!(!filter.contains(&parse_datetime_for_start("2025-02-01T00:00:00Z").unwrap()));
    }

    #[test]
    fn test_time_range_filter_empty() {
        let filter = TimeRangeFilter {
            start: None,
            end: None,
        };

        assert!(filter.contains(&parse_datetime_for_start("2020-01-01T00:00:00Z").unwrap()));
        assert!(filter.contains(&parse_datetime_for_start("2030-01-01T00:00:00Z").unwrap()));
    }

    #[test]
    fn test_time_range_filter_new_validates_start_before_end() {
        let error = TimeRangeFilter::new(
            Some("2025-01-02T00:00:00Z".to_string()),
            Some("2025-01-01T00:00:00Z".to_string()),
        )
        .unwrap_err();

        assert!(
            format!("{error}").contains("must be before end time"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn date_timezone_maps_dates_in_both_modes() {
        let timestamp = parse_datetime_for_start("2025-01-01T23:30:00Z").unwrap();

        assert_eq!(
            DateTimezone::Utc.to_date(&timestamp),
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap()
        );
        let _ = DateTimezone::Local.to_date(&timestamp);
    }
}
