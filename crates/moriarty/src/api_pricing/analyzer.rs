use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

use async_walkdir::WalkDir;
use chrono::{DateTime, Local, NaiveDate, Utc};
use futures::stream::{self, StreamExt};
use miette::IntoDiagnostic;
use rayon::prelude::*;
use tracing::{debug, trace, warn};

#[cfg(test)]
use crate::logs::parser;
use crate::logs::parser::{LogLine, LogMessageContent, LogMessageTaggedContent};

use super::{
    line_counter,
    pricing::{ModelType, TokenCosts, TokenCounts},
};

/// Timezone to use when extracting dates from timestamps
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateTimezone {
    /// Use the system's local timezone
    Local,
    /// Use UTC timezone
    Utc,
}

impl DateTimezone {
    /// Convert a UTC timestamp to a date in this timezone
    pub fn to_date(self, timestamp: &chrono::DateTime<chrono::Utc>) -> NaiveDate {
        match self {
            Self::Local => timestamp.with_timezone(&Local).date_naive(),
            Self::Utc => timestamp.date_naive(),
        }
    }
}

/// Returns true if the model string represents a synthetic/internal model
/// that should be excluded from billing calculations.
///
/// Synthetic models are internal processing steps, not billable API calls.
#[inline]
fn is_synthetic_model(model: &str) -> bool {
    model.eq_ignore_ascii_case("<synthetic>")
}

#[derive(Debug, Default)]
pub struct AnalysisResult {
    pub daily_costs: Vec<DailyCosts>,
    pub unknown_models: HashSet<String>,
    pub total_unknown_tokens: TokenCounts,
    pub files_parsed: usize,
    pub files_failed: usize,
}

/// Result of analyzing log files by conversation/session
///
/// Sessions are identified by session_id from log files and sorted chronologically.
#[derive(Debug, Default)]
pub struct SessionAnalysisResult {
    pub session_costs: Vec<SessionCosts>,
    pub unknown_models: HashSet<String>,
    pub total_unknown_tokens: TokenCounts,
    pub files_parsed: usize,
    pub files_failed: usize,
}

#[derive(Debug)]
pub struct DailyUsage {
    pub date: NaiveDate,
    pub sonnet_usage: TokenCounts,
    pub haiku_usage: TokenCounts,
    pub opus_usage: TokenCounts,
    pub unknown_usage: TokenCounts,
    pub lines_changed: usize,
}

impl DailyUsage {
    pub fn new(date: NaiveDate) -> Self {
        Self {
            date,
            sonnet_usage: TokenCounts::default(),
            haiku_usage: TokenCounts::default(),
            opus_usage: TokenCounts::default(),
            unknown_usage: TokenCounts::default(),
            lines_changed: 0,
        }
    }

    pub fn add_usage(&mut self, model_type: ModelType, counts: TokenCounts) {
        match model_type {
            ModelType::Sonnet => self.sonnet_usage.add(&counts),
            ModelType::Haiku => self.haiku_usage.add(&counts),
            ModelType::Opus => self.opus_usage.add(&counts),
            ModelType::Unknown => self.unknown_usage.add(&counts),
        }
    }

    pub fn add_lines_changed(&mut self, lines: usize) {
        self.lines_changed += lines;
    }

    pub fn calculate_costs(&self) -> DailyCosts {
        let sonnet_costs = ModelType::Sonnet
            .pricing()
            .map(|p| p.calculate_cost(&self.sonnet_usage))
            .unwrap_or_default();

        let haiku_costs = ModelType::Haiku
            .pricing()
            .map(|p| p.calculate_cost(&self.haiku_usage))
            .unwrap_or_default();

        let opus_costs = ModelType::Opus
            .pricing()
            .map(|p| p.calculate_cost(&self.opus_usage))
            .unwrap_or_default();

        DailyCosts {
            date: self.date,
            sonnet_costs,
            haiku_costs,
            opus_costs,
            lines_changed: self.lines_changed,
        }
    }
}

