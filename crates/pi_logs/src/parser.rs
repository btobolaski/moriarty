//! Strongly typed serde models for pi session log lines.
//!
//! A pi session log file is newline-delimited JSON. Each line is a
//! [`PiLogLine`] keyed by the top-level `type` field. Most nested payloads are
//! modeled as tagged enums or concrete structs with
//! `#[serde(deny_unknown_fields)]` so that upstream format changes surface as
//! parse errors rather than silent data loss.
//!
//! Two categories of structure legitimately deviate from the strict default:
//!
//! * **`serde(flatten)` of an internally-tagged enum** — when the flattened
//!   target is an internally-tagged enum (one with `#[serde(tag = "...")]`
//!   and no `content`), serde's flatten codegen does not register the tag
//!   field as "claimed" by the inner enum, so a strict outer struct rejects
//!   it as unknown. The only struct in this category is
//!   [`WebSearchResultsData`], which therefore omits `deny_unknown_fields`
//!   and relies on the closed-enum discriminator of the flattened payload
//!   plus per-variant strict structs to catch field-level drift. Adjacently
//!   tagged flatten targets (those with both `tag` and `content`) do *not*
//!   suffer this collision, so [`CustomLine`], [`CustomMessageLine`], and
//!   [`ToolCallContent`] all keep `deny_unknown_fields` despite flattening.
//!
//! * **Corrupt-stream tolerance** — some payloads are absorbed via
//!   permissive structs, targeted field aliases, or untagged fallback enums
//!   so a single corrupted record cannot abort an entire log file. Four
//!   flavors exist:
//!     1. Tool-argument structs ([`EditArgs`], [`EditReplacement`],
//!        [`GrepArgs`]) that omit `deny_unknown_fields` to ignore
//!        hallucinated sibling keys (e.g. `:path` on grep).
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
//! Each corrupt-stream exception carries an inline comment naming the
//! observed failure mode.

