use chrono::{DateTime, NaiveDate, Utc};

use super::ChartSegment;

/// Carries the bucket's date instead of a rendered label so a combined chart can
/// merge same-day contributions from several sources before labelling.
pub(crate) struct DatedSegments {
    pub(crate) date: NaiveDate,
    pub(crate) segments: Vec<ChartSegment>,
}

/// `start_time` is kept alongside the id because a combined chart orders
/// conversations chronologically across sources, not by id.
pub(crate) struct SessionSegments {
    pub(crate) session_id: String,
    pub(crate) start_time: DateTime<Utc>,
    pub(crate) segments: Vec<ChartSegment>,
}