#[derive(Debug)]
pub struct DailyCosts {
    pub date: NaiveDate,
    pub sonnet_costs: TokenCosts,
    pub haiku_costs: TokenCosts,
    pub opus_costs: TokenCosts,
    pub lines_changed: usize,
}

impl DailyCosts {
    pub fn total(&self) -> f64 {
        self.sonnet_costs.total() + self.haiku_costs.total() + self.opus_costs.total()
    }
}

/// Accumulator for session token usage.
///
/// Automatically expands start_time/end_time bounds as usage is added,
/// enabling duration tracking without manual timestamp management.
#[derive(Debug)]
pub struct SessionUsage {
    pub session_id: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub sonnet_usage: TokenCounts,
    pub haiku_usage: TokenCounts,
    pub opus_usage: TokenCounts,
    pub unknown_usage: TokenCounts,
    pub lines_changed: usize,
}

impl SessionUsage {
    pub fn new(session_id: String, timestamp: DateTime<Utc>) -> Self {
        Self {
            session_id,
            start_time: timestamp,
            end_time: timestamp,
            sonnet_usage: TokenCounts::default(),
            haiku_usage: TokenCounts::default(),
            opus_usage: TokenCounts::default(),
            unknown_usage: TokenCounts::default(),
            lines_changed: 0,
        }
    }

    fn update_time_range(&mut self, timestamp: DateTime<Utc>) {
        if timestamp < self.start_time {
            self.start_time = timestamp;
        }
        if timestamp > self.end_time {
            self.end_time = timestamp;
        }
    }

    pub fn add_usage(
        &mut self,
        model_type: ModelType,
        counts: TokenCounts,
        timestamp: DateTime<Utc>,
    ) {
        match model_type {
            ModelType::Sonnet => self.sonnet_usage.add(&counts),
            ModelType::Haiku => self.haiku_usage.add(&counts),
            ModelType::Opus => self.opus_usage.add(&counts),
            ModelType::Unknown => self.unknown_usage.add(&counts),
        }
        self.update_time_range(timestamp);
    }

    pub fn add_lines_changed(&mut self, lines: usize, timestamp: DateTime<Utc>) {
        self.lines_changed += lines;
        self.update_time_range(timestamp);
    }

    pub fn calculate_costs(&self) -> SessionCosts {
        let sonnet_costs = ModelType::Sonnet
            .pricing()
            .map(|p| p.calculate_cost(&self.sonnet_usage))
            .unwrap_or_default();

        let haiku_costs = ModelType::Haiku
            .pricing()
            .map(|p| p.calculate_cost(&self.haiku_usage))
            .unwrap_or_default();

        let opus_costs = ModelType::Opus
            .pricing()
            .map(|p| p.calculate_cost(&self.opus_usage))
            .unwrap_or_default();

        SessionCosts {
            session_id: self.session_id.clone(),
            start_time: self.start_time,
            end_time: self.end_time,
            sonnet_costs,
            haiku_costs,
            opus_costs,
            lines_changed: self.lines_changed,
        }
    }
}

/// Computed costs for a single conversation/session
///
/// Contains dollar amounts for each model's usage within a session,
/// along with the session's time range and code changes. Includes
/// a convenience method to calculate session duration in minutes.
#[derive(Debug)]
pub struct SessionCosts {
    pub session_id: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub sonnet_costs: TokenCosts,
    pub haiku_costs: TokenCosts,
    pub opus_costs: TokenCosts,
    pub lines_changed: usize,
}

impl SessionCosts {
    pub fn total(&self) -> f64 {
        self.sonnet_costs.total() + self.haiku_costs.total() + self.opus_costs.total()
    }

    pub fn duration_minutes(&self) -> i64 {
        (self.end_time - self.start_time).num_minutes()
    }
}

