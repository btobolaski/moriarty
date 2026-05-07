use std::{cmp::Ordering, fmt, hash::Hash};

use chrono::{DateTime, Utc};
use miette::{Context, IntoDiagnostic};
use rust_decimal::Decimal;
use serde::de::DeserializeOwned;
use tracing::error;
use uuid::Uuid;

use claude_logs::{
    AssistantLogLine as ClaudeAssistantLogLine, AssistantUsage as ClaudeAssistantUsage,
    LogLine as ClaudeLogLine, SystemLogLine as ClaudeSystemLogLine,
};
use pi_logs::{AssistantMessage, PiLogLine, Provider, RoleMessage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LlmCost {
    pub input: Decimal,
    pub cache_write: Decimal,
    pub cache_read: Decimal,
    pub output: Decimal,
}

impl Ord for LlmCost {
    fn cmp(&self, other: &Self) -> Ordering {
        self.total()
            .cmp(&other.total())
            .then_with(|| self.input.cmp(&other.input))
            .then_with(|| self.cache_write.cmp(&other.cache_write))
            .then_with(|| self.cache_read.cmp(&other.cache_read))
    }
}

impl PartialOrd for LlmCost {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl LlmCost {
    pub fn total(&self) -> Decimal {
        self.input + self.cache_write + self.cache_read + self.output
    }
}

pub trait Identifier: fmt::Debug + Clone + Eq + Ord + Hash + Send + Sync + 'static {}

impl<T> Identifier for T where T: fmt::Debug + Clone + Eq + Ord + Hash + Send + Sync + 'static {}

pub(crate) const LOG_LINE_PARSE_CONTEXT: &str = "failed to parse log line";

pub(crate) fn parse_json_line<T: DeserializeOwned>(
    value: &str,
    context: &'static str,
) -> miette::Result<T> {
    serde_json::from_str(value)
        .into_diagnostic()
        .context(context)
}

fn parse_json_backed_log<T: DeserializeOwned>(value: &str) -> miette::Result<T> {
    parse_json_line(value, LOG_LINE_PARSE_CONTEXT)
}

pub trait AnalyzableLog: std::fmt::Debug + Clone + Send + Sync + 'static {
    type LogId: Identifier;
    type ModelId: Identifier;

    /// cost returns Option<LlmCost> because not all entries in the log have a cost i.e. entries from users
    fn cost(&self) -> Option<LlmCost>;
    fn timestamp(&self) -> DateTime<Utc>;
    fn identifier(&self) -> Self::LogId;
    /// model is only set on messages from the LLM so, this returns option
    fn model(&self) -> Option<Self::ModelId>;
    /// Returns a normalized conversation/session identifier when the parsed log
    /// line exposes one.
    fn session_id(&self) -> Option<String>;
    fn parse(value: &str) -> miette::Result<Self>;
}

#[derive(Debug, Clone)]
pub struct LineWithCost<Log>
where
    Log: AnalyzableLog,
{
    pub id: Log::LogId,
    pub model: Log::ModelId,
    pub timestamp: DateTime<Utc>,
    pub session_id: Option<String>,
    pub log: Box<Log>,
    pub cost: LlmCost,
}

impl<Log> LineWithCost<Log>
where
    Log: AnalyzableLog,
{
    pub fn from_log(log: Log, session_id: Option<String>) -> Option<Self> {
        match (log.cost(), log.model()) {
            (Some(cost), Some(model)) => Some(Self {
                id: log.identifier(),
                model,
                cost,
                timestamp: log.timestamp(),
                session_id,
                log: Box::new(log),
            }),
            _ => None,
        }
    }

    pub fn parse(value: &str) -> miette::Result<Option<Self>> {
        let log = Log::parse(value)?;
        let session_id = log.session_id();
        Ok(Self::from_log(log, session_id))
    }
}

/// Builds a fixed-point decimal equal to `mantissa * 10^(-scale)`.
const fn decimal_rate(mantissa: u32, scale: u32) -> Decimal {
    Decimal::from_parts(mantissa, 0, 0, false, scale)
}

#[derive(Debug, Clone, Copy)]
pub struct ClaudeModelPricing {
    /// Price per million input tokens.
    pub input: Decimal,
    /// Price per million output tokens.
    pub output: Decimal,
    /// Price per million prompt cache write tokens.
    pub cache_write: Decimal,
    /// Price per million prompt cache read tokens.
    pub cache_read: Decimal,
}

impl ClaudeModelPricing {
    /// Pricing for Sonnet models (effective as of 2025-10-23).
    /// Applies to `claude-sonnet-*` and `claude-3-*-sonnet-*` model strings.
    pub const SONNET: Self = Self {
        input: decimal_rate(3, 0),
        output: decimal_rate(15, 0),
        cache_write: decimal_rate(375, 2),
        cache_read: decimal_rate(30, 2),
    };

    /// Pricing for Haiku models (effective as of 2025-10-23).
    /// Applies to `claude-haiku-*` and `claude-3-*-haiku-*` model strings.
    pub const HAIKU: Self = Self {
        input: decimal_rate(1, 0),
        output: decimal_rate(5, 0),
        cache_write: decimal_rate(125, 2),
        cache_read: decimal_rate(1, 1),
    };

    /// Pricing for Opus 3 models (effective as of 2025-10-23).
    /// Applies to `claude-3-*-opus-*` model strings.
    pub const OPUS: Self = Self {
        input: decimal_rate(15, 0),
        output: decimal_rate(75, 0),
        cache_write: decimal_rate(1875, 2),
        cache_read: decimal_rate(15, 1),
    };

    /// Pricing for Opus 4 models (effective as of 2025-05-22).
    /// Applies to `claude-opus-4*` model strings.
    pub const OPUS_4: Self = Self {
        input: decimal_rate(5, 0),
        output: decimal_rate(25, 0),
        cache_write: decimal_rate(625, 2),
        cache_read: decimal_rate(50, 2),
    };

    pub fn calculate_cost(&self, usage: &ClaudeTokenCounts) -> LlmCost {
        LlmCost {
            input: per_million_token_cost(usage.input_tokens, self.input),
            cache_write: per_million_token_cost(usage.cache_write_tokens, self.cache_write),
            cache_read: per_million_token_cost(usage.cache_read_tokens, self.cache_read),
            output: per_million_token_cost(usage.output_tokens, self.output),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClaudeModelType {
    Sonnet,
    Haiku,
    Opus,
    Opus4,
    Unknown,
}

impl ClaudeModelType {
    pub fn from_model_string(model: &str) -> Self {
        let model_lower = model.to_lowercase();
        if model_lower.contains("sonnet") {
            Self::Sonnet
        } else if model_lower.contains("haiku") {
            Self::Haiku
        } else if model_lower.contains("opus-4") {
            // Check Opus 4 before the general Opus branch: every Opus 4 model string also
            // contains "opus", so reversing these arms would misclassify Opus 4 as Opus 3.
            Self::Opus4
        } else if model_lower.contains("opus") {
            Self::Opus
        } else {
            Self::Unknown
        }
    }

    fn info(&self) -> ClaudeModelInfo {
        match self {
            Self::Sonnet => ClaudeModelInfo {
                pricing: Some(ClaudeModelPricing::SONNET),
                display_name: "Sonnet",
            },
            Self::Haiku => ClaudeModelInfo {
                pricing: Some(ClaudeModelPricing::HAIKU),
                display_name: "Haiku",
            },
            Self::Opus => ClaudeModelInfo {
                pricing: Some(ClaudeModelPricing::OPUS),
                display_name: "Opus",
            },
            Self::Opus4 => ClaudeModelInfo {
                pricing: Some(ClaudeModelPricing::OPUS_4),
                display_name: "Opus 4",
            },
            Self::Unknown => ClaudeModelInfo {
                pricing: None,
                display_name: "Unknown",
            },
        }
    }

    pub fn pricing(&self) -> Option<ClaudeModelPricing> {
        self.info().pricing
    }
}

impl fmt::Display for ClaudeModelType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.info().display_name)
    }
}

struct ClaudeModelInfo {
    pricing: Option<ClaudeModelPricing>,
    display_name: &'static str,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClaudeTokenCounts {
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub cache_write_tokens: usize,
    pub cache_read_tokens: usize,
}

fn per_million_token_cost(token_count: usize, price_per_million_tokens: Decimal) -> Decimal {
    Decimal::from(token_count) * price_per_million_tokens / Decimal::from(1_000_000_u64)
}

fn claude_assistant_line(line: &ClaudeLogLine) -> Option<&ClaudeAssistantLogLine> {
    match line {
        ClaudeLogLine::Assistant(assistant) => Some(assistant),
        _ => None,
    }
}

fn is_synthetic_claude_model(model: &str) -> bool {
    model.eq_ignore_ascii_case("<synthetic>")
}

fn claude_token_counts_from_usage(usage: &ClaudeAssistantUsage) -> ClaudeTokenCounts {
    ClaudeTokenCounts {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cache_write_tokens: usage.cache_creation_input_tokens,
        cache_read_tokens: usage.cache_read_input_tokens,
    }
}

fn priced_claude_assistant(
    line: &ClaudeLogLine,
) -> Option<(&ClaudeAssistantLogLine, ClaudeModelPricing)> {
    let assistant = claude_assistant_line(line)?;

    if is_synthetic_claude_model(&assistant.message.model) {
        return None;
    }

    let model_type = ClaudeModelType::from_model_string(&assistant.message.model);
    let pricing = match model_type.pricing() {
        Some(pricing) => pricing,
        None => {
            if assistant
                .message
                .model
                .to_ascii_lowercase()
                .starts_with("claude")
            {
                error!(
                    model = %assistant.message.model,
                    "unrecognized Claude model omitted from cost analysis"
                );
            }
            return None;
        }
    };

    Some((assistant, pricing))
}

fn claude_assistant_identifier(assistant: &ClaudeAssistantLogLine) -> String {
    assistant
        .request_id
        .as_ref()
        .filter(|request_id| !request_id.is_empty())
        .cloned()
        .or_else(|| (!assistant.message.id.is_empty()).then(|| assistant.message.id.clone()))
        .unwrap_or_else(|| assistant.uuid.to_string())
}

/// Sentinel timestamp returned by Claude log variants that do not carry a real timestamp.
///
/// `UNIX_EPOCH` is safe here because every variant routed to this helper is non-billable:
/// `cost()` and `model()` both return `None`, so the sentinel never appears in `LineWithCost`.
/// New billable variants must provide a real timestamp instead of using this helper.
fn claude_timestamp_sentinel() -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH
}

