//! Strongly typed serde models for pi session log lines.
//!
//! A pi session log file is newline-delimited JSON. Each line is a
//! [`PiLogLine`] keyed by the top-level `type` field. Most nested payloads are
//! modeled as tagged enums or concrete structs with
//! `#[serde(deny_unknown_fields)]` so that most upstream format changes surface
//! as parse errors rather than silent data loss.
//!
//! Tool-call envelopes stay typed and strict, but [`ToolCallContent`]
//! deliberately preserves the inner `arguments` payload as raw JSON. Pi logs the
//! model-emitted payload before the runtime validates it, so hard-coding tool
//! schemas into the parser would reject or misrepresent real sessions.
//!
//! Six categories of structure legitimately deviate from the strict default:
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
//!   so a single corrupted record cannot abort an entire log file:
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
//!     5. Transparent value wrappers ([`ToolCallArguments`]) that accept
//!        any JSON value, so a stray `"}"` or other non-object artifact
//!        in the tool-call stream does not abort the whole session.
//!
//! * **Open-ended protocol discriminators** — [`AssistantApi`] uses a custom
//!   deserializer that accepts a small set of well-known API identifiers
//!   (`anthropic-messages`, `openai-responses`, `openai-completions`) plus
//!   `faux:`-prefixed routing strings emitted by the faux AI provider. Any
//!   other string value still fails loudly. This is intentionally narrower
//!   than a fully open `String` but wider than a strict enum, because faux
//!   session IDs are dynamic and cannot be enumerated ahead of time.
//!
//! * **Shape-branching custom deserializer** — [`McpDetails`] uses a
//!   custom `Deserialize` impl that accepts two structurally incompatible
//!   wire formats: a sessions-only action shape (`{ sessions: N }` with no
//!   `mode`, emitted by pi's `action: "ui-messages"` MCP server status
//!   call) and the normal mode-based shape (all standard fields with
//!   `mode` required). Both branches enforce strict unknown-field
//!   rejection — the sessions-only path via
//!   `reject_unknown_object_fields` and the mode-based path via an
//!   internally-derived `StrictMcpDetails` struct with
//!   `#[serde(deny_unknown_fields)]`.
//!
//! * **Forward-compatible protocol schemas** — structs representing
//!   server-defined or runtime-defined protocol envelopes whose field
//!   sets evolve independently of the parser (e.g. [`McpCallResult`] for
//!   MCP tool-call results, which pi's runtime regularly extends with new
//!   metadata fields like `contentBlocks`, `outputGuard`, and `omitted`).
//!   These structs omit `deny_unknown_fields` at the struct level and use
//!   `#[serde(default)]` on every field, so unrecognized fields are
//!   silently accepted rather than rejecting the whole log line.
//!
//! * **Versioned payloads** — [`CompactionDetails`] uses `#[serde(untagged)]`
//!   to accept two structurally incompatible wire formats: a legacy shape
//!   with `readFiles`/`modifiedFiles` and a current shape with compaction
//!   metadata (`compactor`, `version`, `sections`, …). Serde tries each
//!   variant in declaration order and picks the first that deserializes
//!   successfully. Both variant structs keep `#[serde(deny_unknown_fields)]`
//!   so unexpected fields are still caught regardless of which shape is
//!   selected.
//!
//! Each corrupt-stream exception carries an inline comment naming the
//! observed failure mode.

use std::{
    cmp::Ordering,
    collections::BTreeMap,
    fmt,
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
    pub parent_id: Option<String>,
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
    /// Cost/token usage of the summarization call pi made to produce this
    /// compaction. Added by pi after the initial compaction schema, so older
    /// logs omit it; the line records no provider/model of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<AssistantUsage>,
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
    /// Cost/token usage of the summarization call, mirroring
    /// [`CompactionLine::usage`]; likewise optional for backward compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<AssistantUsage>,
    pub from_hook: bool,
}

