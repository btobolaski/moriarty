//! Strongly typed serde models for pi session log lines.
//!
//! A pi session log file is newline-delimited JSON. Each line is a
//! [`PiLogLine`] keyed by the top-level `type` field. All nested payloads are
//! modeled as tagged enums or concrete structs with
//! `#[serde(deny_unknown_fields)]` so that upstream format changes surface as
//! parse errors rather than silent data loss.

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
// discriminator selects a strongly typed payload. Serde rejects
// `deny_unknown_fields` on a struct that also uses `flatten`, so the outer
// wrapper cannot be fully strict here; only the flattened payload enum stays
// tightly typed.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
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
    #[serde(rename = "plannotator-complete")]
    PlannotatorComplete,
    /// Synthetic message injected by the DCP loop asking the assistant to
    /// invoke the `compress` tool. Carries no extra `details` payload; the
    /// human-readable prompt lives in the outer `content` field.
    #[serde(rename = "dcp-compress-trigger")]
    DcpCompressTrigger,
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
}

impl ToolCallArguments {
    pub fn name(&self) -> ToolName {
        match self {
            Self::AskUser(_) => ToolName::AskUser,
            Self::Bash(_) => ToolName::Bash,
            Self::CodeSearch(_) => ToolName::CodeSearch,
            Self::Compress(_) => ToolName::Compress,
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
        }
    }
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadArgs {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BashArgs {
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriteArgs {
    pub path: PathBuf,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditArgs {
    pub path: PathBuf,
    pub edits: Vec<EditReplacement>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditReplacement {
    pub old_text: String,
    pub new_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FindArgs {
    pub pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    Read(ReadDetails),
    Grep(GrepDetails),
    Mcp(McpDetails),
    Bash(BashDetails),
    PlannotatorSubmitPlan(PlannotatorSubmitPlanDetails),
    Todo(TodoDetails),
    Compress(CompressDetails),
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadDetails {
    pub truncation: TruncationInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines_truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_limit_reached: Option<u32>,
}

/// Either or both of `matchLimitReached` and `linesTruncated` may be
/// present depending on which limit (or both) was hit, with at least one
/// always present when `details` is emitted.
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
    /// Path to a temp file containing the full untruncated bash output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_output_path: Option<PathBuf>,
}

/// Truncation metadata shared between `read` and `bash` tool results.
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
    pub saved_state: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebSearchResultsData {
    pub id: String,
    pub timestamp: i64,
    #[serde(rename = "type")]
    pub kind: WebSearchResultsKind,
    pub queries: Vec<WebSearchQueryResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebSearchResultsKind {
    Search,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebSearchQueryResult {
    pub query: String,
    pub answer: String,
    pub results: Vec<WebSearchResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebSearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
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
