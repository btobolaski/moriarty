use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
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

    /// Convert a naive datetime (no timezone) to UTC using the command timezone:
    /// `Local` reads the naive as local wall-clock time; `Utc` reads it as UTC.
    pub fn naive_to_utc(self, naive: NaiveDateTime) -> Result<DateTime<Utc>> {
        match self {
            Self::Utc => Ok(DateTime::from_naive_utc_and_offset(naive, Utc)),
            Self::Local => Local
                .from_local_datetime(&naive)
                .single()
                .map(|dt| dt.with_timezone(&Utc))
                .ok_or_else(|| {
                    miette::miette!(
                        "Ambiguous or non-existent local datetime: {}",
                        naive.format("%Y-%m-%dT%H:%M:%S")
                    )
                }),
        }
    }

    /// Convert a date to midnight in the command timezone, returning a UTC instant.
    /// Fails only when midnight does not exist or is ambiguous in the local timezone
    /// (an extreme edge case: timezone transitions like Samoa's 2011 date-line jump).
    pub fn date_to_utc(self, date: NaiveDate) -> Result<DateTime<Utc>> {
        let naive = date
            .and_hms_opt(0, 0, 0)
            .expect("00:00:00 is always a valid time");
        self.naive_to_utc(naive)
    }

    /// The day after `date` at midnight in the command timezone (whole-day inclusive end
    /// convenience). Fails under the same conditions as [`Self::date_to_utc`].
    pub fn next_midnight_to_utc(self, date: NaiveDate) -> Result<DateTime<Utc>> {
        let next = date
            .succ_opt()
            .expect("Date overflow only occurs beyond year 262000, unreachable for API logs");
        self.date_to_utc(next)
    }

    pub fn display_timestamp(self, ts: &DateTime<Utc>) -> String {
        match self {
            Self::Utc => ts.to_rfc3339(),
            Self::Local => ts.with_timezone(&Local).to_rfc3339(),
        }
    }
}

/// Parse a `--timezone` CLI value into a [`DateTimezone`].
///
/// Accepted values (case-insensitive): `"local"` and `"utc"`.
/// Returns an error with a descriptive message for any other value.
pub fn parse_timezone(timezone: &str) -> Result<DateTimezone> {
    match timezone.to_ascii_lowercase().as_str() {
        "local" => Ok(DateTimezone::Local),
        "utc" => Ok(DateTimezone::Utc),
        _ => Err(miette::miette!(
            "Invalid timezone '{}'. Must be 'local' or 'utc'",
            timezone
        )),
    }
}

/// Half-open time filter shared by cost reports, hooks, and rules.
///
/// Stores UTC instants internally for efficient comparison against log timestamps.
/// Date-only and timezone-less inputs are interpreted in the provided [`DateTimezone`];
/// explicit RFC 3339 offsets are authoritative regardless of the timezone.
#[derive(Debug, Clone)]
pub struct TimeRangeFilter {
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
}

impl TimeRangeFilter {
    /// Date-only inputs map to whole-day bounds in `timezone` so callers can filter
    /// by day without having to spell out the exclusive end timestamp themselves.
    pub fn new(start: Option<String>, end: Option<String>, timezone: DateTimezone) -> Result<Self> {
        let start_dt = start
            .map(|s| parse_datetime_for_start(&s, timezone))
            .transpose()?;
        let end_dt = end
            .map(|s| parse_datetime_for_end(&s, timezone))
            .transpose()?;

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

fn invalid_datetime_error(s: &str) -> miette::Report {
    miette::miette!(
        "Invalid datetime format: '{}'. Expected ISO 8601 (e.g., '2025-01-01T00:00:00Z'), \
         date (e.g., '2025-01-01'), or naive datetime (e.g., '2025-01-01T12:00:00')",
        s
    )
}

fn parse_datetime_with(
    s: &str,
    tz: DateTimezone,
    date_to_dt: impl FnOnce(NaiveDate) -> Result<DateTime<Utc>>,
) -> Result<DateTime<Utc>> {
    // RFC 3339 with explicit offset: authoritative, ignore the command timezone.
    if let Some(dt) = try_parse_rfc3339(s) {
        return Ok(dt);
    }
    // Date-only: interpret in the command timezone.
    if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return date_to_dt(date);
    }
    // Naive datetime: interpret in the command timezone.
    if let Ok(naive) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return tz.naive_to_utc(naive).map_err(|e| {
            let label = match tz {
                DateTimezone::Local => "local",
                DateTimezone::Utc => "UTC",
            };
            miette::miette!("Invalid datetime '{}' with timezone '{}': {}", s, label, e)
        });
    }
    Err(invalid_datetime_error(s))
}