/// Recursively walk a directory and find all .jsonl files
pub async fn find_jsonl_files(dir: &Path) -> miette::Result<Vec<PathBuf>> {
    let mut jsonl_files = Vec::new();
    let mut entries = WalkDir::new(dir);

    while let Some(entry) = entries.next().await {
        let entry = entry.into_diagnostic()?;

        if entry.file_type().await.into_diagnostic()?.is_file() {
            if let Some(extension) = entry.path().extension() {
                if extension == "jsonl" {
                    jsonl_files.push(entry.path());
                }
            }
        }
    }

    Ok(jsonl_files)
}

/// Message containing token usage aggregated by date
#[derive(Debug, Clone)]
pub(crate) struct DateBasedMessage {
    pub(crate) date: NaiveDate,
    pub(crate) model_type: ModelType,
    pub(crate) model_string: String,
    pub(crate) token_counts: TokenCounts,
}

/// Message containing token usage aggregated by session
#[derive(Debug, Clone)]
pub(crate) struct SessionBasedMessage {
    pub(crate) session_id: String,
    pub(crate) timestamp: DateTime<Utc>,
    pub(crate) model_type: ModelType,
    pub(crate) model_string: String,
    pub(crate) token_counts: TokenCounts,
}

/// Deduplicates messages by requestId, keeping only the final message per streaming group.
///
/// For messages with the same requestId, retains the one with the highest output_tokens.
/// Messages without a requestId (non-streaming) are kept as-is.
///
/// ## Design Decision: Using output_tokens as Finality Heuristic
///
/// This function uses `max(output_tokens)` to identify the final message in a streaming group.
/// This works because streaming responses accumulate tokens monotonically - each subsequent
/// message has equal or greater token counts than the previous.
///
/// **Alternative considered**: Using message timestamps for ordering. While more semantically
/// correct, this would require restructuring the message tuples to include timestamps in
/// `parse_log_file`, significantly complicating the codebase. The current approach is simpler
/// and handles all realistic streaming scenarios correctly.
///
/// **Known limitations**:
/// - If all messages have zero output_tokens, keeps the first encountered. This is
///   acceptable because zero-token messages are rare edge cases (API errors, empty
///   responses) and all such messages are functionally equivalent for cost calculation.
/// - Assumes in-order message delivery (true for Claude API in practice)
/// - If the API ever sent decreasing token counts, this would fail (not observed)
fn deduplicate_by_request_id<T>(
    messages: Vec<(Option<String>, T)>,
    get_output_tokens: impl Fn(&T) -> u64,
) -> Vec<T> {
    let total_messages = messages.len();
    let mut result: Vec<T> = Vec::new();
    let mut request_id_to_index: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut duplicates_found = 0;
    let mut duplicate_request_ids: HashSet<String> = HashSet::new();

    debug!(
        "deduplicate_by_request_id: Processing {} messages",
        total_messages
    );

    for (request_id, payload) in messages {
        if let Some(req_id) = request_id {
            // Message has a requestId - part of a streaming group
            if let Some(&existing_idx) = request_id_to_index.get(&req_id) {
                duplicates_found += 1;
                duplicate_request_ids.insert(req_id.clone());
                let old_tokens = get_output_tokens(&result[existing_idx]);
                let new_tokens = get_output_tokens(&payload);
                trace!(
                    "Found duplicate requestId='{}': old_tokens={}, new_tokens={}",
                    req_id,
                    old_tokens,
                    new_tokens
                );
                // Keep the message with higher output_tokens (final message)
                if new_tokens > old_tokens {
                    trace!("  -> Keeping new message (higher token count)");
                    result[existing_idx] = payload;
                } else {
                    trace!("  -> Keeping existing message");
                }
            } else {
                // First occurrence of this requestId
                trace!(
                    "First occurrence of requestId='{}' with {} output tokens",
                    req_id,
                    get_output_tokens(&payload)
                );
                request_id_to_index.insert(req_id, result.len());
                result.push(payload);
            }
        } else {
            // No requestId - non-streaming message, keep as-is
            trace!("Non-streaming message (no requestId)");
            result.push(payload);
        }
    }

    debug!(
        "deduplicate_by_request_id: {} messages -> {} after deduplication ({} duplicates removed, {} unique request_ids had duplicates)",
        total_messages,
        result.len(),
        duplicates_found,
        duplicate_request_ids.len()
    );

    result
}