struct ClaudeSystemFields {
    timestamp: DateTime<Utc>,
    uuid: Uuid,
}

fn claude_system_fields(system: &ClaudeSystemLogLine) -> ClaudeSystemFields {
    match system {
        ClaudeSystemLogLine::Error(line) | ClaudeSystemLogLine::ApiError(line) => {
            ClaudeSystemFields {
                timestamp: line.timestamp,
                uuid: line.uuid,
            }
        }
        ClaudeSystemLogLine::CompactBoundary(line) => ClaudeSystemFields {
            timestamp: line.timestamp,
            uuid: line.uuid,
        },
        ClaudeSystemLogLine::MicrocompactBoundary(line) => ClaudeSystemFields {
            timestamp: line.timestamp,
            uuid: line.uuid,
        },
        ClaudeSystemLogLine::Informational(line) => ClaudeSystemFields {
            timestamp: line.timestamp,
            uuid: line.uuid,
        },
        ClaudeSystemLogLine::LocalCommand(line) => ClaudeSystemFields {
            timestamp: line.timestamp,
            uuid: line.uuid,
        },
        ClaudeSystemLogLine::StopHookSummary(line) => ClaudeSystemFields {
            timestamp: line.timestamp,
            uuid: line.uuid,
        },
        ClaudeSystemLogLine::TurnDuration(line) => ClaudeSystemFields {
            timestamp: line.timestamp,
            uuid: line.uuid,
        },
    }
}

fn claude_system_timestamp(system: &ClaudeSystemLogLine) -> DateTime<Utc> {
    claude_system_fields(system).timestamp
}

fn claude_system_uuid(system: &ClaudeSystemLogLine) -> Uuid {
    claude_system_fields(system).uuid
}

fn claude_system_identifier(system: &ClaudeSystemLogLine) -> String {
    claude_system_uuid(system).to_string()
}

impl AnalyzableLog for ClaudeLogLine {
    type LogId = String;
    type ModelId = String;

    fn cost(&self) -> Option<LlmCost> {
        let (assistant, pricing) = priced_claude_assistant(self)?;
        let token_counts = claude_token_counts_from_usage(&assistant.message.usage);
        Some(pricing.calculate_cost(&token_counts))
    }

    fn timestamp(&self) -> DateTime<Utc> {
        match self {
            ClaudeLogLine::User(line) => line.timestamp,
            ClaudeLogLine::Assistant(line) => line.timestamp,
            ClaudeLogLine::FileHistorySnapshot(line) => line.snapshot.timestamp,
            ClaudeLogLine::Summary(_) => claude_timestamp_sentinel(),
            ClaudeLogLine::System(line) => claude_system_timestamp(line),
            ClaudeLogLine::QueueOperation(line) => line.timestamp,
            ClaudeLogLine::Progress(line) => line.timestamp,
            ClaudeLogLine::CustomTitle(_) => claude_timestamp_sentinel(),
            ClaudeLogLine::AgentName(_) => claude_timestamp_sentinel(),
            ClaudeLogLine::LastPrompt(_) => claude_timestamp_sentinel(),
            ClaudeLogLine::PermissionModeChange(_) => claude_timestamp_sentinel(),
            ClaudeLogLine::Attachment(line) => line.timestamp,
        }
    }

    fn identifier(&self) -> Self::LogId {
        match self {
            ClaudeLogLine::User(line) => line.uuid.to_string(),
            ClaudeLogLine::Assistant(line) => claude_assistant_identifier(line),
            ClaudeLogLine::FileHistorySnapshot(line) => line.message_id.to_string(),
            ClaudeLogLine::Summary(line) => line.leaf_uuid.to_string(),
            ClaudeLogLine::System(line) => claude_system_identifier(line),
            ClaudeLogLine::QueueOperation(line) => {
                format!(
                    "queue-operation:{}:{}:{}",
                    line.session_id, line.operation, line.timestamp
                )
            }
            ClaudeLogLine::Progress(line) => line.uuid.to_string(),
            ClaudeLogLine::CustomTitle(line) => format!("custom-title:{}", line.session_id),
            ClaudeLogLine::AgentName(line) => format!("agent-name:{}", line.session_id),
            ClaudeLogLine::LastPrompt(line) => format!("last-prompt:{}", line.session_id),
            ClaudeLogLine::PermissionModeChange(line) => {
                format!("permission-mode:{}", line.session_id)
            }
            ClaudeLogLine::Attachment(line) => line.uuid.to_string(),
        }
    }

