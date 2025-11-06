use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use miette::Result;

/// Time range filter for API pricing analysis
#[derive(Debug, Clone)]
pub struct TimeRangeFilter {
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
}

impl TimeRangeFilter {
    /// Create a new time range filter from optional start/end strings
    ///
    /// For date-only strings (YYYY-MM-DD):
    /// - start_time: parsed as 00:00:00 (beginning of day)
    /// - end_time: parsed as 00:00:00 of the NEXT day (to include entire day with exclusive end)
    pub fn new(start: Option<String>, end: Option<String>) -> Result<Self> {
        let start_dt = start.map(|s| parse_datetime_for_start(&s)).transpose()?;
        let end_dt = end.map(|s| parse_datetime_for_end(&s)).transpose()?;

        // Validate that start < end if both provided (note: < not <=, since end is exclusive)
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

    /// Check if a timestamp is within the filter range
    ///
    /// Uses half-open interval semantics: [start, end)
    /// - start is inclusive
    /// - end is exclusive
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

    /// Returns true if no filters are set (matches all timestamps)
    pub fn is_unrestricted(&self) -> bool {
        self.start.is_none() && self.end.is_none()
    }
}

/// Parses start time boundary. Date-only strings use 00:00:00 of the specified day
/// to include messages from the beginning of that day (inclusive start).
fn parse_datetime_for_start(s: &str) -> Result<DateTime<Utc>> {
    // Try ISO 8601 with timezone first
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }

    // Try date only (YYYY-MM-DD) - use start of day UTC
    if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let datetime = date
            .and_hms_opt(0, 0, 0)
            .expect("00:00:00 is always a valid time");
        return Ok(DateTime::from_naive_utc_and_offset(datetime, Utc));
    }

    // Try datetime without timezone (assume UTC)
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Ok(DateTime::from_naive_utc_and_offset(dt, Utc));
    }

    Err(miette::miette!(
        "Invalid datetime format: '{}'. Expected ISO 8601 (e.g., '2025-01-01T00:00:00Z') or date (e.g., '2025-01-01')",
        s
    ))
}