/// Parse a log file and extract token usage
///
/// Filters out entries with model `<synthetic>` (case-insensitive), as these represent
/// internal processing steps rather than billable API usage.
///
/// Deduplicates streaming API responses by requestId. When multiple messages share
/// the same requestId (streaming responses), only the final message (with highest
/// output_tokens) is counted to avoid inflating costs.
///
/// NOTE: This async function is only used in tests. Production code uses the synchronous
/// `parse_log_content` function with rayon for file-level parallelism.
#[cfg(test)]
pub async fn parse_log_file(
    file: &Path,
    timezone: DateTimezone,
) -> miette::Result<Vec<DateBasedMessage>> {
    let log_lines = parser::read_file(file).await?;

    // Temporary structure to hold all messages with their requestId
    let mut all_messages: Vec<(Option<String>, DateBasedMessage)> = Vec::new();

    for line in log_lines {
        if let LogLine::Assistant(assistant_line) = line {
            if is_synthetic_model(&assistant_line.message.model) {
                continue;
            }

            let date = timezone.to_date(&assistant_line.timestamp);
            let model_string = assistant_line.message.model.clone();
            let model_type = ModelType::from_model_string(&model_string);
            let usage = &assistant_line.message.usage;

            let counts = TokenCounts {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cache_write_tokens: usage.cache_creation_input_tokens,
                cache_read_tokens: usage.cache_read_input_tokens,
            };

            all_messages.push((
                assistant_line.request_id.clone(),
                DateBasedMessage {
                    date,
                    model_type,
                    model_string,
                    token_counts: counts,
                },
            ));
        }
    }

    // Deduplicate streaming messages by requestId
    let usages = deduplicate_by_request_id(all_messages, |msg: &DateBasedMessage| {
        msg.token_counts.output_tokens as u64
    });

    Ok(usages)
}

/// Parse a log file and extract token usage by session
///
/// Filters out entries with model `<synthetic>` (case-insensitive), as these represent
/// internal processing steps rather than billable API usage.
///
/// Deduplicates streaming API responses by requestId. When multiple messages share
/// the same requestId (streaming responses), only the final message (with highest
/// output_tokens) is counted to avoid inflating costs.
///
/// NOTE: This async function is only used in tests. Production code uses the synchronous
/// `parse_log_content_by_session` function with rayon for file-level parallelism.
#[cfg(test)]
pub async fn parse_log_file_by_session(file: &Path) -> miette::Result<Vec<SessionBasedMessage>> {
    let log_lines = parser::read_file(file).await?;

    // Temporary structure to hold all messages with their requestId
    let mut all_messages: Vec<(Option<String>, SessionBasedMessage)> = Vec::new();

    for line in log_lines {
        if let LogLine::Assistant(assistant_line) = line {
            if is_synthetic_model(&assistant_line.message.model) {
                continue;
            }

            let session_id = assistant_line.session_id.clone();
            let timestamp = assistant_line.timestamp;
            let model_string = assistant_line.message.model.clone();
            let model_type = ModelType::from_model_string(&model_string);
            let usage = &assistant_line.message.usage;

            let counts = TokenCounts {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cache_write_tokens: usage.cache_creation_input_tokens,
                cache_read_tokens: usage.cache_read_input_tokens,
            };

            all_messages.push((
                assistant_line.request_id.clone(),
                SessionBasedMessage {
                    session_id,
                    timestamp,
                    model_type,
                    model_string,
                    token_counts: counts,
                },
            ));
        }
    }

    // Deduplicate streaming messages by requestId
    let usages = deduplicate_by_request_id(all_messages, |msg: &SessionBasedMessage| {
        msg.token_counts.output_tokens as u64
    });

    Ok(usages)
}

