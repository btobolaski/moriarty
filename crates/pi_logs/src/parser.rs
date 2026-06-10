//! Strongly typed serde models for pi session log lines.
//!
//! A pi session log file is newline-delimited JSON. Each line is a
//! [`PiLogLine`] keyed by the top-level `type` field. Most nested payloads are
//! modeled as tagged enums or concrete structs with
//! `#[serde(deny_unknown_fields)]` so that most upstream format changes surface
//! as parse errors rather than silent data loss.
//!
//! Tool-call envelopes stay typed and strict, but [`ToolCallContent`]
//! deliberately preserves the inner `arguments` object as raw JSON. Pi logs the
//! model-emitted payload before the runtime validates it, so hard-coding tool
//! schemas into the parser would reject or misrepresent real sessions.
//!
//! Three categories of structure legitimately deviate from the strict default:
//!
//! * **`serde(flatten)` of an internally-tagged enum** — when the flattened
//!   target is an internally-tagged enum (one with `#[serde(tag = "...")]`
//!   and no `content`), serde's flatten codegen does not register the tag
//!   field as "claimed" by the inner enum, so a strict outer struct rejects
//!   it as unknown. [`WebSearchResultsData`] is the only struct in this
//!   category; it keeps the flattened internally tagged shape, but restores
//!   strict outer-key validation with a manual deserializer. Adjacently
//!   tagged flatten targets (those with both `tag` and `content`) do *not*
//!   suffer this collision, so [`CustomLine`] and [`CustomMessageLine`] keep
//!   derived `deny_unknown_fields` handling.
//!
//! * **Corrupt-stream tolerance** — some payloads are absorbed via
//!   permissive structs, targeted field aliases, or untagged fallback enums
//!   so a single corrupted record cannot abort an entire log file. Four
//!   flavors exist:
//!     1. Permissive argument/payload structs ([`EditArgs`], nested edit
//!        payloads like [`EditReplacement`], and [`GrepArgs`]) that omit
//!        `deny_unknown_fields` to ignore hallucinated sibling keys (e.g.
//!        `:path` on grep).
//!     2. Field-level aliases (for example on [`FindArgs`]) that map an
//!        observed punctuated key corruption like `.limit` back onto the
//!        intended schema field without relaxing the whole struct.
//!     3. Array-element fallback enums ([`EditEntry`]) whose `Fragment`
//!        variant captures raw JSON tokens (`,`, `},{`) interspersed
//!        between real entries when the model truncates mid-stream.
//!     4. Value-level fallback enums ([`MaybeU32`]) whose `Garbage` variant
//!        absorbs string-typed corruption (e.g. `"limit": "limit"` where
//!        the model echoed the schema field name as the value).
//!
//! * **Open-ended protocol discriminators** — [`AssistantApi`] uses a custom
//!   deserializer that accepts a small set of well-known API identifiers
//!   (`anthropic-messages`, `openai-responses`, `openai-completions`) plus
//!   `faux:`-prefixed routing strings emitted by the faux AI provider. Any
//!   other string value still fails loudly. This is intentionally narrower
//!   than a fully open `String` but wider than a strict enum, because faux
//!   session IDs are dynamic and cannot be enumerated ahead of time.
//!
//! Each corrupt-stream exception carries an inline comment naming the
//! observed failure mode.

use std::{
    cmp::Ordering,
    collections::BTreeMap,
    fs::File,
    hash::{Hash, Hasher},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use uuid::Uuid;

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JsonBlob(pub Value);

impl JsonBlob {
    fn canonical_json(&self) -> String {
        serde_json::to_string(&self.0).expect("json values should always serialize")
    }
}

impl From<Value> for JsonBlob {
    fn from(value: Value) -> Self {
        Self(value)
    }
}

impl Hash for JsonBlob {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.canonical_json().hash(state);
    }
}

impl PartialOrd for JsonBlob {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for JsonBlob {
    fn cmp(&self, other: &Self) -> Ordering {
        self.canonical_json().cmp(&other.canonical_json())
    }
}

// ---------------------------------------------------------------------------
// Top-level line
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PiLogLine {
    Session(SessionLine),
    SessionInfo(SessionInfoLine),
    ModelChange(ModelChangeLine),
    ThinkingLevelChange(ThinkingLevelChangeLine),
    Compaction(CompactionLine),
    BranchSummary(BranchSummaryLine),
    Custom(CustomLine),
    CustomMessage(Box<CustomMessageLine>),
    Message(MessageLine),
}

// ---------------------------------------------------------------------------
// Session / model / thinking header lines
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionLine {
    pub version: u32,
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub cwd: PathBuf,
    /// Path to the parent session jsonl when this session was spawned as a
    /// subagent run; absent for top-level sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session: Option<PathBuf>,
}

/// Child subagent session logs now emit a short `session_info` banner after
/// the root `session` header so parents can label nested runs without reusing
/// the UUID-shaped top-level session payload.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionInfoLine {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelChangeLine {
    pub id: String,
    pub parent_id: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub provider: Provider,
    pub model_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThinkingLevelChangeLine {
    pub id: String,
    pub parent_id: String,
    pub timestamp: DateTime<Utc>,
    pub thinking_level: ThinkingLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompactionLine {
    pub id: String,
    pub parent_id: String,
    pub timestamp: DateTime<Utc>,
    pub summary: String,
    pub first_kept_entry_id: String,
    pub tokens_before: u64,
    pub details: CompactionDetails,
    pub from_hook: bool,
}

/// Branch summaries snapshot the detour taken on another conversation branch
/// so the active branch can reference it without replaying every message.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BranchSummaryLine {
    pub id: String,
    pub parent_id: String,
    pub timestamp: DateTime<Utc>,
    pub from_id: String,
    pub summary: String,
    pub details: CompactionDetails,
    pub from_hook: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompactionDetails {
    pub read_files: Vec<PathBuf>,
    pub modified_files: Vec<PathBuf>,
}

// ---------------------------------------------------------------------------
// Custom / custom_message
//
// Both of these have a discriminator (`customType`) that lives at the outer
// level alongside `id`, `parentId`, and `timestamp`. We keep those as normal
// fields and `#[serde(flatten)]` an adjacently-tagged enum so that the
// discriminator selects a strongly typed payload. Because the flattened
// enums use both `tag` and `content`, the discriminator and the variant
// body live in their own JSON keys, so the outer wrappers stay strict via
// `deny_unknown_fields` and catch any unknown sibling keys.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomLine {
    pub id: String,
    pub parent_id: String,
    pub timestamp: DateTime<Utc>,
    #[serde(flatten)]
    pub payload: CustomPayload,
}

/// Adjacently tagged enum selected by `customType` with the typed body living
/// under `data`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "customType", content = "data")]
pub enum CustomPayload {
    #[serde(rename = "plannotator")]
    Plannotator(PlannotatorData),
    #[serde(rename = "dcp-state")]
    DcpState(DcpStateData),
    #[serde(rename = "web-search-results")]
    WebSearchResults(WebSearchResultsData),
    #[serde(rename = "plannotator-execute")]
    PlannotatorExecute(PlannotatorExecuteData),
    #[serde(rename = "intercom_sent")]
    IntercomSent(IntercomSentData),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomMessageLine {
    pub id: String,
    pub parent_id: String,
    pub timestamp: DateTime<Utc>,
    pub content: String,
    pub display: bool,
    #[serde(flatten)]
    pub payload: CustomMessagePayload,
}

/// Adjacently tagged enum selected by `customType` with the typed body living
/// under `details`. `details` is optional because some variants
/// (`plannotator-complete`) omit it entirely.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "customType", content = "details")]
pub enum CustomMessagePayload {
    #[serde(rename = "pi-loaded-tools")]
    PiLoadedTools(PiLoadedToolsDetails),
    /// Marker emitted by the plannotator extension when planning finishes;
    /// the human-readable summary lives in the outer `content` field and no
    /// structured `details` payload is attached.
    #[serde(rename = "plannotator-complete")]
    PlannotatorComplete,
    /// Synthetic message injected by the DCP loop asking the assistant to
    /// invoke the `compress` tool. Carries no extra `details` payload; the
    /// human-readable prompt lives in the outer `content` field.
    #[serde(rename = "dcp-compress-trigger")]
    DcpCompressTrigger,
    /// Surface-level notification emitted by the subagent harness when a
    /// background run finishes (success or failure). Carries no `details`
    /// payload; the human-readable summary lives in the outer `content`
    /// field.
    #[serde(rename = "subagent-notify")]
    SubagentNotify,
    /// Intercom relays render as custom messages so the UI can show the rich
    /// sender banner without teaching the top-level log format about inboxes.
    #[serde(rename = "intercom_message")]
    IntercomMessage(IntercomMessageDetails),
    /// Richer subagent notice that repeats the rendered notice text and the
    /// underlying control event that triggered it.
    #[serde(rename = "subagent_control_notice")]
    SubagentControlNotice(Box<SubagentControlNoticeDetails>),
}

// ---------------------------------------------------------------------------
// Message line + role payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MessageLine {
    pub id: String,
    pub parent_id: String,
    pub timestamp: DateTime<Utc>,
    pub message: RoleMessage,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "camelCase")]
