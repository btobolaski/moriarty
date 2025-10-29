use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
};

use async_walkdir::WalkDir;
use chrono::{DateTime, Local, NaiveDate, Utc};
use futures::stream::StreamExt;
use miette::IntoDiagnostic;

use crate::logs::parser::{self, LogLine, LogMessageContent, LogMessageTaggedContent};

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
pub async fn find_jsonl_files(dir: &Path) -> miette::Result<Vec<std::path::PathBuf>> {
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

// Type aliases for message tuples
type DateBasedMessage = (NaiveDate, ModelType, String, TokenCounts);
type SessionBasedMessage = (String, DateTime<Utc>, ModelType, String, TokenCounts);

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
/// - If all messages have zero output_tokens, keeps the first encountered (acceptable)
/// - Assumes in-order message delivery (true for Claude API in practice)
/// - If the API ever sent decreasing token counts, this would fail (not observed)
fn deduplicate_by_request_id<T>(
    messages: Vec<(Option<String>, T)>,
    get_output_tokens: impl Fn(&T) -> u64,
) -> Vec<T> {
    let mut result: Vec<T> = Vec::new();
    let mut request_id_to_index: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();

    for (request_id, payload) in messages {
        if let Some(req_id) = request_id {
            // Message has a requestId - part of a streaming group
            if let Some(&existing_idx) = request_id_to_index.get(&req_id) {
                // Keep the message with higher output_tokens (final message)
                if get_output_tokens(&payload) > get_output_tokens(&result[existing_idx]) {
                    result[existing_idx] = payload;
                }
            } else {
                // First occurrence of this requestId
                request_id_to_index.insert(req_id, result.len());
                result.push(payload);
            }
        } else {
            // No requestId - non-streaming message, keep as-is
            result.push(payload);
        }
    }

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
pub async fn parse_log_file(
    file: &Path,
    timezone: DateTimezone,
) -> miette::Result<Vec<(NaiveDate, ModelType, String, TokenCounts)>> {
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
                (date, model_type, model_string, counts),
            ));
        }
    }

    // Deduplicate streaming messages by requestId
    let usages = deduplicate_by_request_id(
        all_messages,
        |tuple: &(NaiveDate, ModelType, String, TokenCounts)| tuple.3.output_tokens as u64,
    );

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
pub async fn parse_log_file_by_session(
    file: &Path,
) -> miette::Result<Vec<(String, DateTime<Utc>, ModelType, String, TokenCounts)>> {
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
                (session_id, timestamp, model_type, model_string, counts),
            ));
        }
    }

    // Deduplicate streaming messages by requestId
    let usages = deduplicate_by_request_id(
        all_messages,
        |tuple: &(String, DateTime<Utc>, ModelType, String, TokenCounts)| {
            tuple.4.output_tokens as u64
        },
    );

    Ok(usages)
}

/// Extract lines changed from assistant messages in a log file
///
/// Filters out entries with model `<synthetic>` (case-insensitive), as these represent
/// internal processing steps rather than billable API usage.
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