/// Extract lines changed from assistant messages in a log file
///
/// Filters out entries with model `<synthetic>` (case-insensitive), as these represent
/// internal processing steps rather than billable API usage.
///
/// NOTE: This async function is only used in tests. Production code uses the synchronous
/// `parse_lines_changed_content` function with rayon for file-level parallelism.
#[cfg(test)]
pub async fn parse_lines_changed(
    file: &Path,
    timezone: DateTimezone,
) -> miette::Result<Vec<(NaiveDate, usize)>> {
    let log_lines = parser::read_file(file).await?;
    let mut results = Vec::new();

    for line in log_lines {
        if let LogLine::Assistant(assistant_line) = line {
            if is_synthetic_model(&assistant_line.message.model) {
                continue;
            }

            let date = timezone.to_date(&assistant_line.timestamp);

            // Check if content is an array (contains tool uses)
            if let LogMessageContent::Vec(content_blocks) = &assistant_line.message.content {
                for content_item in content_blocks {
                    if let LogMessageTaggedContent::ToolUse { name, input, .. } = content_item {
                        if let Some(lines) =
                            line_counter::extract_lines_from_tool(name.as_str(), input)
                        {
                            results.push((date, lines));
                        }
                    }
                }
            }
        }
    }

    Ok(results)
}

/// Aggregate usage data by date
pub fn aggregate_by_date(
    usages: Vec<DateBasedMessage>,
    lines_changed: Vec<(NaiveDate, usize)>,
    unknown_models: &mut HashSet<String>,
    total_unknown_tokens: &mut TokenCounts,
) -> BTreeMap<NaiveDate, DailyUsage> {
    let mut daily_usage: BTreeMap<NaiveDate, DailyUsage> = BTreeMap::new();

    // Aggregate token usage
    for msg in usages {
        // Track unknown models
        if msg.model_type == ModelType::Unknown {
            unknown_models.insert(msg.model_string);
            total_unknown_tokens.add(&msg.token_counts);
        }

        daily_usage
            .entry(msg.date)
            .or_insert_with(|| DailyUsage::new(msg.date))
            .add_usage(msg.model_type, msg.token_counts);
    }

    // Aggregate lines changed
    for (date, lines) in lines_changed {
        daily_usage
            .entry(date)
            .or_insert_with(|| DailyUsage::new(date))
            .add_lines_changed(lines);
    }

    daily_usage
}

/// Aggregate usage data by session
pub fn aggregate_by_session(
    usages: Vec<SessionBasedMessage>,
    lines_changed: Vec<(String, DateTime<Utc>, usize)>,
    unknown_models: &mut HashSet<String>,
    total_unknown_tokens: &mut TokenCounts,
) -> BTreeMap<String, SessionUsage> {
    let mut session_usage: BTreeMap<String, SessionUsage> = BTreeMap::new();

    // Aggregate token usage
    for msg in usages {
        // Track unknown models
        if msg.model_type == ModelType::Unknown {
            unknown_models.insert(msg.model_string);
            total_unknown_tokens.add(&msg.token_counts);
        }

        session_usage
            .entry(msg.session_id.clone())
            .or_insert_with(|| SessionUsage::new(msg.session_id, msg.timestamp))
            .add_usage(msg.model_type, msg.token_counts, msg.timestamp);
    }

    // Aggregate lines changed
    for (session_id, timestamp, lines) in lines_changed {
        session_usage
            .entry(session_id.clone())
            .or_insert_with(|| SessionUsage::new(session_id, timestamp))
            .add_lines_changed(lines, timestamp);
    }

    session_usage
}