/// Pi has changed the shape of this field over time: the original format
/// carried `readFiles` / `modifiedFiles`; the current format carries
/// compaction metadata (`compactor`, `version`, `sections`, …).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CompactionDetails {
    /// Current format: compaction metadata (pi ≥ mid-2026).
    V2(CompactionDetailsV2),
    /// Legacy format: per-file read / modified lists.
    V1(CompactionDetailsV1),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompactionDetailsV1 {
    pub read_files: Vec<PathBuf>,
    pub modified_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompactionDetailsV2 {
    pub compactor: String,
    pub version: u32,
    pub sections: Vec<String>,
    pub source_message_count: u64,
    pub previous_summary_used: bool,
    #[serde(rename = "om.folded")]
    pub om_folded: OmFolded,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OmFolded {
    #[serde(rename = "type")]
    pub typ: String,
    pub version: u32,
    pub full_fold: bool,
    pub observations: Vec<JsonBlob>,
    pub reflections: Vec<JsonBlob>,
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
    #[serde(rename = "om.observations.recorded")]
    OmObservationsRecorded(OmObservationsRecordedData),
    #[serde(rename = "om.reflections.recorded")]
    OmReflectionsRecorded(OmReflectionsRecordedData),
    #[serde(rename = "om.observations.dropped")]
    OmObservationsDropped(OmObservationsDroppedData),
    #[serde(rename = "om.reflections.dropped")]
    OmReflectionsDropped(OmReflectionsDroppedData),
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
    /// Supervisor request relayed via intercom (e.g. child subagent asking
    /// the parent for a decision). Added in newer pi versions.
    #[serde(rename = "subagent_supervisor_request")]
    SubagentSupervisorRequest(SubagentSupervisorRequestDetails),
    /// Announced by the web-search extension when background content
    /// fetching completes. The human-readable progress summary lives in
    /// the outer `content` field; no structured `details` payload.
    #[serde(rename = "web-search-content-ready")]
    WebSearchContentReady,
    /// Injected after a compaction that interrupted a parent task, telling
    /// the assistant to pick the task back up. Carries no `details`
    /// payload; the resume instruction lives in the outer `content` field.
    #[serde(rename = "subagent-compaction-resume")]
    SubagentCompactionResume,
    /// Framing instruction injected by the plannotator extension at the
    /// start of a planning phase. The structured `details` carry only the
    /// phase; the full framing text, including the applicable rules, lives
    /// in the outer `content` field.
    #[serde(rename = "plannotator-framing")]
    PlannotatorFraming(PlannotatorFramingDetails),
    /// Emitted when a `subagent_wait` subscription wakes, carrying the
    /// subscription token, run id, and wake outcome.
    #[serde(rename = "subagent-wait-subscription")]
    SubagentWaitSubscription(SubagentWaitSubscriptionDetails),
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
    /// The provider's own stop-reason string, passed through un-normalized
    /// beside the `stop_reason` pi maps it onto (e.g. the OpenAI Responses
    /// `completed` that pi records as `toolUse` or `stop`). Kept as a
    /// `String` rather than a strict enum because the vocabulary belongs to
    /// whichever provider served the turn, so it is open-ended by
    /// construction. Only some providers emit it, hence `Option`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_stop_reason: Option<String>,
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
    /// WebSocket close code when the transport error is a socket-level
    /// close (e.g. `1000` for normal closure). Absent for non-WebSocket
    /// transport failures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultMessage {
    pub tool_call_id: String,
    pub tool_name: String,
    pub content: Vec<ToolResultContentItem>,
    pub is_error: bool,
    pub timestamp: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<ToolResultDetails>,
    /// Tools the model gained mid-turn (e.g. `contact_supervisor` after a
    /// supervisor prompt). Pi appends this metadata to tool results in
    /// newer runtimes; optional so older logs still parse. Not billable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_tool_names: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawToolResultMessage {
    pub tool_call_id: String,
    pub tool_name: String,
    pub content: Vec<ToolResultContentItem>,
    pub is_error: bool,
    pub timestamp: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_tool_names: Option<Vec<String>>,
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
            added_tool_names,
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
            added_tool_names,
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
    Text {
        text: String,
    },
    /// Base64-encoded image data from a user `@`-mentioning an image file.
    /// The wire protocol embeds the image inline as a base64 string rather
    /// than referencing a path, so the content is self-contained.
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
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
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
/// Pi tool results can now carry image data alongside text — the `read`
/// tool embeds images inline when the model requests an image file, using
/// the same base64 wire format as `UserContentItem::Image`.
pub enum ToolResultContentItem {
    Text {
        text: String,
    },
    /// Base64-encoded image data embedded in a tool result (e.g. from `read`
    /// of an image file). Same wire format as `UserContentItem::Image`.
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
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
    /// Reasoning tokens (e.g. from DeepSeek-V4-Pro, GLM-5.2).
    /// Pi added this field later; optional for backward compatibility.
    #[serde(default)]
    pub reasoning: u64,
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
    /// Added in pi 0.80+; maps to extended thinking mode.
    Max,
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
                return Ok(AssistantApi::Known(ApiKind::OpenAiCodexResponses));
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
    #[serde(rename = "length")]
    Length,
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

/// Pi records the model-emitted `arguments` payload before the tool runtime
/// has a chance to validate it, so the parser preserves the raw JSON value
/// instead of hard-coding per-tool schemas into `ToolCallContent`.
///
/// Accepts any JSON value, not just an object, so a stray `"}"` or other
/// corrupt-model-stream artifact does not poison the entire session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ToolCallArguments(pub Value);

impl ToolCallArguments {
    fn canonical_json(&self) -> String {
        serde_json::to_string(&self.0).expect("json values should always serialize")
    }
}

impl PartialEq for ToolCallArguments {
    fn eq(&self, other: &Self) -> bool {
        self.canonical_json() == other.canonical_json()
    }
}

impl Eq for ToolCallArguments {}

impl Hash for ToolCallArguments {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.canonical_json().hash(state);
    }
}

impl PartialOrd for ToolCallArguments {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ToolCallArguments {
    fn cmp(&self, other: &Self) -> Ordering {
        self.canonical_json().cmp(&other.canonical_json())
    }
}

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
    pub subjects: Option<Vec<String>>,
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

/// Emitted by the om extension when it records observations extracted
/// during a session.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OmObservationsRecordedData {
    pub observations: Vec<ObservationEntry>,
    pub covers_up_to_id: String,
}

/// Emitted by the om extension when it records reflections synthesized
/// from observations.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OmReflectionsRecordedData {
    pub reflections: Vec<ReflectionEntry>,
    pub covers_up_to_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservationEntry {
    pub id: String,
    pub content: String,
    /// Wall-clock timestamp in `"YYYY-MM-DD HH:MM"` format (not ISO 8601).
    pub timestamp: String,
    pub relevance: String,
    pub source_entry_ids: Vec<String>,
    pub token_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReflectionEntry {
    pub id: String,
    pub content: String,
    pub supporting_observation_ids: Vec<String>,
    pub token_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OmObservationsDroppedData {
    pub observation_ids: Vec<String>,
    pub covers_up_to_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OmReflectionsDroppedData {
    pub reflection_ids: Vec<String>,
    pub covers_up_to_id: String,
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
            serde_json::from_value(details).map(ToolResultDetails::McpToolResult)
        }
        "instinct_write" => serde_json::from_value(details).map(ToolResultDetails::InstinctWrite),
        "intercom" => serde_json::from_value(details).map(ToolResultDetails::Intercom),
        "lens_diagnostics" => {
            serde_json::from_value(details).map(ToolResultDetails::LensDiagnostics)
        }
        "ls" => serde_json::from_value(details).map(ToolResultDetails::Ls),
        "lsp_diagnostics" => serde_json::from_value(details).map(ToolResultDetails::LspDiagnostics),
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
        "module_report" => serde_json::from_value(details).map(ToolResultDetails::ModuleReport),
        "project_tools_run_build"
        | "project_tools_run_formatter"
        | "project_tools_run_lint"
        | "project_tools_run_tests"
        | "jj_read_only_run" => {
            serde_json::from_value(details).map(ToolResultDetails::McpToolResult)
        }
        "pi_lens_activate_tools" => {
            serde_json::from_value(details).map(ToolResultDetails::PiLensActivateTools)
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
        "subagent" | "subagent_wait" => {
            serde_json::from_value(details).map(ToolResultDetails::Subagent)
        }
        "subagent_supervisor" => {
            serde_json::from_value(details).map(ToolResultDetails::SubagentSupervisor)
        }
        "todo" => serde_json::from_value(details).map(ToolResultDetails::Todo),
        "web_search" => parse_web_search_details(details),
        _ => serde_json::from_value(details),
    }
}

/// Handles the web_search lean cases where `details` omits `queryCount`
/// and related fields: a cancelled-stale search or a failed search that
/// records only `{error}`. A small strict helper struct preserves
/// `deny_unknown_fields` and type validation; the normal (serde) path keeps
/// `WebSearchDetails` fields required, which preserves the dispatch
/// invariant documented on the untagged `ToolResultDetails` enum that
/// `WebSearchDetails` cannot absorb a bare `{error}` payload.
fn parse_web_search_details(details: Value) -> Result<ToolResultDetails, serde_json::Error> {
    let is_lean = details.get("queryCount").is_none()
        && (details.get("error").and_then(|v| v.as_str()).is_some()
            || details.get("cancelled").and_then(|v| v.as_bool()) == Some(true));
    if is_lean {
        // The full shape always carries `queryCount`. A lean shape is a
        // cancelled-stale search (`cancelled: true`) or a failed search
        // recorded as a bare `{error}`, both of which omit the query fields.
        // Requiring one of those discriminators keeps empty or arbitrary
        // success payloads (e.g. `{}`) failing loudly on the strict path.
        let lean: WebSearchLeanDetails = serde_json::from_value(details)?;
        Ok(ToolResultDetails::WebSearch(WebSearchDetails {
            fetch_id: lean.fetch_id,
            search_id: lean.search_id,
            fetch_urls: None,
            query_count: 0,
            successful_queries: 0,
            total_results: 0,
            include_content: false,
            queries: Vec::new(),
            curated: lean.curated.unwrap_or(false),
            curated_from: lean.curated_from,
            curated_queries: None,
            summary: None,
            cancelled: lean.cancelled.unwrap_or(false),
            error: lean.error,
            cancel_reason: lean.cancel_reason,
        }))
    } else {
        serde_json::from_value(details).map(ToolResultDetails::WebSearch)
    }
}

/// Strict helper for web_search results that omit the query-count fields:
/// a cancelled-stale search (`cancelled: true`, optional `cancel_reason`)
/// or a failed search recorded as a bare `{error}`. Every field is optional
/// because either shape may carry only a subset of them.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WebSearchLeanDetails {
    #[serde(default)]
    fetch_id: Option<String>,
    #[serde(default)]
    search_id: Option<String>,
    #[serde(default)]
    curated: Option<bool>,
    #[serde(default)]
    curated_from: Option<u32>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    cancel_reason: Option<String>,
    #[serde(default)]
    cancelled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolResultDetails {
    Edit(EditDetails),
    // Boxed because SubagentResultDetails dwarfs every other variant and
    // would otherwise set the size of the whole enum.
    Subagent(Box<SubagentResultDetails>),
    SubagentSupervisor(SubagentSupervisorResultDetails),
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
    // `git_read_only_*` and other direct MCP tool results (also
    // `project_tools_run_*`, `jj_read_only_run`) are dispatched explicitly
    // by `parse_tool_result_details` to `McpToolResult`, whose Breadcrumb
    // arm carries their `{server, tool}` breadcrumb and whose Error arm
    // carries the pi client-side `{error, server}` failure shape.
    //
    // FetchContent has no shape overlap with anything above (it declares
    // `urls`, `urlCount`, ... that no other variant carries).
    //
    // GetSearchContent is dual-shape: its Success arm is uniquely
    // identified by `{url, title, contentLength}`, and its Error arm is
    // `{error}` with an optional `url`. Earlier variants that also declare
    // an `error` field (CodeSearchDetails, McpDetails, TodoDetails,
    // SubagentResultDetails-via-Subagent, WebSearchDetails) all require
    // additional discriminator fields (e.g. `query`+`maxTokens`,
    // `mode`, `action`+`params`+`nextId`, `mode`+`results`,
    // `queryCount`+`successfulQueries`+...), so either error shape cannot
    // be absorbed by any of them and safely falls through here.
    FetchContent(FetchContentDetails),
    GetSearchContent(GetSearchContentDetails),
    // Hermes `memory` and `skill` are intentionally parsed by `tool_name`
    // first because their error shapes can collapse to `{}` or a bare
    // `{error}` and would otherwise be ambiguous in direct untagged
    // deserialization.
    Empty(EmptyDetails),
    Memory(MemoryDetails),
    Skill(SkillDetails),
    // pi-lens tool results; each tool has its own typed detail struct
    // routed explicitly by `parse_tool_result_details`.
    LensDiagnostics(LensDiagnosticsDetails),
    LspDiagnostics(LspDiagnosticsDetails),
    ModuleReport(ModuleReportDetails),
    PiLensActivateTools(PiLensActivateToolsDetails),
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

/// `contact_supervisor` results carry `requestId` and `reason` when a human
/// supervisor reply is needed, plus an optional `error` marker for failures.
/// The human-readable outcome remains in the tool-result text.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContactSupervisorResultDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered: Option<bool>,
}

/// `ls` tool results are either a plain listing (no `details`), a lean-ctx
/// augmented listing, or a raw-output truncation payload when the serialized
/// entry list exceeds pi's message byte cap. `entry_limit_reached` is
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
    pub truncation: Option<TruncationInfo>,
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
    Chain,
    Management,
    Parallel,
    Single,
    Workflow,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowGraph {
    pub run_id: String,
    pub mode: String,
    /// Phase descriptors emitted by pi when the runtime tracks named
    /// workflow phases (e.g. `{"name": "review"}`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phases: Vec<JsonBlob>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<WorkflowGraphNode>,
}

/// A node in the workflow execution tree. Group nodes (e.g. `kind =
/// "parallel-group"`) carry `children`; leaf nodes (e.g. `kind =
/// "agent"`) carry agent-specific fields.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowGraphNode {
    pub id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub status: String,
    pub step_index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flat_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<WorkflowGraphNode>>,
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
    /// Opaque fingerprint of the launch contract, retained to correlate this
    /// immediate result with its lifecycle artifacts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_contract_digest: Option<String>,
    /// A revived run preserves the original contract's digest alongside the
    /// new launch digest so lifecycle artifacts remain traceable across runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_launch_contract_digest: Option<String>,
    /// Kept as raw JSON because the extension resolver's provenance envelope
    /// can evolve without a corresponding log-schema release.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_resolved_extensions: Option<Box<JsonBlob>>,
    /// Kept as raw JSON because this runtime-owned envelope evolves separately
    /// from the log format; `processTerminal` is one observed nested status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_status: Option<JsonBlob>,
    /// Kept as raw JSON because the runtime may add budget-accounting fields
    /// without coordinating a parser schema release.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_budget: Option<JsonBlob>,
    /// Added in newer pi versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Absolute deadline (Unix epoch ms) after which the subagent will be
    /// interrupted. Added in newer pi versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_at: Option<u64>,
    /// Structured execution tree for multi-agent workflow runs. Present
    /// when the subagent operation tracks parallel groups and nested
    /// agents; absent for single-agent or non-structured runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_graph: Option<WorkflowGraph>,
    /// Tool budget configuration for the subagent run (soft/hard limits and
    /// blocked tools). Added in newer pi versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_budget: Option<SubagentToolBudget>,
    /// Turn budget configuration for the subagent run (max/grace turns).
    /// Added in newer pi versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_budget: Option<SubagentTurnBudget>,
    /// Aggregate usage across all child subagents in the run. Added in newer
    /// pi versions; optional for backward compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_child_usage: Option<SubagentUsage>,
    /// Aggregate cost summary across the entire subagent run. Added in newer
    /// pi versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_cost: Option<SubagentTotalCost>,
    /// Steering details recorded when an external caller steers an async
    /// subagent run via management RPC. Present only for steered runs; absent
    /// for normal invocations and completed-from-launch runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steering: Option<SubagentSteeringDetails>,
    /// Configuration for mirroring the run's progress into the parent chat,
    /// including how the run's repository relates to the parent session's.
    /// Added in newer pi versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_progress: Option<SubagentChatProgress>,
    /// The launching tool call's id echoed into the details so detached-run
    /// lifecycle artifacts can be correlated back to the call that spawned
    /// them. Added in newer pi versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Id of the mission the run belongs to; keys the mission file echoed
    /// in `mission`/`mission_path`. Added in newer pi versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_id: Option<String>,
    /// Path to the mission file whose content is echoed in `mission`.
    /// Added in newer pi versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_path: Option<PathBuf>,
    /// Snapshot of the mission file at `mission_path`. Kept as raw JSON
    /// because the extension owns this envelope and versions it
    /// independently of the log format (it carries its own `schemaVersion`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission: Option<Box<JsonBlob>>,
    /// Async runs that finished while a `subagent_wait` management call was
    /// blocking, one entry per awaited run. Added in newer pi versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completions: Option<Vec<SubagentWaitCompletion>>,
}

/// One awaited async run reported by a `subagent_wait` management result.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentWaitCompletion {
    pub run_id: String,
    pub agent: String,
    pub mode: SubagentResultMode,
    /// Kept as `String` rather than a strict enum because the run-lifecycle
    /// vocabulary (observed: `complete`) is undocumented and volatile.
    pub state: String,
    pub success: bool,
    /// Defaults empty because newer pi versions omit `results` from a
    /// completion entry when the run had no child results to summarise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub results: Vec<SubagentWaitCompletionResult>,
    /// A wait result can arrive before the runtime writes its output archive,
    /// so no archive path is guaranteed even after the run completes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_path: Option<PathBuf>,
}

/// Per-agent result summary inside a [`SubagentWaitCompletion`]. Leaner than
/// [`SubagentResultSummary`]: the wait call reports only identity, outcome,
/// and where the archived output lives.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentWaitCompletionResult {
    /// Absent on a child run that failed before the harness bound it to an
    /// agent; such an entry carries only its outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Independent of `agent`: an identified, successful child that reports
    /// its full artifact set has still been observed without a run id, so
    /// neither field implies the other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub success: bool,
    /// Kept as `String` rather than a strict enum because the vocabulary
    /// (observed: `present`) is undocumented and volatile.
    pub output_state: String,
    /// The model that served the child run, recorded only for runs that
    /// reached a provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Omitted for a failed child run — the only observed omission carries
    /// `success: false` — so a saved output path cannot be assumed even when
    /// `output_state` says `present`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_paths: Option<SubagentWaitArtifactPaths>,
    /// Error message for a failed child run; absent on success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Wait completions record either the saved output path alone or the full
/// launch-result set, so only `output_path` is guaranteed; the rest mirror
/// [`SubagentArtifactPaths`] but stay optional.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentWaitArtifactPaths {
    pub output_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jsonl_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentChatProgress {
    /// Kept as `String` rather than a strict enum because the mode
    /// vocabulary (observed: `off`) is undocumented and volatile.
    pub mode: String,
    /// Kept as `String` — observed values are `same` and `other`, but the
    /// vocabulary is undocumented and volatile.
    pub repo_relation: String,
    /// Human-readable label for the run's repository; observed only when
    /// `repo_relation` is `same`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_label: Option<String>,
}

/// Steering details for an async subagent run, recorded when an external
/// caller steers the agent mid-execution.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentSteeringDetails {
    pub request_id: String,
    /// Kept as `String` rather than a strict enum because the steering
    /// lifecycle vocabulary is undocumented and volatile.
    pub state: String,
    /// Delivery outcome reported beside `state`; the two have been observed
    /// carrying the same value, but pi records them separately so neither is
    /// derived from the other. Added in newer pi versions, hence optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_status: Option<String>,
    pub source_run_id: String,
    pub targets: Vec<SubagentSteeringTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentSteeringTarget {
    pub index: u32,
    /// Kept as `String` — see [`SubagentSteeringDetails::state`].
    pub state: String,
    /// Unix epoch milliseconds when the steering was delivered to this target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_at: Option<u64>,
}

/// The native supervisor tool uses a different strict payload for each action.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SubagentSupervisorResultDetails {
    Reply(SubagentSupervisorReplyDetails),
    Status(SubagentSupervisorStatusDetails),
    Pending(SubagentSupervisorPendingDetails),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentSupervisorReplyDetails {
    pub reply_to: String,
    pub run_id: String,
    pub agent: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentSupervisorStatusDetails {
    pub active: bool,
    pub pending: u32,
    pub root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentSupervisorPendingDetails {
    pub pending: Vec<PendingSupervisorRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PendingSupervisorRequest {
    pub id: String,
    pub run_id: String,
    pub agent: String,
    pub child_index: u32,
    pub reason: String,
    pub expects_reply: bool,
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
    /// Acceptance evidence status (e.g. `"attested"`). Added in newer pi
    /// versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_status: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Finalization config for the acceptance contract (mode, max turns).
    /// Optional for backward compatibility with older pi log files that
    /// did not include this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalization: Option<AcceptanceFinalizationConfig>,
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
    pub timed_out: Option<bool>,
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
    /// Tool budget configuration for the subagent run (soft/hard limits and
    /// blocked tools). Added in newer pi versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_budget: Option<SubagentToolBudget>,
    /// Turn budget configuration for the subagent run (max/grace turns).
    /// Added in newer pi versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_budget: Option<SubagentTurnBudget>,
    /// Whether the subagent exceeded its turn budget. Added in newer pi
    /// versions; absent when the budget was not configured or not exceeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_budget_exceeded: Option<bool>,
    /// Whether the subagent was asked to wrap up (turn limit approaching).
    /// Added in newer pi versions; absent when wrap-up was not requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap_up_requested: Option<bool>,
    /// Whether the subagent detached for intercom coordination (as opposed
    /// to completing in-process). Added in newer pi versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detached: Option<bool>,
    /// Reason for detachment (e.g. "intercom coordination"). Added in newer
    /// pi versions alongside `detached`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detached_reason: Option<String>,
    /// Top-level transcript path for the subagent session (distinct from the
    /// per-file `transcriptPath` inside `artifactPaths`). Added in newer pi
    /// versions; optional for backward compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<PathBuf>,
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
    /// Inheritance mode the subagent was launched with (e.g. "fresh" or
    /// "fork"). Added in newer pi versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Launch contract digest for the subagent run. Added in newer pi
    /// versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_contract_digest: Option<String>,
    /// Effects of the subagent's execution on the file system (currently
    /// only `fileMutation`). Added in newer pi versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<SubagentEffects>,
    /// Thinking level for the subagent (e.g. `"medium"`). Kept as a
    /// free-form `String` rather than a strict enum because pi may add new
    /// levels in future releases. Added in newer pi versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentOutputReference {
    pub path: PathBuf,
    pub bytes: u64,
    pub lines: u64,
    pub message: String,
}

/// Outcome of a subagent's file mutations.
///
/// `status` is a free-form string (e.g. `"not-applicable"`, `"mutated"`).
/// `expected` is true when the mutation was anticipated by the launch
/// contract. `attempted` is true when the subagent tried to mutate files
/// (independently of whether the mutation succeeded or was expected).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentFileMutation {
    pub status: String,
    pub expected: bool,
    pub attempted: bool,
}

/// Effects of a subagent's execution on the file system (currently only
/// `fileMutation`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentEffects {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_mutation: Option<SubagentFileMutation>,
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
    /// Async idle notices can fire before the child reports usage counters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turns: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_tool_duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_path: Option<PathBuf>,
    /// The runtime measures elapsed time with a sub-millisecond clock.
    /// Omitted by event producers such as `completion_guard` that do not
    /// include runtime observability counters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<Decimal>,
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

/// Captures the tool-budget envelope that newer pi runtimes attach to
/// subagent results. Kept as a separate strict struct rather than inline
/// fields so the budget shape can evolve without touching every consumer.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentToolBudget {
    /// Some newer pi runtimes omit this when only a hard limit applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soft: Option<u32>,
    pub hard: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<SubagentBlock>,
    /// Timestamp (ms since epoch) when the hard budget was reached.
    /// Added in newer pi versions; absent when the budget was not exceeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hard_reached_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soft_reached_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_count: Option<u64>,
    /// Budget outcome (e.g. "completed", "exceeded"). Added in newer pi
    /// versions; absent when the budget was not configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

/// The tool-block policy attached to a subagent's budget. Newer pi versions
/// serialize the wildcard (all tools) as `"*"` rather than a list, so this
/// enum accepts both wire shapes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SubagentBlock {
    List(Vec<String>),
    All,
}

impl Serialize for SubagentBlock {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::List(tools) => tools.serialize(serializer),
            Self::All => serializer.serialize_str("*"),
        }
    }
}

impl<'de> Deserialize<'de> for SubagentBlock {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = SubagentBlock;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a list of tool names or the string \"*\"")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if v == "*" {
                    Ok(SubagentBlock::All)
                } else {
                    Err(de::Error::invalid_value(de::Unexpected::Str(v), &self))
                }
            }

            fn visit_seq<A>(self, seq: A) -> Result<Self::Value, A::Error>
            where
                A: de::SeqAccess<'de>,
            {
                let tools = Vec::deserialize(de::value::SeqAccessDeserializer::new(seq))?;
                Ok(SubagentBlock::List(tools))
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Captures the turn-budget envelope that newer pi runtimes attach to
/// subagent results. Mirror of `SubagentToolBudget` for turn-count
/// limits rather than tool-call limits.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentTurnBudget {
    pub max_turns: u32,
    pub grace_turns: u32,
    /// Turn number at which the budget was exceeded. Added in newer pi
    /// versions; absent when the budget was not exceeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exceeded_at_turn: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_count: Option<u64>,
    /// Turn number at which wrap-up was requested. Added in newer pi
    /// versions; absent when wrap-up was not requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap_up_requested_at_turn: Option<u32>,
    /// Budget outcome (e.g. "completed", "exceeded"). Added in newer pi
    /// versions; absent when the budget was not configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

/// Aggregate token/cost totals that newer pi runtimes attach to the
/// top-level subagent result envelope (distinct from per-result usage).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentTotalCost {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentArtifactPaths {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub jsonl_path: PathBuf,
    pub metadata_path: PathBuf,
    /// Transcript of the subagent session.
    /// Pi added this field later; optional for backward compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentControlNoticeDetails {
    pub event: SubagentControlEvent,
    pub source: String,
    /// Async notices retain the run directory so the parent can inspect its artifacts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub async_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_intercom_target: Option<String>,
    pub notice_text: String,
}

/// Payload for the `subagent_supervisor_request` custom message, which pi
/// emits when a child subagent uses intercom to ask the parent for a
/// decision. Carries the request identity, reason, and routing fields so
/// the parent can correlate the request with the child's run and reply.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentSupervisorRequestDetails {
    pub id: String,
    pub reason: String,
    pub expects_reply: bool,
    pub run_id: String,
    pub agent: String,
    pub child_index: u32,
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
    /// URLs fetched asynchronously for content extraction (present
    /// when `include_content` is true). Absent from older log entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetch_urls: Option<Vec<String>>,
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
    #[serde(default)]
    pub cancelled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Reason a search curation was cancelled (e.g. "stale").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_reason: Option<String>,
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
/// large the cached body is. Failed retrievals may instead repeat the
/// requested URL beside the error message, so the error variant retains that
/// breadcrumb while keeping both wire shapes strict.
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
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
    Instructions,
    List,
    Search,
    Status,
    #[serde(rename = "auth-start")]
    AuthStart,
    #[serde(rename = "auth-complete")]
    AuthComplete,
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<McpMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sessions: Option<u32>,
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
    /// How many of `servers` the user has disabled (status mode); reported
    /// separately from `connected_count` because a disabled server is neither
    /// connected nor failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_count: Option<u32>,
    /// `mode: "list"` of tools exposed by a single server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    /// Number of tools in `tools` (list mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    /// Optional because older MCP list results omit this metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_instructions: Option<bool>,
    /// `mode: "search"` search results; kept as raw `JsonBlob` because
    /// MCP tool search schemas are server-defined (same reasoning as
    /// `McpCallResult.content`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matches: Option<Vec<JsonBlob>>,
    /// `mode: "search"` query that produced `matches`; absent for
    /// local-state-only searches that don't issue a remote query.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// Overflow/truncation metadata added in newer pi versions when an
    /// MCP response exceeds pi's in-message byte cap. Stored as raw JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_guard: Option<JsonBlob>,
    /// Length of the server instructions text (`mode: "instructions"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<u32>,
}