    fn model(&self) -> Option<Self::ModelId> {
        let (assistant, _) = priced_claude_assistant(self)?;
        Some(assistant.message.model.clone())
    }

    fn session_id(&self) -> Option<String> {
        match self {
            ClaudeLogLine::Assistant(assistant) => Some(assistant.session_id.clone()),
            _ => None,
        }
    }

    fn parse(value: &str) -> miette::Result<Self> {
        parse_json_backed_log(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PiModel {
    pub provider: Provider,
    pub model: String,
}

fn pi_assistant_message(line: &PiLogLine) -> Option<&AssistantMessage> {
    match line {
        PiLogLine::Message(message) => match &message.message {
            RoleMessage::Assistant(assistant) => Some(assistant),
            _ => None,
        },
        _ => None,
    }
}

impl AnalyzableLog for PiLogLine {
    type LogId = String;
    type ModelId = PiModel;

    fn cost(&self) -> Option<LlmCost> {
        pi_assistant_message(self).map(|assistant| LlmCost {
            input: assistant.usage.cost.input,
            cache_read: assistant.usage.cost.cache_read,
            cache_write: assistant.usage.cost.cache_write,
            output: assistant.usage.cost.output,
        })
    }

    fn identifier(&self) -> String {
        match self {
            PiLogLine::Compaction(compaction) => compaction.id.clone(),
            PiLogLine::Custom(custom) => custom.id.clone(),
            PiLogLine::CustomMessage(message) => message.id.clone(),
            PiLogLine::Message(message) => message.id.clone(),
            PiLogLine::ModelChange(model_change) => model_change.id.clone(),
            PiLogLine::Session(session) => session.id.to_string(),
            PiLogLine::SessionInfo(session_info) => session_info.id.clone(),
            PiLogLine::ThinkingLevelChange(thinking_level) => thinking_level.id.clone(),
        }
    }

    fn model(&self) -> Option<PiModel> {
        pi_assistant_message(self).map(|assistant| PiModel {
            model: assistant.model.clone(),
            provider: assistant.provider,
        })
    }

    fn timestamp(&self) -> DateTime<Utc> {
        match self {
            PiLogLine::Compaction(compaction) => compaction.timestamp,
            PiLogLine::Custom(custom) => custom.timestamp,
            PiLogLine::CustomMessage(message) => message.timestamp,
            PiLogLine::Message(message) => message.timestamp,
            PiLogLine::ModelChange(model_change) => model_change.timestamp,
            PiLogLine::Session(session) => session.timestamp,
            PiLogLine::SessionInfo(session_info) => session_info.timestamp,
            PiLogLine::ThinkingLevelChange(thinking_level) => thinking_level.timestamp,
        }
    }

    fn session_id(&self) -> Option<String> {
        match self {
            // `session_info` announces nested run labels (for example subagent
            // banners), but `pi cost --conversations` should stay grouped by
            // the top-level session header that owns the whole transcript.
            PiLogLine::Session(session) => Some(session.id.to_string()),
            _ => None,
        }
    }

    fn parse(value: &str) -> miette::Result<Self> {
        parse_json_backed_log(value)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use rust_decimal::Decimal;
    use serde_json::json;

    use super::*;
    use crate::test_support::{
        claude_assistant_json, claude_transcript_envelope, claude_usage_json,
        CLAUDE_ASSISTANT_UUID, CLAUDE_BRANCH, CLAUDE_CWD, CLAUDE_LEAF_UUID, CLAUDE_PARENT_UUID,
        CLAUDE_SESSION_ID, CLAUDE_TIMESTAMP, CLAUDE_USER_UUID, CLAUDE_VERSION,
    };

    fn timestamp() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(CLAUDE_TIMESTAMP)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn integer_cost(input: i64, cache_write: i64, cache_read: i64, output: i64) -> LlmCost {
        LlmCost {
            input: Decimal::new(input, 0),
            cache_write: Decimal::new(cache_write, 0),
            cache_read: Decimal::new(cache_read, 0),
            output: Decimal::new(output, 0),
        }
    }

    const CLAUDE_SYSTEM_INFORMATIONAL_UUID: &str = "77777777-7777-4777-8777-777777777777";
    const CLAUDE_SYSTEM_ERROR_UUID: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
    const CLAUDE_SYSTEM_API_ERROR_UUID: &str = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
    const CLAUDE_SYSTEM_COMPACT_BOUNDARY_UUID: &str = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee";
    const CLAUDE_SYSTEM_MICROCOMPACT_BOUNDARY_UUID: &str = "ffffffff-ffff-4fff-8fff-ffffffffffff";
    const CLAUDE_SYSTEM_LOCAL_COMMAND_UUID: &str = "12121212-1212-4212-8212-121212121212";
    const CLAUDE_SYSTEM_STOP_HOOK_SUMMARY_UUID: &str = "13131313-1313-4313-8313-131313131313";
    const CLAUDE_SYSTEM_TURN_DURATION_UUID: &str = "14141414-1414-4414-8414-141414141414";
    const CLAUDE_SYSTEM_LOGICAL_PARENT_UUID: &str = "15151515-1515-4515-8515-151515151515";
    const CLAUDE_SYSTEM_CLEARED_ATTACHMENT_UUID: &str = "16161616-1616-4616-8616-161616161616";

    fn assistant_usage_json() -> serde_json::Value {
        json!({
            "input": 10,
            "output": 5,
            "cacheRead": 2,
            "cacheWrite": 1,
            "totalTokens": 18,
            "cost": {
                "input": "3",
                "output": "5",
                "cacheRead": "2",
                "cacheWrite": "1",
                "total": "11",
            },
        })
    }

    fn assistant_message_json() -> serde_json::Value {
        json!({
            "type": "message",
            "id": "a1",
            "parentId": "u1",
            "timestamp": CLAUDE_TIMESTAMP,
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": "hello"}],
                "api": "anthropic-messages",
                "provider": "anthropic",
                "model": "claude-sonnet-4-5",
                "usage": assistant_usage_json(),
                "stopReason": "stop",
                "timestamp": 1_700_000_000,
            },
        })
    }

    fn user_message_json() -> serde_json::Value {
        json!({
            "type": "message",
            "id": "u1",
            "parentId": "p1",
            "timestamp": CLAUDE_TIMESTAMP,
            "message": {
                "role": "user",
                "content": [{"type": "text", "text": "hello"}],
                "timestamp": 1_700_000_000,
            },
        })
    }

    fn tool_result_message_json() -> serde_json::Value {
        json!({
            "type": "message",
            "id": "tr1",
            "parentId": "a1",
            "timestamp": CLAUDE_TIMESTAMP,
            "message": {
                "role": "toolResult",
                "toolCallId": "call_1",
                "toolName": "bash",
                "content": [{"type": "text", "text": "ok"}],
                "isError": false,
                "timestamp": 1_700_000_000,
            },
        })
    }

    fn session_json() -> serde_json::Value {
        json!({
            "type": "session",
            "version": 1,
            "id": CLAUDE_SESSION_ID,
            "timestamp": CLAUDE_TIMESTAMP,
            "cwd": CLAUDE_CWD,
        })
    }

    fn session_info_json() -> serde_json::Value {
        json!({
            "type": "session_info",
            "id": "info1",
            "parentId": "session-parent-1",
            "timestamp": CLAUDE_TIMESTAMP,
            "name": "subagent-reviewer",
        })
    }

    fn model_change_json() -> serde_json::Value {
        json!({
            "type": "model_change",
            "id": "m1",
            "parentId": null,
            "timestamp": CLAUDE_TIMESTAMP,
            "provider": "anthropic",
            "modelId": "claude-sonnet-4-5",
        })
    }

    fn thinking_level_change_json() -> serde_json::Value {
        json!({
            "type": "thinking_level_change",
            "id": "t1",
            "parentId": "m1",
            "timestamp": CLAUDE_TIMESTAMP,
            "thinkingLevel": "high",
        })
    }

    fn custom_json() -> serde_json::Value {
        json!({
            "type": "custom",
            "id": "c1",
            "parentId": "p1",
            "timestamp": CLAUDE_TIMESTAMP,
            "customType": "dcp-state",
            "data": {
                "compressionBlocks": [{
                    "id": 1,
                    "topic": "Test topic",
                    "summary": "Test summary",
                    "startTimestamp": 1777084923000_i64,
                    "endTimestamp": 1777084924000_i64,
                    "anchorTimestamp": 1777084924000_i64,
                    "active": true,
                    "summaryTokenEstimate": 100,
                    "createdAt": 1777084924500_i64
                }],
                "nextBlockId": 2,
                "prunedToolIds": ["call_1"],
                "tokensSaved": 1000,
                "totalPruneCount": 3,
                "manualMode": false,
            },
        })
    }

    fn compaction_json() -> serde_json::Value {
        json!({
            "type": "compaction",
            "id": "cmp1",
            "parentId": "p1",
            "timestamp": CLAUDE_TIMESTAMP,
            "summary": "Compacted earlier work",
            "firstKeptEntryId": "m42",
            "tokensBefore": 12345,
            "details": {
                "readFiles": ["src/main.rs"],
                "modifiedFiles": ["crates/pi_logs/src/parser.rs"]
            },
            "fromHook": false,
        })
    }

    fn custom_message_json() -> serde_json::Value {
        json!({
            "type": "custom_message",
            "id": "cm1",
            "parentId": "p1",
            "timestamp": CLAUDE_TIMESTAMP,
            "content": "Plan complete",
            "display": true,
            "customType": "plannotator-complete",
        })
    }

    fn claude_user_json() -> serde_json::Value {
        let mut metadata = claude_transcript_envelope(None);
        metadata.insert("type".to_string(), json!("user"));
        metadata.insert(
            "message".to_string(),
            json!({
                "role": "user",
                "content": "hello",
            }),
        );
        metadata.insert("isMeta".to_string(), serde_json::Value::Null);
        metadata.insert("uuid".to_string(), json!(CLAUDE_USER_UUID));
        metadata.insert("timestamp".to_string(), json!(CLAUDE_TIMESTAMP));
        metadata.insert("toolUseResult".to_string(), serde_json::Value::Null);
        metadata.insert("thinkingMetadata".to_string(), serde_json::Value::Null);
        metadata.insert(
            "isVisibleInTranscriptOnly".to_string(),
            serde_json::Value::Null,
        );
        metadata.insert("isCompactSummary".to_string(), serde_json::Value::Null);
        metadata.insert("todos".to_string(), serde_json::Value::Null);
        metadata.insert(
            "sourceToolAssistantUUID".to_string(),
            serde_json::Value::Null,
        );
        metadata.insert("promptId".to_string(), serde_json::Value::Null);
        metadata.insert("permissionMode".to_string(), serde_json::Value::Null);
        metadata.insert("planContent".to_string(), serde_json::Value::Null);
        metadata.insert("entrypoint".to_string(), serde_json::Value::Null);
        metadata.insert("origin".to_string(), serde_json::Value::Null);
        serde_json::Value::Object(metadata)
    }

    fn claude_summary_json() -> serde_json::Value {
        json!({
            "type": "summary",
            "summary": "short summary",
            "leafUuid": CLAUDE_LEAF_UUID,
        })
    }

    fn claude_metadata_json(kind: &str, field: &str, value: &str) -> serde_json::Value {
        let mut metadata = serde_json::Map::new();
        metadata.insert("type".to_string(), json!(kind));
        metadata.insert(field.to_string(), json!(value));
        metadata.insert("sessionId".to_string(), json!(CLAUDE_SESSION_ID));
        serde_json::Value::Object(metadata)
    }

    fn claude_custom_title_json() -> serde_json::Value {
        claude_metadata_json("custom-title", "customTitle", "A title")
    }

    fn claude_agent_name_json() -> serde_json::Value {
        claude_metadata_json("agent-name", "agentName", "researcher")
    }

    fn claude_last_prompt_json() -> serde_json::Value {
        claude_metadata_json("last-prompt", "lastPrompt", "finish this")
    }

    fn claude_permission_mode_json() -> serde_json::Value {
        claude_metadata_json("permission-mode", "permissionMode", "acceptEdits")
    }

    fn claude_system_json(
        subtype: &str,
        parent_uuid: Option<&str>,
        uuid: &str,
    ) -> serde_json::Map<String, serde_json::Value> {
        let mut metadata = claude_transcript_envelope(parent_uuid);
        metadata.remove("agentId");
        metadata.insert("type".to_string(), json!("system"));
        metadata.insert("subtype".to_string(), json!(subtype));
        metadata.insert("timestamp".to_string(), json!(CLAUDE_TIMESTAMP));
        metadata.insert("uuid".to_string(), json!(uuid));
        metadata.insert("entrypoint".to_string(), serde_json::Value::Null);
        metadata
    }

    fn claude_system_informational_json() -> serde_json::Value {
        let mut metadata = claude_system_json(
            "informational",
            Some(CLAUDE_PARENT_UUID),
            CLAUDE_SYSTEM_INFORMATIONAL_UUID,
        );
        metadata.insert("content".to_string(), json!("informational message"));
        metadata.insert("isMeta".to_string(), json!(false));
        metadata.insert("level".to_string(), json!("info"));
        serde_json::Value::Object(metadata)
    }

    fn claude_system_error_json() -> serde_json::Value {
        let mut metadata =
            claude_system_json("error", Some(CLAUDE_PARENT_UUID), CLAUDE_SYSTEM_ERROR_UUID);
        metadata.insert("level".to_string(), json!("error"));
        metadata.insert("cause".to_string(), serde_json::Value::Null);
        metadata.insert(
            "error".to_string(),
            json!({
                "requestID": "req_error_123",
                "status": 500,
            }),
        );
        metadata.insert("retryInMs".to_string(), json!(250.0));
        metadata.insert("retryAttempt".to_string(), json!(1));
        metadata.insert("maxRetries".to_string(), json!(3));
        serde_json::Value::Object(metadata)
    }

    fn claude_system_api_error_json() -> serde_json::Value {
        let mut metadata = claude_system_json(
            "api_error",
            Some(CLAUDE_PARENT_UUID),
            CLAUDE_SYSTEM_API_ERROR_UUID,
        );
        metadata.insert("level".to_string(), json!("error"));
        metadata.insert("cause".to_string(), json!("rate_limited"));
        metadata.insert(
            "error".to_string(),
            json!({
                "requestID": "req_api_123",
                "status": 429,
            }),
        );
        metadata.insert("retryInMs".to_string(), json!(500.0));
        metadata.insert("retryAttempt".to_string(), json!(2));
        metadata.insert("maxRetries".to_string(), json!(5));
        serde_json::Value::Object(metadata)
    }

    fn claude_system_compact_boundary_json() -> serde_json::Value {
        let mut metadata = claude_system_json(
            "compact_boundary",
            Some(CLAUDE_PARENT_UUID),
            CLAUDE_SYSTEM_COMPACT_BOUNDARY_UUID,
        );
        metadata.insert(
            "logicalParentUuid".to_string(),
            json!(CLAUDE_SYSTEM_LOGICAL_PARENT_UUID),
        );
        metadata.insert("content".to_string(), json!("compacted context"));
        metadata.insert("isMeta".to_string(), json!(false));
        metadata.insert("level".to_string(), json!("info"));
        metadata.insert(
            "compactMetadata".to_string(),
            json!({
                "trigger": "manual",
                "preTokens": 123,
            }),
        );
        serde_json::Value::Object(metadata)
    }

    fn claude_system_microcompact_boundary_json() -> serde_json::Value {
        let mut metadata = claude_system_json(
            "microcompact_boundary",
            Some(CLAUDE_PARENT_UUID),
            CLAUDE_SYSTEM_MICROCOMPACT_BOUNDARY_UUID,
        );
        metadata.insert("content".to_string(), json!("microcompacted context"));
        metadata.insert("isMeta".to_string(), json!(false));
        metadata.insert("level".to_string(), json!("info"));
        metadata.insert(
            "microcompactMetadata".to_string(),
            json!({
                "trigger": "manual",
                "preTokens": 123,
                "tokensSaved": 45,
                "compactedToolIds": ["toolu_1"],
                "clearedAttachmentUUIDs": [CLAUDE_SYSTEM_CLEARED_ATTACHMENT_UUID],
            }),
        );
        serde_json::Value::Object(metadata)
    }

    fn claude_system_local_command_json() -> serde_json::Value {
        let mut metadata = claude_system_json(
            "local_command",
            Some(CLAUDE_PARENT_UUID),
            CLAUDE_SYSTEM_LOCAL_COMMAND_UUID,
        );
        metadata.insert("content".to_string(), json!("git status"));
        metadata.insert("level".to_string(), json!("info"));
        metadata.insert("isMeta".to_string(), json!(false));
        serde_json::Value::Object(metadata)
    }

    fn claude_system_stop_hook_summary_json() -> serde_json::Value {
        let mut metadata = claude_system_json(
            "stop_hook_summary",
            Some(CLAUDE_PARENT_UUID),
            CLAUDE_SYSTEM_STOP_HOOK_SUMMARY_UUID,
        );
        metadata.insert("hookCount".to_string(), json!(1));
        metadata.insert(
            "hookInfos".to_string(),
            json!([{
                "command": "echo ok",
                "durationMs": 12,
            }]),
        );
        metadata.insert("hookErrors".to_string(), json!([]));
        metadata.insert("preventedContinuation".to_string(), json!(false));
        metadata.insert("stopReason".to_string(), json!("completed"));
        metadata.insert("hasOutput".to_string(), json!(true));
        metadata.insert("level".to_string(), json!("info"));
        metadata.insert("toolUseID".to_string(), json!("toolu_stop_hook"));
        serde_json::Value::Object(metadata)
    }

    fn claude_system_turn_duration_json() -> serde_json::Value {
        let mut metadata = claude_system_json(
            "turn_duration",
            Some(CLAUDE_PARENT_UUID),
            CLAUDE_SYSTEM_TURN_DURATION_UUID,
        );
        metadata.insert("durationMs".to_string(), json!(1234));
        metadata.insert("isMeta".to_string(), json!(false));
        metadata.insert("messageCount".to_string(), json!(2));
        serde_json::Value::Object(metadata)
    }

    fn claude_queue_operation_json() -> serde_json::Value {
        json!({
            "type": "queue-operation",
            "operation": "enqueue",
            "timestamp": CLAUDE_TIMESTAMP,
            "content": null,
            "sessionId": CLAUDE_SESSION_ID,
        })
    }

    fn claude_file_history_snapshot_json() -> serde_json::Value {
        json!({
            "type": "file-history-snapshot",
            "messageId": "88888888-8888-4888-8888-888888888888",
            "snapshot": {
                "messageId": "99999999-9999-4999-8999-999999999999",
                "trackedFileBackups": {},
                "timestamp": CLAUDE_TIMESTAMP,
            },
            "isSnapshotUpdate": false,
        })
    }

    fn claude_progress_json() -> serde_json::Value {
        json!({
            "type": "progress",
            "parentUuid": CLAUDE_PARENT_UUID,
            "isSidechain": false,
            "userType": "external",
            "cwd": CLAUDE_CWD,
            "sessionId": CLAUDE_SESSION_ID,
            "version": CLAUDE_VERSION,
            "gitBranch": CLAUDE_BRANCH,
            "agentId": null,
            "slug": null,
            "data": {
                "type": "hook_progress",
                "hookEvent": "PreToolUse",
                "hookName": "test-hook",
                "command": "echo ok",
            },
            "toolUseID": "toolu_progress",
            "parentToolUseID": "toolu_parent",
            "uuid": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
            "timestamp": CLAUDE_TIMESTAMP,
            "entrypoint": null,
        })
    }

    fn claude_attachment_json() -> serde_json::Value {
        json!({
            "type": "attachment",
            "parentUuid": null,
            "isSidechain": false,
            "attachment": {
                "type": "auto_mode",
                "reminderType": "auto-mode",
            },
            "uuid": "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
            "timestamp": CLAUDE_TIMESTAMP,
            "userType": "external",
            "entrypoint": null,
            "cwd": CLAUDE_CWD,
            "sessionId": CLAUDE_SESSION_ID,
            "version": CLAUDE_VERSION,
            "gitBranch": CLAUDE_BRANCH,
            "slug": null,
        })
    }

    fn parse_pi_log(value: serde_json::Value) -> PiLogLine {
        <PiLogLine as AnalyzableLog>::parse(&value.to_string()).unwrap()
    }

    fn parse_claude_log(value: serde_json::Value) -> ClaudeLogLine {
        <ClaudeLogLine as AnalyzableLog>::parse(&value.to_string()).unwrap()
    }

    fn parse_claude_line_with_cost(value: serde_json::Value) -> LineWithCost<ClaudeLogLine> {
        LineWithCost::<ClaudeLogLine>::parse(&value.to_string())
            .unwrap()
            .unwrap()
    }

    fn assert_claude_line_with_cost_is_none(value: &serde_json::Value) {
        assert!(
            LineWithCost::<ClaudeLogLine>::parse(&value.to_string())
                .unwrap()
                .is_none(),
            "expected non-billable Claude log line: {value}"
        );
    }

    fn assert_claude_non_billable_value(
        name: &str,
        value: serde_json::Value,
        expected_timestamp: DateTime<Utc>,
        expected_id: &str,
    ) {
        let line = parse_claude_log(value.clone());

        assert_eq!(line.identifier(), expected_id, "case {name}");
        assert_eq!(line.timestamp(), expected_timestamp, "case {name}");
        assert!(line.cost().is_none(), "case {name}");
        assert!(line.model().is_none(), "case {name}");
        assert_claude_line_with_cost_is_none(&value);
    }

    #[test]
    fn llm_cost_total_sums_all_fields() {
        let cost = integer_cost(3, 1, 2, 5);

        assert_eq!(cost.total(), Decimal::new(11, 0));
    }

    #[test]
    fn llm_cost_ordering_prioritizes_total() {
        let higher_total = integer_cost(1, 0, 0, 10);
        let lower_total = integer_cost(5, 0, 0, 3);

        assert!(higher_total > lower_total);
    }

    #[test]
    fn llm_cost_ordering_breaks_equal_total_ties_by_field_priority() {
        let higher_input = integer_cost(10, 0, 0, 0);
        let lower_input = integer_cost(1, 9, 0, 0);
        let higher_cache_write = integer_cost(1, 9, 0, 0);
        let lower_cache_write = integer_cost(1, 1, 8, 0);
        let higher_cache_read = integer_cost(1, 1, 5, 3);
        let lower_cache_read = integer_cost(1, 1, 4, 4);

        assert!(higher_input > lower_input);
        assert!(higher_cache_write > lower_cache_write);
        assert!(higher_cache_read > lower_cache_read);
    }

    #[test]
    fn line_with_cost_parse_returns_some_for_assistant_messages() {
        let parsed = LineWithCost::<PiLogLine>::parse(&assistant_message_json().to_string())
            .unwrap()
            .unwrap();

        assert_eq!(parsed.id, "a1");
        assert_eq!(parsed.timestamp, timestamp());
        assert_eq!(
            parsed.model,
            PiModel {
                provider: Provider::Anthropic,
                model: "claude-sonnet-4-5".to_string(),
            }
        );
        assert_eq!(parsed.cost.total(), Decimal::new(11, 0));
        assert_eq!(parsed.session_id, None);
    }

    #[test]
    fn line_with_cost_parse_returns_none_for_non_assistant_messages() {
        let cases = [
            user_message_json(),
            tool_result_message_json(),
            session_json(),
            model_change_json(),
            thinking_level_change_json(),
            custom_json(),
            custom_message_json(),
        ];

        for (index, value) in cases.into_iter().enumerate() {
            assert!(
                LineWithCost::<PiLogLine>::parse(&value.to_string())
                    .unwrap()
                    .is_none(),
                "case {index} should return None: {value}"
            );
        }
    }

    #[derive(Debug, Clone)]
    struct AsymmetricMockLog {
        timestamp: DateTime<Utc>,
        cost: Option<LlmCost>,
        model: Option<String>,
    }

    impl AnalyzableLog for AsymmetricMockLog {
        type LogId = String;
        type ModelId = String;

        fn cost(&self) -> Option<LlmCost> {
            self.cost
        }

        fn identifier(&self) -> Self::LogId {
            "asymmetric".to_string()
        }

        fn model(&self) -> Option<Self::ModelId> {
            self.model.clone()
        }

        fn timestamp(&self) -> DateTime<Utc> {
            self.timestamp
        }

        fn session_id(&self) -> Option<String> {
            None
        }

        fn parse(value: &str) -> miette::Result<Self> {
            match value {
                "cost-only" => Ok(Self {
                    timestamp: timestamp(),
                    cost: Some(LlmCost {
                        input: Decimal::new(1, 0),
                        cache_write: Decimal::ZERO,
                        cache_read: Decimal::ZERO,
                        output: Decimal::ZERO,
                    }),
                    model: None,
                }),
                "model-only" => Ok(Self {
                    timestamp: timestamp(),
                    cost: None,
                    model: Some("model-a".to_string()),
                }),
                _ => panic!("unexpected asymmetric mock input: {value}"),
            }
        }
    }

    #[test]
    fn line_with_cost_parse_returns_none_when_only_cost_or_model_is_present() {
        assert!(LineWithCost::<AsymmetricMockLog>::parse("cost-only")
            .unwrap()
            .is_none());
        assert!(LineWithCost::<AsymmetricMockLog>::parse("model-only")
            .unwrap()
            .is_none());
    }

    #[test]
    fn line_with_cost_parse_returns_error_for_invalid_json() {
        let error = LineWithCost::<PiLogLine>::parse("not-json").unwrap_err();

        assert!(format!("{error}").contains(LOG_LINE_PARSE_CONTEXT));
    }

    #[test]
    fn pi_log_line_trait_methods_cover_all_variants() {
        let cases = [
            (
                session_json(),
                "019dc252-e50e-766c-8182-d654b46881af",
                false,
                false,
                Some(CLAUDE_SESSION_ID),
            ),
            (session_info_json(), "info1", false, false, None),
            (model_change_json(), "m1", false, false, None),
            (thinking_level_change_json(), "t1", false, false, None),
            (compaction_json(), "cmp1", false, false, None),
            (custom_json(), "c1", false, false, None),
            (custom_message_json(), "cm1", false, false, None),
            (user_message_json(), "u1", false, false, None),
            (tool_result_message_json(), "tr1", false, false, None),
            (assistant_message_json(), "a1", true, true, None),
        ];

        for (value, expected_id, expect_cost, expect_model, expected_session_id) in cases {
            let line = parse_pi_log(value);

            assert_eq!(line.identifier(), expected_id);
            assert_eq!(line.timestamp(), timestamp());
            assert_eq!(line.cost().is_some(), expect_cost);
            assert_eq!(line.model().is_some(), expect_model);
            assert_eq!(line.session_id().as_deref(), expected_session_id);
        }
    }

    #[test]
    fn line_with_cost_parse_keeps_claude_assistant_session_id() {
        let parsed = parse_claude_line_with_cost(claude_assistant_json(
            None,
            Some("req-1"),
            "msg-1",
            CLAUDE_ASSISTANT_UUID,
            "claude-sonnet-4-20250514",
            claude_usage_json(1, 0, 0, 0),
        ));

        assert_eq!(parsed.session_id.as_deref(), Some(CLAUDE_SESSION_ID));
    }

    #[test]
    fn claude_model_type_matches_api_pricing_families() {
        let cases = [
            ("claude-sonnet-4-20250514", ClaudeModelType::Sonnet),
            ("CLAUDE-3-5-SONNET-20241022", ClaudeModelType::Sonnet),
            ("claude-3-haiku-20240307", ClaudeModelType::Haiku),
            ("claude-3-opus-20240229", ClaudeModelType::Opus),
            ("claude-opus-4-20250514", ClaudeModelType::Opus4),
            ("claude-opus-45", ClaudeModelType::Opus4),
            ("", ClaudeModelType::Unknown),
            ("gpt-4", ClaudeModelType::Unknown),
            ("claude-opus-4-sonnet-preview", ClaudeModelType::Sonnet),
        ];

        for (model, expected) in cases {
            assert_eq!(ClaudeModelType::from_model_string(model), expected);
        }
    }

    #[test]
    fn claude_model_type_display_matches_variant_names() {
        let cases = [
            (ClaudeModelType::Sonnet, "Sonnet"),
            (ClaudeModelType::Haiku, "Haiku"),
            (ClaudeModelType::Opus, "Opus"),
            (ClaudeModelType::Opus4, "Opus 4"),
            (ClaudeModelType::Unknown, "Unknown"),
        ];

        for (model_type, expected) in cases {
            assert_eq!(model_type.to_string(), expected);
        }
    }

    #[test]
    fn claude_pricing_constants_use_decimal_rates() {
        assert_eq!(ClaudeModelPricing::SONNET.input, Decimal::new(3, 0));
        assert_eq!(ClaudeModelPricing::SONNET.output, Decimal::new(15, 0));
        assert_eq!(ClaudeModelPricing::SONNET.cache_write, Decimal::new(375, 2));
        assert_eq!(ClaudeModelPricing::SONNET.cache_read, Decimal::new(30, 2));
        assert_eq!(ClaudeModelPricing::HAIKU.cache_read, Decimal::new(1, 1));
        assert_eq!(ClaudeModelPricing::OPUS.input, Decimal::new(15, 0));
        assert_eq!(ClaudeModelPricing::OPUS_4.cache_write, Decimal::new(625, 2));
    }

    #[test]
    fn claude_pricing_zero_tokens_produces_zero_cost() {
        let cost = ClaudeModelPricing::SONNET.calculate_cost(&ClaudeTokenCounts::default());

        assert_eq!(cost, integer_cost(0, 0, 0, 0));
    }

    #[derive(Clone, Copy)]
    struct ClaudePricedAssistantCase<'a> {
        name: &'a str,
        request_id: Option<&'a str>,
        message_id: &'a str,
        uuid: &'a str,
        model: &'a str,
        usage: (usize, usize, usize, usize),
        expected_id: &'a str,
        expected_cost: LlmCost,
    }

    fn assert_claude_priced_assistant_case(case: ClaudePricedAssistantCase<'_>) {
        let (input_tokens, output_tokens, cache_write_tokens, cache_read_tokens) = case.usage;
        let parsed = parse_claude_line_with_cost(claude_assistant_json(
            Some(CLAUDE_PARENT_UUID),
            case.request_id,
            case.message_id,
            case.uuid,
            case.model,
            claude_usage_json(
                input_tokens,
                output_tokens,
                cache_write_tokens,
                cache_read_tokens,
            ),
        ));

        assert_eq!(parsed.id, case.expected_id, "case {}", case.name);
        assert_eq!(parsed.model, case.model, "case {}", case.name);
        assert_eq!(parsed.timestamp, timestamp(), "case {}", case.name);
        assert_eq!(parsed.cost, case.expected_cost, "case {}", case.name);
    }

    #[derive(Clone, Copy)]
    enum ExpectedClaudeTimestamp {
        Real,
        Sentinel,
    }

    impl ExpectedClaudeTimestamp {
        fn value(self) -> DateTime<Utc> {
            match self {
                Self::Real => timestamp(),
                Self::Sentinel => claude_timestamp_sentinel(),
            }
        }
    }

    struct ClaudeNonBillableCase {
        name: &'static str,
        value: fn() -> serde_json::Value,
        expected_timestamp: ExpectedClaudeTimestamp,
        expected_id: String,
    }

    fn claude_metadata_identifier(kind: &str) -> String {
        format!("{kind}:{CLAUDE_SESSION_ID}")
    }

    fn claude_queue_operation_identifier(operation: &str) -> String {
        format!(
            "queue-operation:{CLAUDE_SESSION_ID}:{operation}:{}",
            timestamp()
        )
    }

    fn assert_claude_non_billable_case(case: ClaudeNonBillableCase) {
        assert_claude_non_billable_value(
            case.name,
            (case.value)(),
            case.expected_timestamp.value(),
            &case.expected_id,
        );
    }

    #[derive(Clone, Copy)]
    struct ClaudeIdentifierCase<'a> {
        name: &'a str,
        request_id: Option<&'a str>,
        message_id: &'a str,
        expected_id: &'a str,
    }

    #[test]
    fn claude_priced_assistant_cases_preserve_model_ids_and_costs() {
        let cases = [
            ClaudePricedAssistantCase {
                name: "sonnet multimillion usage",
                request_id: Some("req-1"),
                message_id: "msg_1",
                uuid: CLAUDE_ASSISTANT_UUID,
                model: "claude-sonnet-4-20250514",
                usage: (1_000_000, 2_000_000, 3_000_000, 4_000_000),
                expected_id: "req-1",
                expected_cost: LlmCost {
                    input: Decimal::new(3, 0),
                    cache_write: Decimal::new(1125, 2),
                    cache_read: Decimal::new(12, 1),
                    output: Decimal::new(30, 0),
                },
            },
            ClaudePricedAssistantCase {
                name: "raw model id is preserved",
                request_id: Some("req-model"),
                message_id: "msg_model",
                uuid: CLAUDE_ASSISTANT_UUID,
                model: "claude-3-5-sonnet-20241022",
                usage: (1, 1, 1, 1),
                expected_id: "req-model",
                expected_cost: LlmCost {
                    input: Decimal::new(3, 6),
                    cache_write: Decimal::new(375, 8),
                    cache_read: Decimal::new(3, 7),
                    output: Decimal::new(15, 6),
                },
            },
            ClaudePricedAssistantCase {
                name: "haiku single-token costs stay exact",
                request_id: Some("req-decimal"),
                message_id: "msg_decimal",
                uuid: CLAUDE_ASSISTANT_UUID,
                model: "claude-3-haiku-20240307",
                usage: (1, 1, 1, 1),
                expected_id: "req-decimal",
                expected_cost: LlmCost {
                    input: Decimal::new(1, 6),
                    cache_write: Decimal::new(125, 8),
                    cache_read: Decimal::new(1, 7),
                    output: Decimal::new(5, 6),
                },
            },
            ClaudePricedAssistantCase {
                name: "haiku million-token pricing",
                request_id: Some("req-haiku-million"),
                message_id: "msg_haiku_million",
                uuid: CLAUDE_ASSISTANT_UUID,
                model: "claude-3-haiku-20240307",
                usage: (1_000_000, 1_000_000, 1_000_000, 1_000_000),
                expected_id: "req-haiku-million",
                expected_cost: LlmCost {
                    input: Decimal::new(1, 0),
                    cache_write: Decimal::new(125, 2),
                    cache_read: Decimal::new(1, 1),
                    output: Decimal::new(5, 0),
                },
            },
            ClaudePricedAssistantCase {
                name: "opus pricing path",
                request_id: Some("req-opus"),
                message_id: "msg_opus",
                uuid: CLAUDE_ASSISTANT_UUID,
                model: "claude-3-opus-20240229",
                usage: (1_000_000, 1_000_000, 1_000_000, 1_000_000),
                expected_id: "req-opus",
                expected_cost: LlmCost {
                    input: Decimal::new(15, 0),
                    cache_write: Decimal::new(1875, 2),
                    cache_read: Decimal::new(15, 1),
                    output: Decimal::new(75, 0),
                },
            },
            ClaudePricedAssistantCase {
                name: "opus 4 pricing path",
                request_id: Some("req-opus4"),
                message_id: "msg_opus4",
                uuid: CLAUDE_ASSISTANT_UUID,
                model: "claude-opus-4-20250514",
                usage: (1_000_000, 1_000_000, 1_000_000, 1_000_000),
                expected_id: "req-opus4",
                expected_cost: LlmCost {
                    input: Decimal::new(5, 0),
                    cache_write: Decimal::new(625, 2),
                    cache_read: Decimal::new(50, 2),
                    output: Decimal::new(25, 0),
                },
            },
        ];

        for case in cases {
            assert_claude_priced_assistant_case(case);
        }
    }

    #[test]
    fn claude_non_assistant_variants_are_non_billable() {
        let cases = [
            ClaudeNonBillableCase {
                name: "user",
                value: claude_user_json,
                expected_timestamp: ExpectedClaudeTimestamp::Real,
                expected_id: CLAUDE_USER_UUID.to_string(),
            },
            ClaudeNonBillableCase {
                name: "summary",
                value: claude_summary_json,
                expected_timestamp: ExpectedClaudeTimestamp::Sentinel,
                expected_id: CLAUDE_LEAF_UUID.to_string(),
            },
            ClaudeNonBillableCase {
                name: "custom title",
                value: claude_custom_title_json,
                expected_timestamp: ExpectedClaudeTimestamp::Sentinel,
                expected_id: claude_metadata_identifier("custom-title"),
            },
            ClaudeNonBillableCase {
                name: "agent name",
                value: claude_agent_name_json,
                expected_timestamp: ExpectedClaudeTimestamp::Sentinel,
                expected_id: claude_metadata_identifier("agent-name"),
            },
            ClaudeNonBillableCase {
                name: "last prompt",
                value: claude_last_prompt_json,
                expected_timestamp: ExpectedClaudeTimestamp::Sentinel,
                expected_id: claude_metadata_identifier("last-prompt"),
            },
            ClaudeNonBillableCase {
                name: "permission mode",
                value: claude_permission_mode_json,
                expected_timestamp: ExpectedClaudeTimestamp::Sentinel,
                expected_id: claude_metadata_identifier("permission-mode"),
            },
            ClaudeNonBillableCase {
                name: "system informational",
                value: claude_system_informational_json,
                expected_timestamp: ExpectedClaudeTimestamp::Real,
                expected_id: CLAUDE_SYSTEM_INFORMATIONAL_UUID.to_string(),
            },
            ClaudeNonBillableCase {
                name: "system error",
                value: claude_system_error_json,
                expected_timestamp: ExpectedClaudeTimestamp::Real,
                expected_id: CLAUDE_SYSTEM_ERROR_UUID.to_string(),
            },
            ClaudeNonBillableCase {
                name: "system api error",
                value: claude_system_api_error_json,
                expected_timestamp: ExpectedClaudeTimestamp::Real,
                expected_id: CLAUDE_SYSTEM_API_ERROR_UUID.to_string(),
            },
            ClaudeNonBillableCase {
                name: "system compact boundary",
                value: claude_system_compact_boundary_json,
                expected_timestamp: ExpectedClaudeTimestamp::Real,
                expected_id: CLAUDE_SYSTEM_COMPACT_BOUNDARY_UUID.to_string(),
            },
            ClaudeNonBillableCase {
                name: "system microcompact boundary",
                value: claude_system_microcompact_boundary_json,
                expected_timestamp: ExpectedClaudeTimestamp::Real,
                expected_id: CLAUDE_SYSTEM_MICROCOMPACT_BOUNDARY_UUID.to_string(),
            },
            ClaudeNonBillableCase {
                name: "system local command",
                value: claude_system_local_command_json,
                expected_timestamp: ExpectedClaudeTimestamp::Real,
                expected_id: CLAUDE_SYSTEM_LOCAL_COMMAND_UUID.to_string(),
            },
            ClaudeNonBillableCase {
                name: "system stop hook summary",
                value: claude_system_stop_hook_summary_json,
                expected_timestamp: ExpectedClaudeTimestamp::Real,
                expected_id: CLAUDE_SYSTEM_STOP_HOOK_SUMMARY_UUID.to_string(),
            },
            ClaudeNonBillableCase {
                name: "system turn duration",
                value: claude_system_turn_duration_json,
                expected_timestamp: ExpectedClaudeTimestamp::Real,
                expected_id: CLAUDE_SYSTEM_TURN_DURATION_UUID.to_string(),
            },
            ClaudeNonBillableCase {
                name: "queue operation",
                value: claude_queue_operation_json,
                expected_timestamp: ExpectedClaudeTimestamp::Real,
                expected_id: claude_queue_operation_identifier("enqueue"),
            },
            ClaudeNonBillableCase {
                name: "file history snapshot",
                value: claude_file_history_snapshot_json,
                expected_timestamp: ExpectedClaudeTimestamp::Real,
                expected_id: "88888888-8888-4888-8888-888888888888".to_string(),
            },
            ClaudeNonBillableCase {
                name: "progress",
                value: claude_progress_json,
                expected_timestamp: ExpectedClaudeTimestamp::Real,
                expected_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_string(),
            },
            ClaudeNonBillableCase {
                name: "attachment",
                value: claude_attachment_json,
                expected_timestamp: ExpectedClaudeTimestamp::Real,
                expected_id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".to_string(),
            },
        ];

        for case in cases {
            assert_claude_non_billable_case(case);
        }
    }

    #[test]
    fn claude_synthetic_and_unknown_models_are_non_billable() {
        let cases = [
            (
                "synthetic model",
                claude_assistant_json(
                    Some(CLAUDE_PARENT_UUID),
                    Some("req-synthetic"),
                    "msg_synthetic",
                    CLAUDE_ASSISTANT_UUID,
                    "<SyNtHeTiC>",
                    claude_usage_json(1_000_000, 1_000_000, 1_000_000, 1_000_000),
                ),
                "req-synthetic",
            ),
            (
                "unknown model",
                claude_assistant_json(
                    Some(CLAUDE_PARENT_UUID),
                    Some("req-unknown"),
                    "msg_unknown",
                    CLAUDE_ASSISTANT_UUID,
                    "gpt-4",
                    claude_usage_json(1_000_000, 1_000_000, 1_000_000, 1_000_000),
                ),
                "req-unknown",
            ),
        ];

        for (name, value, expected_id) in cases {
            assert_claude_non_billable_value(name, value, timestamp(), expected_id);
        }
    }

    #[test]
    fn claude_assistant_identifier_prefers_request_id_then_message_id_then_uuid() {
        let cases = [
            ClaudeIdentifierCase {
                name: "request id wins",
                request_id: Some("req-1"),
                message_id: "msg_1",
                expected_id: "req-1",
            },
            ClaudeIdentifierCase {
                name: "message id fallback",
                request_id: None,
                message_id: "msg_2",
                expected_id: "msg_2",
            },
            ClaudeIdentifierCase {
                name: "empty request id falls back to message id",
                request_id: Some(""),
                message_id: "msg_3",
                expected_id: "msg_3",
            },
            ClaudeIdentifierCase {
                name: "uuid fallback when message id is absent",
                request_id: None,
                message_id: "",
                expected_id: CLAUDE_ASSISTANT_UUID,
            },
            ClaudeIdentifierCase {
                name: "uuid fallback when request and message ids are empty",
                request_id: Some(""),
                message_id: "",
                expected_id: CLAUDE_ASSISTANT_UUID,
            },
        ];

        for case in cases {
            let line = parse_claude_log(claude_assistant_json(
                Some(CLAUDE_PARENT_UUID),
                case.request_id,
                case.message_id,
                CLAUDE_ASSISTANT_UUID,
                "claude-sonnet-4-20250514",
                claude_usage_json(1, 1, 1, 1),
            ));

            assert_eq!(line.identifier(), case.expected_id, "case {}", case.name);
        }
    }
}
