use std::collections::HashMap;
#[cfg(test)]
use std::path::Path;

use chrono::{DateTime, NaiveDate, Utc};
#[cfg(test)]
use miette::IntoDiagnostic;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use tokio::fs::read_to_string;
use uuid::Uuid;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct QueueOperation {
    pub operation: String,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
    pub session_id: String,
}

/// Progress events from Claude Code 2.1+.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ProgressLogLine {
    pub parent_uuid: Uuid,
    pub is_sidechain: bool,
    pub user_type: String,
    pub cwd: String,
    pub session_id: Uuid,
    pub version: String,
    pub git_branch: String,
    /// Identifier for the agent/task that created this message. None for main conversation messages.
    pub agent_id: Option<String>,
    /// Session slug identifier (e.g., "noble-floating-lemon"). Added in Claude Code 2.0.51.
    pub slug: Option<String>,
    pub data: ProgressData,
    #[serde(rename = "toolUseID")]
    pub tool_use_id: String,
    #[serde(rename = "parentToolUseID")]
    pub parent_tool_use_id: String,
    pub uuid: Uuid,
    pub timestamp: DateTime<Utc>,
    /// Entry point that started the session (e.g., "cli"). Added in Claude Code 2.1.104+.
    pub entrypoint: Option<String>,
}

/// Progress event data types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ProgressData {
    HookProgress(HookProgressData),
    McpProgress(McpProgressData),
    BashProgress(BashProgressData),
    AgentProgress(Box<AgentProgressData>),
    WaitingForTask(WaitingForTaskData),
    QueryUpdate(QueryUpdateData),
    SearchResultsReceived(SearchResultsReceivedData),
}

/// Hook progress event data for tracking hook execution.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct HookProgressData {
    pub hook_event: String,
    pub hook_name: String,
    pub command: String,
}

/// MCP progress event data for tracking MCP tool execution.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct McpProgressData {
    pub status: String,
    pub server_name: String,
    pub tool_name: String,
    /// Elapsed time in milliseconds (present when status is "completed").
    pub elapsed_time_ms: Option<u64>,
}

/// Bash command progress event data for tracking long-running shell commands.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct BashProgressData {
    pub output: String,
    pub full_output: String,
    pub elapsed_time_seconds: u64,
    pub total_lines: usize,
}

/// Agent progress event data for tracking sub-agent execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct AgentProgressData {
    pub message: AgentProgressMessage,
    pub normalized_messages: Option<Vec<AgentProgressMessage>>,
    pub prompt: String,
    pub agent_id: String,
    /// Agent ID to resume from a previous execution.
    pub resume: Option<String>,
}

/// Message wrapper used in agent progress events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
pub enum AgentProgressMessage {
    #[serde(rename = "user")]
    #[serde(rename_all = "camelCase")]
    User {
        message: LogMessage,
        uuid: Uuid,
        timestamp: DateTime<Utc>,
        /// Tool use result data, present when this is a tool result message.
        tool_use_result: Option<serde_json::Value>,
    },
    #[serde(rename = "assistant")]
    #[serde(rename_all = "camelCase")]
    Assistant {
        message: Box<AssistantLogMessage>,
        request_id: String,
        uuid: Uuid,
        timestamp: DateTime<Utc>,
    },
    /// Progress message nested within agent progress.
    #[serde(rename = "progress")]
    #[serde(rename_all = "camelCase")]
    Progress {
        data: NestedProgressData,
        #[serde(rename = "toolUseID")]
        tool_use_id: String,
        #[serde(rename = "parentToolUseID")]
        parent_tool_use_id: String,
        uuid: Uuid,
        timestamp: DateTime<Utc>,
    },
    /// Attachment message containing hook execution results or failure details.
    /// Uses serde_json::Value because attachment schemas vary by hook type.
    #[serde(rename = "attachment")]
    #[serde(rename_all = "camelCase")]
    Attachment {
        attachment: serde_json::Value,
        uuid: Uuid,
        timestamp: DateTime<Utc>,
    },
}

/// Nested progress data within agent progress normalizedMessages.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum NestedProgressData {
    HookProgress(HookProgressData),
    McpProgress(McpProgressData),
    BashProgress(BashProgressData),
    QueryUpdate(QueryUpdateData),
    SearchResultsReceived(SearchResultsReceivedData),
}

/// Waiting for task progress data for tracking background task status.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct WaitingForTaskData {
    pub task_description: String,
    pub task_type: String,
}

/// Emitted by Claude Code during web searches when the search query is refined or updated.
/// Allows tracking query evolution during agent research phases.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct QueryUpdateData {
    pub query: String,
}