impl<'de> Deserialize<'de> for McpDetails {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| de::Error::custom("McpDetails payload must be an object"))?;

        // `action: "ui-messages"` shape: { sessions: N } with no mode field.
        if !object.contains_key("mode") && object.contains_key("sessions") {
            let sessions = object_field::<u32>(object, "sessions").map_err(de::Error::custom)?;
            let output_guard = object
                .get("outputGuard")
                .map(|v| serde_json::from_value(v.clone()).map_err(de::Error::custom))
                .transpose()?;
            reject_unknown_object_fields(object, &["sessions", "outputGuard"])
                .map_err(de::Error::custom)?;
            return Ok(McpDetails {
                mode: None,
                sessions: Some(sessions),
                mcp_result: None,
                server: None,
                tool: None,
                error: None,
                message: None,
                requested_tool: None,
                hint_server: None,
                servers: None,
                total_tools: None,
                connected_count: None,
                disabled_count: None,
                tools: None,
                count: None,
                has_instructions: None,
                matches: None,
                query: None,
                output_guard,
                length: None,
            });
        }

        // Fall through: validate through a strict intermediate struct.
        // The intermediate struct enforces deny_unknown_fields so the
        // usual mode-based McpDetails wire shape is still strict; only
        // the sessions-only action shape bypasses it above.
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct StrictMcpDetails {
            mode: McpMode,
            #[serde(default)]
            sessions: Option<u32>,
            #[serde(default)]
            mcp_result: Option<McpCallResult>,
            #[serde(default)]
            server: Option<String>,
            #[serde(default)]
            tool: Option<McpTool>,
            #[serde(default)]
            error: Option<String>,
            #[serde(default)]
            message: Option<String>,
            #[serde(default)]
            requested_tool: Option<String>,
            #[serde(default)]
            hint_server: Option<String>,
            #[serde(default)]
            servers: Option<Vec<McpServerStatus>>,
            #[serde(default)]
            total_tools: Option<u32>,
            #[serde(default)]
            connected_count: Option<u32>,
            #[serde(default)]
            disabled_count: Option<u32>,
            #[serde(default)]
            tools: Option<Vec<String>>,
            #[serde(default)]
            count: Option<u32>,
            #[serde(default)]
            has_instructions: Option<bool>,
            #[serde(default)]
            matches: Option<Vec<JsonBlob>>,
            #[serde(default)]
            query: Option<String>,
            #[serde(default)]
            output_guard: Option<JsonBlob>,
            #[serde(default)]
            length: Option<u32>,
        }

        let strict = StrictMcpDetails::deserialize(value).map_err(de::Error::custom)?;
        Ok(McpDetails {
            mode: Some(strict.mode),
            sessions: strict.sessions,
            mcp_result: strict.mcp_result,
            server: strict.server,
            tool: strict.tool,
            error: strict.error,
            message: strict.message,
            requested_tool: strict.requested_tool,
            hint_server: strict.hint_server,
            servers: strict.servers,
            total_tools: strict.total_tools,
            connected_count: strict.connected_count,
            disabled_count: strict.disabled_count,
            tools: strict.tools,
            count: strict.count,
            has_instructions: strict.has_instructions,
            matches: strict.matches,
            query: strict.query,
            output_guard: strict.output_guard,
            length: strict.length,
        })
    }
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