pub enum RoleMessage {
    #[serde(rename = "user")]
    User(UserMessage),
    #[serde(rename = "assistant")]
    Assistant(Box<AssistantMessage>),
    #[serde(rename = "toolResult")]
    ToolResult(Box<ToolResultMessage>),
    #[serde(rename = "bashExecution")]
    BashExecution(Box<BashExecutionMessage>),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserMessage {
    pub content: Vec<UserContentItem>,
    pub timestamp: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantMessage {
    pub content: Vec<AssistantContentItem>,
    pub api: AssistantApi,
    pub provider: Provider,
    pub model: String,
    pub usage: AssistantUsage,
    pub stop_reason: AssistantStopReason,
    pub timestamp: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// Provider diagnostics attached when the assistant turn fails due to
    /// a transport error, rate limit, or similar provider-side issue.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiagnosticItem {
    #[serde(rename = "type")]
    pub kind: String,
    pub timestamp: i64,
    pub error: DiagnosticError,
    /// Provider-specific diagnostic payload whose shape varies by the
    /// diagnostic `type`:
    /// * `provider_transport_failure` — `{configuredTransport, eventsEmitted, phase, requestBytes}`
    pub details: JsonBlob,
}

/// Standard JavaScript Error shape embedded in provider diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiagnosticError {
    pub name: String,
    pub message: String,
    pub stack: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultMessage {
    pub tool_call_id: String,
    pub tool_name: String,
    pub content: Vec<TextContentItem>,
    pub is_error: bool,
    pub timestamp: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<ToolResultDetails>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawToolResultMessage {
    pub tool_call_id: String,
    pub tool_name: String,
    pub content: Vec<TextContentItem>,
    pub is_error: bool,
    pub timestamp: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

/// Pi can emit `null` or omit `details` entirely when no structured result is
/// available; both map to `None` so callers see a uniform absent-details state.
/// Empty error objects (`{}`) are dropped for most tools because pi uses them as
/// a generic "error occurred" sentinel with no payload, but `memory` and `skill`
/// are exceptions where `{}` is the real validation-error body the extension
/// reads back — those tools are kept by [`preserves_empty_error_details`].
fn resolve_tool_result_details(
    raw_details: Option<Value>,
    tool_name: &str,
    is_error: bool,
) -> Result<Option<ToolResultDetails>, serde_json::Error> {
    let details = match raw_details {
        Some(value) => value,
        None => return Ok(None),
    };
    if details.is_null() {
        return Ok(None);
    }
    if let Value::Object(ref map) = details {
        let drop_empty_error_object = is_error && !preserves_empty_error_details(tool_name);
        if map.is_empty() && drop_empty_error_object {
            return Ok(None);
        }
    }
    parse_tool_result_details(tool_name, details).map(Some)
}

impl<'de> Deserialize<'de> for ToolResultMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let RawToolResultMessage {
            tool_call_id,
            tool_name,
            content,
            is_error,
            timestamp,
            details: raw_details,
        } = RawToolResultMessage::deserialize(deserializer)?;
        let resolved = resolve_tool_result_details(raw_details, &tool_name, is_error);
        let details = resolved.map_err(de::Error::custom)?;
        Ok(Self {
            tool_call_id,
            tool_name,
            content,
            is_error,
            timestamp,
            details,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BashExecutionMessage {
    pub command: String,
    pub output: String,
    /// `None` when the command was cancelled or interrupted before producing an exit code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub cancelled: bool,
    pub truncated: bool,
    pub timestamp: i64,
    pub exclude_from_context: bool,
    /// When the response would exceed pi's in-message byte cap, pi spills
    /// the raw command output to a temp file and exposes the path here so a
    /// caller can read the untruncated output without re-running the
    /// command. `None` means no overflow occurred.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_output_path: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// Content items
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum UserContentItem {
    Text { text: String },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AssistantContentItem {
    #[serde(rename = "text")]
    Text(TextAssistantContent),
    #[serde(rename = "thinking")]
    Thinking(ThinkingAssistantContent),
    // Boxed because ToolCallContent is much larger than Text or Thinking.
    #[serde(rename = "toolCall")]
    ToolCall(Box<ToolCallContent>),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextAssistantContent {
    pub text: String,
    /// Opaque provider-supplied signature. Sometimes an opaque token,
    /// sometimes a JSON-encoded object stored as a string. Kept as a
    /// plain string so we never have to speculate about its contents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThinkingAssistantContent {
    pub thinking: String,
    /// Absent on aborted assistant turns where the model produced no
    /// thinking content (and therefore no signature to attest to).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_signature: Option<ThinkingSignature>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextContentItem {
    #[serde(rename = "type")]
    pub kind: TextContentKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextContentKind {
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolCallContent {
    pub id: String,
    pub name: String,
    pub arguments: ToolCallArguments,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_json: Option<String>,
}

impl ToolCallContent {
    pub fn name(&self) -> &str {
        &self.name
    }
}

// ---------------------------------------------------------------------------
// Usage + cost
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssistantUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total_tokens: u64,
    pub cost: UsageCost,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsageCost {
    pub input: Decimal,
    pub output: Decimal,
    pub cache_read: Decimal,
    pub cache_write: Decimal,
    pub total: Decimal,
    /// Origin of the cost values (e.g. "provider", "pi"). Pi adds this field;
    /// optional for backward compatibility with older log files that lack it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

// ---------------------------------------------------------------------------
// Discriminator enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Anthropic,
    #[serde(rename = "openai")]
    OpenAi,
    #[serde(rename = "openai-codex")]
    OpenAiCodex,
    #[serde(rename = "openrouter")]
    OpenRouter,
    #[serde(rename = "faux")]
    Faux,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel {
    Off,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssistantApi {
    /// One of the well-known API protocol identifiers (anthropic-messages,
    /// openai-responses, openai-completions).
    Known(ApiKind),
    /// Faux-internal API routing identifier of the form
    /// `"faux:<session-id>:<worker-id>"`. This is not a standard API protocol
    /// but a pi internal routing label that should be preserved rather than
    /// rejected, so faux sessions produce usable reports.
    Faux(String),
}

// Custom serde to keep the wire format as a plain string rather than an
// externally tagged JSON object. Deserialize enforces the narrow set of
// accepted strings; Serialize preserves the same shape for round-tripping.
impl serde::Serialize for AssistantApi {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            AssistantApi::Known(kind) => kind.serialize(serializer),
            AssistantApi::Faux(api) => serializer.serialize_str(api),
        }
    }
}

impl<'de> Deserialize<'de> for AssistantApi {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        match s.as_str() {
            "anthropic-messages" => return Ok(AssistantApi::Known(ApiKind::AnthropicMessages)),
            "openai-codex-responses" => {
                return Ok(AssistantApi::Known(ApiKind::OpenAiCodexResponses))
            }
            "openai-completions" => return Ok(AssistantApi::Known(ApiKind::OpenAiCompletions)),
            "openai-responses" => return Ok(AssistantApi::Known(ApiKind::OpenAiResponses)),
            _ if s.starts_with("faux:") => return Ok(AssistantApi::Faux(s)),
            _ => {}
        }
        Err(serde::de::Error::unknown_variant(
            &s,
            &[
                "anthropic-messages",
                "openai-codex-responses",
                "openai-completions",
                "openai-responses",
                "faux:*",
            ],
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApiKind {
    AnthropicMessages,
    #[serde(rename = "openai-codex-responses")]
    OpenAiCodexResponses,
    #[serde(rename = "openai-completions")]
    OpenAiCompletions,
    #[serde(rename = "openai-responses")]
    OpenAiResponses,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AssistantStopReason {
    #[serde(rename = "toolUse")]
    ToolUse,
    #[serde(rename = "stop")]
    Stop,
    #[serde(rename = "aborted")]
    Aborted,
    #[serde(rename = "error")]
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolSource {
    Builtin,
    Extension,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolScope {
    Temporary,
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolOrigin {
    TopLevel,
    Package,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlannotatorPhase {
    Idle,
    Planning,
    Executing,
}

// ---------------------------------------------------------------------------
// Signatures
// ---------------------------------------------------------------------------

/// Providers sometimes hand back a signature token whose internal structure is
/// undocumented, so we preserve the raw string instead of guessing how to
/// decode it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ThinkingSignature {
    Opaque(String),
    Structured(StructuredThinkingSignature),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredThinkingSignature {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub encrypted_content: String,
    pub summary: Vec<String>,
}

// ---------------------------------------------------------------------------
// Tool call arguments
// ---------------------------------------------------------------------------

/// Pi records the model-emitted `arguments` object before the tool runtime has
/// a chance to validate it, so the parser preserves the raw JSON map instead of
/// hard-coding per-tool schemas into `ToolCallContent`.
pub type ToolCallArguments = BTreeMap<String, JsonBlob>;

// ---------------------------------------------------------------------------
// Typed helper structs for known tool schemas
// ---------------------------------------------------------------------------

/// Common shape for the `git_read_only_*` MCP tools surfaced by
/// `pi-tool-display`. Every variant takes the same `{project_dir, args}`
/// pair, so we share a single struct.
///
/// Unlike most arg structs in this file, `GitReadOnlyArgs` deliberately does
/// not declare `rename_all = "camelCase"`. The MCP tool definitions emit
/// arguments in snake_case (`project_dir`), so the field names already match
/// the wire format verbatim.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitReadOnlyArgs {
    pub project_dir: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompressArgs {
    pub topic: String,
    pub ranges: Vec<CompressRange>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompressRange {
    pub start_id: String,
    pub end_id: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentStatusArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TodoArgs {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_form: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_by: Option<Vec<u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add_blocked_by: Option<Vec<u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove_blocked_by: Option<Vec<u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_deleted: Option<bool>,
    /// Some assistant-side todo updates attach extra blocker context under a
    /// free-form `metadata` object even though the user-facing tool schema does
    /// not advertise it; preserve that payload instead of failing the log line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonBlob>,
}

/// Numeric tool-call arguments that pi normally records as integers, but
/// which corrupted model streams have been observed to echo back the field
/// name as a string value (`"limit": "limit"`, `"offset": "offset"`).
///
/// `Garbage` is therefore only valid when a field-specific deserializer sees
/// that exact echoed name for its own field. Other strings stay loud so new
/// corruption modes do not silently become part of the accepted schema.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MaybeU32 {
    Number(u32),
    Garbage(String),
}

impl Serialize for MaybeU32 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Number(value) => serializer.serialize_u32(*value),
            Self::Garbage(value) => serializer.serialize_str(value),
        }
    }
}

fn parse_named_maybe_u32_value(
    value: Value,
    field_name: &'static str,
) -> Result<MaybeU32, serde_json::Error> {
    match value {
        Value::String(string) if string == field_name => Ok(MaybeU32::Garbage(string)),
        Value::String(string) => Err(serde_json::Error::io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid string value `{string}` for `{field_name}`; expected `{field_name}`"),
        ))),
        other => serde_json::from_value::<u32>(other).map(MaybeU32::Number),
    }
}

fn deserialize_named_maybe_u32_field<'de, D>(
    deserializer: D,
    field_name: &'static str,
) -> Result<Option<MaybeU32>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    parse_named_maybe_u32_value(value, field_name)
        .map(Some)
        .map_err(de::Error::custom)
}

fn deserialize_offset_field<'de, D>(deserializer: D) -> Result<Option<MaybeU32>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_named_maybe_u32_field(deserializer, "offset")
}

fn deserialize_limit_field<'de, D>(deserializer: D) -> Result<Option<MaybeU32>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_named_maybe_u32_field(deserializer, "limit")
}

fn deserialize_required_nullable_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadArgs {
    /// Optional because aborted toolCalls (`stopReason: "aborted"`) can
    /// land in the log with `arguments: {}` before the model finished
    /// streaming a path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(
        default,
        deserialize_with = "deserialize_offset_field",
        skip_serializing_if = "Option::is_none"
    )]
    pub offset: Option<MaybeU32>,
    #[serde(
        default,
        deserialize_with = "deserialize_limit_field",
        skip_serializing_if = "Option::is_none"
    )]
    pub limit: Option<MaybeU32>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BashArgs {
    /// Optional because aborted toolCalls can record `arguments: {}` when
    /// the model never finished emitting the command string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriteArgs {
    pub path: PathBuf,
    pub content: String,
}

/// Pi has emitted two shapes for `edit` tool arguments over time:
/// 1. Modern multi-edit form: `{path, edits: [{oldText, newText}, ...]}`.
/// 2. Older single-edit shorthand: `{path, oldText, newText}` with no
///    `edits` array.
///
/// Both `edits` and the `(old_text, new_text)` pair are therefore optional,
/// with the invariant that exactly one shape is populated for a well-formed
/// call. `path` is also optional because aborted toolCalls can land here
/// with `arguments: {}`.
///
/// `deny_unknown_fields` is intentionally NOT applied here: completed-but-
/// corrupted model streams have been observed emitting hallucinated
/// top-level sibling keys such as `},{` whose values are also garbage
/// fragments. We silently drop those rather than fail the whole log.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edits: Option<Vec<EditEntry>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_text: Option<String>,
}

/// One element of the `edits` array. Normally a structured replacement,
/// but completed-but-corrupted model streams sometimes intersperse raw
/// JSON fragments (e.g. `","`, `"},"`) between real entries; we capture
/// those as `Fragment` so the surrounding call still parses.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EditEntry {
    Full(EditReplacement),
    Fragment(String),
}

/// All fields are optional to tolerate truncated / errored streaming
/// where the assistant message has `stopReason: "error"` and one of the
/// entries in `edits` is missing a half before the JSON parser gave up.
/// `description` is a recently-added free-form annotation pi attaches to
/// each replacement (e.g. "Encode the user's two decisions...").
///
/// `deny_unknown_fields` is intentionally absent: models occasionally
/// emit hallucinated sibling keys (e.g. `newText_TYPO_GUARD`) or stream
/// out structurally-valid objects with garbage keys like `},` / `]` /
/// `:` mid-edit. Tolerating unknown keys keeps those completed-but-
/// corrupt tool calls parseable instead of poisoning whole sessions.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditReplacement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// `ctx_cache` is the only `ctx_*` extension tool we keep a typed helper
/// schema for. Unlike its siblings (which only ever appear in the
/// `pi-loaded-tools` manifest), `ctx_cache` is invoked directly by the
/// assistant in real session logs, so documenting its observed argument
/// shape is useful for post-parse consumers even though tool-call
/// arguments stay raw JSON in `ToolCallContent`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CtxCacheArgs {
    pub action: String,
    pub path: PathBuf,
}

/// `deny_unknown_fields` is intentionally omitted. Models occasionally
/// hallucinate sibling keys here — we've observed gpt-5.4 emitting
/// `:path` alongside `path`, Sonnet emitting an `offset` parameter that grep
/// does not support, and aborted tool-call streams landing as an empty `{}`.
/// Tolerating unknown fields and defaulting `pattern` keeps those partial
/// traces parseable without pretending grep's real runtime schema is looser.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrepArgs {
    #[serde(default)]
    pub pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glob: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignore_case: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub literal: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// `pattern` and `limit` accept the observed dotted-key corruption aliases.
/// Some completed tool calls have emitted a leading `.` in those keys, and we
/// want the malformed arguments to stay tied to their intended fields instead
/// of poisoning the whole log line.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FindArgs {
    #[serde(alias = ".pattern")]
    pub pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(default, alias = ".limit", skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LsArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AskUserArgs {
    pub question: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<AskUserOption>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_multiple: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_freeform: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_comment: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_mode: Option<AskUserDisplayMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlay_toggle_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment_toggle_key: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskUserDisplayMode {
    Overlay,
    Inline,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AskUserOption {
    Title(String),
    Detailed(AskUserDetailedOption),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AskUserDetailedOption {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeSearchArgs {
    pub query: String,
    pub max_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebSearchArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queries: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_results: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_content: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopeDomainArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

pub type FactListArgs = ScopeDomainArgs;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IdArgs {
    pub id: String,
}

pub type FactReadArgs = IdArgs;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservationCountersArgs {
    #[serde(
        default,
        alias = "observation_count",
        skip_serializing_if = "Option::is_none"
    )]
    pub observation_count: Option<u32>,
    #[serde(
        default,
        alias = "confirmed_count",
        skip_serializing_if = "Option::is_none"
    )]
    pub confirmed_count: Option<u32>,
    #[serde(
        default,
        alias = "contradicted_count",
        skip_serializing_if = "Option::is_none"
    )]
    pub contradicted_count: Option<u32>,
    #[serde(
        default,
        alias = "inactive_count",
        skip_serializing_if = "Option::is_none"
    )]
    pub inactive_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactWriteArgs {
    pub id: String,
    pub title: String,
    pub content: String,
    pub confidence: Decimal,
    pub domain: String,
    pub scope: String,
    #[serde(flatten)]
    pub counters: ObservationCountersArgs,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OptionalScopeIdArgs {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

pub type FactDeleteArgs = OptionalScopeIdArgs;

/// Both `url` and `urls` are optional because the caller passes one or the
/// other (single fetch vs batch); aborted tool calls may also land with
/// neither set. Both being `Some` is malformed but parses without error
/// because we cannot express "exactly one of" in serde without a custom
/// deserializer; downstream analysis is responsible for flagging that case.
/// `prompt` is Gemini-specific (used to direct video/page analysis) and is
/// absent for plain Readability extraction.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FetchContentArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub urls: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_clone: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// `response_id` is the only required field because it is the cache key
/// referencing the prior `fetch_content` / `web_search` call whose body is
/// being replayed; without it there is nothing to look up. The four
/// optional fields form two independent selection axes for picking a
/// specific entry inside that cached response: `query`/`query_index` for
/// search results, `url`/`url_index` for fetched pages. Mixing axes is
/// caller error but parses successfully; with all four absent the entire
/// cached response is returned.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetSearchContentArgs {
    pub response_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url_index: Option<u32>,
}

pub type InstinctListArgs = ScopeDomainArgs;

pub type InstinctReadArgs = IdArgs;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstinctWriteArgs {
    pub id: String,
    pub title: String,
    pub trigger: String,
    pub action: String,
    pub confidence: Decimal,
    pub domain: String,
    pub scope: String,
    #[serde(flatten)]
    pub counters: ObservationCountersArgs,
}

pub type InstinctDeleteArgs = OptionalScopeIdArgs;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstinctMergeArgs {
    pub merged: MergedInstinct,
    pub delete_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete_scoped_ids: Option<Vec<ScopedInstinctDelete>>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MergedInstinct {
    pub id: String,
    pub title: String,
    pub trigger: String,
    pub action: String,
    pub confidence: Decimal,
    pub domain: String,
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopedInstinctDelete {
    pub id: String,
    pub scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContactSupervisorArgs {
    pub reason: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntercomArgs {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<IntercomAttachment>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntercomAttachment {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub describe: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Server name passed when the caller wants to force a connect rather
    /// than just listing tools (`mcp({connect: "..."})`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlannotatorSubmitPlanArgs {
    /// Path to the plan markdown file. Present in current-format calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<PathBuf>,
    /// Free-form plan summary. Present in older-format calls that pre-date
    /// the file-path argument.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntercomSentData {
    pub to: String,
    pub message: JsonBlob,
    pub message_id: String,
    pub timestamp: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent: Option<JsonBlob>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntercomMessageDetails {
    pub from: JsonBlob,
    pub message: JsonBlob,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_command: Option<String>,
    pub body_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tasks: Option<Vec<SubagentTask>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain: Option<Vec<SubagentChainStep>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_dir: Option<PathBuf>,
    /// Raw key in the JSON is `async`; Rust keyword, so we rename.
    #[serde(rename = "async", default, skip_serializing_if = "Option::is_none")]
    pub async_: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_progress: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clarify: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<SubagentControlArgs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<SubagentOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<SubagentSkill>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// JSON-encoded agent or chain configuration passed to `subagent` management
    /// actions (`create`, `update`). Kept as a string because pi historically
    /// serialized the full config blob as a string argument.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
    /// Scope filter for agent management subcommands (`list`, `get`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_scope: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentTask {
    pub agent: String,
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<SubagentOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reads: Option<SubagentReads>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<SubagentSkill>,
}

/// Pi reuses booleans here as feature toggles and strings as explicit output
/// paths, so the parser has to accept both wire shapes without inventing a new
/// tagged wrapper that never appears in session logs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SubagentOutput {
    Path(String),
    Enabled(bool),
}

/// `reads` follows the same boolean-or-array convention as `output`: `false`
/// disables pre-reads, while a string array records the exact files pi loaded.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SubagentReads {
    Files(Vec<String>),
    Enabled(bool),
}

/// `skill` is the most permissive subagent selector because pi can serialize it
/// as a feature toggle, a single skill name, or a list of names.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SubagentSkill {
    Names(Vec<String>),
    Enabled(bool),
    Name(String),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentControlArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_attention_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_notice_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_notice_after_turns: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_notice_after_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_tool_attempts_before_attention: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify_on: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify_channels: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentChainStep {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<SubagentOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reads: Option<SubagentReads>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<SubagentSkill>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel: Option<Vec<SubagentParallelTask>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fail_fast: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentParallelTask {
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<SubagentOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reads: Option<SubagentReads>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<SubagentSkill>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

// ---------------------------------------------------------------------------
// Tool result details
//
// `details` on a toolResult message varies by tool. `ToolResultMessage`
// deserializes the raw JSON through the surrounding `tool_name` first so
// ambiguous all-optional shapes like bash's `{compression}` breadcrumb stay
// attached to the originating tool. The untagged enum remains as a strict
// fallback for tools whose detail payloads are shared or still shape-routed.
// ---------------------------------------------------------------------------

fn is_empty_details_object(details: &Value) -> bool {
    matches!(details, Value::Object(map) if map.is_empty())
}

fn preserves_empty_error_details(tool_name: &str) -> bool {
    matches!(tool_name, "memory" | "skill")
}

fn parse_tool_result_details(
    tool_name: &str,
    details: Value,
) -> Result<ToolResultDetails, serde_json::Error> {
    match tool_name {
        "ask_user" => serde_json::from_value(details).map(ToolResultDetails::AskUser),
        "bash" => serde_json::from_value(details).map(ToolResultDetails::Bash),
        "code_search" => serde_json::from_value(details).map(ToolResultDetails::CodeSearch),
        "compress" => serde_json::from_value(details).map(ToolResultDetails::Compress),
        "contact_supervisor" => {
            serde_json::from_value(details).map(ToolResultDetails::ContactSupervisor)
        }
        "edit" => serde_json::from_value(details).map(ToolResultDetails::Edit),
        "fetch_content" => serde_json::from_value(details).map(ToolResultDetails::FetchContent),
        "fact_list" | "instinct_list" => {
            serde_json::from_value(details).map(ToolResultDetails::Count)
        }
        "find" => serde_json::from_value(details).map(ToolResultDetails::Find),
        "get_search_content" => {
            serde_json::from_value(details).map(ToolResultDetails::GetSearchContent)
        }
        "grep" => serde_json::from_value(details).map(ToolResultDetails::Grep),
        "git_read_only_diff"
        | "git_read_only_log"
        | "git_read_only_show"
        | "git_read_only_status" => {
            serde_json::from_value(details).map(ToolResultDetails::GitReadOnly)
        }
        "instinct_write" => serde_json::from_value(details).map(ToolResultDetails::InstinctWrite),
        "intercom" => serde_json::from_value(details).map(ToolResultDetails::Intercom),
        "ls" => serde_json::from_value(details).map(ToolResultDetails::Ls),
        "memory" => {
            if is_empty_details_object(&details) {
                Ok(ToolResultDetails::Empty(EmptyDetails {}))
            } else {
                serde_json::from_value(details).map(ToolResultDetails::Memory)
            }
        }
        "memory_search" | "session_search" => {
            serde_json::from_value(details).map(ToolResultDetails::SearchResult)
        }
        "mcp" => serde_json::from_value(details).map(ToolResultDetails::Mcp),
        "project_tools_run_build"
        | "project_tools_run_formatter"
        | "project_tools_run_lint"
        | "project_tools_run_tests"
        | "jj_read_only_run" => {
            serde_json::from_value(details).map(ToolResultDetails::McpToolResult)
        }
        "plannotator_submit_plan" => {
            serde_json::from_value(details).map(ToolResultDetails::PlannotatorSubmitPlan)
        }
        "read" => serde_json::from_value(details).map(ToolResultDetails::Read),
        "skill" => {
            if is_empty_details_object(&details) {
                Ok(ToolResultDetails::Empty(EmptyDetails {}))
            } else {
                serde_json::from_value(details).map(ToolResultDetails::Skill)
            }
        }
        "subagent" => serde_json::from_value(details).map(ToolResultDetails::Subagent),
        "todo" => serde_json::from_value(details).map(ToolResultDetails::Todo),
        "web_search" => serde_json::from_value(details).map(ToolResultDetails::WebSearch),
        _ => serde_json::from_value(details),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolResultDetails {
    Edit(EditDetails),
    Subagent(SubagentResultDetails),
    AskUser(AskUserDetails),
    CodeSearch(CodeSearchDetails),
    ContactSupervisor(ContactSupervisorResultDetails),
    WebSearch(WebSearchDetails),
    // Grep precedes Read for direct `ToolResultDetails` shape matching
    // because both accept `{matchLimitReached, linesTruncated}` and
    // ReadDetails is permissive enough to absorb that payload too. Normal
    // `ToolResultMessage` parsing routes `tool_name: "grep"` and
    // `tool_name: "read"` explicitly before it ever falls back to this
    // untagged enum, but we keep the ordering deterministic for direct enum
    // deserialization and unknown-tool fallback paths.
    Grep(GrepDetails),
    Read(ReadDetails),
    Count(CountDetails),
    // `memory_search` and `session_search` share the same compact
    // success/count/message envelope, so one typed variant avoids duplicating
    // their parser surface.
    SearchResult(SearchResultDetails),
    Intercom(IntercomResultDetails),
    Mcp(McpDetails),
    McpToolResult(McpToolResult),
    Bash(BashDetails),
    PlannotatorSubmitPlan(PlannotatorSubmitPlanDetails),
    Todo(TodoDetails),
    Compress(CompressDetails),
    // InstinctWriteDetails has shape {id, action}; no other variant in this
    // enum declares both fields, so untagged dispatch routes uniquely.
    InstinctWrite(InstinctWriteDetails),
    // Ls precedes Find because their lean-ctx augmentation shapes overlap on
    // path/source/truncated/compression. The discriminator is `pattern`:
    // find's lean-ctx output always carries it, ls never does. With Ls first,
    // a payload without `pattern` matches Ls; a payload with `pattern` is
    // rejected by LsDetails (deny_unknown_fields) and falls through to Find.
    Ls(LsDetails),
    Find(FindDetails),
    // McpToolResult::Breadcrumb and GitReadOnly both declare `{server,
    // tool}`; Breadcrumb hits first because McpToolResult is ordered
    // above. `git_read_only_*` tools are still routed to `GitReadOnly`
    // explicitly by `parse_tool_result_details`, so the variant ordering
    // only affects the shape-based fallback path for unknown tools.
    //
    // FetchContent has no shape overlap with anything above (it declares
    // `urls`, `urlCount`, ... that no other variant carries).
    //
    // GetSearchContent is dual-shape: its Success arm is uniquely
    // identified by `{url, title, contentLength}`, and its Error arm is
    // a bare `{error}` payload. Earlier variants that also declare an
    // `error` field (CodeSearchDetails, McpDetails, TodoDetails,
    // SubagentResultDetails-via-Subagent, WebSearchDetails) all require
    // additional discriminator fields (e.g. `query`+`maxTokens`,
    // `mode`, `action`+`params`+`nextId`, `mode`+`results`,
    // `queryCount`+`successfulQueries`+...), so a bare `{error}` cannot
    // be absorbed by any of them and safely falls through here.
    GitReadOnly(GitReadOnlyDetails),
    FetchContent(FetchContentDetails),
    GetSearchContent(GetSearchContentDetails),
    // Hermes `memory` and `skill` are intentionally parsed by `tool_name`
    // first because their error shapes can collapse to `{}` or a bare
    // `{error}` and would otherwise be ambiguous in direct untagged
    // deserialization.
    Empty(EmptyDetails),
    Memory(MemoryDetails),
    Skill(SkillDetails),
}

/// Compression breadcrumb appended to tool results that flowed through the
/// `lean-ctx` extension. The extension records how many tokens it saved
/// versus the raw tool output.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompressionInfo {
    pub original_tokens: u32,
    pub compressed_tokens: u32,
    /// Signed because pathological inputs can grow under compression.
    pub percent_saved: i32,
}

/// Closed enum so any new `source` value introduced upstream surfaces as a
/// loud parse error rather than silently being dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolResultSource {
    LeanCtx,
}

/// `contact_supervisor` results currently only route an `error` marker in
/// `details`; the human-readable outcome remains in the tool-result text.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContactSupervisorResultDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<bool>,
}

/// `ls` tool results are always either a plain listing (no `details`) or a
/// lean-ctx augmented listing with this shape. `entry_limit_reached` is
/// orthogonal to the lean-ctx augmentation and reports the truncation cap
/// when the directory had more entries than the tool was willing to emit.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LsDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ToolResultSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression: Option<CompressionInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_limit_reached: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditDetails {
    pub diff: String,
    /// Ni emits this alongside `diff`; keep it optional so upstream pi logs
    /// without the fork-specific field still parse strictly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_changed_line: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubagentResultMode {
    Async,
    Management,
    Parallel,
    Single,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentResultDetails {
    pub mode: SubagentResultMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub results: Vec<SubagentResultSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<SubagentArtifacts>,
    /// Inheritance mode the parent passed to the subagent (for example
    /// "fork" when the child inherits the parent conversation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Per-result progress snapshots reported while the subagent was still
    /// running. Present for streaming runs and elided when the agent ran
    /// to completion before any progress event was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<Vec<SubagentProgressEntry>>,
    /// Run id assigned to an `async` subagent invocation; absent for
    /// synchronous runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub async_id: Option<String>,
    /// Working directory where the async run is staging its artifacts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub async_dir: Option<PathBuf>,
}

/// One streaming progress record per subagent result. The pi runtime emits
/// these while the child is still active so the parent can surface activity
/// without waiting for completion.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentProgressEntry {
    pub index: u32,
    pub agent: String,
    pub status: String,
    pub task: String,
    pub tool_count: u32,
    pub tokens: u64,
    pub duration_ms: u64,
    pub recent_tools: Vec<String>,
    pub recent_output: Vec<String>,
}

// --- Acceptance ledger types ---

/// Status values are free-form strings rather than a Rust enum because pi may
/// add new statuses in future releases.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcceptanceLedger {
    pub status: String,
    pub explicit: bool,
    pub effective_acceptance: ResolvedAcceptanceConfig,
    pub inferred_reason: Vec<String>,
    pub criteria: Vec<ResolvedAcceptanceGate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_report: Option<AcceptanceReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_report_parse_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_child_report: Option<AcceptanceReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_child_report_parse_error: Option<String>,
    pub runtime_checks: Vec<AcceptanceRuntimeCheck>,
    pub verify_runs: Vec<AcceptanceVerifyResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_result: Option<AcceptanceReviewResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalization: Option<AcceptanceFinalizationLedger>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_decision: Option<AcceptanceParentDecision>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedAcceptanceConfig {
    pub level: String,
    pub explicit: bool,
    pub inferred_reason: Vec<String>,
    pub criteria: Vec<ResolvedAcceptanceGate>,
    pub evidence: Vec<String>,
    pub verify: Vec<AcceptanceVerifyCommand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<AcceptanceReviewGate>,
    pub stop_rules: Vec<String>,
    pub finalization: AcceptanceFinalizationConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcceptanceFinalizationConfig {
    pub mode: String,
    pub max_turns: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedAcceptanceGate {
    pub id: String,
    pub must: String,
    pub evidence: Vec<String>,
    pub severity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcceptanceVerifyCommand {
    pub id: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_failure: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcceptanceReviewGate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcceptanceReport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criteria_satisfied: Option<Vec<AcceptanceCriterionResult>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed_files: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tests_added_or_updated: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commands_run: Option<Vec<AcceptanceCommandResult>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_output: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub residual_risks: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_staged_files: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_findings: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcceptanceCriterionResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub status: String,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcceptanceCommandResult {
    pub command: String,
    pub result: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcceptanceRuntimeCheck {
    pub id: String,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcceptanceVerifyResult {
    pub id: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcceptanceReviewResult {
    pub status: String,
    pub findings: Vec<AcceptanceFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcceptanceFinding {
    pub severity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    pub issue: String,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcceptanceFinalizationLedger {
    pub mode: String,
    pub status: String,
    pub max_turns: u32,
    pub turns: Vec<AcceptanceFinalizationTurn>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcceptanceFinalizationTurn {
    pub turn: u32,
    pub prompt: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<AcceptanceReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parse_error: Option<String>,
    pub runtime_checks: Vec<AcceptanceRuntimeCheck>,
    pub verify_runs: Vec<AcceptanceVerifyResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcceptanceParentDecision {
    pub status: String,
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentResultSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<SubagentUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_paths: Option<SubagentArtifactPaths>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_summary: Option<SubagentProgressSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_output_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_reference: Option<SubagentOutputReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempted_models: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_attempts: Option<Vec<SubagentModelAttempt>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<SubagentToolCallSummary>>,
    /// Control-channel events emitted by the pi subagent runtime while the
    /// child was running (for example the `active_long_running` notice). The
    /// parent surfaces these so downstream consumers can correlate notices
    /// with the per-result usage and timing summary above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_events: Option<Vec<SubagentControlEvent>>,
    /// When a subagent runs with an explicit acceptance contract, the runtime
    /// records the acceptance ledger including criteria status, runtime checks,
    /// verify runs, and optional finalization turns. Absent when acceptance was
    /// not configured for the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance: Option<AcceptanceLedger>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentOutputReference {
    pub path: PathBuf,
    pub bytes: u64,
    pub lines: u64,
    pub message: String,
}

/// Internally-tagged on `type` because pi's subagent runtime emits each
/// control event with a closed set of discriminator values; if a new
/// variant ships upstream we want a loud parse failure rather than a
/// silent drop, matching the rest of this parser's strict-by-default
/// posture.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SubagentControlEvent {
    ActiveLongRunning(SubagentControlEventPayload),
    NeedsAttention(SubagentControlEventPayload),
}

/// Both currently observed control-event variants share one payload schema;
/// keeping that shape in one struct prevents the two arms from drifting as the
/// runtime adds optional observability fields like `currentPath` and newer
/// state-transition metadata.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentControlEventPayload {
    /// Transition target reported by the runtime state machine. Currently
    /// observed to equal the event type, but kept as a separate field
    /// because the runtime models it as a distinct concept.
    pub to: String,
    /// Newer runtimes report the previous control state when an event reflects
    /// a state transition (for example `active_long_running` →
    /// `needs_attention`). Older logs omit it entirely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Wall-clock timestamp of the event in milliseconds since the Unix epoch.
    pub ts: u64,
    pub run_id: String,
    pub agent: String,
    /// Index of the parallel result this event is attributed to; matches the
    /// position of the owning entry in `SubagentResultDetails::results`.
    pub index: u32,
    pub message: String,
    /// Free-form reason string (for example `turn_threshold`,
    /// `tokens_threshold`, `time_threshold`). Left as `String` because the
    /// runtime threshold knobs are user-configurable and the full set of
    /// reasons is not documented as a closed protocol enum.
    pub reason: String,
    pub turns: u32,
    pub tokens: u64,
    pub tool_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_tool_duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_path: Option<PathBuf>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost: Decimal,
    pub turns: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentArtifactPaths {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub jsonl_path: PathBuf,
    pub metadata_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentControlNoticeDetails {
    pub event: SubagentControlEvent,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_intercom_target: Option<String>,
    pub notice_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentProgressSummary {
    pub tool_count: u32,
    pub tokens: u64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentModelAttempt {
    pub model: String,
    pub success: bool,
    pub exit_code: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub usage: SubagentUsage,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentToolCallSummary {
    pub text: String,
    pub expanded_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentArtifacts {
    pub dir: PathBuf,
    pub files: Vec<SubagentArtifactPaths>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AskUserDetails {
    pub question: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub options: Vec<AskUserOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<AskUserResponse>,
    pub cancelled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum AskUserResponse {
    #[serde(rename = "selection")]
    Selection {
        selections: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        comment: Option<String>,
    },
    /// User typed a freeform answer instead of picking an option. Pi
    /// records the entered text under `text`.
    #[serde(rename = "freeform")]
    Freeform { text: String },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeSearchDetails {
    pub query: String,
    pub max_tokens: u32,
    /// Search strategy label emitted by the runtime (for example
    /// `"web-search-fallback"`). This is left open-ended because the set of
    /// fallback modes is not part of a documented closed protocol.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CuratedQuerySource {
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CuratedQueryInfo {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<CuratedQuerySource>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchSummary {
    pub text: String,
    pub workflow: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_estimate: Option<u64>,
    #[serde(default)]
    pub fallback_used: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
    #[serde(default)]
    pub edited: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebSearchDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetch_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_id: Option<String>,
    pub query_count: u32,
    pub successful_queries: u32,
    pub total_results: u32,
    pub include_content: bool,
    pub queries: Vec<String>,
    #[serde(default)]
    pub curated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curated_from: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curated_queries: Option<Vec<CuratedQueryInfo>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<SearchSummary>,
}

/// `read` emits two sub-shapes for `details` that classify here: a plain
/// truncation block when raw byte/line caps fired, or a lean-ctx augmented
/// summary describing how the extension compressed the response. All fields
/// are optional because either may be present alone, both together, or
/// neither (for plain successful reads).
///
/// A third shape - the pattern-scoped match summary `read` emits when it
/// performed grep-style filtering and hit its match or line cap - carries
/// only `{matchLimitReached, linesTruncated}`. `ToolResultMessage`
/// deserializes `tool_name: "read"` directly into this struct, so those two
/// fields are part of the real routed contract for read results. The same
/// payload also fits `GrepDetails`, which is why `Grep` still precedes
/// `Read` in the raw untagged `ToolResultDetails` enum used for direct
/// shape matching and fallback parsing.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadDetails {
    /// Present when raw `read` truncated by line/byte caps. Absent for
    /// lean-ctx augmented reads because the extension reports its own
    /// compression metrics in `compression` instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncation: Option<TruncationInfo>,
    /// Pattern-scoped read mode reuses grep's `linesTruncated` /
    /// `matchLimitReached` caps; the field names mirror `GrepDetails`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines_truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_limit_reached: Option<u32>,
    /// Lean-ctx augmentation: path the read was scoped to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ToolResultSource>,
    /// Lean-ctx selected read mode (e.g. "full", "map", "signatures").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Number of lines lean-ctx returned after compression.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression: Option<CompressionInfo>,
}

/// `find` emits three shapes for `details`: a plain `{resultLimitReached}`
/// when the result list was capped, a raw-output `{truncation}` block when
/// the serialized match list exceeded pi's message byte cap, or a lean-ctx
/// augmented shape carrying the queried path/pattern plus a `compression`
/// breadcrumb. All fields are optional because any combination of those
/// breadcrumbs may appear together.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FindDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_limit_reached: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncation: Option<TruncationInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ToolResultSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression: Option<CompressionInfo>,
}

/// Tool-result details emitted by the flat `git_read_only_*` MCP tools.
/// The pi-tool-display extension just records which MCP `server` and `tool`
/// the call was dispatched to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitReadOnlyDetails {
    pub server: String,
    pub tool: String,
}

/// Summary metadata recorded by the `fetch_content` tool. Newer failed
/// fetches can still emit the same breadcrumb shape with an added top-level
/// error summary, so the field stays optional to preserve compatibility with
/// both older success payloads and newer partial / failed runs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FetchContentDetails {
    pub urls: Vec<String>,
    pub url_count: u32,
    pub successful: u32,
    #[serde(default)]
    pub total_chars: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub response_id: String,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub has_image: bool,
    #[serde(default)]
    pub image_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Replaying a single previously-fetched URL via `get_search_content`
/// emits a small breadcrumb describing which URL was returned and how
/// large the cached body is. When the requested URL is not in the cache
/// the tool emits an error breadcrumb instead, with the available URLs
/// listed in the textual content; we model both shapes via an untagged
/// enum so the strict variants stay tight.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GetSearchContentDetails {
    Success(GetSearchContentSuccessDetails),
    Error(GetSearchContentErrorDetails),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetSearchContentSuccessDetails {
    pub url: String,
    pub title: String,
    pub content_length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetSearchContentErrorDetails {
    pub error: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "responseId"
    )]
    pub response_id: Option<String>,
}

/// Either of `matchLimitReached` / `linesTruncated` may be present when
/// raw grep hit its caps. Lean-ctx augmented grep results add the queried
/// `path`/`pattern` plus a `compression` breadcrumb instead. Some historical
/// grep results also carried the same raw-output `truncation` block that read
/// uses, so we accept that here too.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GrepDetails {
    /// Number of matches the grep tool truncated at, when the match cap was
    /// reached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_limit_reached: Option<u32>,
    /// Whether output was further truncated because line/byte caps were hit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines_truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncation: Option<TruncationInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ToolResultSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression: Option<CompressionInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpMode {
    Call,
    Describe,
    List,
    Search,
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpTool {
    Name(String),
    Described(McpDescribedTool),
}

impl McpTool {
    pub fn name(&self) -> &str {
        match self {
            Self::Name(name) => name,
            Self::Described(tool) => &tool.name,
        }
    }

    pub fn described(&self) -> Option<&McpDescribedTool> {
        match self {
            Self::Name(_) => None,
            Self::Described(tool) => Some(tool),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpDescribedTool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_name: Option<String>,
    pub description: String,
    pub input_schema: JsonBlob,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpDetails {
    pub mode: McpMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_result: Option<McpCallResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<McpTool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Set on `mode: "call"` errors of kind `tool_not_found`; names the
    /// missing MCP tool the caller asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_tool: Option<String>,
    /// Newer tool-not-found errors also identify the server that exposed the
    /// suggestion list when the requested tool name was ambiguous or wrong.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint_server: Option<String>,
    /// `mode: "status"` snapshot of every configured MCP server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub servers: Option<Vec<McpServerStatus>>,
    /// Total tools available across connected servers (status mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tools: Option<u32>,
    /// How many of `servers` are currently connected (status mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connected_count: Option<u32>,
    /// `mode: "list"` of tools exposed by a single server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    /// Number of tools in `tools` (list mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    /// `mode: "search"` search results; kept as raw `JsonBlob` because
    /// MCP tool search schemas are server-defined (same reasoning as
    /// `McpCallResult.content`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matches: Option<Vec<JsonBlob>>,
    /// `mode: "search"` query that produced `matches`; absent for
    /// local-state-only searches that don't issue a remote query.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpServerStatus {
    pub name: String,
    pub status: String,
    pub tool_count: u32,
    /// Seconds since the last failed connection attempt; `null` when the
    /// server has not failed since startup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_ago: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpCallResult {
    /// Generic MCP call payloads are server-defined, so preserve them as raw
    /// JSON instead of hard-coding Moriarty's command-result schema.
    pub content: Vec<JsonBlob>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<JsonBlob>,
    pub is_error: bool,
}

/// Client-side MCP call failure recorded when pi's MCP transport itself
/// rejects a tool call before it reaches the server (e.g. config hash
/// mismatch after approval).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpClientError {
    pub error: String,
    pub server: String,
}

/// Compact breadcrumb emitted when a direct MCP tool call succeeds (e.g.
/// `project_tools_run_*`, `jj_read_only_run`, `git_read_only_*`). The
/// full output is in the tool-result text; the breadcrumb just names the
/// server and tool so the parser can attribute the result.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpBreadcrumb {
    pub server: String,
    pub tool: String,
}

/// Tool result details for MCP-based tools that are called directly (e.g.
/// `project_tools_run_*`, `jj_read_only_run`). A successful call either
/// passes through the MCP `CallToolResult` or records a compact
/// `{server, tool}` breadcrumb; a client-side transport failure records
/// only the server name and a compact error string.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpToolResult {
    Call(McpCallResult),
    Breadcrumb(McpBreadcrumb),
    Error(McpClientError),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BashDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncation: Option<TruncationInfo>,
    /// When the response would exceed pi's in-message byte cap, pi spills
    /// the raw command output to a temp file and exposes the path here so a
    /// caller can read the untruncated output without re-running the
    /// command. `None` means no overflow occurred.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_output_path: Option<PathBuf>,
    /// Lean-ctx augmentation: only `compression` is present for bash because
    /// the extension does not record path/pattern for shell output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression: Option<CompressionInfo>,
}

/// Shared between `read` and `bash` tool results.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TruncationInfo {
    /// The truncated payload that the model actually saw.
    pub content: String,
    pub truncated: bool,
    pub truncated_by: TruncatedBy,
    pub total_lines: u64,
    pub total_bytes: u64,
    pub output_lines: u64,
    pub output_bytes: u64,
    pub last_line_partial: bool,
    pub first_line_exceeds_limit: bool,
    pub max_lines: u64,
    pub max_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TruncatedBy {
    Bytes,
    Lines,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlannotatorSubmitPlanDetails {
    pub approved: bool,
    /// Reviewer feedback supplied when the user denies the plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompressDetails {
    pub block_ids: Vec<u32>,
    pub topic: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub superseded_block_ids: Vec<u32>,
}

/// `instinct_write` returns a result-details payload identifying the
/// instinct that was upserted and whether it was newly created or
/// updated. Modeled as a closed enum on `action` so any new outcome the
/// pi runtime introduces (for example `unchanged`) surfaces as a parse
/// error rather than being silently dropped.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstinctWriteDetails {
    pub id: String,
    pub action: InstinctWriteAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstinctWriteAction {
    Created,
    Updated,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CountDetails {
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchResultDetails {
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyDetails {}

/// Hermes memory writes emit a small snake_case result envelope whose exact
/// optional fields depend on the action (`add`, `replace`, `remove`). The log
/// parser only needs the shared shape so cost analysis can keep scanning.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entries: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evicted_entries: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evicted_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matches: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillIndexDetails {
    pub skill_id: String,
    pub scope: String,
    pub file_name: String,
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub description: String,
}

/// Hermes skill results mix an index listing (`{skills:[...]}`), document
/// reads (`{success, skillId, body, ...}`), and mutation summaries
/// (`{success, message, skillId, ...}`). Capturing the shared key set keeps
/// session parsing resilient without mirroring the extension's full control
/// flow inside the log parser.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub similar_skill_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<SkillIndexDetails>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntercomResultDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TodoDetails {
    pub action: String,
    pub params: TodoArgs,
    pub tasks: Vec<TodoTask>,
    pub next_id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TodoTask {
    pub id: u64,
    pub subject: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_form: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_by: Option<Vec<u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonBlob>,
}

// ---------------------------------------------------------------------------
// Custom payload bodies
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlannotatorData {
    pub phase: PlannotatorPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_file_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_submitted_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_state: Option<PlannotatorSavedState>,
}

/// Plannotator originally serialised `savedState` as an opaque marker
/// string (e.g. `"draft"`); newer pi versions snapshot the active
/// session settings as a structured object. We accept both shapes via
/// an untagged enum so old logs continue to parse.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PlannotatorSavedState {
    Legacy(String),
    Snapshot(PlannotatorSavedStateSnapshot),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlannotatorSavedStateSnapshot {
    pub active_tools: Vec<String>,
    pub model: PlannotatorModelRef,
    pub thinking_level: ThinkingLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlannotatorModelRef {
    pub provider: Provider,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DcpStateData {
    pub compression_blocks: Vec<CompressionBlock>,
    pub next_block_id: u32,
    pub pruned_tool_ids: Vec<String>,
    pub tokens_saved: u64,
    pub total_prune_count: u64,
    pub manual_mode: bool,
}

/// One compressed conversation segment recorded in the DCP state snapshot.
///
/// Pi stores enough metadata here to render the compressed block in the UI
/// and to allow rehydration: the topic + summary text, the time range it
/// spans, and bookkeeping fields used by the DCP loop. Start/end/anchor
/// timestamps use `Decimal` because DCP can anchor a block halfway between
/// two messages, which shows up in logs as a `.5` epoch-millis value.
/// `created_at` is just the wall-clock write time for the block itself, so
/// it stays a whole-millisecond `i64`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompressionBlock {
    pub id: u32,
    pub topic: String,
    pub summary: String,
    pub start_timestamp: Decimal,
    pub end_timestamp: Decimal,
    pub anchor_timestamp: Decimal,
    pub active: bool,
    pub summary_token_estimate: u32,
    pub created_at: i64,
    /// Newer DCP snapshots estimate how many raw tokens the block replaced;
    /// older snapshots omit the field entirely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_saved_estimate: Option<u64>,
    /// Newer pi builds mark whether this block came from a compression that
    /// reported token savings. Older snapshots omit the field entirely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub savings_applied: Option<bool>,
    /// When DCP later replaces this block with a newer summary, the state
    /// snapshot records the successor block id and the supersede timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by_block_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_at: Option<i64>,
    /// Newer DCP snapshots also record which older blocks were folded into
    /// this summary block. Older snapshots omit the field entirely.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supersedes_block_ids: Vec<u32>,
}

/// `web-search-results` payload. The `type` discriminator selects between
/// search results (`queries`) and direct URL fetches (`urls`); the shared
/// `id` and `timestamp` live on the outer struct alongside the variant body.
/// Serde cannot enforce `deny_unknown_fields` on this shape with a flattened
/// internally tagged enum, so we deserialize manually to keep the outer key
/// set strict while still routing on the shared `type` field.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchResultsData {
    pub id: String,
    pub timestamp: i64,
    #[serde(flatten)]
    pub payload: WebSearchResultsPayload,
}

impl<'de> Deserialize<'de> for WebSearchResultsData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| de::Error::custom("web-search-results payload must be an object"))?;

        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| de::Error::missing_field("type"))?;

        let expected_fields = match kind {
            "search" => &["id", "timestamp", "type", "queries"][..],
            "fetch" => &["id", "timestamp", "type", "urls"][..],
            other => {
                return Err(de::Error::unknown_variant(other, &["search", "fetch"]));
            }
        };

        reject_unknown_object_fields(object, expected_fields).map_err(de::Error::custom)?;

        let id = object_field(object, "id").map_err(de::Error::custom)?;
        let timestamp = object_field(object, "timestamp").map_err(de::Error::custom)?;
        let payload = match kind {
            "search" => WebSearchResultsPayload::Search(WebSearchResultsSearch {
                queries: object_field(object, "queries").map_err(de::Error::custom)?,
            }),
            "fetch" => WebSearchResultsPayload::Fetch(WebSearchResultsFetch {
                urls: object_field(object, "urls").map_err(de::Error::custom)?,
            }),
            _ => unreachable!("kind validated above"),
        };

        Ok(Self {
            id,
            timestamp,
            payload,
        })
    }
}

fn reject_unknown_object_fields(
    object: &Map<String, Value>,
    expected_fields: &[&str],
) -> Result<(), String> {
    for key in object.keys() {
        if !expected_fields.contains(&key.as_str()) {
            return Err(format!(
                "unknown field `{key}`, expected one of {}",
                expected_fields
                    .iter()
                    .map(|field| format!("`{field}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    Ok(())
}

fn object_field<T>(object: &Map<String, Value>, field: &'static str) -> Result<T, serde_json::Error>
where
    T: serde::de::DeserializeOwned,
{
    let value = object.get(field).cloned().ok_or_else(|| {
        serde_json::Error::io(std::io::Error::other(format!("missing field `{field}`")))
    })?;
    serde_json::from_value(value)
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebSearchResultsPayload {
    Search(WebSearchResultsSearch),
    Fetch(WebSearchResultsFetch),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebSearchResultsSearch {
    pub queries: Vec<WebSearchQueryResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebSearchResultsFetch {
    pub urls: Vec<WebFetchResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebSearchQueryResult {
    pub query: String,
    pub answer: String,
    pub results: Vec<WebSearchResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Optional because aborted queries (with `error: "This operation was
    /// aborted"`) can be recorded before the provider was selected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebSearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Single URL result from a `fetch_content` call. The protocol always emits
/// `error` (as `null` on success or a string describing the failure), so the
/// field is required-but-nullable: omitting it from the JSON is a parse error
/// because that would indicate a real protocol regression rather than a
/// success.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebFetchResult {
    pub url: String,
    pub title: String,
    pub content: String,
    #[serde(deserialize_with = "deserialize_required_nullable_string")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlannotatorExecuteData {
    /// Path to the plan markdown that newer pi versions store as
    /// `lastSubmittedPath` after the user approves execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_submitted_path: Option<PathBuf>,
    /// Older pi versions stored the same path under `planFilePath`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_file_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PiLoadedToolsDetails {
    pub tools: Vec<LoadedTool>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoadedTool {
    pub name: String,
    pub description: String,
    pub active: bool,
    pub source: ToolSource,
    pub scope: ToolScope,
    pub origin: ToolOrigin,
    /// Set only when `source` is [`ToolSource::Extension`]; gives the
    /// on-disk location of the extension that registered the tool.
    /// Built-in and MCP-registered tools record `None` because they have
    /// no extension file to report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension_path: Option<String>,
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

/// Error type returned by the file/line parsing helpers. Carries enough
/// context to identify the specific file and line that failed so that the
/// parse-all binary can report coverage gaps precisely.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum ParseError {
    #[error("failed to open {path}")]
    Open {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("I/O error while reading {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}:{line}: {source}\n  line: {content}")]
    LineParse {
        path: PathBuf,
        line: usize,
        content: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to parse line: {source}\n  line: {content}")]
    SingleLine {
        content: String,
        #[source]
        source: serde_json::Error,
    },
}

pub fn parse_line(raw: &str) -> Result<PiLogLine, ParseError> {
    serde_json::from_str::<PiLogLine>(raw).map_err(|source| ParseError::SingleLine {
        content: raw.to_owned(),
        source,
    })
}

/// Errors carry the file path and 1-based line number of the offending line
/// so callers can report precise coverage failures.
pub fn parse_file(path: impl AsRef<Path>) -> Result<Vec<PiLogLine>, ParseError> {
    let path = path.as_ref();
    let file = File::open(path).map_err(|source| ParseError::Open {
        path: path.to_path_buf(),
        source,
    })?;
    let reader = BufReader::new(file);

    let mut out = Vec::new();
    for (idx, line) in reader.lines().enumerate() {
        let line = line.map_err(|source| ParseError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let parsed =
            serde_json::from_str::<PiLogLine>(&line).map_err(|source| ParseError::LineParse {
                path: path.to_path_buf(),
                line: idx + 1,
                content: line.clone(),
                source,
            })?;
        out.push(parsed);
    }

    Ok(out)
}