/// For date-only strings, returns start of NEXT day to include the entire specified day
/// (since end boundary is exclusive). Time-based strings are used as-is.
fn parse_datetime_for_end(s: &str) -> Result<DateTime<Utc>> {
    // Try ISO 8601 with timezone first
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }

    // Try date only (YYYY-MM-DD) - use start of NEXT day for exclusive end
    if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let next_day = date
            .succ_opt()
            .expect("Date overflow only occurs beyond year 262000, unreachable for API logs");
        let datetime = next_day
            .and_hms_opt(0, 0, 0)
            .expect("00:00:00 is always a valid time");
        return Ok(DateTime::from_naive_utc_and_offset(datetime, Utc));
    }

    // Try datetime without timezone (assume UTC)
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Ok(DateTime::from_naive_utc_and_offset(dt, Utc));
    }

    Err(miette::miette!(
        "Invalid datetime format: '{}'. Expected ISO 8601 (e.g., '2025-01-01T00:00:00Z') or date (e.g., '2025-01-01')",
        s
    ))
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
        // Should convert to UTC
        assert_eq!(dt.to_rfc3339(), "2025-01-01T17:00:00+00:00");
    }

    #[test]
    fn test_parse_datetime_date_only_for_start() {
        let dt = parse_datetime_for_start("2025-01-01").unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-01-01T00:00:00+00:00");
    }

    #[test]
    fn test_parse_datetime_date_only_for_end() {
        // End date should be parsed as start of NEXT day for exclusive end
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
            end: Some(parse_datetime_for_start("2025-02-01T00:00:00Z").unwrap()), // Exclusive end
        };

        // Inside range
        assert!(filter.contains(&parse_datetime_for_start("2025-01-15T12:00:00Z").unwrap()));
        // Before start
        assert!(!filter.contains(&parse_datetime_for_start("2024-12-31T23:59:59Z").unwrap()));
        // At or after end (exclusive)
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
            end: Some(parse_datetime_for_start("2025-02-01T00:00:00Z").unwrap()), // Exclusive end
        };

        assert!(filter.contains(&parse_datetime_for_start("2025-01-15T12:00:00Z").unwrap()));
        assert!(filter.contains(&parse_datetime_for_start("2020-01-01T00:00:00Z").unwrap()));
        assert!(filter.contains(&parse_datetime_for_start("2025-01-31T23:59:59Z").unwrap()));
        assert!(!filter.contains(&parse_datetime_for_start("2025-02-01T00:00:00Z").unwrap()));
        // Exclusive
    }

    #[test]
    fn test_time_range_filter_empty() {
        let filter = TimeRangeFilter {
            start: None,
            end: None,
        };

        assert!(filter.is_unrestricted());
        assert!(filter.contains(&parse_datetime_for_start("2025-01-15T12:00:00Z").unwrap()));
        assert!(filter.contains(&parse_datetime_for_start("2020-01-01T00:00:00Z").unwrap()));
        assert!(filter.contains(&parse_datetime_for_start("2030-01-01T00:00:00Z").unwrap()));
    }

    #[test]
    fn test_time_range_filter_boundary_conditions() {
        let filter = TimeRangeFilter {
            start: Some(parse_datetime_for_start("2025-01-01T00:00:00Z").unwrap()),
            end: Some(parse_datetime_for_start("2025-02-01T00:00:00Z").unwrap()), // Exclusive end
        };

        // Start boundary is inclusive
        assert!(filter.contains(&parse_datetime_for_start("2025-01-01T00:00:00Z").unwrap()));
        // One microsecond before start
        assert!(!filter.contains(&parse_datetime_for_start("2024-12-31T23:59:59Z").unwrap()));
        // Just before end (should be included)
        assert!(filter.contains(&parse_datetime_for_start("2025-01-31T23:59:59Z").unwrap()));
        // At end boundary (exclusive, should NOT be included)
        assert!(!filter.contains(&parse_datetime_for_start("2025-02-01T00:00:00Z").unwrap()));
    }

    #[test]
    fn test_date_only_end_includes_entire_day() {
        // Test that --end-time "2025-01-31" includes all of Jan 31
        let filter = TimeRangeFilter::new(
            Some("2025-01-01".to_string()),
            Some("2025-01-31".to_string()),
        )
        .unwrap();

        // Should include messages throughout Jan 31
        assert!(filter.contains(&parse_datetime_for_start("2025-01-31T00:00:00Z").unwrap()));
        assert!(filter.contains(&parse_datetime_for_start("2025-01-31T12:00:00Z").unwrap()));
        assert!(filter.contains(&parse_datetime_for_start("2025-01-31T23:59:59Z").unwrap()));

        // But not Feb 1
        assert!(!filter.contains(&parse_datetime_for_start("2025-02-01T00:00:00Z").unwrap()));
    }

    #[test]
    fn test_time_range_filter_new_validation() {
        let err = TimeRangeFilter::new(
            Some("2025-02-01".to_string()),
            Some("2025-01-01".to_string()),
        )
        .expect_err("Should fail when start is after end");
        assert!(err.to_string().contains("must be before end time"));
    }

    #[test]
    fn test_time_range_filter_rejects_equal_start_end() {
        let err = TimeRangeFilter::new(
            Some("2025-01-01T12:00:00Z".to_string()),
            Some("2025-01-01T12:00:00Z".to_string()),
        )
        .expect_err("Should fail when start equals end");
        assert!(err.to_string().contains("must be before end time"));
    }

    #[test]
    fn test_time_range_filter_new_valid() {
        let filter = TimeRangeFilter::new(
            Some("2025-01-01".to_string()),
            Some("2025-01-31".to_string()),
        )
        .unwrap();

        assert!(filter.start.is_some());
        assert!(filter.end.is_some());
        assert!(!filter.is_unrestricted());
    }

    #[test]
    fn test_time_range_filter_new_empty() {
        let filter = TimeRangeFilter::new(None, None).unwrap();
        assert!(filter.is_unrestricted());
    }
}