/// Generic MCP call payloads are server-defined, so preserve them as raw
/// JSON instead of hard-coding Moriarty's command-result schema.
/// `deny_unknown_fields` is intentionally omitted here because the MCP
/// protocol and pi's runtime regularly add new fields to tool-result
/// envelopes (e.g. `contentBlocks`, `outputGuard`, `omitted`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCallResult {
    /// Optional because newer pi versions may omit `content` when `omitted`
    /// is true and only `contentBlocks` is populated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<JsonBlob>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<JsonBlob>,
    pub is_error: bool,
    /// Opaque content-block representation added in newer pi versions
    /// alongside the flat `content` array; currently observed as null when
    /// absent and present when pi records per-block tool-result metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_blocks: Option<JsonBlob>,
    /// Short summary of tool-result content added in newer pi versions;
    /// observable as null when absent and present when pi records a
    /// content summary alongside the flat `content` array.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_summary: Option<JsonBlob>,
    /// Path to an external file containing the full MCP tool result when
    /// the result exceeded pi's in-message byte cap. Added in newer pi
    /// versions; null or absent when the result fit in `content`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_result_path: Option<PathBuf>,
    /// Overflow/truncation metadata added in newer pi versions when an MCP
    /// tool result exceeds pi's in-message byte cap. Stored as raw JSON
    /// because the schema is still evolving.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_guard: Option<JsonBlob>,
    /// Flag indicating the tool result content was omitted (e.g. for
    /// performance or privacy). Added in newer pi versions; absent when
    /// content was included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub omitted: Option<bool>,
    /// Size in bytes of the raw tool result before truncation.
    /// Added in newer pi versions; absent when content was not truncated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_result_bytes: Option<u64>,
    /// Reason string for omitted or truncated results (e.g. "oversized",
    /// "mcp-protocol-limits"). Added in newer pi versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Client-side MCP call failure recorded when pi's MCP transport itself