/// Extract lines changed from assistant messages in a log file by session
///
/// Filters out entries with model `<synthetic>` (case-insensitive), as these represent
/// internal processing steps rather than billable API usage.
pub async fn parse_lines_changed_by_session(
    file: &Path,
) -> miette::Result<Vec<(String, DateTime<Utc>, usize)>> {
    let log_lines = parser::read_file(file).await?;
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

/// Aggregate usage data by date
pub fn aggregate_by_date(
    usages: Vec<(NaiveDate, ModelType, String, TokenCounts)>,
    lines_changed: Vec<(NaiveDate, usize)>,
    unknown_models: &mut HashSet<String>,
    total_unknown_tokens: &mut TokenCounts,
) -> BTreeMap<NaiveDate, DailyUsage> {
    let mut daily_usage: BTreeMap<NaiveDate, DailyUsage> = BTreeMap::new();

    // Aggregate token usage
    for (date, model_type, model_string, counts) in usages {
        // Track unknown models
        if model_type == ModelType::Unknown {
            unknown_models.insert(model_string);
            total_unknown_tokens.add(&counts);
        }

        daily_usage
            .entry(date)
            .or_insert_with(|| DailyUsage::new(date))
            .add_usage(model_type, counts);
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
    usages: Vec<(String, DateTime<Utc>, ModelType, String, TokenCounts)>,
    lines_changed: Vec<(String, DateTime<Utc>, usize)>,
    unknown_models: &mut HashSet<String>,
    total_unknown_tokens: &mut TokenCounts,
) -> BTreeMap<String, SessionUsage> {
    let mut session_usage: BTreeMap<String, SessionUsage> = BTreeMap::new();

    // Aggregate token usage
    for (session_id, timestamp, model_type, model_string, counts) in usages {
        // Track unknown models
        if model_type == ModelType::Unknown {
            unknown_models.insert(model_string);
            total_unknown_tokens.add(&counts);
        }

        session_usage
            .entry(session_id.clone())
            .or_insert_with(|| SessionUsage::new(session_id, timestamp))
            .add_usage(model_type, counts, timestamp);
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

/// Analyze all log files in a directory and return daily costs
pub async fn analyze_directory(
    dir: &Path,
    timezone: DateTimezone,
) -> miette::Result<AnalysisResult> {
    let jsonl_files = find_jsonl_files(dir).await?;

    if jsonl_files.is_empty() {
        eprintln!("Warning: No .jsonl files found in directory");
        return Ok(AnalysisResult::default());
    }

    println!("Found {} log files to analyze", jsonl_files.len());

    let mut all_usages = Vec::new();
    let mut all_lines_changed = Vec::new();
    let mut files_parsed = 0;
    let mut files_failed = 0;

    for file in &jsonl_files {
        match parse_log_file(file, timezone).await {
            Ok(usages) => {
                all_usages.extend(usages);
                files_parsed += 1;
            }
            Err(e) => {
                eprintln!("Warning: Failed to parse {:?}: {}", file, e);
                files_failed += 1;
            }
        }

        // Parse lines changed (don't fail if this errors)
        match parse_lines_changed(file, timezone).await {
            Ok(lines) => all_lines_changed.extend(lines),
            Err(e) => {
                eprintln!(
                    "Warning: Failed to parse lines changed from {:?}: {}",
                    file, e
                );
            }
        }
    }

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
        eprintln!("Warning: No .jsonl files found in directory");
        return Ok(SessionAnalysisResult::default());
    }

    println!("Found {} log files to analyze", jsonl_files.len());

    let mut all_usages = Vec::new();
    let mut all_lines_changed = Vec::new();
    let mut files_parsed = 0;
    let mut files_failed = 0;

    for file in &jsonl_files {
        match parse_log_file_by_session(file).await {
            Ok(usages) => {
                all_usages.extend(usages);
                files_parsed += 1;
            }
            Err(e) => {
                eprintln!("Warning: Failed to parse {:?}: {}", file, e);
                files_failed += 1;
            }
        }

        // Parse lines changed (don't fail if this errors)
        match parse_lines_changed_by_session(file).await {
            Ok(lines) => all_lines_changed.extend(lines),
            Err(e) => {
                eprintln!(
                    "Warning: Failed to parse lines changed from {:?}: {}",
                    file, e
                );
            }
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn test_daily_usage_new() {
        let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
        let usage = DailyUsage::new(date);

        assert_eq!(usage.date, date);
        assert_eq!(usage.sonnet_usage.input_tokens, 0);
        assert_eq!(usage.haiku_usage.input_tokens, 0);
        assert_eq!(usage.opus_usage.input_tokens, 0);
        assert_eq!(usage.unknown_usage.input_tokens, 0);
        assert_eq!(usage.lines_changed, 0);
    }

    #[test]
    fn test_daily_usage_add_sonnet() {
        let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
        let mut usage = DailyUsage::new(date);

        let counts = TokenCounts {
            input_tokens: 1000,
            output_tokens: 500,
            cache_write_tokens: 100,
            cache_read_tokens: 50,
        };

        usage.add_usage(ModelType::Sonnet, counts);

        assert_eq!(usage.sonnet_usage.input_tokens, 1000);
        assert_eq!(usage.sonnet_usage.output_tokens, 500);
        assert_eq!(usage.haiku_usage.input_tokens, 0);
        assert_eq!(usage.unknown_usage.input_tokens, 0);
    }

    #[test]
    fn test_daily_usage_add_opus() {
        let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
        let mut usage = DailyUsage::new(date);

        let counts = TokenCounts {
            input_tokens: 1000,
            output_tokens: 500,
            cache_write_tokens: 100,
            cache_read_tokens: 50,
        };

        usage.add_usage(ModelType::Opus, counts);

        assert_eq!(usage.opus_usage.input_tokens, 1000);
        assert_eq!(usage.opus_usage.output_tokens, 500);
        assert_eq!(usage.sonnet_usage.input_tokens, 0);
        assert_eq!(usage.haiku_usage.input_tokens, 0);
    }

    #[test]
    fn test_daily_usage_add_multiple_models() {
        let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
        let mut usage = DailyUsage::new(date);

        usage.add_usage(
            ModelType::Sonnet,
            TokenCounts {
                input_tokens: 1000,
                output_tokens: 500,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
            },
        );

        usage.add_usage(
            ModelType::Haiku,
            TokenCounts {
                input_tokens: 2000,
                output_tokens: 1000,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
            },
        );

        assert_eq!(usage.sonnet_usage.input_tokens, 1000);
        assert_eq!(usage.haiku_usage.input_tokens, 2000);
    }

    #[test]
    fn test_daily_usage_add_accumulates() {
        let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
        let mut usage = DailyUsage::new(date);

        usage.add_usage(
            ModelType::Sonnet,
            TokenCounts {
                input_tokens: 1000,
                output_tokens: 500,
                cache_write_tokens: 100,
                cache_read_tokens: 50,
            },
        );

        usage.add_usage(
            ModelType::Sonnet,
            TokenCounts {
                input_tokens: 500,
                output_tokens: 250,
                cache_write_tokens: 50,
                cache_read_tokens: 25,
            },
        );

        assert_eq!(usage.sonnet_usage.input_tokens, 1500);
        assert_eq!(usage.sonnet_usage.output_tokens, 750);
        assert_eq!(usage.sonnet_usage.cache_write_tokens, 150);
        assert_eq!(usage.sonnet_usage.cache_read_tokens, 75);
    }

    #[test]
    fn test_daily_usage_calculate_costs() {
        let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
        let mut usage = DailyUsage::new(date);

        usage.add_usage(
            ModelType::Sonnet,
            TokenCounts {
                input_tokens: 1_000_000,
                output_tokens: 1_000_000,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
            },
        );

        let costs = usage.calculate_costs();

        assert_eq!(costs.date, date);
        assert_eq!(costs.sonnet_costs.input, 3.0);
        assert_eq!(costs.sonnet_costs.output, 15.0);
        assert_eq!(costs.lines_changed, 0);
    }

    #[test]
    fn test_daily_costs_total() {
        let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
        let costs = DailyCosts {
            date,
            sonnet_costs: TokenCosts {
                input: 1.0,
                output: 2.0,
                cache_write: 0.5,
                cache_read: 0.25,
            },
            haiku_costs: TokenCosts {
                input: 0.5,
                output: 1.0,
                cache_write: 0.25,
                cache_read: 0.1,
            },
            opus_costs: TokenCosts {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
            lines_changed: 0,
        };

        assert!((costs.total() - 5.6).abs() < 1e-10);
    }

    #[test]
    fn test_aggregate_by_date_empty() {
        let usages = Vec::new();
        let mut unknown_models = HashSet::new();
        let mut total_unknown_tokens = TokenCounts::default();

        let result = aggregate_by_date(
            usages,
            Vec::new(),
            &mut unknown_models,
            &mut total_unknown_tokens,
        );

        assert!(result.is_empty());
        assert!(unknown_models.is_empty());
    }

    #[test]
    fn test_aggregate_by_date_single_entry() {
        let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
        let counts = TokenCounts {
            input_tokens: 1000,
            output_tokens: 500,
            cache_write_tokens: 0,
            cache_read_tokens: 0,
        };

        let usages = vec![(
            date,
            ModelType::Sonnet,
            "claude-sonnet-4".to_string(),
            counts,
        )];
        let mut unknown_models = HashSet::new();
        let mut total_unknown_tokens = TokenCounts::default();

        let result = aggregate_by_date(
            usages,
            Vec::new(),
            &mut unknown_models,
            &mut total_unknown_tokens,
        );

        assert_eq!(result.len(), 1);
        assert!(result.contains_key(&date));
        assert_eq!(result[&date].sonnet_usage.input_tokens, 1000);
    }

    #[test]
    fn test_aggregate_by_date_multiple_dates() {
        let date1 = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
        let date2 = NaiveDate::from_ymd_opt(2025, 10, 24).unwrap();

        let usages = vec![
            (
                date1,
                ModelType::Sonnet,
                "claude-sonnet-4".to_string(),
                TokenCounts {
                    input_tokens: 1000,
                    output_tokens: 500,
                    cache_write_tokens: 0,
                    cache_read_tokens: 0,
                },
            ),
            (
                date2,
                ModelType::Haiku,
                "claude-haiku-3".to_string(),
                TokenCounts {
                    input_tokens: 2000,
                    output_tokens: 1000,
                    cache_write_tokens: 0,
                    cache_read_tokens: 0,
                },
            ),
        ];
        let mut unknown_models = HashSet::new();
        let mut total_unknown_tokens = TokenCounts::default();

        let result = aggregate_by_date(
            usages,
            Vec::new(),
            &mut unknown_models,
            &mut total_unknown_tokens,
        );

        assert_eq!(result.len(), 2);
        assert_eq!(result[&date1].sonnet_usage.input_tokens, 1000);
        assert_eq!(result[&date2].haiku_usage.input_tokens, 2000);
    }

    #[test]
    fn test_aggregate_by_date_same_date_accumulates() {
        let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();

        let usages = vec![
            (
                date,
                ModelType::Sonnet,
                "claude-sonnet-4".to_string(),
                TokenCounts {
                    input_tokens: 1000,
                    output_tokens: 500,
                    cache_write_tokens: 0,
                    cache_read_tokens: 0,
                },
            ),
            (
                date,
                ModelType::Sonnet,
                "claude-sonnet-4".to_string(),
                TokenCounts {
                    input_tokens: 500,
                    output_tokens: 250,
                    cache_write_tokens: 0,
                    cache_read_tokens: 0,
                },
            ),
        ];
        let mut unknown_models = HashSet::new();
        let mut total_unknown_tokens = TokenCounts::default();

        let result = aggregate_by_date(
            usages,
            Vec::new(),
            &mut unknown_models,
            &mut total_unknown_tokens,
        );

        assert_eq!(result.len(), 1);
        assert_eq!(result[&date].sonnet_usage.input_tokens, 1500);
        assert_eq!(result[&date].sonnet_usage.output_tokens, 750);
    }

    #[test]
    fn test_aggregate_by_date_tracks_unknown_models() {
        let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();

        let usages = vec![
            (
                date,
                ModelType::Unknown,
                "claude-opus-4".to_string(),
                TokenCounts {
                    input_tokens: 1000,
                    output_tokens: 500,
                    cache_write_tokens: 100,
                    cache_read_tokens: 50,
                },
            ),
            (
                date,
                ModelType::Unknown,
                "gpt-4".to_string(),
                TokenCounts {
                    input_tokens: 500,
                    output_tokens: 250,
                    cache_write_tokens: 0,
                    cache_read_tokens: 0,
                },
            ),
        ];
        let mut unknown_models = HashSet::new();
        let mut total_unknown_tokens = TokenCounts::default();

        let result = aggregate_by_date(
            usages,
            Vec::new(),
            &mut unknown_models,
            &mut total_unknown_tokens,
        );

        assert_eq!(unknown_models.len(), 2);
        assert!(unknown_models.contains("claude-opus-4"));
        assert!(unknown_models.contains("gpt-4"));
        assert_eq!(total_unknown_tokens.input_tokens, 1500);
        assert_eq!(total_unknown_tokens.output_tokens, 750);
        assert_eq!(total_unknown_tokens.cache_write_tokens, 100);
        assert_eq!(total_unknown_tokens.cache_read_tokens, 50);
        assert_eq!(result[&date].unknown_usage.input_tokens, 1500);
    }

    #[test]
    fn test_aggregate_by_date_sorted_by_date() {
        let date1 = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
        let date2 = NaiveDate::from_ymd_opt(2025, 10, 21).unwrap();
        let date3 = NaiveDate::from_ymd_opt(2025, 10, 25).unwrap();

        let usages = vec![
            (
                date1,
                ModelType::Sonnet,
                "claude-sonnet-4".to_string(),
                TokenCounts::default(),
            ),
            (
                date2,
                ModelType::Sonnet,
                "claude-sonnet-4".to_string(),
                TokenCounts::default(),
            ),
            (
                date3,
                ModelType::Sonnet,
                "claude-sonnet-4".to_string(),
                TokenCounts::default(),
            ),
        ];
        let mut unknown_models = HashSet::new();
        let mut total_unknown_tokens = TokenCounts::default();

        let result = aggregate_by_date(
            usages,
            Vec::new(),
            &mut unknown_models,
            &mut total_unknown_tokens,
        );
        let dates: Vec<_> = result.keys().collect();

        assert_eq!(dates, vec![&date2, &date1, &date3]);
    }

    #[tokio::test]
    async fn test_find_jsonl_files_empty_directory() {
        let temp_dir = tempfile::tempdir().unwrap();
        let result = find_jsonl_files(temp_dir.path()).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_find_jsonl_files_no_jsonl_files() {
        let temp_dir = tempfile::tempdir().unwrap();
        tokio::fs::write(temp_dir.path().join("test.txt"), "content")
            .await
            .unwrap();
        tokio::fs::write(temp_dir.path().join("test.json"), "{}")
            .await
            .unwrap();

        let result = find_jsonl_files(temp_dir.path()).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_find_jsonl_files_single_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        tokio::fs::write(temp_dir.path().join("test.jsonl"), "")
            .await
            .unwrap();

        let result = find_jsonl_files(temp_dir.path()).await.unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].ends_with("test.jsonl"));
    }

    #[tokio::test]
    async fn test_find_jsonl_files_recursive() {
        let temp_dir = tempfile::tempdir().unwrap();
        let subdir = temp_dir.path().join("subdir");
        tokio::fs::create_dir(&subdir).await.unwrap();

        tokio::fs::write(temp_dir.path().join("root.jsonl"), "")
            .await
            .unwrap();
        tokio::fs::write(subdir.join("nested.jsonl"), "")
            .await
            .unwrap();

        let result = find_jsonl_files(temp_dir.path()).await.unwrap();
        assert_eq!(result.len(), 2);
    }

    #[tokio::test]
    async fn test_find_jsonl_files_deep_nesting() {
        let temp_dir = tempfile::tempdir().unwrap();
        let deep_path = temp_dir.path().join("a").join("b").join("c");
        tokio::fs::create_dir_all(&deep_path).await.unwrap();

        tokio::fs::write(deep_path.join("deep.jsonl"), "")
            .await
            .unwrap();

        let result = find_jsonl_files(temp_dir.path()).await.unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].ends_with("deep.jsonl"));
    }

    #[tokio::test]
    async fn test_parse_log_file_empty_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("empty.jsonl");
        tokio::fs::write(&file_path, "").await.unwrap();

        let result = parse_log_file(&file_path, DateTimezone::Utc).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_parse_log_file_extracts_usage_correctly() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.jsonl");

        let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4-20250514","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":100,"cache_read_input_tokens":50,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
        tokio::fs::write(&file_path, log_content).await.unwrap();

        let result = parse_log_file(&file_path, DateTimezone::Utc).await.unwrap();

        assert_eq!(result.len(), 1);
        let (date, model_type, model_string, counts) = &result[0];

        assert_eq!(date, &NaiveDate::from_ymd_opt(2025, 10, 23).unwrap());
        assert_eq!(model_type, &ModelType::Sonnet);
        assert_eq!(model_string, "claude-sonnet-4-20250514");
        assert_eq!(counts.input_tokens, 1000);
        assert_eq!(counts.output_tokens, 500);
        assert_eq!(counts.cache_write_tokens, 100);
        assert_eq!(counts.cache_read_tokens, 50);
    }

    #[tokio::test]
    async fn test_parse_log_file_handles_multiple_assistant_messages() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.jsonl");

        let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-haiku-3","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":1000,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-24T12:00:00Z","isApiErrorMessage":null}"#;
        tokio::fs::write(&file_path, log_content).await.unwrap();

        let result = parse_log_file(&file_path, DateTimezone::Utc).await.unwrap();

        assert_eq!(result.len(), 2);

        assert_eq!(result[0].1, ModelType::Sonnet);
        assert_eq!(result[0].3.input_tokens, 1000);

        assert_eq!(result[1].1, ModelType::Haiku);
        assert_eq!(result[1].3.input_tokens, 2000);
    }

    #[tokio::test]
    async fn test_parse_log_file_ignores_non_assistant_messages() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.jsonl");

        let log_content = r#"{"type":"user","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"role":"user","content":"test"},"isMeta":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T11:00:00Z","toolUseResult":null,"thinkingMetadata":null,"isVisibleInTranscriptOnly":null,"isCompactSummary":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
        tokio::fs::write(&file_path, log_content).await.unwrap();

        let result = parse_log_file(&file_path, DateTimezone::Utc).await.unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, ModelType::Sonnet);
    }

    #[tokio::test]
    async fn test_analyze_directory_empty() {
        let temp_dir = tempfile::tempdir().unwrap();
        let result = analyze_directory(temp_dir.path(), DateTimezone::Utc)
            .await
            .unwrap();

        assert!(result.daily_costs.is_empty());
        assert_eq!(result.files_parsed, 0);
        assert_eq!(result.files_failed, 0);
    }

    #[tokio::test]
    async fn test_analyze_directory_with_invalid_jsonl() {
        let temp_dir = tempfile::tempdir().unwrap();
        tokio::fs::write(temp_dir.path().join("invalid.jsonl"), "not json")
            .await
            .unwrap();

        let result = analyze_directory(temp_dir.path(), DateTimezone::Utc)
            .await
            .unwrap();
        assert!(result.daily_costs.is_empty());
        assert_eq!(result.files_parsed, 0);
        assert_eq!(result.files_failed, 1);
    }

    #[test]
    fn test_daily_usage_add_lines_changed() {
        let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
        let mut usage = DailyUsage::new(date);

        usage.add_lines_changed(100);
        assert_eq!(usage.lines_changed, 100);

        usage.add_lines_changed(50);
        assert_eq!(usage.lines_changed, 150);
    }

    #[test]
    fn test_aggregate_by_date_with_lines_changed() {
        let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();

        let usages = vec![(
            date,
            ModelType::Sonnet,
            "claude-sonnet-4".to_string(),
            TokenCounts::default(),
        )];

        let lines_changed = vec![(date, 100), (date, 50)];

        let mut unknown_models = HashSet::new();
        let mut total_unknown_tokens = TokenCounts::default();

        let result = aggregate_by_date(
            usages,
            lines_changed,
            &mut unknown_models,
            &mut total_unknown_tokens,
        );

        assert_eq!(result[&date].lines_changed, 150);
    }

    #[test]
    fn test_aggregate_by_date_lines_changed_different_dates() {
        let date1 = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();
        let date2 = NaiveDate::from_ymd_opt(2025, 10, 24).unwrap();

        let lines_changed = vec![(date1, 100), (date2, 200), (date1, 50)];

        let mut unknown_models = HashSet::new();
        let mut total_unknown_tokens = TokenCounts::default();

        let result = aggregate_by_date(
            Vec::new(),
            lines_changed,
            &mut unknown_models,
            &mut total_unknown_tokens,
        );

        assert_eq!(result[&date1].lines_changed, 150);
        assert_eq!(result[&date2].lines_changed, 200);
    }

    #[tokio::test]
    async fn test_parse_lines_changed_with_edit_tool() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.jsonl");

        let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":[{"type":"tool_use","id":"tool_1","name":"Edit","input":{"file_path":"/test.rs","old_string":"line1\nline2","new_string":"line1\nmodified\nline3"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
        tokio::fs::write(&file_path, log_content).await.unwrap();

        let result = parse_lines_changed(&file_path, DateTimezone::Utc)
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, NaiveDate::from_ymd_opt(2025, 10, 23).unwrap());
        assert_eq!(result[0].1, 3);
    }

    #[tokio::test]
    async fn test_parse_lines_changed_with_write_tool() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.jsonl");

        let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":[{"type":"tool_use","id":"tool_1","name":"Write","input":{"file_path":"/test.rs","content":"line1\nline2\nline3\nline4"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
        tokio::fs::write(&file_path, log_content).await.unwrap();

        let result = parse_lines_changed(&file_path, DateTimezone::Utc)
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, NaiveDate::from_ymd_opt(2025, 10, 23).unwrap());
        assert_eq!(result[0].1, 4);
    }

    #[tokio::test]
    async fn test_parse_lines_changed_with_notebook_edit_tool() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.jsonl");

        let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":[{"type":"tool_use","id":"tool_1","name":"NotebookEdit","input":{"notebook_path":"/test.ipynb","new_source":"print('hello')\nprint('world')"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
        tokio::fs::write(&file_path, log_content).await.unwrap();

        let result = parse_lines_changed(&file_path, DateTimezone::Utc)
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, NaiveDate::from_ymd_opt(2025, 10, 23).unwrap());
        assert_eq!(result[0].1, 2);
    }

    #[tokio::test]
    async fn test_parse_lines_changed_multiple_tools_same_message() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.jsonl");

        let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":[{"type":"tool_use","id":"tool_1","name":"Edit","input":{"file_path":"/test.rs","old_string":"old","new_string":"new"}},{"type":"tool_use","id":"tool_2","name":"Write","input":{"file_path":"/test2.rs","content":"line1\nline2"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
        tokio::fs::write(&file_path, log_content).await.unwrap();

        let result = parse_lines_changed(&file_path, DateTimezone::Utc)
            .await
            .unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, NaiveDate::from_ymd_opt(2025, 10, 23).unwrap());
        assert_eq!(result[1].0, NaiveDate::from_ymd_opt(2025, 10, 23).unwrap());
    }

    #[tokio::test]
    async fn test_parse_lines_changed_empty_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("empty.jsonl");
        tokio::fs::write(&file_path, "").await.unwrap();

        let result = parse_lines_changed(&file_path, DateTimezone::Utc)
            .await
            .unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_parse_lines_changed_no_tool_uses() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.jsonl");

        let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"just text, no tools","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
        tokio::fs::write(&file_path, log_content).await.unwrap();

        let result = parse_lines_changed(&file_path, DateTimezone::Utc)
            .await
            .unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_parse_lines_changed_ignores_non_modifying_tools() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.jsonl");

        let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":[{"type":"tool_use","id":"tool_1","name":"Read","input":{"file_path":"/test.rs"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
        tokio::fs::write(&file_path, log_content).await.unwrap();

        let result = parse_lines_changed(&file_path, DateTimezone::Utc)
            .await
            .unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_parse_log_file_filters_synthetic_model() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.jsonl");

        let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"<synthetic>","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2000,"cache_creation_input_tokens":100,"cache_read_input_tokens":50,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":1000,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
        tokio::fs::write(&file_path, log_content).await.unwrap();

        let result = parse_log_file(&file_path, DateTimezone::Utc).await.unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].2, "claude-sonnet-4");
        assert_eq!(result[0].3.input_tokens, 2000);
    }

    #[tokio::test]
    async fn test_parse_log_file_filters_synthetic_model_case_insensitive() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.jsonl");

        let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"<SYNTHETIC>","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"<Synthetic>","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1500,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":750,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
        tokio::fs::write(&file_path, log_content).await.unwrap();

        let result = parse_log_file(&file_path, DateTimezone::Utc).await.unwrap();

        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_parse_lines_changed_filters_synthetic_model() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.jsonl");

        let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"<synthetic>","container":null,"content":[{"type":"tool_use","id":"tool_1","name":"Edit","input":{"file_path":"/test.rs","old_string":"old","new_string":"new\ncode"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":[{"type":"tool_use","id":"tool_2","name":"Edit","input":{"file_path":"/test2.rs","old_string":"a","new_string":"b\nc"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":1000,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
        tokio::fs::write(&file_path, log_content).await.unwrap();

        let result = parse_lines_changed(&file_path, DateTimezone::Utc)
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, NaiveDate::from_ymd_opt(2025, 10, 23).unwrap());
        assert_eq!(result[0].1, 3);
    }

    #[tokio::test]
    async fn test_parse_lines_changed_filters_synthetic_model_case_insensitive() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.jsonl");

        let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"<SYNTHETIC>","container":null,"content":[{"type":"tool_use","id":"tool_1","name":"Edit","input":{"file_path":"/test.rs","old_string":"old","new_string":"new"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"<Synthetic>","container":null,"content":[{"type":"tool_use","id":"tool_2","name":"Write","input":{"file_path":"/test2.rs","content":"line1\nline2"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":1000,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
        tokio::fs::write(&file_path, log_content).await.unwrap();

        let result = parse_lines_changed(&file_path, DateTimezone::Utc)
            .await
            .unwrap();

        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_parse_log_file_all_synthetic_entries() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.jsonl");

        let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"<synthetic>","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"<synthetic>","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":1000,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
        tokio::fs::write(&file_path, log_content).await.unwrap();

        let result = parse_log_file(&file_path, DateTimezone::Utc).await.unwrap();

        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_parse_lines_changed_all_synthetic_entries() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.jsonl");

        let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"<synthetic>","container":null,"content":[{"type":"tool_use","id":"tool_1","name":"Edit","input":{"file_path":"/test.rs","old_string":"old","new_string":"new"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"<synthetic>","container":null,"content":[{"type":"tool_use","id":"tool_2","name":"Write","input":{"file_path":"/test2.rs","content":"test"}}],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":1000,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
        tokio::fs::write(&file_path, log_content).await.unwrap();

        let result = parse_lines_changed(&file_path, DateTimezone::Utc)
            .await
            .unwrap();

        assert!(result.is_empty());
    }

    // ============================================================================
    // Tests for streaming message deduplication
    // ============================================================================
    // Streaming API responses send multiple partial messages with the same requestId.
    // These tests verify that we correctly deduplicate by keeping only the message
    // with the highest output_tokens (the final, complete message).

    #[tokio::test]
    async fn test_parse_log_file_deduplicates_streaming_messages_by_request_id() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.jsonl");

        let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"partial","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":8,"cache_creation_input_tokens":17932,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":2,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"partial","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":8,"cache_creation_input_tokens":17932,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":2,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:01Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_3","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"partial","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":8,"cache_creation_input_tokens":17932,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":2,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000003","timestamp":"2025-10-23T12:00:02Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_4","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"final complete response","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":8,"cache_creation_input_tokens":17932,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":835,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000004","timestamp":"2025-10-23T12:00:03Z","isApiErrorMessage":null}"#;
        tokio::fs::write(&file_path, log_content).await.unwrap();

        let result = parse_log_file(&file_path, DateTimezone::Utc).await.unwrap();

        assert_eq!(
            result.len(),
            1,
            "Should only count the final message in streaming group, but got {} messages",
            result.len()
        );

        let (_date, model_type, _model_string, counts) = &result[0];
        assert_eq!(model_type, &ModelType::Sonnet);
        assert_eq!(counts.input_tokens, 8);
        assert_eq!(counts.output_tokens, 835, "Should have final output count");
        assert_eq!(
            counts.cache_write_tokens, 17932,
            "Should count cache_write only once"
        );
    }

    #[tokio::test]
    async fn test_parse_log_file_by_session_deduplicates_streaming_messages() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.jsonl");

        // Same test data as above
        let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"partial","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":8,"cache_creation_input_tokens":17932,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":2,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"partial","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":8,"cache_creation_input_tokens":17932,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":2,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:01Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_3","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"partial","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":8,"cache_creation_input_tokens":17932,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":2,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000003","timestamp":"2025-10-23T12:00:02Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_4","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"final","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":8,"cache_creation_input_tokens":17932,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":835,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000004","timestamp":"2025-10-23T12:00:03Z","isApiErrorMessage":null}"#;
        tokio::fs::write(&file_path, log_content).await.unwrap();

        let result = parse_log_file_by_session(&file_path).await.unwrap();

        assert_eq!(
            result.len(),
            1,
            "Should only count the final message, but got {} messages",
            result.len()
        );

        let (session_id, _timestamp, _model_type, _model_string, counts) = &result[0];
        assert_eq!(session_id, "session-1");
        assert_eq!(counts.output_tokens, 835);
        assert_eq!(counts.cache_write_tokens, 17932);
    }

    #[tokio::test]
    async fn test_parse_log_file_handles_multiple_streaming_groups() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.jsonl");

        let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":8,"cache_creation_input_tokens":17932,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":2,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":8,"cache_creation_input_tokens":17932,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":2,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:01Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_3","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":8,"cache_creation_input_tokens":17932,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":835,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000003","timestamp":"2025-10-23T12:00:02Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_4","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":12,"cache_creation_input_tokens":5361,"cache_read_input_tokens":17932,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":2,"service_tier":null,"server_tool_use":null}},"requestId":"req-456","uuid":"00000000-0000-0000-0000-000000000004","timestamp":"2025-10-23T12:01:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_5","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":12,"cache_creation_input_tokens":5361,"cache_read_input_tokens":17932,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":867,"service_tier":null,"server_tool_use":null}},"requestId":"req-456","uuid":"00000000-0000-0000-0000-000000000005","timestamp":"2025-10-23T12:01:01Z","isApiErrorMessage":null}"#;
        tokio::fs::write(&file_path, log_content).await.unwrap();

        let result = parse_log_file(&file_path, DateTimezone::Utc).await.unwrap();

        assert_eq!(
            result.len(),
            2,
            "Should have 2 entries (one per request), but got {}",
            result.len()
        );

        // First request: final message has output_tokens=835
        assert_eq!(result[0].3.output_tokens, 835);
        assert_eq!(result[0].3.cache_write_tokens, 17932);

        // Second request: final message has output_tokens=867
        assert_eq!(result[1].3.output_tokens, 867);
        assert_eq!(result[1].3.cache_write_tokens, 5361);
        assert_eq!(result[1].3.cache_read_tokens, 17932);
    }

    #[tokio::test]
    async fn test_parse_log_file_preserves_non_streaming_messages() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.jsonl");

        // Message without requestId should be preserved
        let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"session-1","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"test","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":500,"service_tier":null,"server_tool_use":null}},"requestId":null,"uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}"#;
        tokio::fs::write(&file_path, log_content).await.unwrap();

        let result = parse_log_file(&file_path, DateTimezone::Utc).await.unwrap();

        // Messages without requestId should be kept as-is
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].3.input_tokens, 1000);
        assert_eq!(result[0].3.output_tokens, 500);
    }

    #[test]
    fn test_aggregate_by_date_with_deduplicated_streaming_data() {
        // This test shows the CORRECT cost when using deduplicated data
        let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();

        // Only the final message from a 4-message streaming group
        let usages = vec![(
            date,
            ModelType::Sonnet,
            "claude-sonnet-4".to_string(),
            TokenCounts {
                input_tokens: 8,
                output_tokens: 835,
                cache_write_tokens: 17932,
                cache_read_tokens: 0,
            },
        )];

        let mut unknown_models = HashSet::new();
        let mut total_unknown_tokens = TokenCounts::default();

        let result = aggregate_by_date(
            usages,
            Vec::new(),
            &mut unknown_models,
            &mut total_unknown_tokens,
        );

        assert_eq!(result.len(), 1);
        let daily_usage = &result[&date];
        assert_eq!(daily_usage.sonnet_usage.cache_write_tokens, 17932);

        // Calculate cost (Sonnet pricing: cache_write = $3.75 per million)
        // 17932 tokens * $3.75 / 1M = $0.06725
        let daily_costs = daily_usage.calculate_costs();
        // This should be around $0.12 (input + output + cache_write)
        assert!(
            daily_costs.sonnet_costs.cache_write > 0.06
                && daily_costs.sonnet_costs.cache_write < 0.07,
            "Cache write cost should be ~$0.067, got ${}",
            daily_costs.sonnet_costs.cache_write
        );
    }

    #[test]
    fn test_aggregate_by_date_with_buggy_non_deduplicated_data() {
        // This test shows the INCORRECT cost when NOT deduplicating (current bug)
        let date = NaiveDate::from_ymd_opt(2025, 10, 23).unwrap();

        // All 4 messages from streaming group (buggy behavior)
        let usages = vec![
            (
                date,
                ModelType::Sonnet,
                "claude-sonnet-4".to_string(),
                TokenCounts {
                    input_tokens: 8,
                    output_tokens: 2,
                    cache_write_tokens: 17932,
                    cache_read_tokens: 0,
                },
            ),
            (
                date,
                ModelType::Sonnet,
                "claude-sonnet-4".to_string(),
                TokenCounts {
                    input_tokens: 8,
                    output_tokens: 2,
                    cache_write_tokens: 17932,
                    cache_read_tokens: 0,
                },
            ),
            (
                date,
                ModelType::Sonnet,
                "claude-sonnet-4".to_string(),
                TokenCounts {
                    input_tokens: 8,
                    output_tokens: 2,
                    cache_write_tokens: 17932,
                    cache_read_tokens: 0,
                },
            ),
            (
                date,
                ModelType::Sonnet,
                "claude-sonnet-4".to_string(),
                TokenCounts {
                    input_tokens: 8,
                    output_tokens: 835,
                    cache_write_tokens: 17932,
                    cache_read_tokens: 0,
                },
            ),
        ];

        let mut unknown_models = HashSet::new();
        let mut total_unknown_tokens = TokenCounts::default();

        let result = aggregate_by_date(
            usages,
            Vec::new(),
            &mut unknown_models,
            &mut total_unknown_tokens,
        );

        assert_eq!(result.len(), 1);
        let daily_usage = &result[&date];

        // BUG: cache_write counted 4 times!
        assert_eq!(
            daily_usage.sonnet_usage.cache_write_tokens,
            17932 * 4,
            "Bug: cache_write counted 4 times instead of once"
        );

        let daily_costs = daily_usage.calculate_costs();
        // With 4x counting: 71728 * $3.75 / 1M = $0.269
        assert!(
            daily_costs.sonnet_costs.cache_write > 0.26,
            "Buggy behavior: cache_write should be inflated to ~$0.27, got ${}",
            daily_costs.sonnet_costs.cache_write
        );
    }

    #[tokio::test]
    async fn test_parse_log_file_handles_zero_output_tokens() {
        // Edge case: all messages in streaming group have zero output_tokens
        // Should keep the first message encountered
        let temp_dir = tempfile::tempdir().unwrap();
        let log_file = temp_dir.path().join("test.jsonl");

        // Create a log with streaming messages all having zero output_tokens
        let log_content = r#"{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"response 1","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":10,"cache_creation_input_tokens":100,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":0,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000001","timestamp":"2025-10-23T12:00:00Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_2","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"response 2","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":10,"cache_creation_input_tokens":100,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":0,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000002","timestamp":"2025-10-23T12:00:01Z","isApiErrorMessage":null}
{"type":"assistant","parentUuid":null,"isSidechain":false,"userType":"user","cwd":"/test","sessionId":"00000000-0000-0000-0000-000000000000","version":"1.0.0","gitBranch":"main","message":{"id":"msg_3","type":"message","role":"assistant","model":"claude-sonnet-4","container":null,"content":"response 3","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":10,"cache_creation_input_tokens":100,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0},"output_tokens":0,"service_tier":null,"server_tool_use":null}},"requestId":"req-123","uuid":"00000000-0000-0000-0000-000000000003","timestamp":"2025-10-23T12:00:02Z","isApiErrorMessage":null}"#;

        tokio::fs::write(&log_file, log_content).await.unwrap();

        let result = parse_log_file(&log_file, DateTimezone::Utc).await.unwrap();

        // Should have exactly 1 message (deduplicated from 3)
        assert_eq!(result.len(), 1);

        let (_, _, _, counts) = &result[0];
        // When all have zero output_tokens, keeps the first encountered
        assert_eq!(counts.output_tokens, 0);
        assert_eq!(counts.input_tokens, 10);
        assert_eq!(counts.cache_write_tokens, 100);
    }
}