use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------
// Top-level line
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PiLogLine {
    Session(SessionLine),
    ModelChange(ModelChangeLine),
    ThinkingLevelChange(ThinkingLevelChangeLine),
    Compaction(CompactionLine),
    Custom(CustomLine),
    CustomMessage(CustomMessageLine),
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
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolResultMessage {
    pub tool_call_id: String,
    pub tool_name: ToolName,
    pub content: Vec<TextContentItem>,
    pub is_error: bool,
    pub timestamp: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<ToolResultDetails>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BashExecutionMessage {
    pub command: String,
    pub output: String,
    pub exit_code: i32,
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

/// `Text` and `Thinking` are inline variants of an internally tagged enum, so
/// serde cannot enforce `deny_unknown_fields` on them. `ToolCall` stays strict
/// because it delegates to the `ToolCallContent` struct, which can carry that
/// attribute.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AssistantContentItem {
    #[serde(rename = "text", rename_all = "camelCase")]
    Text {
        text: String,
        /// Opaque provider-supplied signature. Sometimes an opaque token,
        /// sometimes a JSON-encoded object stored as a string. Kept as a
        /// plain string so we never have to speculate about its contents.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text_signature: Option<String>,
    },
    #[serde(rename = "thinking", rename_all = "camelCase")]
    Thinking {
        thinking: String,
        /// Absent on aborted assistant turns where the model produced no
        /// thinking content (and therefore no signature to attest to).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thinking_signature: Option<ThinkingSignature>,
    },
    // Boxed because ToolCallContent is much larger than Text or Thinking.
    #[serde(rename = "toolCall")]
    ToolCall(Box<ToolCallContent>),
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
    #[serde(flatten)]
    pub tool: ToolCallArguments,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_json: Option<String>,
}

impl ToolCallContent {
    pub fn name(&self) -> ToolName {
        self.tool.name()
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
}

// ---------------------------------------------------------------------------
// Closed discriminator enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Anthropic,
    #[serde(rename = "openai")]
    OpenAi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel {
    Off,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AssistantApi {
    #[serde(rename = "anthropic-messages")]
    AnthropicMessages,
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolName {
    AskUser,
    Bash,
    CodeSearch,
    Compress,
    Edit,
    FactDelete,
    FactList,
    FactRead,
    FactWrite,
    FetchContent,
    Find,
    GetSearchContent,
    Grep,
    InstinctDelete,
    InstinctList,
    InstinctMerge,
    InstinctRead,
    InstinctWrite,
    Intercom,
    Ls,
    Mcp,
    PlannotatorSubmitPlan,
    Read,
    Subagent,
    SubagentStatus,
    Todo,
    WebSearch,
    Write,
    // Tools provided by the pi-lean-ctx extension. They appear in the
    // `pi-loaded-tools` manifest of sessions where lean-ctx is loaded; we
    // do not model their argument schemas because we never invoke them
    // directly.
    CtxAgent,
    CtxAnalyze,
    CtxArchitecture,
    CtxBenchmark,
    CtxCache,
    CtxCallees,
    CtxCallers,
    CtxCompress,
    CtxCompressMemory,
    CtxContext,
    CtxCost,
    CtxDedup,
    CtxDelta,
    CtxDiscover,
    CtxEdit,
    CtxExecute,
    CtxExpand,
    CtxFeedback,
    CtxFill,
    CtxGain,
    CtxGraph,
    CtxGraphDiagram,
    CtxHandoff,
    CtxHeatmap,
    CtxImpact,
    CtxIntent,
    CtxKnowledge,
    CtxMetrics,
    CtxOutline,
    CtxOverview,
    CtxPrefetch,
    CtxPreload,
    CtxResponse,
    CtxRoutes,
    CtxSemanticSearch,
    CtxSession,
    CtxShare,
    CtxSmartRead,
    CtxSymbol,
    CtxTask,
    CtxWorkflow,
    CtxWrapped,
    // MCP-server tools surfaced as flat top-level names by the
    // pi-tool-display extension (rather than going through the generic
    // `mcp` tool). They appear in the `pi-loaded-tools` manifest and as
    // direct toolCalls in assistant messages.
    GitReadOnlyDiff,
    GitReadOnlyLog,
    GitReadOnlyShow,
    GitReadOnlyStatus,
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
//
// Each tool has a well known argument schema. We model tool calls as a tagged
// enum keyed by the sibling `name` field so zero-argument tools stay tied to
// their declared tool name instead of falling through to whichever all-optional
// struct happens to appear first.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "name", content = "arguments", rename_all = "snake_case")]
pub enum ToolCallArguments {
    AskUser(AskUserArgs),
    Bash(BashArgs),
    CodeSearch(CodeSearchArgs),
    Compress(CompressArgs),
    // `ctx_cache` is the only `ctx_*` extension tool the assistant invokes
    // directly; the other 30+ `Ctx*` variants in `ToolName` are surfaced only
    // through the `pi-loaded-tools` manifest and never appear as tool calls,
    // so we deliberately do not model their argument schemas here.
    CtxCache(CtxCacheArgs),
    Edit(EditArgs),
    FactDelete(FactDeleteArgs),
    FactList(FactListArgs),
    FactRead(FactReadArgs),
    FactWrite(FactWriteArgs),
    FetchContent(FetchContentArgs),
    Find(FindArgs),
    GetSearchContent(GetSearchContentArgs),
    Grep(GrepArgs),
    InstinctDelete(InstinctDeleteArgs),
    InstinctList(InstinctListArgs),
    InstinctMerge(InstinctMergeArgs),
    InstinctRead(InstinctReadArgs),
    InstinctWrite(InstinctWriteArgs),
    Intercom(IntercomArgs),
    Ls(LsArgs),
    Mcp(McpArgs),
    PlannotatorSubmitPlan(PlannotatorSubmitPlanArgs),
    Read(ReadArgs),
    Subagent(SubagentArgs),
    SubagentStatus(SubagentStatusArgs),
    Todo(TodoArgs),
    WebSearch(WebSearchArgs),
    Write(WriteArgs),
    GitReadOnlyDiff(GitReadOnlyArgs),
    GitReadOnlyLog(GitReadOnlyArgs),
    GitReadOnlyShow(GitReadOnlyArgs),
    GitReadOnlyStatus(GitReadOnlyArgs),
}

impl ToolCallArguments {
    pub fn name(&self) -> ToolName {
        match self {
            Self::AskUser(_) => ToolName::AskUser,
            Self::Bash(_) => ToolName::Bash,
            Self::CodeSearch(_) => ToolName::CodeSearch,
            Self::Compress(_) => ToolName::Compress,
            Self::CtxCache(_) => ToolName::CtxCache,
            Self::Edit(_) => ToolName::Edit,
            Self::FactDelete(_) => ToolName::FactDelete,
            Self::FactList(_) => ToolName::FactList,
            Self::FactRead(_) => ToolName::FactRead,
            Self::FactWrite(_) => ToolName::FactWrite,
            Self::FetchContent(_) => ToolName::FetchContent,
            Self::Find(_) => ToolName::Find,
            Self::GetSearchContent(_) => ToolName::GetSearchContent,
            Self::Grep(_) => ToolName::Grep,
            Self::InstinctDelete(_) => ToolName::InstinctDelete,
            Self::InstinctList(_) => ToolName::InstinctList,
            Self::InstinctMerge(_) => ToolName::InstinctMerge,
            Self::InstinctRead(_) => ToolName::InstinctRead,
            Self::InstinctWrite(_) => ToolName::InstinctWrite,
            Self::Intercom(_) => ToolName::Intercom,
            Self::Ls(_) => ToolName::Ls,
            Self::Mcp(_) => ToolName::Mcp,
            Self::PlannotatorSubmitPlan(_) => ToolName::PlannotatorSubmitPlan,
            Self::Read(_) => ToolName::Read,
            Self::Subagent(_) => ToolName::Subagent,
            Self::SubagentStatus(_) => ToolName::SubagentStatus,
            Self::Todo(_) => ToolName::Todo,
            Self::WebSearch(_) => ToolName::WebSearch,
            Self::Write(_) => ToolName::Write,
            Self::GitReadOnlyDiff(_) => ToolName::GitReadOnlyDiff,
            Self::GitReadOnlyLog(_) => ToolName::GitReadOnlyLog,
            Self::GitReadOnlyShow(_) => ToolName::GitReadOnlyShow,
            Self::GitReadOnlyStatus(_) => ToolName::GitReadOnlyStatus,
        }
    }
}

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
}

/// Numeric tool-call arguments that pi normally records as integers, but
/// which corrupted model streams have been observed to emit as the
/// JSON-Schema field name as a string value (e.g. `"limit": "limit"`).
///
/// `Garbage` only absorbs JSON-string-typed corruption. Other malformed
/// shapes — booleans (`true`), `null`, floats, or out-of-range integers —
/// still produce a hard parse error because they match neither variant.
/// We constrain the fallback this narrowly so unexpected new corruption
/// modes stay loud at parse time.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MaybeU32 {
    Number(u32),
    Garbage(String),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadArgs {
    /// Optional because aborted toolCalls (`stopReason: "aborted"`) can
    /// land in the log with `arguments: {}` before the model finished
    /// streaming a path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<MaybeU32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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

/// `ctx_cache` is the only `ctx_*` extension tool we model with typed
/// argument structs. Unlike its siblings (which only ever appear in the
/// `pi-loaded-tools` manifest), `ctx_cache` is invoked directly by the
/// assistant in real session logs, so we need its argument schema to
/// deserialize those tool calls cleanly.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CtxCacheArgs {
    pub action: String,
    pub path: PathBuf,
}

/// `deny_unknown_fields` is intentionally omitted. Models occasionally
/// hallucinate sibling keys here — we've observed gpt-5.4 emitting
/// `:path` alongside `path`, and Sonnet emitting an `offset` parameter
/// that grep does not support. Tolerating unknown fields keeps these
/// otherwise-valid tool calls parseable.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrepArgs {
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

/// `limit` accepts the observed `.limit` corruption as an alias. Some
/// completed tool calls have emitted a leading `.` in the key, and we want
/// that single malformed argument to stay tied to the intended field instead
/// of poisoning the whole log line.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FindArgs {
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
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AskUserOption {
    Title(String),
    Detailed {
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contradicted_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
pub struct SubagentArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tasks: Option<Vec<SubagentTask>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<u32>,
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
    pub output: Option<SubagentOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// JSON-encoded agent or chain configuration passed to `subagent` management
    /// actions (`create`, `update`). Kept as a string because pi serializes the
    /// full config blob as a string argument.
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
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SubagentOutput {
    Path(String),
    Enabled(bool),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentChainStep {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
}

// ---------------------------------------------------------------------------
// Tool result details
//
// `details` on a toolResult message varies by tool. We use an untagged enum
// where each variant is a struct that `deny_unknown_fields`. This preserves
// strict parsing while matching the flat JSON layout produced by pi.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolResultDetails {
    Edit(EditDetails),
    Subagent(SubagentResultDetails),
    AskUser(AskUserDetails),
    CodeSearch(CodeSearchDetails),
    WebSearch(WebSearchDetails),
    // Grep precedes Read because both share `matchLimitReached` and
    // `linesTruncated`, and ReadDetails is now permissive enough (its
    // `truncation` field is optional to support lean-ctx augmentations) that
    // a bare `{matchLimitReached, linesTruncated}` payload would otherwise
    // be matched by ReadDetails first.
    //
    // Reciprocal-risk note: this ordering is asymmetric. A legitimate
    // pattern-mode `read` result whose details payload only carries
    // `{matchLimitReached, linesTruncated}` will silently classify as
    // `Grep(GrepDetails)` rather than `Read(ReadDetails)`. The two structs
    // share those exact fields, so no data is lost — and `tool_name` on
    // the surrounding message disambiguates the originating tool — but
    // typed-enum routing alone cannot tell them apart from this payload
    // shape. Consumers that branch on the enum variant (rather than on
    // `tool_name`) must treat `Grep` as covering both cases.
    Grep(GrepDetails),
    Read(ReadDetails),
    Mcp(McpDetails),
    Bash(BashDetails),
    PlannotatorSubmitPlan(PlannotatorSubmitPlanDetails),
    Todo(TodoDetails),
    Compress(CompressDetails),
    // Ls precedes Find because their lean-ctx augmentation shapes overlap on
    // path/source/truncated/compression. The discriminator is `pattern`:
    // find's lean-ctx output always carries it, ls never does. With Ls first,
    // a payload without `pattern` matches Ls; a payload with `pattern` is
    // rejected by LsDetails (deny_unknown_fields) and falls through to Find.
    Ls(LsDetails),
    Find(FindDetails),
    // GitReadOnly and FetchContent have no shape overlap with anything
    // above (each carries fields no other variant declares).
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_changed_line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubagentResultDetails {
    pub mode: String,
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
    pub attempted_models: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_attempts: Option<Vec<SubagentModelAttempt>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<SubagentToolCallSummary>>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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
}

/// `read` emits two sub-shapes for `details` that classify here: a plain
/// truncation block when raw byte/line caps fired, or a lean-ctx augmented
/// summary describing how the extension compressed the response. All fields
/// are optional because either may be present alone, both together, or
/// neither (for plain successful reads).
///
/// A third shape — the pattern-scoped match summary `read` emits when it
/// performed grep-style filtering and hit its match or line cap — carries
/// only `{matchLimitReached, linesTruncated}`, which `GrepDetails` declares
/// with the same field names. Because `Grep` precedes `Read` in the
/// untagged `ToolResultDetails` enum (see the ordering comment there),
/// pattern-mode read results land in `Grep(GrepDetails)`, never here. The
/// `lines_truncated` and `match_limit_reached` fields below remain only to
/// keep this struct accepting any payload that contains them, so an
/// unexpected variant ordering change does not silently start failing
/// parses; in practice they are unreachable through the enum dispatch.
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

/// `find` emits two shapes for `details`: a plain `{resultLimitReached}`
/// when the result list was capped, or a lean-ctx augmented shape carrying
/// the queried path/pattern plus a `compression` breadcrumb. All fields are
/// optional because either shape may appear independently.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FindDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_limit_reached: Option<u32>,
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

/// Summary metadata recorded by the `fetch_content` tool. `prompt` is only
/// present when the caller passed an explicit Gemini prompt for video / page
/// analysis; the rest of the fields are always emitted.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FetchContentDetails {
    pub urls: Vec<String>,
    pub url_count: u32,
    pub successful: u32,
    pub total_chars: u64,
    pub title: String,
    pub response_id: String,
    pub truncated: bool,
    pub has_image: bool,
    pub image_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
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
}

/// Either of `matchLimitReached` / `linesTruncated` may be present when
/// raw grep hit its caps. Lean-ctx augmented grep results add the queried
/// `path`/`pattern` plus a `compression` breadcrumb instead.
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
    pub path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ToolResultSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression: Option<CompressionInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpDetails {
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_result: Option<McpCallResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Set on `mode: "call"` errors of kind `tool_not_found`; names the
    /// missing MCP tool the caller asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_tool: Option<String>,
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
    pub content: Vec<TextContentItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<McpStructuredContent>,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpStructuredContent {
    pub exit_code: i32,
    pub stderr: String,
    pub stdout: String,
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
    pub active_tools: Vec<ToolName>,
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
}

/// `web-search-results` payload. The `type` discriminator selects between
/// search results (`queries`) and direct URL fetches (`urls`); the shared
/// `id` and `timestamp` live on the outer struct alongside the flattened
/// payload. `deny_unknown_fields` cannot be used here because
/// `WebSearchResultsPayload` is an *internally* tagged enum (`tag = "type"`
/// without `content`), so its `type` discriminator appears at the same JSON
/// level as the outer struct's fields. Serde's flatten codegen does not
/// register the discriminator as "claimed", and the strict outer struct then
/// rejects it as unknown. The inner payload variants stay strict via their
/// own structs, which is enough to catch field-level drift inside `data`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchResultsData {
    pub id: String,
    pub timestamp: i64,
    #[serde(flatten)]
    pub payload: WebSearchResultsPayload,
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
    pub name: ToolName,
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
        if line.is_empty() {
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