/// rejects a tool call before it reaches the server (e.g. an item not approved
/// or changed since approval).
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
    /// Overflow/truncation metadata added in newer pi versions when an
    /// MCP tool result exceeds pi's in-message byte cap. Stored as raw
    /// JSON because the schema is still evolving.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_guard: Option<JsonBlob>,
}

/// Tool result details for MCP-based tools that are called directly (e.g.
/// `project_tools_run_*`, `jj_read_only_run`, `git_read_only_*`). A
/// successful call either passes through the MCP `CallToolResult` or records
/// a compact `{server, tool}` breadcrumb; a client-side transport failure
/// records only the server name and a compact error string.
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

/// Shared between `bash`, `find`, `grep`, `ls`, and `read` tool results.
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

/// Newtype over `serde_json::Value` that supplies the `Ord`, `PartialOrd`,
/// and `Hash` trait impls needed by detail struct derives. Two
/// `JsonValue`s are compared/hashed by their canonical string
/// representation, so structural equality is preserved.
#[derive(Debug, Clone)]
pub struct JsonValue(pub Value);

impl PartialEq for JsonValue {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for JsonValue {}

impl PartialOrd for JsonValue {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for JsonValue {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let a = serde_json::to_string(&self.0).unwrap_or_default();
        let b = serde_json::to_string(&other.0).unwrap_or_default();
        a.cmp(&b)
    }
}

impl std::hash::Hash for JsonValue {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let s = serde_json::to_string(&self.0).unwrap_or_default();
        s.hash(state);
    }
}