/// Parse log file contents (from string) and extract token usage
///
/// Synchronous version of parse_log_file that works with pre-loaded file contents.
/// Used for parallel file processing with rayon.
fn parse_log_content(
    contents: &str,
    timezone: DateTimezone,
) -> miette::Result<Vec<DateBasedMessage>> {
    // Parse lines sequentially from the string contents
    let log_lines: Vec<LogLine> = contents
        .split('\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str::<LogLine>(line).inspect_err(|_| println!("{line}")))
        .collect::<Result<Vec<_>, _>>()
        .into_diagnostic()?;

    // Temporary structure to hold all messages with their requestId
    let mut all_messages: Vec<(Option<String>, DateBasedMessage)> = Vec::new();

    for line in log_lines {
        if let LogLine::Assistant(assistant_line) = line {
            if is_synthetic_model(&assistant_line.message.model) {
                continue;
            }

            let date = timezone.to_date(&assistant_line.timestamp);
            let model_string = assistant_line.message.model.clone();
            let model_type = ModelType::from_model_string(&model_string);
            let usage = &assistant_line.message.usage;

            let counts = TokenCounts {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cache_write_tokens: usage.cache_creation_input_tokens,
                cache_read_tokens: usage.cache_read_input_tokens,
            };

            all_messages.push((
                assistant_line.request_id.clone(),
                DateBasedMessage {
                    date,
                    model_type,
                    model_string,
                    token_counts: counts,
                },
            ));
        }
    }

    // Deduplicate streaming messages by requestId
    let usages = deduplicate_by_request_id(all_messages, |msg: &DateBasedMessage| {
        msg.token_counts.output_tokens as u64
    });

    Ok(usages)
}

/// Parse log file contents (from string) and extract lines changed
///
/// Synchronous version of parse_lines_changed that works with pre-loaded file contents.
/// Used for parallel file processing with rayon.
fn parse_lines_changed_content(
    contents: &str,
    timezone: DateTimezone,
) -> miette::Result<Vec<(NaiveDate, usize)>> {
    // Parse lines sequentially from the string contents
    let log_lines: Vec<LogLine> = contents
        .split('\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str::<LogLine>(line).inspect_err(|_| println!("{line}")))
        .collect::<Result<Vec<_>, _>>()
        .into_diagnostic()?;

    let mut results = Vec::new();

    for line in log_lines {
        if let LogLine::Assistant(assistant_line) = line {
            if is_synthetic_model(&assistant_line.message.model) {
                continue;
            }

            let date = timezone.to_date(&assistant_line.timestamp);

            // Check if content is an array (contains tool uses)
            if let LogMessageContent::Vec(content_blocks) = &assistant_line.message.content {
                for content_item in content_blocks {
                    if let LogMessageTaggedContent::ToolUse { name, input, .. } = content_item {
                        if let Some(lines) =
                            line_counter::extract_lines_from_tool(name.as_str(), input)
                        {
                            results.push((date, lines));
                        }
                    }
                }
            }
        }
    }

    Ok(results)
}

/// Parse log file contents (from string) and extract token usage by session
///
/// Synchronous version of parse_log_file_by_session that works with pre-loaded file contents.
/// Used for parallel file processing with rayon.
fn parse_log_content_by_session(contents: &str) -> miette::Result<Vec<SessionBasedMessage>> {
    // Parse lines sequentially from the string contents
    let log_lines: Vec<LogLine> = contents
        .split('\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str::<LogLine>(line).inspect_err(|_| println!("{line}")))
        .collect::<Result<Vec<_>, _>>()
        .into_diagnostic()?;

    // Temporary structure to hold all messages with their requestId
    let mut all_messages: Vec<(Option<String>, SessionBasedMessage)> = Vec::new();

    for line in log_lines {
        if let LogLine::Assistant(assistant_line) = line {
            if is_synthetic_model(&assistant_line.message.model) {
                continue;
            }

            let session_id = assistant_line.session_id.clone();
            let timestamp = assistant_line.timestamp;
            let model_string = assistant_line.message.model.clone();
            let model_type = ModelType::from_model_string(&model_string);
            let usage = &assistant_line.message.usage;

            let counts = TokenCounts {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cache_write_tokens: usage.cache_creation_input_tokens,
                cache_read_tokens: usage.cache_read_input_tokens,
            };

            all_messages.push((
                assistant_line.request_id.clone(),
                SessionBasedMessage {
                    session_id,
                    timestamp,
                    model_type,
                    model_string,
                    token_counts: counts,
                },
            ));
        }
    }

    // Deduplicate streaming messages by requestId
    let usages = deduplicate_by_request_id(all_messages, |msg: &SessionBasedMessage| {
        msg.token_counts.output_tokens as u64
    });

    Ok(usages)
}

/// Parse log file contents (from string) and extract lines changed by session
///
/// Synchronous version of parse_lines_changed_by_session that works with pre-loaded file contents.
/// Used for parallel file processing with rayon.
fn parse_lines_changed_content_by_session(
    contents: &str,
) -> miette::Result<Vec<(String, DateTime<Utc>, usize)>> {
    // Parse lines sequentially from the string contents
    let log_lines: Vec<LogLine> = contents
        .split('\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str::<LogLine>(line).inspect_err(|_| println!("{line}")))
        .collect::<Result<Vec<_>, _>>()
        .into_diagnostic()?;

    let mut results = Vec::new();

    for line in log_lines {
        if let LogLine::Assistant(assistant_line) = line {
            if is_synthetic_model(&assistant_line.message.model) {
                continue;
            }

            let session_id = assistant_line.session_id.clone();
            let timestamp = assistant_line.timestamp;

            // Check if content is an array (contains tool uses)
            if let LogMessageContent::Vec(content_blocks) = &assistant_line.message.content {
                for content_item in content_blocks {
                    if let LogMessageTaggedContent::ToolUse { name, input, .. } = content_item {
                        if let Some(lines) =
                            line_counter::extract_lines_from_tool(name.as_str(), input)
                        {
                            results.push((session_id.clone(), timestamp, lines));
                        }
                    }
                }
            }
        }
    }

    Ok(results)
}

/// Reads files asynchronously with concurrency limit to prevent file descriptor exhaustion.
///
/// The limit of 10 concurrent operations balances throughput with system resource constraints.
async fn read_files_parallel(
    jsonl_files: Vec<PathBuf>,
) -> (Vec<(PathBuf, String)>, usize) {
    let file_stream = stream::iter(jsonl_files.into_iter());
    let read_futures = file_stream.map(|path| async move {
        let contents = tokio::fs::read_to_string(&path).await;
        (path, contents)
    });

    let mut read_results = read_futures.buffer_unordered(10);
    let mut file_contents = Vec::new();
    let mut files_failed = 0;

    while let Some((path, contents_result)) = read_results.next().await {
        match contents_result {
            Ok(contents) => file_contents.push((path, contents)),
            Err(e) => {
                warn!("Failed to read file: {:?}: {}", path, e);
                files_failed += 1;
            }
        }
    }

    (file_contents, files_failed)
}

/// Parses file contents using rayon for CPU-bound JSON deserialization.
///
/// Generic over parsing functions to enable reuse between date-based and session-based analysis.
/// Partial failures are tolerated - individual file parse errors don't halt processing.
fn parse_files_parallel<UsageType, LinesType, FUsage, FLines>(
    file_contents: Vec<(PathBuf, String)>,
    parse_usage: FUsage,
    parse_lines: FLines,
) -> (Vec<UsageType>, Vec<LinesType>, usize, usize)
where
    UsageType: Send,
    LinesType: Send,
    FUsage: Fn(&str) -> miette::Result<Vec<UsageType>> + Sync,
    FLines: Fn(&str) -> miette::Result<Vec<LinesType>> + Sync,
{
    let parse_results: Vec<_> = file_contents
        .par_iter()
        .map(|(path, contents)| {
            let usage_result = parse_usage(contents);
            let lines_result = parse_lines(contents);
            (path.clone(), usage_result, lines_result)
        })
        .collect();

    let mut all_usages = Vec::new();
    let mut all_lines = Vec::new();
    let mut files_parsed = 0;
    let mut files_failed = 0;

    for (file, usage_result, lines_result) in parse_results {
        match usage_result {
            Ok(usages) => {
                all_usages.extend(usages);
                files_parsed += 1;
            }
            Err(e) => {
                warn!("Failed to parse file: {:?}: {}", file, e);
                files_failed += 1;
            }
        }

        match lines_result {
            Ok(lines) => all_lines.extend(lines),
            Err(e) => {
                warn!("Failed to parse lines changed from file: {:?}: {}", file, e);
                // Lines parsing is optional for cost calculation, so we don't increment files_failed
            }
        }
    }

    (all_usages, all_lines, files_parsed, files_failed)
}

/// Analyze all log files in a directory and return daily costs
pub async fn analyze_directory(
    dir: &Path,
    timezone: DateTimezone,
) -> miette::Result<AnalysisResult> {
    let jsonl_files = find_jsonl_files(dir).await?;

    if jsonl_files.is_empty() {
        warn!("No .jsonl files found in directory");
        return Ok(AnalysisResult::default());
    }

    println!("Found {} log files to analyze", jsonl_files.len());

    // Step 1: Read all files in parallel
    let (file_contents, mut files_failed) = read_files_parallel(jsonl_files).await;

    // Step 2: Parse files in parallel using rayon
    let (all_usages, all_lines_changed, files_parsed, parse_failed) = parse_files_parallel(
        file_contents,
        |contents| parse_log_content(contents, timezone),
        |contents| parse_lines_changed_content(contents, timezone),
    );

    files_failed += parse_failed;

    // Step 3: Aggregate results
    let mut unknown_models = HashSet::new();
    let mut total_unknown_tokens = TokenCounts::default();

    let daily_usage = aggregate_by_date(
        all_usages,
        all_lines_changed,
        &mut unknown_models,
        &mut total_unknown_tokens,
    );
    let daily_costs: Vec<DailyCosts> = daily_usage
        .into_values()
        .map(|usage| usage.calculate_costs())
        .collect();

    Ok(AnalysisResult {
        daily_costs,
        unknown_models,
        total_unknown_tokens,
        files_parsed,
        files_failed,
    })
}

/// Analyze all log files in a directory and return session costs
pub async fn analyze_directory_by_session(dir: &Path) -> miette::Result<SessionAnalysisResult> {
    let jsonl_files = find_jsonl_files(dir).await?;

    if jsonl_files.is_empty() {
        warn!("No .jsonl files found in directory");
        return Ok(SessionAnalysisResult::default());
    }

    println!("Found {} log files to analyze", jsonl_files.len());

    // Step 1: Read all files in parallel
    let (file_contents, mut files_failed) = read_files_parallel(jsonl_files).await;

    // Step 2: Parse files in parallel using rayon
    let (all_usages, all_lines_changed, files_parsed, parse_failed) = parse_files_parallel(
        file_contents,
        parse_log_content_by_session,
        parse_lines_changed_content_by_session,
    );

    files_failed += parse_failed;

    // Step 3: Aggregate results
    let mut unknown_models = HashSet::new();
    let mut total_unknown_tokens = TokenCounts::default();

    let session_usage = aggregate_by_session(
        all_usages,
        all_lines_changed,
        &mut unknown_models,
        &mut total_unknown_tokens,
    );
    let mut session_costs: Vec<SessionCosts> = session_usage
        .into_values()
        .map(|usage| usage.calculate_costs())
        .collect();

    // Sort sessions chronologically (oldest first) by start_time
    session_costs.sort_by_key(|s| s.start_time);

    Ok(SessionAnalysisResult {
        session_costs,
        unknown_models,
        total_unknown_tokens,
        files_parsed,
        files_failed,
    })
}