/// Emitted by Claude Code when web search results are received from the search backend.
/// Includes result count to track search effectiveness and query quality.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SearchResultsReceivedData {
    pub result_count: u32,
    pub query: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
pub enum LogLine {
    #[serde(rename = "user")]
    User(UserLogLine),
    #[serde(rename = "assistant")]
    Assistant(Box<AssistantLogLine>),
    #[serde(rename = "file-history-snapshot")]
    FileHistorySnapshot(FileHistorySnapshot),
    #[serde(rename = "summary")]
    Summary(Summary),
    #[serde(rename = "system")]
    System(SystemLogLine),
    #[serde(rename = "queue-operation")]
    QueueOperation(QueueOperation),
    #[serde(rename = "progress")]
    Progress(ProgressLogLine),
    #[serde(rename = "custom-title")]
    CustomTitle(CustomTitle),
    #[serde(rename = "ai-title")]
    AiTitle(AiTitle),
    #[serde(rename = "agent-name")]
    AgentName(AgentName),
    #[serde(rename = "last-prompt")]
    LastPrompt(LastPrompt),
    #[serde(rename = "permission-mode")]
    PermissionModeChange(PermissionModeChange),
    #[serde(rename = "mode")]
    Mode(ModeLine),
    #[serde(rename = "attachment")]
    Attachment(Box<AttachmentLogLine>),
    #[serde(rename = "pr-link")]
    PrLink(PrLink),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CustomTitle {
    pub custom_title: String,
    pub session_id: Uuid,
}

/// AI-generated conversation title. Added in Claude Code 2.1.141+ alongside the existing
/// `custom-title` records to capture titles Claude Code derives automatically from the
/// conversation rather than ones the user provides.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct AiTitle {
    pub ai_title: String,
    pub session_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct AgentName {
    pub agent_name: String,
    pub session_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct LastPrompt {
    /// Optional starting in Claude Code 2.1.141+ — newer entries can identify the prompt via
    /// `leaf_uuid` alone without storing the prompt text.
    pub last_prompt: Option<String>,
    pub leaf_uuid: Option<Uuid>,
    pub session_id: Uuid,
}

/// Permission mode change event. Added in Claude Code 2.1.104+.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PermissionModeChange {
    pub permission_mode: PermissionMode,
    pub session_id: Uuid,
}

/// `mode` log line recording the session's operating mode. Added in Claude Code 2.1.158+.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ModeLine {
    pub mode: SessionMode,
    pub session_id: Uuid,
}

/// Carried by the `mode` log line; `normal` is the only value observed so far. Kept a closed enum
/// (no `#[serde(other)]` catch-all) so an unrecognized mode fails to parse instead of being silently
/// accepted, matching the strict-parse convention this crate uses for Claude Code protocol data: a
/// new mode surfaces as a parse error rather than passing unnoticed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionMode {
    Normal,
}

/// Associates the session with a GitHub pull request Claude Code opened or updated; a session can
/// emit several (one per PR it touches). Added in Claude Code 2.1.158+. `pr_number` is `u64` to
/// avoid ever rejecting a large upstream value, even though real PR numbers stay small.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PrLink {
    pub session_id: Uuid,
    pub pr_number: u64,
    pub pr_url: String,
    pub pr_repository: String,
    pub timestamp: DateTime<Utc>,
}

/// Attachment log line for deferred tools, hooks, and other metadata. Added in Claude Code 2.1.104+.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct AttachmentLogLine {
    pub parent_uuid: Option<Uuid>,
    pub is_sidechain: bool,
    /// Identifier for the subagent that emitted this attachment. Only set on subagent transcripts.
    pub agent_id: Option<String>,
    pub attachment: AttachmentData,
    pub uuid: Uuid,
    pub timestamp: DateTime<Utc>,
    pub user_type: String,
    pub entrypoint: Option<String>,
    pub cwd: String,
    pub session_id: Uuid,
    pub version: String,
    pub git_branch: String,
    pub slug: Option<String>,
}