impl Serialize for JsonValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for JsonValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Value::deserialize(deserializer).map(JsonValue)
    }
}

/// Results from `lens_diagnostics` carry the diagnostic mode, count of
/// files checked, and how many stale cached entries were dropped.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LensDiagnosticsDetails {
    pub mode: String,
    pub files_checked: u32,
    pub stale_dropped: u32,
}

/// Results from `lsp_diagnostics` come in two shapes discriminated by
/// `mode`: single-file (`"file"`) and batch (`"batch"`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase", deny_unknown_fields)]
pub enum LspDiagnosticsDetails {
    #[serde(rename = "file")]
    File(LspDiagnosticsFile),
    #[serde(rename = "batch")]
    Batch(LspDiagnosticsBatch),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LspDiagnosticsFile {
    pub file_path: PathBuf,
    pub severity: String,
    pub server_scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_server_id: Option<String>,
    pub primary_diagnostics_count: u32,
    pub auxiliary_diagnostics_count: u32,
    pub diagnostics: Vec<JsonValue>,
    pub total_diagnostics: u32,
    pub truncated: bool,
    pub unconfirmed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lsp_health: Option<JsonValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LspDiagnosticsBatch {
    pub files_checked: u32,
    pub concurrency: u32,
    pub severity: String,
    pub server_scope: String,
    pub diagnostics: Vec<JsonValue>,
    pub primary_diagnostics_count: u32,
    pub auxiliary_diagnostics_count: u32,
    pub total_diagnostics: u32,
    pub truncated: bool,
    pub clean_files: u32,
    pub unconfirmed_files: u32,
    pub outcomes: Vec<JsonValue>,
    pub outcome_counts: JsonValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lsp_health_warnings: Option<Vec<String>>,
}

/// Results from `module_report` carry graph availability, staleness, and
/// symbol/callback/export counts.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModuleReportDetails {
    pub available: bool,
    pub staleness: String,
    pub symbols: u32,
    pub exports: u32,
    pub callbacks: u32,
    pub callback_support: String,
    pub view: String,
}

/// Results from `pi_lens_activate_tools` carry which tools matched the
/// request and which were newly activated.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PiLensActivateToolsDetails {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matches: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<String>,
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
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
    /// Whether the intercom message is currently active (pending a response).
    /// Added in newer pi versions; optional for backward compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    /// Request identifier for the intercom message. Added in newer pi
    /// versions; optional for backward compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Each element is kept as a raw `JsonBlob` because the
    /// session-entry schema is owned by the pi runtime and evolves
    /// independently of this parser. Added in newer pi versions;
    /// optional for backward compatibility.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<JsonBlob>,
    /// Subagent runs awaiting a decision from the parent (e.g. `need_decision`).
    /// Added in newer pi versions; optional for backward compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<Vec<IntercomPendingItem>>,
}