fn parse_datetime_for_start(s: &str, tz: DateTimezone) -> Result<DateTime<Utc>> {
    parse_datetime_with(s, tz, |date| tz.date_to_utc(date))
}

/// Date-only end bounds advance to the next midnight so whole-day filters keep
/// the same inclusive-start / exclusive-end contract as timestamp inputs.
fn parse_datetime_for_end(s: &str, tz: DateTimezone) -> Result<DateTime<Utc>> {
    parse_datetime_with(s, tz, |date| tz.next_midnight_to_utc(date))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt_utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn test_parse_datetime_iso8601_for_start() {
        let dt = parse_datetime_for_start("2025-01-01T12:00:00Z", DateTimezone::Utc).unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-01-01T12:00:00+00:00");
    }

    #[test]
    fn test_parse_datetime_iso8601_with_offset() {
        let dt = parse_datetime_for_start("2025-01-01T12:00:00-05:00", DateTimezone::Utc).unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-01-01T17:00:00+00:00");
    }

    #[test]
    fn test_parse_datetime_date_only_for_start_utc() {
        let dt = parse_datetime_for_start("2025-01-01", DateTimezone::Utc).unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-01-01T00:00:00+00:00");
    }

    #[test]
    fn test_parse_datetime_date_only_for_end_utc() {
        let dt = parse_datetime_for_end("2025-01-01", DateTimezone::Utc).unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-01-02T00:00:00+00:00");
    }

    #[test]
    fn test_parse_datetime_date_only_for_start_local() {
        // Date-only start in local tz → local midnight → UTC.
        let dt = parse_datetime_for_start("2025-01-01", DateTimezone::Local).unwrap();
        let local_midnight = Local
            .with_ymd_and_hms(2025, 1, 1, 0, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(dt, local_midnight);
    }

    #[test]
    fn test_parse_datetime_date_only_for_end_local() {
        // Date-only end in local tz → next local midnight → UTC.
        let dt = parse_datetime_for_end("2025-01-01", DateTimezone::Local).unwrap();
        let next_local_midnight = Local
            .with_ymd_and_hms(2025, 1, 2, 0, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(dt, next_local_midnight);
    }

    #[test]
    fn test_parse_naive_datetime_utc() {
        let dt = parse_datetime_for_start("2025-01-01T12:00:00", DateTimezone::Utc).unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-01-01T12:00:00+00:00");
    }

    #[test]
    fn test_parse_naive_datetime_local() {
        // A naive datetime in local timezone should convert correctly.
        let dt = parse_datetime_for_start("2025-01-01T12:00:00", DateTimezone::Local).unwrap();
        let local_noon = Local
            .with_ymd_and_hms(2025, 1, 1, 12, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(dt, local_noon);
    }

    #[test]
    fn test_parse_datetime_explicit_offset_ignores_tz() {
        // Explicit offset is authoritative regardless of the command timezone.
        let dt_utc = parse_datetime_for_start("2025-01-01T12:00:00Z", DateTimezone::Local).unwrap();
        let dt_local = parse_datetime_for_start("2025-01-01T12:00:00Z", DateTimezone::Utc).unwrap();
        assert_eq!(dt_utc, dt_local);
    }

    #[test]
    fn test_parse_datetime_invalid() {
        let _err = parse_datetime_for_start("invalid", DateTimezone::Utc)
            .expect_err("Should fail with invalid datetime");
    }

    #[test]
    fn test_time_range_filter_contains() {
        let start = dt_utc("2025-01-01T00:00:00Z");
        let end = dt_utc("2025-02-01T00:00:00Z");
        let filter = TimeRangeFilter {
            start: Some(start),
            end: Some(end),
        };

        assert!(filter.contains(&dt_utc("2025-01-01T00:00:00Z")));
        assert!(!filter.contains(&dt_utc("2024-12-31T23:59:59Z")));
        assert!(filter.contains(&dt_utc("2025-01-15T12:00:00Z")));
        assert!(filter.contains(&dt_utc("2025-01-31T23:59:59Z")));
        assert!(!filter.contains(&dt_utc("2025-02-01T00:00:00Z")));
        assert!(!filter.contains(&dt_utc("2025-02-01T00:00:01Z")));
    }

    #[test]
    fn test_time_range_filter_start_only() {
        let start = dt_utc("2025-01-01T00:00:00Z");
        let filter = TimeRangeFilter {
            start: Some(start),
            end: None,
        };

        assert!(filter.contains(&dt_utc("2025-01-01T00:00:00Z")));
        assert!(filter.contains(&dt_utc("2030-01-01T00:00:00Z")));
        assert!(!filter.contains(&dt_utc("2024-12-31T23:59:59Z")));
    }

    #[test]
    fn test_time_range_filter_end_only() {
        let end = dt_utc("2025-02-01T00:00:00Z");
        let filter = TimeRangeFilter {
            start: None,
            end: Some(end),
        };

        assert!(filter.contains(&dt_utc("2025-01-15T12:00:00Z")));
        assert!(filter.contains(&dt_utc("2020-01-01T00:00:00Z")));
        assert!(filter.contains(&dt_utc("2025-01-31T23:59:59Z")));
        assert!(!filter.contains(&dt_utc("2025-02-01T00:00:00Z")));
    }

    #[test]
    fn test_time_range_filter_empty() {
        let filter = TimeRangeFilter {
            start: None,
            end: None,
        };

        assert!(filter.contains(&dt_utc("2020-01-01T00:00:00Z")));
        assert!(filter.contains(&dt_utc("2030-01-01T00:00:00Z")));
    }

    #[test]
    fn test_time_range_filter_new_validates_start_before_end() {
        let error = TimeRangeFilter::new(
            Some("2025-01-02T00:00:00Z".to_string()),
            Some("2025-01-01T00:00:00Z".to_string()),
            DateTimezone::Utc,
        )
        .unwrap_err();

        assert!(
            format!("{error}").contains("must be before end time"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn date_timezone_maps_dates_in_both_modes() {
        let timestamp = dt_utc("2025-01-01T23:30:00Z");

        assert_eq!(
            DateTimezone::Utc.to_date(&timestamp),
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap()
        );
        let _ = DateTimezone::Local.to_date(&timestamp);
    }

    #[test]
    fn date_only_start_local_excludes_previous_utc_day() {
        // `--start-time 2026-06-01 --timezone local` must exclude 2026-05-31.
        // Local June 1 midnight is at earliest 2026-05-31T10:00:00Z (UTC+14),
        // so UTC timestamps before that are unambiguously excluded everywhere.
        let filter =
            TimeRangeFilter::new(Some("2026-06-01".to_string()), None, DateTimezone::Local)
                .unwrap();

        assert!(filter.contains(&dt_utc("2026-06-02T00:00:00Z")));
        // 09:59:59Z is before local midnight June 1 in every timezone.
        assert!(!filter.contains(&dt_utc("2026-05-31T09:59:59Z")));
    }

    #[test]
    fn date_only_end_local_excludes_past_midnight() {
        // `--end-time 2026-06-01 --timezone local` covers whole local June 1;
        // use local noon (always within the day) rather than a fixed UTC instant.
        let filter = TimeRangeFilter::new(
            Some("2026-06-01".to_string()),
            Some("2026-06-01".to_string()),
            DateTimezone::Local,
        )
        .unwrap();

        let local_noon = Local
            .with_ymd_and_hms(2026, 6, 1, 12, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        assert!(filter.contains(&local_noon));
        // Anything 2026-06-03 or later is unambiguously past local June 2 midnight.
        assert!(!filter.contains(&dt_utc("2026-06-03T00:00:00Z")));
    }

    #[test]
    fn display_timestamp_render_in_both_modes() {
        let ts = dt_utc("2025-01-01T12:00:00Z");
        assert_eq!(
            DateTimezone::Utc.display_timestamp(&ts),
            "2025-01-01T12:00:00+00:00"
        );
        // Local mode renders in the local offset and must round-trip back to the same UTC instant.
        let displayed = DateTimezone::Local.display_timestamp(&ts);
        let reparsed = DateTime::parse_from_rfc3339(&displayed)
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(reparsed, ts);
    }
}