/// Attachment payload types. Added in Claude Code 2.1.104+.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum AttachmentData {
    AutoMode(AutoMode),
    AutoModeExit(AutoModeExit),
    CommandPermissions(CommandPermissions),
    CompactFileReference(CompactFileReference),
    DateChange(DateChange),
    DeferredToolsDelta(DeferredToolsDelta),
    Directory(DirectoryAttachment),
    EditedTextFile(EditedTextFile),
    File(FileAttachment),
    HookBlockingError(HookBlockingError),
    HookCancelled(HookCancelled),
    HookNonBlockingError(HookNonBlockingError),
    HookPermissionDecision(HookPermissionDecision),
    HookSuccess(HookSuccess),
    HookSystemMessage(HookSystemMessage),
    McpInstructionsDelta(McpInstructionsDelta),
    NestedMemory(NestedMemory),
    PlanFileReference(PlanFileReference),
    PlanMode(PlanModeAttachment),
    PlanModeExit(PlanModeExitAttachment),
    PlanModeReentry(PlanModeReentryAttachment),
    QueuedCommand(QueuedCommand),
    SkillListing(SkillListing),
    TaskReminder(TaskReminder),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct AutoMode {
    pub reminder_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutoModeExit {}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CommandPermissions {
    pub allowed_tools: Vec<String>,
}

/// File whose prior reference is carried across a compaction so the post-compaction context still
/// knows it was seen. Added in Claude Code 2.1.158+.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CompactFileReference {
    pub filename: String,
    pub display_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct DateChange {
    pub new_date: NaiveDate,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct DeferredToolsDelta {
    pub added_names: Vec<String>,
    pub added_lines: Vec<String>,
    pub removed_names: Vec<String>,
    #[serde(default)]
    pub readded_names: Vec<String>,
    #[serde(default)]
    pub pending_mcp_servers: Vec<String>,
}

/// Directory contents attached to a turn (e.g. via an `@dir` reference); `content` is a
/// newline-separated listing of the directory's immediate entries. Added in Claude Code 2.1.158+.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct DirectoryAttachment {
    pub path: String,
    pub content: String,
    pub display_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct EditedTextFile {
    pub filename: String,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct FileAttachment {
    pub filename: String,
    pub content: FileAttachmentContent,
    pub display_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum FileAttachmentContent {
    Text { file: FileAttachmentTextBody },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct FileAttachmentTextBody {
    pub file_path: String,
    pub content: String,
    pub num_lines: u32,
    pub start_line: u32,
    pub total_lines: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct HookBlockingError {
    pub hook_name: String,
    #[serde(rename = "toolUseID")]
    pub tool_use_id: String,
    pub hook_event: String,
    pub blocking_error: BlockingErrorDetails,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct BlockingErrorDetails {
    pub blocking_error: String,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct HookCancelled {
    pub hook_name: String,
    #[serde(rename = "toolUseID")]
    pub tool_use_id: String,
    pub hook_event: String,
    pub command: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct HookNonBlockingError {
    pub hook_name: String,
    #[serde(rename = "toolUseID")]
    pub tool_use_id: String,
    pub hook_event: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub command: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct HookPermissionDecision {
    pub decision: PermissionDecisionKind,
    #[serde(rename = "toolUseID")]
    pub tool_use_id: String,
    pub hook_event: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionDecisionKind {
    Allow,
    Deny,
    Ask,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct HookSuccess {
    pub hook_name: String,
    #[serde(rename = "toolUseID")]
    pub tool_use_id: String,
    pub hook_event: String,
    pub content: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub command: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct HookSystemMessage {
    pub content: String,
    pub hook_name: String,
    #[serde(rename = "toolUseID")]
    pub tool_use_id: String,
    pub hook_event: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct McpInstructionsDelta {
    pub added_names: Vec<String>,
    pub added_blocks: Vec<String>,
    pub removed_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct NestedMemory {
    pub path: String,
    pub content: NestedMemoryContent,
    pub display_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct NestedMemoryContent {
    pub path: String,
    pub r#type: String,
    pub content: String,
    pub content_differs_from_disk: bool,
    // Claude Code only populates this when content_differs_from_disk is true;
    // it holds the unprocessed on-disk text before template/diff handling. The
    // type does not enforce that pairing — it is an upstream protocol invariant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_content: Option<String>,
}

/// Plan file surfaced into the conversation with its full text inlined in `plan_content`; the
/// `plan_mode*` reminders identify their plan only by path. Added in Claude Code 2.1.158+.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PlanFileReference {
    pub plan_file_path: String,
    pub plan_content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PlanModeAttachment {
    pub reminder_type: String,
    pub is_sub_agent: bool,
    pub plan_file_path: String,
    pub plan_exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PlanModeExitAttachment {
    pub plan_file_path: String,
    pub plan_exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PlanModeReentryAttachment {
    pub plan_file_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct QueuedCommand {
    pub prompt: String,
    pub command_mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SkillListing {
    pub content: String,
    pub skill_count: u32,
    pub is_initial: bool,
    /// Skill names summarized in `content`; absent in older Claude Code logs that listed only
    /// `content`. Added in Claude Code 2.1.158+.
    pub names: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct TaskReminder {
    pub content: Vec<TaskReminderItem>,
    pub item_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct TaskReminderItem {
    pub id: String,
    pub subject: String,
    pub description: String,
    pub active_form: Option<String>,
    pub status: String,
    pub blocks: Vec<String>,
    pub blocked_by: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "subtype")]
#[serde(rename_all = "snake_case")]
pub enum SystemLogLine {
    Error(SystemLogError),
    CompactBoundary(CompactBoundary),
    MicrocompactBoundary(MicrocompactBoundary),
    Informational(SystemLogInformational),
    ApiError(SystemLogError),
    LocalCommand(LocalCommandLog),
    ScheduledTaskFire(ScheduledTaskFire),
    StopHookSummary(StopHookSummary),
    TurnDuration(TurnDuration),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct LocalCommandLog {
    pub parent_uuid: Option<Uuid>,
    pub is_sidechain: bool,
    pub user_type: String,
    pub cwd: String,
    pub session_id: Uuid,
    pub version: String,
    pub git_branch: String,
    /// Session slug identifier (e.g., "noble-floating-lemon"). Added in Claude Code 2.0.51.
    pub slug: Option<String>,
    pub content: String,
    pub level: String,
    pub timestamp: DateTime<Utc>,
    pub uuid: Uuid,
    pub is_meta: bool,
    /// Entry point that started the session (e.g., "cli"). Added in Claude Code 2.1.104+.
    pub entrypoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct StopHookSummary {
    pub parent_uuid: Uuid,
    pub is_sidechain: bool,
    pub user_type: String,
    pub cwd: String,
    pub session_id: Uuid,
    pub version: String,
    pub git_branch: String,
    /// Session slug identifier (e.g., "noble-floating-lemon"). Added in Claude Code 2.0.51.
    pub slug: Option<String>,
    pub hook_count: usize,
    pub hook_infos: Vec<HookInfo>,
    pub hook_errors: Vec<HookError>,
    pub prevented_continuation: bool,
    pub stop_reason: String,
    pub has_output: bool,
    pub level: String,
    pub timestamp: DateTime<Utc>,
    pub uuid: Uuid,
    #[serde(rename = "toolUseID")]
    pub tool_use_id: String,
    /// Entry point that started the session (e.g., "cli"). Added in Claude Code 2.1.104+.
    pub entrypoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct HookInfo {
    pub command: String,
    pub duration_ms: Option<u64>,
}

/// Hook errors from Claude Code, supporting both legacy and current formats to maintain
/// backward compatibility when parsing logs from different Claude Code versions. Uses untagged
/// serde to automatically deserialize either format without requiring version detection logic.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HookError {
    /// String-only format introduced in Claude Code 2.0.47 to simplify error reporting
    /// when command and exit_code details aren't available or relevant
    String(String),
    /// Structured format used in earlier versions to provide additional debugging context
    /// about which hook failed and how
    Structured(HookErrorDetails),
}

// Accessors that preserve a uniform interface across the legacy string-only and
// structured hook error formats. Keep these available in normal builds so
// downstream crates can inspect parsed hook errors without matching variants.
impl HookError {
    pub fn message(&self) -> &str {
        match self {
            HookError::String(s) => s,
            HookError::Structured(details) => &details.message,
        }
    }

    pub fn command(&self) -> Option<&str> {
        match self {
            HookError::String(_) => None,
            HookError::Structured(details) => details.command.as_deref(),
        }
    }

    pub fn exit_code(&self) -> Option<i32> {
        match self {
            HookError::String(_) => None,
            HookError::Structured(details) => details.exit_code,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct HookErrorDetails {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SystemLogError {
    pub parent_uuid: Uuid,
    pub is_sidechain: bool,
    pub user_type: String,
    pub cwd: String,
    pub session_id: String,
    pub version: String,
    pub git_branch: String,
    /// Session slug identifier (e.g., "noble-floating-lemon"). Added in Claude Code 2.0.51.
    pub slug: Option<String>,
    pub level: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<serde_json::Value>,
    pub error: SystemErrorBody,
    pub retry_in_ms: f64,
    pub retry_attempt: usize,
    pub max_retries: usize,
    pub timestamp: DateTime<Utc>,
    pub uuid: Uuid,
    /// Entry point that started the session (e.g., "cli"). Added in Claude Code 2.1.104+.
    pub entrypoint: Option<String>,
}

/// The `error` payload of a system `error`/`api_error` line arrives in two distinct
/// envelopes with no discriminator field, so they are matched structurally. `Client`
/// is listed first because its required `message`/`formatted` fields are absent from
/// the SDK envelope; this makes disambiguation depend on differing *required* fields
/// rather than on `deny_unknown_fields` being honored inside `#[serde(untagged)]`,
/// which serde does not guarantee.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SystemErrorBody {
    /// Claude Code's networking-layer error wrapper (seen from Claude Code 2.1.158):
    /// a human-formatted message plus connection / rate-limit diagnostics.
    Client(SystemLogClientError),
    /// Anthropic SDK `APIError` shape: HTTP status with optional headers/requestID/cause.
    Sdk(SystemLogErrorError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SystemLogClientError {
    pub message: String,
    pub status: u16,
    pub request_id: String,
    pub formatted: String,
    /// Only ever observed as JSON null, but always present on the wire, so (unlike the
    /// genuinely-optional fields elsewhere) it is serialized as explicit null rather than
    /// skipped. Typed as `Option<()>` rather than a guessed struct or `serde_json::Value`
    /// so any future populated payload fails to parse, surfacing as a partial-failure
    /// warning that forces us to model its real shape.
    pub connection: Option<()>,
    pub is_network_down: bool,
    /// See `connection`: always present as null so far; a populated value must break parsing.
    pub rate_limits: Option<()>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SystemLogErrorError {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
    /// This envelope's wire key is `requestID` (capital D), intentionally distinct from
    /// `SystemLogClientError`'s `requestId` — they are different upstream shapes, so do not
    /// "normalize" the capitalization or one of the two envelopes will stop parsing.
    #[serde(rename = "requestID")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<serde_json::Value>,
}

macro_rules! define_boundary_log {
    (
        $(#[$struct_meta:meta])*
        $name:ident,
        parent_uuid: $parent_uuid:ty,
        $(logical_parent_uuid: $logical_parent_uuid:ty,)?
        metadata: $metadata_field:ident => $metadata_ty:ty,
        derives: [$($derive:tt)+]
    ) => {
        $(#[$struct_meta])*
        #[derive($($derive)+)]
        #[serde(rename_all = "camelCase")]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            pub parent_uuid: $parent_uuid,
            $(pub logical_parent_uuid: $logical_parent_uuid,)?
            pub is_sidechain: bool,
            pub user_type: String,
            pub cwd: String,
            pub session_id: Uuid,
            pub version: String,
            pub git_branch: String,
            /// Session slug identifier (e.g., "noble-floating-lemon"). Added in Claude Code 2.0.51.
            pub slug: Option<String>,
            pub content: String,
            pub is_meta: bool,
            pub timestamp: DateTime<Utc>,
            pub uuid: Uuid,
            pub level: String,
            pub $metadata_field: $metadata_ty,
            /// Entry point that started the session (e.g., "cli"). Added in Claude Code 2.1.104+.
            pub entrypoint: Option<String>,
        }
    };
}

define_boundary_log! {
    CompactBoundary,
    parent_uuid: Option<Uuid>,
    logical_parent_uuid: Uuid,
    metadata: compact_metadata => CompactMetadata,
    derives: [Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize]
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SystemLogInformational {
    pub parent_uuid: Uuid,
    pub is_sidechain: bool,
    pub git_branch: Option<String>,
    /// Session slug identifier (e.g., "noble-floating-lemon"). Added in Claude Code 2.0.51.
    pub slug: Option<String>,
    pub user_type: String,
    pub cwd: String,
    pub session_id: Uuid,
    pub version: String,
    pub content: String,
    pub is_meta: bool,
    pub timestamp: DateTime<Utc>,
    pub uuid: Uuid,
    pub level: String,
    /// Entry point that started the session (e.g., "cli"). Added in Claude Code 2.1.104+.
    pub entrypoint: Option<String>,
}

/// `trigger` and `pre_tokens` are the only fields present before Claude Code 2.1.158; the rest
/// arrived in 2.1.158 with the preserved-segment feature and stay `Option` so the older two-field
/// records still parse.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CompactMetadata {
    pub trigger: String,
    pub pre_tokens: usize,
    pub post_tokens: Option<usize>,
    pub duration_ms: Option<u64>,
    pub pre_compact_discovered_tools: Option<Vec<String>>,
    pub preserved_segment: Option<PreservedSegment>,
    pub preserved_messages: Option<PreservedMessages>,
}

/// Added in Claude Code 2.1.158+.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PreservedSegment {
    pub head_uuid: Uuid,
    pub anchor_uuid: Uuid,
    pub tail_uuid: Uuid,
}

/// Added in Claude Code 2.1.158+.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PreservedMessages {
    pub anchor_uuid: Uuid,
    pub uuids: Vec<Uuid>,
    pub all_uuids: Vec<Uuid>,
}

define_boundary_log! {
    /// Microcompact boundary event from Claude Code 2.1.12+. Unlike full compaction,
    /// microcompaction selectively removes tool use content to reduce context size while
    /// preserving conversation flow.
    MicrocompactBoundary,
    parent_uuid: Uuid,
    metadata: microcompact_metadata => MicrocompactMetadata,
    derives: [Debug, Clone, PartialEq, Eq, Serialize, Deserialize]
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct MicrocompactMetadata {
    pub trigger: String,
    pub pre_tokens: usize,
    pub tokens_saved: usize,
    pub compacted_tool_ids: Vec<String>,
    #[serde(rename = "clearedAttachmentUUIDs")]
    pub cleared_attachment_uuids: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ScheduledTaskFire {
    pub parent_uuid: Option<Uuid>,
    pub is_sidechain: bool,
    pub user_type: String,
    pub cwd: String,
    pub session_id: Uuid,
    pub version: String,
    pub git_branch: String,
    /// Session slug identifier (e.g., "noble-floating-lemon"). Added in Claude Code 2.0.51.
    pub slug: Option<String>,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub uuid: Uuid,
    pub is_meta: bool,
    /// Entry point that started the session (e.g., "cli"). Added in Claude Code 2.1.104+.
    pub entrypoint: Option<String>,
}

/// Duration of a single turn (user message → assistant response cycle).
/// Added in Claude Code 2.0.51+ for performance tracking.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct TurnDuration {
    pub parent_uuid: Uuid,
    pub is_sidechain: bool,
    pub user_type: String,
    pub cwd: String,
    pub session_id: Uuid,
    pub version: String,
    pub git_branch: String,
    /// Session slug identifier (e.g., "noble-floating-lemon"). Added in Claude Code 2.0.51.
    pub slug: Option<String>,
    pub duration_ms: u64,
    pub timestamp: DateTime<Utc>,
    pub uuid: Uuid,
    pub is_meta: bool,
    /// Entry point that started the session (e.g., "cli"). Added in Claude Code 2.1.104+.
    pub entrypoint: Option<String>,
    /// Number of messages in the turn. Added in Claude Code 2.1.104+.
    pub message_count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct FileHistorySnapshot {
    pub message_id: Uuid,
    pub snapshot: FileHistorySnapshotSnapshot,
    pub is_snapshot_update: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub summary: String,
    pub leaf_uuid: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct FileHistorySnapshotSnapshot {
    pub message_id: Uuid,
    pub tracked_file_backups: HashMap<String, serde_json::Value>,
    pub timestamp: DateTime<Utc>,
}

/// Task execution state tracked in Claude Code's todo system. These states enable Claude Code
/// to persist task progress across session restarts and provide status visibility in the UI.
/// The linear progression (Pending → InProgress → Completed) ensures only one task is active
/// at a time, preventing execution chaos from parallel task attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

/// Task item from Claude Code's todo tracking system. Todos are persisted in log files to enable
/// session recovery after crashes or disconnections - Claude Code can resume incomplete work by
/// reading the last todo state from logs. Storing both imperative and continuous forms avoids
/// complex string transformations and ensures consistent UI messaging across different task states.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct Todo {
    /// Task description in imperative form (e.g., "Run tests")
    pub content: String,
    /// Task status
    pub status: TodoStatus,
    /// Present continuous form for display during execution (e.g., "Running tests")
    pub active_form: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct UserLogLine {
    pub parent_uuid: Option<Uuid>,
    pub is_sidechain: bool,
    /// Identifier for the agent/task that created this message. None for main conversation messages.
    pub agent_id: Option<String>,
    pub user_type: String,
    pub cwd: String,
    pub session_id: Uuid,
    pub version: String,
    pub git_branch: String,
    /// Session slug identifier (e.g., "noble-floating-lemon"). Added in Claude Code 2.0.51.
    pub slug: Option<String>,
    pub message: LogMessage,
    pub is_meta: Option<bool>,
    pub uuid: Uuid,
    pub timestamp: DateTime<Utc>,
    pub tool_use_result: Option<ToolUseResult>,
    pub thinking_metadata: Option<ThinkingMetadata>,
    pub is_visible_in_transcript_only: Option<bool>,
    pub is_compact_summary: Option<bool>,
    /// Todo list from Claude Code 2.0.47+. Contains tasks being tracked in the conversation.
    pub todos: Option<Vec<Todo>>,
    /// UUID of the assistant message that triggered the tool use. Added in Claude Code 2.0.51+.
    #[serde(rename = "sourceToolAssistantUUID")]
    pub source_tool_assistant_uuid: Option<Uuid>,
    /// Prompt identifier for tracking prompt lineage. Added in Claude Code 2.1.77+.
    pub prompt_id: Option<Uuid>,
    /// Current permission mode. Added in Claude Code 2.1.77+.
    pub permission_mode: Option<PermissionMode>,
    /// Plan content when in plan mode. Added in Claude Code 2.1.77+.
    pub plan_content: Option<String>,
    /// Entry point that started the session (e.g., "cli"). Added in Claude Code 2.1.104+.
    pub entrypoint: Option<String>,
    /// Origin of the message (e.g., task-notification). Added in Claude Code 2.1.104+.
    pub origin: Option<MessageOrigin>,
    /// Set when this user turn interrupted a streaming assistant message; holds that message's id
    /// (an Anthropic `msg_…` id, not a UUID). Can be `null` in the log, hence `Option`.
    /// Added in Claude Code 2.1.158+.
    pub interrupted_message_id: Option<String>,
    /// Present only on user turns that deliver an MCP tool result; absent on ordinary turns and on
    /// built-in (non-MCP) tool results, hence `Option`. Added in Claude Code 2.1.158+.
    pub mcp_meta: Option<McpMeta>,
}

/// Strict envelope mirroring Claude Code's `mcpMeta` object. Kept `deny_unknown_fields` so a future
/// sibling key (which would be Claude Code protocol) surfaces as a parse error rather than being
/// silently dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct McpMeta {
    /// MCP `structuredContent`, whose shape is defined by the responding MCP server rather than by
    /// Claude Code; it therefore reuses the tool-defined [`ToolUseResult`] representation (as the
    /// sibling `tool_use_result` does) instead of a fixed schema. `None` when the server returns no
    /// structured content: the MCP spec makes it optional, and the untagged `ToolUseResult` cannot
    /// itself represent a JSON `null`.
    pub structured_content: Option<ToolUseResult>,
}

/// Origin metadata for a message. Added in Claude Code 2.1.104+.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct MessageOrigin {
    pub kind: String,
}

/// Permission mode for the conversation. Added in Claude Code 2.1.77+.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    Default,
    Plan,
    AcceptEdits,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThinkingMetadata {
    pub level: String,
    pub disabled: bool,
    pub triggers: Vec<ThinkingTrigger>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThinkingTrigger {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolUseResult {
    String(String),
    Map(HashMap<String, serde_json::Value>),
    Vec(Vec<ToolUseResult>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct LogMessage {
    pub role: String,
    pub content: LogMessageContent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
pub enum LogMessageContent {
    String(String),
    Vec(Vec<LogMessageTaggedContent>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum LogMessageTaggedContent {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
        signature: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: HashMap<String, serde_json::Value>,
        /// Caller information for tool invocation tracking. Added in Claude Code 2.1.12+.
        caller: Option<ToolUseCaller>,
    },
    ToolResult(ToolResult),
    Document {
        source: DocumentSource,
    },
    ToolReference {
        tool_name: String,
    },
}

/// Caller information for tool use tracking. Added in Claude Code 2.1.12+.
/// Indicates how the tool was invoked (directly by the agent or through another mechanism).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolUseCaller {
    pub r#type: String,
}

/// Represents a document attached to a message (e.g., PDF, image, text file)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentSource {
    /// Type of data encoding (currently only "base64" is used by Claude Code)
    pub r#type: String,
    /// MIME type of the document (e.g., "application/pdf", "image/png", "text/plain")
    pub media_type: String,
    /// The document data as a string. When `type` is "base64", this contains base64-encoded
    /// binary data. The parser accepts any string without validation.
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
pub enum ToolResult {
    Current {
        content: LogMessageContent,
        is_error: Option<bool>,
        tool_use_id: String,
    },
    V1 {
        tool_use_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct AssistantLogLine {
    pub parent_uuid: Option<Uuid>,
    pub is_sidechain: bool,
    /// Identifier for the agent/task that created this message. None for main conversation messages.
    pub agent_id: Option<String>,
    pub user_type: String,
    pub cwd: String,
    pub session_id: String,
    pub version: String,
    pub git_branch: String,
    /// Session slug identifier (e.g., "noble-floating-lemon"). Added in Claude Code 2.0.51.
    pub slug: Option<String>,
    pub message: AssistantLogMessage,
    pub request_id: Option<String>,
    pub uuid: Uuid,
    pub timestamp: DateTime<Utc>,
    pub is_api_error_message: Option<bool>,
    /// Error type when this is an API error message (e.g., "invalid_request").
    pub error: Option<String>,
    /// Upstream HTTP status for an API-error message (e.g. 529), set alongside
    /// `is_api_error_message`/`error` on synthetic error turns. Added in Claude Code 2.1.158+.
    pub api_error_status: Option<u16>,
    /// Entry point that started the session (e.g., "cli"). Added in Claude Code 2.1.104+.
    pub entrypoint: Option<String>,
    /// Named subagent attributed with producing this message (e.g., "code-quality-reviewer").
    /// Present on subagent transcripts to associate the assistant turn with the spawning
    /// agent type. Added in Claude Code 2.1.141+.
    pub attribution_agent: Option<String>,
    /// Named skill attributed with producing this message (e.g., "plannotator-review").
    /// Present when the assistant turn was driven by a skill invocation.
    /// Added in Claude Code 2.1.141+.
    pub attribution_skill: Option<String>,
    /// MCP server and tool attributed with producing this message (e.g. server `project-tools`,
    /// tool `run_tests`). Set together when the assistant turn was driven by an MCP tool call,
    /// paralleling `attribution_agent`/`attribution_skill` above. Added in Claude Code 2.1.158+.
    pub attribution_mcp_server: Option<String>,
    pub attribution_mcp_tool: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantLogMessage {
    pub id: String,
    pub r#type: String,
    pub role: String,
    pub model: crate::model::Model,
    pub container: Option<String>,
    pub content: LogMessageContent,
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
    pub stop_details: Option<StopDetails>,
    pub usage: AssistantUsage,
    pub context_management: Option<serde_json::Value>,
    /// Diagnostic details from Claude Code about the request (e.g., cache miss reason).
    /// Added in Claude Code 2.1.141+.
    pub diagnostics: Option<Diagnostics>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostics {
    pub cache_miss_reason: Option<CacheMissReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum CacheMissReason {
    PreviousMessageNotFound,
    ToolsChanged { cache_missed_input_tokens: usize },
    MessagesChanged { cache_missed_input_tokens: usize },
    SystemChanged { cache_missed_input_tokens: usize },
    Unavailable,
}

/// Stop details from the Anthropic Messages API. Added in Claude Code 2.1.77+.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StopDetails {
    pub r#type: StopType,
    pub stop_sequence: Option<String>,
}

/// The reason the model stopped generating tokens.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub enum StopType {
    EndTurn,
    StopSequence,
    MaxTokens,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantUsage {
    pub input_tokens: usize,
    pub cache_creation_input_tokens: usize,
    pub cache_read_input_tokens: usize,
    pub cache_creation: AssistantCacheCreation,
    pub output_tokens: usize,
    pub service_tier: Option<String>,
    pub server_tool_use: Option<ServerToolUse>,
    /// Geographic region where inference was performed. Added in Claude Code 2.1.12+.
    pub inference_geo: Option<String>,
    /// Iteration data for responses. Added in Claude Code 2.1.77+.
    pub iterations: Option<Vec<Iteration>>,
    /// Speed setting for the response. Added in Claude Code 2.1.77+.
    pub speed: Option<Speed>,
}

/// Iteration data from Claude Code's response. Added in Claude Code 2.1.77+ as empty objects,
/// populated with token counts in 2.1.104+.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Iteration {
    pub input_tokens: Option<usize>,
    pub output_tokens: Option<usize>,
    pub cache_read_input_tokens: Option<usize>,
    pub cache_creation_input_tokens: Option<usize>,
    pub cache_creation: Option<AssistantCacheCreation>,
    pub r#type: Option<String>,
}

/// Speed setting for inference. Added in Claude Code 2.1.77+.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub enum Speed {
    Standard,
    Fast,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerToolUse {
    pub web_search_requests: usize,
    pub web_fetch_requests: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantCacheCreation {
    pub ephemeral_5m_input_tokens: usize,
    pub ephemeral_1h_input_tokens: usize,
}

#[cfg(test)]
pub async fn read_file(file: impl AsRef<Path>) -> miette::Result<Vec<LogLine>> {
    let string_contents = read_to_string(file).await.into_diagnostic()?;

    // Parse lines sequentially (each file is processed in parallel via rayon at a higher level)
    let log_lines: Result<Vec<LogLine>, _> = string_contents
        .split('\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            serde_json::from_str::<LogLine>(line).inspect_err(|_| {
                println!("{line}");
            })
        })
        .collect();

    log_lines.into_diagnostic()
}