/// A subagent run that is waiting for a parent decision via intercom.
/// Pi populates this on the `intercom` tool result's `details.pending`
/// when child subagents reach a `need_decision` state.
///
/// This mirrors [`PendingSupervisorRequest`] used by the
/// `subagent_supervisor` tool, but keeps `child_index` and
/// `expects_reply` optional because the `intercom` tool result
/// wire shape may omit them while the `subagent_supervisor`
/// shape always includes them.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntercomPendingItem {
    pub id: String,
    pub run_id: String,
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_index: Option<u32>,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expects_reply: Option<bool>,
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
    /// Defaults empty because older records predate phase-specific tool tracking.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phase_added_tools: Vec<String>,
    /// True once the `plannotator-framing` custom message has been delivered
    /// for the current phase. Defaults false because older records predate
    /// framing delivery.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub framing_delivered: bool,
}

/// Payload of a `subagent-wait-subscription` custom message. Emitted when a
/// registered `subagent_wait` subscription wakes, carrying the subscription
/// token, the run id that triggered it, and the wake outcome.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentWaitSubscriptionDetails {
    pub token: String,
    pub run_id: String,
    pub outcome: String,
}

/// Structured payload of a `plannotator-framing` custom message. Carries
/// only the phase; the full framing text (rules, instructions) lives in
/// the custom message's outer `content` field.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlannotatorFramingDetails {
    pub phase: PlannotatorPhase,
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
    /// Defaults empty because newer pi versions omit `activeTools` from the
    /// `savedState` payload (only `model` and `thinkingLevel` are emitted).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
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
