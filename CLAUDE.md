# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Moriarty is a Rust CLI tool for analyzing Claude Code logs and API usage. It provides:

- **Claude API pricing analyzer**: Analyzes Claude API usage from log directories and generates detailed cost or token
  reports
- **Pi cost analyzer**: Analyzes pi session logs and generates daily or per-conversation cost or token reports grouped
  by provider and model
- **Terminal graphs**: Renders chart-focused stacked-bar summaries for Claude/API and pi usage via `graphs all`,
  `graphs claude`, and `graphs pi`
- **MCP servers**: Provides Model Context Protocol servers for read-only git operations, read-only jj operations, and
  project tools
- **Hooks system**: Security integration for validating commands before execution (bash rules, project checks)
- **Project approval TUI**: Interactive interface for approving project-tools commands and checks before execution

## Essential Commands

**Building:**

```bash
cargo build
```

**Running:**

```bash
# Run Claude API pricing analyzer (--dir defaults to ~/.claude/projects)
cargo run -- api-pricing --timezone local|utc
cargo run -- api-pricing --tokens
cargo run -- api-pricing --dir <directory>
cargo run -- api-pricing --last-days 7

# Run pi cost analyzer (--dir defaults to ~/.pi/agent/sessions)
cargo run -- pi cost --timezone local|utc
cargo run -- pi cost --conversations
cargo run -- pi cost --tokens
cargo run -- pi cost --dir <pi-sessions-directory>
cargo run -- pi cost --last-days 3

# Render chart-focused usage graphs
# (graphs all uses both defaults; graphs claude --dir defaults to ~/.claude/projects;
# graphs pi --dir defaults to ~/.pi/agent/sessions)
cargo run -- graphs all --last-days 7
cargo run -- graphs all --claude-dir <claude-projects-directory> --pi-dir <pi-sessions-directory>
cargo run -- graphs claude --timezone local|utc
cargo run -- graphs pi --conversations --tokens
cargo run -- graphs pi --dir <pi-sessions-directory>
cargo run -- graphs claude --last-days 7
cargo run -- graphs pi --last-days 7

# Run MCP servers
cargo run -- mcp git-read-only
cargo run -- mcp jj-read-only
cargo run -- mcp project-tools
cargo run -- mcp install  # Install all servers to Claude Code

# Run project approval TUI
cargo run -- approve-project <project-dir>

# Execute hooks (for debugging)
cargo run -- hooks exec

# Report recorded PreToolUse hook results as JSON (filter by --start-time/--end-time, --tool, --result;
# --dir defaults to ~/.local/state/moriarty/hooks)
cargo run -- hooks report --tool Bash --result deny

# Inspect or evaluate rule behavior, and validate or author bash/tool rules
cargo run -- rules lint --strict          # report ignored rules; warn on empty modes/mode-overlap gaps/shadow/over-broad
cargo run -- rules list-fragments         # show built-in + user pattern fragments
cargo run -- rules schema                 # print a canonical example tool_rules.toml
cargo run -- rules starter                # paste-ready allow-rules for common read-only commands
cargo run -- rules report --timezone utc  # daily counts and percentages for completed hook outcomes
cargo run -- rules report --directories --current-rules  # directory totals for the active rule set
cargo run -- rules suggest --result ask   # propose anchored rules from frequently-prompted commands (hook logs)
cargo run -- rules replay --result allow  # check a candidate config keeps every prior auto-approval

# Preview how the hook splits and decides a (compound) Bash command
cargo run -- test bash-rules --explain --cwd <dir> '<command>'
```

**Testing:**

```bash
# Run tests (MUST use cargo nextest, NOT cargo test)
cargo nextest run

# Run tests for specific packages
cargo nextest run -p moriarty
cargo nextest run -p claude_logs

# Save yourself a great deal of repeated output
cargo nextest run --no-fail-fast --hide-progress-bar --success-output never --status-level fail --final-status-level flaky
```

**⚠️ CRITICAL**: Tests MUST be run using `cargo nextest`, never `cargo test`. Tests use `std::env::set_var` to set up
isolated XDG config directories, which is only safe when each test runs in its own process. `cargo nextest` runs each
test in a separate process, making this safe and preventing tests from clobbering real config files.

## Architecture

### High-Level Module Organization

**`claude_logs/`** - Claude Code log parsing:

- Independent workspace crate for parsing Claude Code JSONL logs into strongly typed serde models
- The `LogLine` enum covers both core conversation records and newer metadata/event records, including user/assistant
  turns, file-history snapshots, summaries, system entries, queue operations, progress updates, custom titles,
  ai-titles, agent names, last prompts, permission-mode changes, session mode records, attachments, PR-link records
  (associating a session with the GitHub PR Claude Code opened or updated; added in Claude Code 2.1.158+),
  model-refusal-fallback records (when Claude Code retries a refused request on another model, e.g. Fable 5 → Opus 4.8),
  `fallback` content blocks recording the from/to model pair inside an assistant message (both added in Claude Code
  2.1.170+), `image` content blocks inside tool_result content (observed in Claude Code 2.1.170 logs when a tool returns
  a screenshot), `image` file-attachment content (a user `@`-referencing an image file; uses Claude Code's own base64
  `file` envelope with size/dimensions rather than the API's `source` envelope), and a `pendingBackgroundAgentCount`
  field on `turn_duration` system records (the count of background agents still running when the turn completed; added
  in Claude Code 2.1.170+), and `agent_listing_delta` attachments (`AgentListingDelta`, the subagent analogue of
  `deferred_tools_delta`, carrying `added_types`/`added_lines`/`removed_types` plus `is_initial` and
  `show_concurrency_note`; added in Claude Code 2.1.175+), and a `fork-context-ref` record (`ForkContextRef`, the first
  line of a subagent's `subagents/agent-*.jsonl` file, carrying
  `agent_id`/`parent_session_id`/`parent_last_uuid`/`context_length` to record where the subagent forked from its parent
  conversation; added in Claude Code 2.1.175+), and `invoked_skills` attachments (`InvokedSkills`, the skills actually
  run during a turn — e.g. a `/code-review` slash command — each carrying `name`/`path`/`content`, distinct from
  `skill_listing` which only advertises available skills; added in Claude Code 2.1.179+), and `origin`/`timestamp`
  fields on `queued_command` attachments (`QueuedCommand`, recording who queued a command and when; `origin` reuses
  `MessageOrigin` since both carry the same `kind` discriminator; added in Claude Code 2.1.197+), and `context_tip`
  attachments (`ContextTip`, a contextual UI hint Claude Code surfaces to the user — e.g. suggesting `/add-dir` when
  searching outside the working directory — whose payload nests `tip`/`featureId`/optional `action` under a `tip`
  object; added in Claude Code 2.1.197+), and a `cumulativeDroppedTokens` field on `compact_boundary` records'
  `compactMetadata` (`CompactMetadata`, a running total of tokens dropped across all compactions in the session; added
  in Claude Code 2.1.197+), and a `toolDenialKind` field on user turns (`ToolDenialKind`, a strict enum —
  `permission-rule` (a rule blocked the call), `user-rejected` (the user declined it at the prompt), or
  `automode-unavailable` (auto mode's safety classifier model was temporarily unavailable, so the call was denied rather
  than approved) — recording why Claude Code denied the tool call the turn responds to; present only on the user turn
  carrying a denied tool's error `tool_result`; added in Claude Code 2.1.201+), and `timedOut`/`timeoutMs` fields on
  `hook_cancelled` attachments (`HookCancelled`, recording whether the hook was cancelled for hitting its timeout and
  the configured timeout in milliseconds; added in Claude Code 2.1.201+), and a `refusedUserMessageUuid` field on
  `model_refusal_fallback` system records (`ModelRefusalFallback`, the user message whose request was refused, nullable;
  added in Claude Code 2.1.201+), and an `errorDetails` field on assistant turns (`AssistantLogLine`, the raw upstream
  error body — e.g. the full `429 {...}` JSON string — kept verbatim as the provider's opaque payload alongside
  `error`/`apiErrorStatus`; added in Claude Code 2.1.201+), and a `queuePriority` field on user turns (`QueuePriority`,
  a strict enum — currently only `later` — giving the scheduling priority of a prompt that was queued while Claude Code
  was busy rather than sent immediately; present only on queued turns; added in Claude Code 2.1.201+), and a
  `displayPath` field on `edited_text_file` attachments (`EditedTextFile`, the shortened path Claude Code shows the user
  for the edited file, distinct from the absolute `filename`; nullable so pre-2.1.201 logs still parse; added in Claude
  Code 2.1.201+), and a `session_id` (snake_case) field mirroring the existing camelCase `sessionId` on the full
  conversation records — user, assistant, and attachment — plus the `stop_hook_summary` and `model_consent_fallback`
  system records (other line types keep just `sessionId`); captured as a separate `session_id_snake` because both keys
  appear at once so a `#[serde(alias)]` would be rejected as a duplicate; the two always carry the same value; added in
  Claude Code 2.1.206+), `read_truncation_notice` attachments carrying the display banner and originating tool-use id, a
  `pendingWorkflowCount` field on `turn_duration`, a `toolEndsTurn` field on workflow-subagent user turns, `refusal`
  stop details carrying the API's category, explanation, prefill-fallback flag, and optional recommended model, and
  `model_consent_fallback` system records describing a session-level switch when the selected model requires unavailable
  usage-credit consent (all observed in Claude Code 2.1.206+), an `effort` field on assistant turns (`AssistantLogLine`,
  the reasoning-effort level the turn was generated at, modeled as the strict `ReasoningEffort` enum —
  `low`/`medium`/`high`/`xhigh`/`max`, Claude Code's documented closed set — so a genuinely new level surfaces as a
  parse error; nullable so pre-2.1.214 logs still parse), and a `file-history-delta` line type (`FileHistoryDelta`, the
  incremental analogue of `file-history-snapshot`: it records a single tracked file's backup — `messageId`,
  `snapshotMessageId`, `trackingPath`, a nested `backup` whose `backupFileName` is null when the backup file does not
  yet exist for a newly tracked file and which may also carry an optional `realParentDir` path, and a `timestamp` —
  rather than re-emitting the whole snapshot), and a relaxation of the `define_boundary_log!` macro's `isMeta` to
  `Option<bool>` (shared by `CompactBoundary` and `MicrocompactBoundary`) because Claude Code 2.1.214 stopped emitting
  it on `compact_boundary` records (the field is read only for schema completeness, so older records that still carry it
  keep parsing), and a `task_status` attachment (`TaskStatus`, a progress record for a spawned background agent carrying
  `taskId`/`taskType`/`description`/`status`/`deltaSummary`/`outputFilePath`; `taskType` and `status` are kept as
  `String` rather than strict enums because their runtime-lifecycle vocabularies are undocumented and volatile and
  nothing downstream reads them, mirroring `TaskReminderItem.status`), a `messagesSummarized` field on
  `compact_boundary` records' `compactMetadata` (`CompactMetadata`, the count of messages folded into the summary by
  this compaction; nullable so older records still parse), and a `summarizeMetadata` object on the compact-summary user
  turn (`SummarizeMetadata`, carrying `messagesSummarized` plus a `direction` string kept as `String` rather than a
  strict enum because its vocabulary — observed `from` — is undocumented and nothing downstream reads it; present only
  alongside `isCompactSummary`, hence `Option`) (all added in Claude Code 2.1.214+), a `userFeedback` field on user
  turns (`UserLogLine`, an optional free-form `String` carrying the instruction that accompanied a rejected tool call),
  and an `isAbortedMidStream` field on assistant turns (`AssistantLogLine`, an optional `bool` marking a response
  stopped before streaming completed; partial responses remain billable), and a `preCheckpoint` field on
  `file-history-snapshot` records' inner snapshot (`FileHistorySnapshotSnapshot`, an optional `bool` marking a snapshot
  taken ahead of a checkpoint rather than as part of ordinary file tracking; nullable because ordinary snapshots omit
  it) (all added in Claude Code 2.1.219+), and a `total_tokens_reminder` attachment (`TotalTokensReminder`, the
  `<total_tokens>` budget reminder Claude Code injects into a turn, carrying just its `text`; added in Claude Code
  2.1.226+), and a second `auto_mode` attachment payload carrying `autoModeConsentFlow`/`bashFirst`/`steerOnly` (auto
  mode's negotiated behavior flags, emitted instead of `reminderType` rather than alongside it; `AutoMode` therefore
  became an untagged enum over the two mutually exclusive shapes — `AutoModeReminder` and `AutoModeBehaviorFlags`, each
  still `deny_unknown_fields` — so neither an all-absent nor a both-present payload is representable. Untagged is the
  cost of a wire shape with no discriminator beyond which keys are present: a new third shape still fails to parse, but
  as an untagged mismatch rather than the more precise unknown-field error; added in Claude Code 2.1.226+), and an
  `atis-latch` line type (`AtisLatch`, a session-scoped record whose undocumented `atis` payload has only ever been
  observed empty, so it stays an opaque `String`), an `output_tokens_details` object on `AssistantUsage`
  (`OutputTokensDetails`, a breakdown of `output_tokens` — currently just `thinking_tokens`, required because every
  observed payload carries it — not an addition to the billable total), a `bypass` field on the `auto_mode` behavior
  flags, `bashFirst`/`steerOnly` fields on the previously empty `auto_mode_exit` attachment (`AutoModeExit`, echoing the
  flags in effect when auto mode ended; like `AutoMode` it became an untagged enum over its two mutually exclusive
  shapes — `AutoModeExitBare` and `AutoModeExitBehaviorFlags` — rather than sibling `Option` fields, so a half-present
  payload the wire format never produces is not representable), a `source_uuid`
  field on `queued_command` attachments (`QueuedCommand`, the message a queued command originated from; emitted
  snake_case unlike its camelCase siblings, so the rename is spelled out on the field), a `turnCompanion` field on user
  turns (`UserLogLine`, marking a meta turn that accompanies the turn it was injected alongside — e.g. an invoked
  skill's instructions — rather than standing on its own), and an `away_summary` system subtype (`AwaySummary`, the
  recap Claude Code writes while the user is away from the session; shaped like `SystemLogInformational` minus its
  `level`) (all added in Claude Code 2.1.238+)
- Also owns the structured view of the raw `model` string via `model::Model { family, version }` plus `ModelFamily` and
  `ModelVersion`. Both `cost_analyzer` (for pricing) and `moriarty::api_pricing` (for grouping/display) consume this one
  parser so family/version classification is not duplicated across crates. The parser preserves capability-decorated raw
  ids such as `claude-opus-4-8[1m]` while excluding the `[1m]` context-window suffix from version classification
- Used by `moriarty`'s `api_pricing` module to analyze Claude Code conversation logs

**`cost_report/`** - Shared cost report rendering and filtering:

- Holds shared time filtering, grouped-table rendering, stacked-chart rendering, `ReportMode`, `CostComponents`,
  `TokenCounts`, `MetricComponents`, and report warning helpers used by both cost-report backends
- `FormattedMetricColumns` and `GrandTotalRow` are mode-aware: cost mode formats dollars, token mode formats integer
  token counts with thousands separators, while preserving the same table shape for both backends
- `display_summary` renders a consolidated "Summary" section (optional provider table for `pi cost`, model table for
  both backends, and grand total) called by each backend after its inline grouped-table rendering
- `charts.rs` renders deterministic horizontal stacked bars for both time-series and share views, including top-N plus
  `Other`, stable glyph/color assignment, and narrow-terminal truncation without changing the table-report path
- `series.rs` carries typed date/session chart handoff data so multiple sources can be merged before rendering
- Keeps the output behavior for `api-pricing`, `pi cost`, and the graph commands aligned without forcing the backends
  into a dynamic-column abstraction

**`api_pricing/`** - Claude API usage cost analysis:

- Aggregates either pre-priced `LlmCost` values or raw token counts from `cost_analyzer` into daily buckets (keyed by
  timezone-adjusted date) or per-conversation buckets (keyed by session ID)
- Per-model aggregation uses `ModelMetricsMap` keyed by `claude_logs::Model` (family + parsed version) so report rows
  and chart legend distinguish e.g. "Sonnet 4" from "Sonnet 4.5"; row/legend ordering is family-first (Fable → Opus →
  Sonnet → Haiku) then version-desc via the local `model_sort_key` helper, so within-family Opus 4.x rows sit above Opus
  3 rows automatically. Token mode stays integer-exact end-to-end instead of passing through floating-point helpers
- Unknown Claude models surface as stderr tracing errors via `cost_analyzer`; they are not rendered in the report
- Also prepares `ChartBucket` data for `graphs claude`, reusing the same analyzer output while keeping the existing
  detailed table report unchanged
- Entry points: `api-pricing` and `graphs claude` subcommands in `main.rs`

**`pi_cost/`** - Pi session cost analysis:

- Aggregates either pre-priced `LlmCost` values or raw token counts from `cost_analyzer` into daily buckets or
  per-conversation buckets keyed by normalized session ID
- Uses raw pi `(provider, model)` pairs for row grouping, with deterministic ordering from a
  `BTreeMap<PiModel, MetricComponents>` accumulator inside `PiModelMetricsMap`
- Conversation mode depends on `cost_analyzer::LineWithCost.session_id`, which is attached during the single-pass parse
  from either Claude assistant lines or pi `SessionLine` headers
- Also prepares provider/model `ChartBucket` data for `graphs pi`, reusing the same analyzer output while keeping the
  existing detailed table report unchanged
- Entry points: `pi cost` and `graphs pi` subcommands in `main.rs`

**`combined_graphs/`** - Concurrently analyzes Claude/API and pi logs, merges typed date/session series, and prefixes
segment labels by source. Cost mode mixes Claude's local pricing with pi's recorded prices; missing defaults are skipped
with warnings, while explicit missing paths and having no available source are errors.

**`pi_logs/`** - Pi session log parsing:

- Independent workspace crate for parsing pi session JSONL logs into strongly typed serde models
- `ToolCallContent` keeps the outer tool-call envelope typed (`id`, `name`, `partial_json`) but preserves `arguments` as
  a raw `BTreeMap<String, JsonBlob>` because pi logs the model-emitted JSON object before tool-side validation; typed
  tool-argument structs are optional post-parse helpers, not the parser's source of truth
- Tool names are raw `String` fields — pi does not validate tool names at the protocol level, so the parser accepts any
  string (including model-invented names like `task` or `remove`) rather than rejecting unknown tools. Tool-specific
  result parsing still routes known tool names by string to their typed result-structs where structured post-parse
  handling is needed; unknown tool results fall through to shape-based deserialization.
- `AssistantMessage.raw_stop_reason` carries the provider's own stop-reason string un-normalized beside the
  `stop_reason` pi maps it onto (e.g. the openai-codex-responses `completed` that pi records as `toolUse` or `stop`).
  Like tool names, it stays a `String` rather than a strict enum because the vocabulary belongs to whichever provider
  served the turn; it is `Option` because only some providers emit it.
- Hermes memory/session-search result details are modeled by their shared envelopes rather than per-action sub-schemas:
  search tools use the `success/count/message/output` summary shape, while `memory` and `skill` are routed by
  `tool_name` first because their error details can collapse to either `{}` or a bare `{error}`; once routed, the parser
  accepts their observed action-agnostic fields plus the real `{}` validation-error payload used by the extension
- Strict by default with `#[serde(deny_unknown_fields)]`, path-aware parse errors, and narrowly documented exceptions
  for shapes that require custom deserialization or specific corrupt-stream tolerance
- Pi-lens result details use tool-name-routed, derived Serde schemas: `lens_diagnostics` distinguishes delta/all/full
  (including unavailable) responses, `lsp_diagnostics` distinguishes file/batch/directory responses,
  `lens_diagnostic_mark` keeps typed dispositions and nonzero lines, and `module_report` accepts optional callback
  support. `fetch_content` also accepts its optional timestamp metadata.
- `CompactionLine` and `BranchSummaryLine` carry an optional `usage: Option<AssistantUsage>` recording the cost/tokens
  of the summarization call pi made to produce them (pi added this field after the initial compaction schema, so it is
  `#[serde(default)]` for backward compatibility); the lines record no provider/model of their own, so attribution is
  left to `cost_analyzer`'s active-model fallback
- Includes a `parse_pi_sessions` binary that recursively smoke-tests a sessions tree by parsing every `*.jsonl` file

**`cost_analyzer/`** - Generic cost-analysis library:

- Workspace crate for recursively scanning JSONL directories, parsing logs in parallel, and deduplicating billable model
  responses. It skips Claude's well-known non-transcript `history.jsonl` and workflow `journal.jsonl` files by basename
  wherever they occur because neither schema contains billable model responses
- Core abstractions: `AnalyzableLog` for pluggable log formats, `LlmCost` for input/cache/output cost breakdowns,
  `TokenType` plus `AnalyzableLog::token_count(...) -> Option<u64>` for raw token extraction, `LineWithCost` for
  normalized billable entries, and `AnalysisResult` for returning those deduplicated lines alongside a partial-failure
  flag
- Concrete implementations currently support `pi_logs::PiLogLine` and `claude_logs::LogLine`. Claude log costs are
  calculated in `cost_analyzer` with local Decimal-based Claude pricing helpers (`ClaudeModelPricing::for_model`) that
  consume `&claude_logs::Model`; the family enum itself lives in `claude_logs` so the parser and pricing layer agree on
  classification without depending on `moriarty::api_pricing` internals. Opus 3 vs Opus 4.x share `ModelFamily::Opus`
  and the pricing dispatch reads the parsed `version.major` to pick the OPUS or OPUS_4 tier; `ModelFamily::Fable` maps
  directly to the flat FABLE tier without version dispatch.
- `moriarty::api_pricing` and `moriarty::pi_cost` both delegate all log loading, deduplication, pricing, and raw token
  extraction to this crate; the backends only bucket the returned billable lines into cost or token report rows
- `LineWithCost.session_id` is normalized during parsing so backends can group by conversation without re-reading log
  files; Claude assistant lines provide it inline and pi logs inherit it from the file's `SessionLine`
- `AnalyzableLog::active_model()` (default: the line's own `model()`) reports the model a line establishes as active for
  subsequent lines in the same file; `reader::parse_file` threads the most recent non-`None` value alongside
  `session_id`. A billable line that carries no model of its own — a pi compaction/branch-summary summarization cost,
  which records no provider/model — is attributed to that active model via `LineWithCost::from_log`'s fallback
  (`log.model().or(active_model)`), so it folds into whatever model was current when it ran. `PiLogLine` overrides
  `active_model()` so an explicit `model_change` (which carries no cost) also updates the active model, matching the
  user's model at that point even mid-session. `LineWithCost::parse` (single-line, no file context) passes no active
  model, so a model-less billable line parsed in isolation is not attributed. If such a line appears in a file before
  any model has been established (in practice unreachable — nothing to summarize before the first turn), its cost is
  dropped rather than attributed to an invented model; `parse_file` emits a `tracing` warning so the (never-observed)
  undercount is visible rather than silent
- Deduplication keeps the highest-cost duplicate for a `(ModelId, LogId)` pair and breaks equal-cost ties by keeping the
  earliest timestamped entry
- Public entry point: `cost_analyzer::analyze_directory(path)`

**`tui/`** - Terminal UI event infrastructure:

- Provides an async event stream (`input_stream`) that maps crossterm terminal events (keys, resize, paste) into the
  internal `Event` / `UIEvent` enum
- Used by `approval_tui/` as its input source

**`mcp/`** - Model Context Protocol servers:

- Three MCP servers: `git_read_only` (status, diff, log, show), `jj_read_only` (status, diff, log, show, op log, file
  show, file list), and `tool_runner` (lint, test, build, format, checks)
- `read_only`: Shared infrastructure used by both `git_read_only` and `jj_read_only`. Provides `CommandResult`,
  `validate_project_dir`, and the generic `run_read_only_command`. It rejects parent-traversal and non-directory targets
  before canonicalizing the working directory, while the per-server wrappers add command-specific flag restrictions
  (`git` forces `--no-optional-locks`, `--no-ext-diff`, and `--no-textconv` while rejecting output-file / no-index
  escape flags; `jj` forces `--ignore-working-copy` and rejects external-tool, config-injection, and repository-override
  flags). Neither server consults `.config/tools.toml` approvals; only `tool_runner` does.
- `tool_runner`'s four command tools (`run_lint`/`run_build`/`run_formatter`/`run_tests`) run a single `[commands]`
  entry via `verify_and_load_project` + `run_command`; its `run_checks` tool runs the project's `[[checks]]` via the
  shared `crate::checks::run_configured_checks` routine — the same approval verification (checks only, not commands),
  resource limits (5-min timeout, 1 MB/check + 10 MB total output caps), and fail-closed semantics as the Stop hook.
  That shared routine is intentionally distinct from `project_config::runner::run_all_checks` (no limits, verifies all
  commands too), which the `moriarty test checks` CLI uses. Both runner paths load `tools.toml` from the caller's
  **workspace root** (`detect_workspace_root`) — so each jj secondary workspace or git worktree runs its own config and
  its own relative programs — while approvals stay keyed by the shared **repository root**, so identical content across
  worktrees needs no re-approval. Both spawn the program path `VerificationResult::Approved` carried back from
  verification (never the raw `command[0]`), so an unapproved local copy cannot shadow the approved binary. Their
  execution cwd differs: `project_config::runner` keeps the caller's canonicalized project dir, while the checks-runner
  uses `detect_workspace_root(project_dir)`, which can walk up past a nested project dir to the enclosing workspace
  root.
- Uses rmcp library with stdio transport for Claude Code integration
- All servers run as stdin/stdout servers that Claude Code can invoke
- `install` command configures all servers in Claude Code's MCP registry
- **Architectural patterns**: git_read_only uses separate MCP tools per command; jj_read_only uses enum-based single
  tool (see MCP Command Patterns below)

**`hooks/`** - Security hook system for Claude Code integration:

- **PreToolUse hook**: Two-tier permission system from `~/.config/moriarty/tool_rules.toml`:
  - **Permission-mode eligibility**: both rule kinds accept `modes`, an optional set of the six exact hook values
    (`default`, `plan`, `acceptEdits`, `auto`, `dontAsk`, `bypassPermissions`; Manual arrives as `default`). Omitted
    means the rule is permanently unrestricted in all modes and remains omitted from serialization so existing effective
    hashes stay stable; `[]` means no modes. Hook events require `permission_mode`; a restricted rule cannot match a
    mode-less replay or test row. Ineligible rules stay in declaration order and are skipped before other gates, so
    first-match-wins applies among eligible rules; the same mode threads through tool-to-Bash fallback, locality
    preflight, compound/bail evaluation, ArgumentFilter recheck, explain, replay, and suggestion de-duplication. Public
    `hooks report` grouping/JSON remains mode-agnostic, but cwd-aware internal rows split by mode; suggestions union
    known contributing modes and become unrestricted if any contributor is mode-less.
  - `tool_rules`: Permission any tool call (Read, Write, Edit, Bash, etc.) with optional legacy field-level regex
    matching and an optional ANDed `conditions` list over literal top-level input keys: `Present`, `Absent`, typed raw
    JSON `Equals`, and scalar-regex `Matches`. Separate ordered rules provide OR/fallback behavior. Actions: Allow,
    Deny, Ask. Checked first. Regex field values that start with the hook input's `cwd` have that prefix stripped so
    rules can use relative paths (e.g., `^src/` instead of absolute paths). `allow_local = true` canonicalizes every
    `path` / `file_path` selected by presence-requiring conditions or the legacy field; for non-existent targets it
    canonicalizes the deepest existing ancestor and safely rebuilds the missing suffix so `..` cannot escape above that
    ancestor.
  - `bash_rules`: Bash-specific command and redirect-endpoint validation with regex patterns. Ordinary actions are
    Allow, Deny, Modify, Ask, and ArgumentFilter; directional `AllowRedirect`/`DenyRedirect` actions form a target-only
    domain that never grants command execution. Existing `AllowRedirect` rules default to output-only. Checked when no
    tool_rule matches a Bash call.
  - **Canonical Bash evaluation** (`hooks/bash_rules.rs` + `hooks/command_split.rs`): `evaluate_sync` invokes the
    `brush-parser` splitter and builds one completed `Evaluation` containing source command decisions, resolved endpoint
    facts, and a closed continuation: none, a Modify redirect check, an ArgumentFilter recheck, or that recheck followed
    by one Modify redirect check. Leaf, whole-policy, rewrite, and final outcomes plus ordered contributors are derived
    from those facts rather than stored. Command and redirect rules are compiled into separate first-match domains, and
    parsed redirect targets are closed descriptor/filesystem/unresolvable states carrying input/output/both direction. A
    simple `^ls` rule still matches the `ls` leaf of a compound, while merge precedence remains Deny, Ask,
    NoMatch/Modify/ArgumentFilter, then Allow with first-leaf attribution on ties; multi-leaf rewrites are never
    stitched together. Replay, normal testing, explain, and the hook consume the same completed evaluation. Suggestions
    call the same canonical original-policy analyzer but deliberately skip unused Modify and ArgumentFilter
    continuations. Explain alone requests diagnostic endpoint facts, and `test_runner` projects each endpoint's
    direction plus whether its matched redirect rule allowed or denied it into the compatibility trace. Compiled rules
    own shared match metadata so repeated evaluations do not rebuild explanation strings.
  - **Configured path-alias preprocessing**: the optional top-level `bash_path_aliases` ordered set names trusted shell
    variables that may be bound by an exact leading `NAME=/literal/path;` declaration or newline-terminated equivalent.
    `command_split` expands parser-identified plain `$NAME`/`${NAME}` references in either unquoted or ordinary
    double-quoted words, including `"$P/runtime/mocks.js"` and `"$P"/runtime/mocks.js`, then applies quote-aware cwd
    stripping. Single-quoted `'$P'` remains literal; localized `$"…"`, indirect, dynamic, and other quoted or escaped
    compositions stay unsupported and cap Allow at Ask. The declaration is analysis metadata only after a later
    supported reference consumes it. Unsupported declarations/references and command-position expansion remain leaves
    with an independent Allow-to-Ask confirmation cap, so later Denies still win. Rather than model per-builtin mutation
    targets, an assignment invalidates all aliases only when a binding is active or it targets a configured alias;
    unrelated standalone environment assignments remain eligible for normal rules. Dynamic command names, wrappers,
    `exec`, and recognized shell-state builtins (including `shopt`, `alias`, and `unalias`) invalidate all active
    aliases and become redirect path-context barriers. This trades extra prompts for a small fail-closed implementation
    without retaining stale bindings. Config deserialization validates shell identifiers and rejects the fixed
    shell-control names before any tool rule can short-circuit Bash analysis. No aliases is the omitted/hash-compatible
    default; configured aliases are trusted policy and must not be application/tool behavior variables.
  - **Component-scoped redirect policy**: command rules match a cwd-normalized view with every parsed input and output
    redirect removed, with first-match-wins among ordinary actions. Redirect-only leaves retain their full syntax so
    endpoint policy alone cannot authorize shell execution. Every endpoint (`<`, `<&`, `>`, `>>`, `>|`, `&>`, `&>>`,
    `>&file`, `<>`, devices, and fd duplication/closure) independently uses the first direction-eligible redirect rule:
    `DenyRedirect` blocks it and `AllowRedirect` authorizes it. Existing allows default to output; input is explicit and
    a read-write endpoint requires `both`. Strict lint reports a redirect rule as shadowed only when the earlier rule
    covers every endpoint class the later rule could match. All command leaves and endpoints must pass. Redirect rules
    are a separate domain, so ordinary rules cannot shadow them or authorize a target. Each endpoint is retained in
    source order with static, alias-expanded, tilde, or descriptor metadata; unresolved parameters, ordinary globs,
    extglobs, braces, invalid descriptor-duplication words, and unsupported forms prompt. The redirect-free normalized
    command match and alias-expanded rewrite source remain separate for `ArgumentFilter`, which preserves prefix,
    interleaved, and suffix redirects. `Modify` expands redirect-free command-domain captures, retains consumed
    path-alias declarations used by redirects, and appends the original redirect operations in source order. Analyzable
    replacements must reparse with those redirects on the same leaf. Direct `Modify` output passes redirect-only
    authorization inside the shared engine evaluation before it is returned; a matching redirect deny still blocks a
    confidently parsed static endpoint on bailout, while any other unanalyzable or confirmation-bearing rewrite prompts.
    Synchronous test/replay/explain and the live hook therefore apply the same policy. The live hook runs the entire
    synchronous evaluation in one blocking task under one two-second deadline; timeout or join failure prompts with no
    deciding rule or contributors, including for plain commands delayed in the blocking pool. `ArgumentFilter` consumes
    normalized argument values but rewrites alias-expanded source spans, preserving unchanged quoting, escapes, literal
    globs, redirect operators, leading `time`/`time -p` and `!` pipeline prefixes, and a trailing background `&` while
    shell-quoting replacement and added values. The result then passes the full command-plus-endpoint evaluation again
    in live, replay, and explain paths. A relative or home-relative filesystem redirect in a path-context barrier
    command, or any later command, asks because changed shell path context is deliberately not modeled. Assignments,
    dynamic command names, `cd`/`pushd`/`popd`, recognized shell-state builtins, and non-isolated compound constructs
    such as brace groups and `if`/`for`/`while`/`case` clauses are barriers; absolute paths and descriptors remain
    evaluable.
  - **Shared hook path resolution** (`hooks/path_resolution.rs`): tool-rule locality and redirect policy share the same
    symlink-safe missing-path implementation, with redirect resolution additionally rejecting non-directory ancestors
    and unsafe virtual paths (`/proc/self`, `/proc/thread-self`, `/dev/fd`, `/dev/std*`, `/dev/tcp`, `/dev/udp`) —
    including components introduced by nested symlink targets — that would resolve differently in the hook or perform
    Bash-specific network I/O. Redirect contexts capture cwd/HOME text without filesystem I/O and initialize canonical
    roots only when an eligible filesystem endpoint needs them; invalid HOME does not block unrelated targets. Paths
    render cwd-relative, `~/...`, or canonical absolute, while devices always remain canonical absolute. Cwd-relative
    names beginning with `~` or `&` gain `./` to keep home and descriptor namespaces distinct, and non-UTF-8 resolved
    paths fail closed instead of using lossy text. Local redirect rules additionally require containment under cwd and
    cannot authorize devices, special files, or fd tokens. Live Bash evaluation always uses one `spawn_blocking` call
    and one two-second deadline for parsing, matching, resolution, and continuations; offline evaluation has no
    deadline. Tool-rule `allow_local` retains its separate conditional blocking preflight, where failure or timeout
    skips all local rules and falls through. Canonicalization is pre-execution and retains the unavoidable filesystem
    TOCTOU limitation.
  - **Bail ⇒ fail safe**: a command containing command substitution `$(...)`, backticks, an uninspectable value-carrying
    parameter expansion (`${x:-$(…)}`), a subshell, process substitution, a here-doc/here-string, a compound construct
    (`if`/`for`/`while`/`case`/`[[ ]]`/`((…))`/brace group/function), or that fails to parse is un-analyzable. The
    splitter retains confidently parsed static redirect facts while deferring the first bailout, so a raw whole-command
    Deny or a matching DenyRedirect is honored; every other outcome becomes a prompt.
  - **Bash cwd stripping**: static paths—including ordinary quoted values, escaped literals, safe glob patterns, and
    known alias expansions—are normalized only when their in-cwd relative remainder contains no `..` component.
    Parent-containing paths, unquoted brace syntax, and glob paths with dot-prefixed components remain unchanged; quoted
    or escaped braces stay literal. An exact-cwd operand becomes `.` rather than disappearing. Thus `^cat src/` matches
    `cat "/abs/cwd/src/x"`. `evaluate_sync(command, context, purpose)` is the completed synchronous runtime evaluator;
    suggestion mining uses its canonical `evaluate_original` source-fact stage directly, and `evaluate_live` supplies
    the live deadline. `RuleResult` is only the compatibility DTO projected at hook, replay, normal-test, and
    explain-trace boundaries. Explain includes the final primary rule plus ordered contributors; matched redirect
    endpoints carry a typed allowed/denied decision, while failure text is reserved for resolution or unmatched-policy
    failures. An unanalyzable Modify rewrite reports its rewritten command, bail reason, and any retained endpoint that
    caused a redirect denial without a second evaluation path.
  - Evaluation order: tool_rules → bash_rules (for Bash) → passthrough (for non-Bash, defers to Claude Code)
- **Stop hook**: Runs the project's configured checks before allowing execution, delegating to the shared
  `crate::checks::run_configured_checks` routine (see `mcp/` above); it maps the routine's `CheckRunOutcome` onto
  allow/deny. Checks execute with the workspace root as their working directory (`detect_workspace_root`: nearest
  `.jj/repo` walking up, else `git rev-parse --show-toplevel`); `.config/tools.toml` is loaded from that same workspace
  root and each check's program is resolved against it, so a secondary workspace or worktree runs its own config and its
  own relative programs. Approvals stay keyed by the shared repository root, binding each check's argv plus resolved
  binary paths and binary hash as match-any versions; each check's program is spawned as the original resolved path
  verification hashed — not the canonical path, since canonicalizing breaks symlinked multi-call binaries (nix
  coreutils, busybox) that dispatch on argv[0], and not the raw `command[0]`, so an unapproved local copy cannot shadow
  the approved one (a workspace-local `./check.sh` runs only if that exact argv+binary was approved)
- Structured logging with tracing crate for debugging hook execution. The "PreToolUse hook completed" log event records
  a clean `result` field (`allow`/`deny`/`ask`/`modify`/`passthrough`) classified from the typed `HookOutput` by
  `hooks::result::pretool_result`, alongside `tool_name`, `tool_args`, `cwd`, `permission_mode`, `rules_hash`, `rule`,
  and `rules`. `rule` remains the primary/deciding compatibility attribution (empty when none); `rules` is a
  JSON-encoded ordered contributor list covering compound command rules, endpoint rules, and filter revalidation.
  Contributors are de-duplicated by matched rule identity rather than display name, so distinct command and redirect
  rules with the same configured name remain visible as repeated names. A bailed command retains a deciding command or
  redirect Deny contributor and otherwise has none.
- **`hooks report` subcommand**: `hooks/report.rs` reads the JSON-lines hook logs, keeps completed PreToolUse records
  that carry the clean `result` field (historical lines lacking it are skipped), and aggregates them by the exact
  `(tool name, arguments, result, rule, ordered contributors)` key into a JSON report with counts; `rule` remains for
  compatibility and `rules` carries complete provenance. Historical lines without `rules` fall back to a non-empty
  singular `rule`; both are omitted when no rule contributed. Reuses `cost_report::TimeRangeFilter` for
  `--start-time`/`--end-time` and supports `--tool` and `--result` filters. `report::aggregate` (used by `hooks report`)
  and `ReportRow` are `pub(crate)`. `report.rs` parses the completion line's `cwd` back into `HookRecord` and folds
  filtered records directly into each caller's accumulator while reading rotated log files concurrently, so no vector
  retains every matching event. `rules suggest`/`rules replay` call `report::aggregate_with_cwd`, which joins `cwd` into
  the grouping key and populates `ReportRow.cwd` (a `#[serde(skip)]` field) so each command is re-normalized with the
  directory it actually ran under. `rules report` calls `report::fold_outcomes`, preserving each event's timestamp only
  until it has been folded into a timezone-aware daily or exact-directory bucket while sharing the same time/hash
  filtering and skip accounting. `aggregate` keeps `cwd` out of its key and serialization, so cwd differences do not
  split `hooks report` rows; contributor differences still do. Rows recorded before `cwd` was logged are excluded (with
  disclosed counts) from replay/suggest because component path policy cannot be reproduced, while the effectiveness
  report retains them in an explicit `Unknown` bucket.
- **Compile diagnostics & `rules` authoring tooling**: `BashRuleEngine::compile_with_diagnostics` and
  `ToolRuleEngine::compile_with_diagnostics` return the engine plus a `RuleDiagnostic` for every dropped rule
  (undefined/circular/over-depth/over-count fragment, invalid regex, or — tool rules only — a `field`/`pattern` given
  without its partner); an invalid `Matches` condition drops the whole tool rule rather than broadening it by retaining
  only the valid predicates. `from_config` delegates to these compilers and logs each diagnostic, preserving the
  fail-open-per-rule hot path. The `crate::rules` command group surfaces them and helps author safe rules: `report`
  (daily outcome counts/percentages by default, exact recorded-cwd buckets with `--directories`, all recorded rule sets
  by default, and opt-in active-hash filtering with `--current-rules`), `lint` (errors when a rule the user wrote is
  silently dropped; `--strict` additionally warns on permanently disabled `modes = []`, missing mode-overlapping
  redirect policy, same-domain rules fully shadowed across their direction/locality scope, over-broad command Allows,
  and broad local/non-local redirect authority), `list-fragments`, `schema` (round-tripped against `UserConfig` in
  tests), `starter` (paste-ready read-only command rules plus `/dev/null`, output-descriptor, and stdin `&0` input
  rules), `suggest` (anchored rules mined from hook logs; each recorded command is split into the leaf simple-commands
  the hook actually evaluates — normalized with the recorded cwd — before pattern generation, so compounds yield
  per-leaf candidates with summed counts, consumed alias declarations omitted, and confirmation-required alias leaves
  excluded; a bailed command stays whole. `--match exact|prefix|fuzzy` picks the shape, where prefix/fuzzy clustering
  reuses brush-derived program and argument metadata rather than reparsing leaf text; leaves whose normalized match text
  does not start with the literal program stay exact-only. Fuzzy generalizes simple-identifier subcommands into a closed
  alternation like `^cargo (build|check)(\s|$)`, falling back to a program prefix; Allow is emitted only with
  `--match exact`). Explicit `--action allow-redirect` is also exact-only and switches to target mining: active redirect
  policy is filtered out; leaves containing dynamic or shell-context-dependent endpoints and command-blocked leaves are
  omitted; project targets preserve each endpoint's direction and emit `allow_local = true`. Local target patterns are
  portable across project cwds. Historical tilde targets use the current process HOME and live filesystem because HOME
  is not recorded. Ordinary Allow suggestions omit leaves already covered by active command policy; Ask and Deny
  suggestions retain command-allowed leaves when redirect policy still prompts. `replay` re-evaluates the full candidate
  component policy, including one `ArgumentFilter` recheck; its migration gate fails on lost auto-approvals or when
  every in-scope row lacks a reproducible cwd. `test bash-rules --explain [--cwd <dir>] [--mode <mode>]` prints consumed
  bindings, original/alias-expanded/normalized leaf text, command matches, endpoint resolution/locality/rules,
  contributors, and the merged decision; normal mode uses the same compound path. Omitted `--mode` runs a mode-less
  evaluation. Alias coverage spans `user_config`, splitter, engine, authoring commands, CLI, and hook tests and must run
  under Nextest like every XDG-mutating test.
- **Rule-set provenance**: each `PreToolUse hook completed` log line records `rules_hash`, a stable hash of the
  effective config (`UserConfig::effective_hash`, computed once per hook invocation). The hash covers the parsed config
  — tool rules, bash rules, fragments, and the deterministically ordered path-alias policy — re-serialized via
  `serde_json::to_value` so map keys (`pattern_fragments`, an ArgumentFilter `replace` table) sort deterministically
  while rule order (significant for first-match-wins) is preserved; cosmetic edits don't change it but any semantic
  change does. The empty alias set is omitted so pre-feature hashes remain unchanged. `rules suggest`/`rules replay`
  default to only the records whose `rules_hash` matches the rule set currently installed at the default config path
  (for `replay` this is the migration source, independent of the `--config` candidate); `--rules-hash <hash>` pins a
  specific set and `--all-rules` disables the filter. Both commands report the active hash and how many records the
  filter excluded (`report::RulesHashFilter`/`HashSkipStats`/`CwdAggregation`); excluded counts are never hidden.
  `rules report` follows the same disclosure rule under `--current-rules`, but defaults to all recorded history.
- Security model: Defaults to "Ask" when unconfigured, fail-closed once configured (verification failures block
  execution)

**`approval_tui/`** - Interactive approval interface:

- Multi-screen TUI flow: ProjectOverview → CommandReview → InProjectWarning → Summary → Approved/Cancelled
- Reviews both commands and checks, showing security details (binary path, hash, writability, in-project status)
- Script contents preview for writable in-project scripts
- Atomic file I/O with locking during final approval save

**`project_config/`** - Project configuration and security:

- Three submodules: `config` (loads `.config/tools.toml`), `approvals` (SHA-256 verification), `runner` (verified
  execution)
- **Design asymmetry**: Commands are fixed struct (lint/test/build/format) for MCP, Checks are dynamic `Vec<Check>` for
  user validations. Check names must be unique: config loading rejects duplicates because approvals and resolved
  programs are keyed by name, preventing one check's approved argv from authorizing another duplicate's arguments
- Binds each command's argv plus resolved binary paths and binary hash as match-any approved versions, keyed by
  repository root with config loaded from the workspace root; uses file locking for atomic persistence

### Key Design Patterns

**Security Model**:

- **Default to Ask for Bash when unconfigured**: If no bash rules configured, Bash defaults to "Ask". Non-Bash tools
  with no matching tool rules return no decision, deferring to Claude Code's native permission system.
- **Fail-closed when configured**: Once security measures are in place, any verification failure blocks execution:
  - ItemNotApproved: a command/check name is configured but has no approved version (e.g. a brand-new name added to
    tools.toml)
  - ItemChanged: an approved item's argv or binary changed since approval (no approved version matches the current
    argv + resolved binary paths + binary hash); the result also reports whether the argv specifically changed and how
    many approved versions exist
  - (`ConfigHashMismatch` and `BinaryHashMismatch` are gone: approvals no longer hash `tools.toml` wholesale, so
    cosmetic edits and deleted commands never trigger re-approval — only a new name, an argument change, or a binary
    change does)
- **Match-any approval sets**: each command/check name maps to a `Vec` of approved versions (argv + storage-normalized
  paths + binary hash); argv-bound versions accrete additively and only match if that exact argv+binary reappears. A
  binary-change re-approval retires legacy `argv: None` versions for the same normalized paths, so reverting to the old
  binary fails closed instead of restoring its wildcard approval. Approvals are keyed by repository root (shared across
  worktrees); the config that runs is loaded from the workspace root, so each worktree runs its own tooling while
  identical content needs no re-approval. Stored binary paths are normalized relative to the workspace root when inside
  it, so a byte-identical `./script.sh` in a second worktree matches the first's approval
- **SHA-256 verification**: All binaries hashed, symlinks resolved before hashing
- **Dual path tracking**: Stores both original and canonical paths to detect symlink changes
- **Atomic updates**: File locking (fs2 crate) prevents race conditions during approval saves
- **Resource limits**: Check timeouts, concurrency limits, and output size caps prevent abuse
- **Sensitive data protection**: Environment variables matching TOKEN|SECRET|KEY|PASSWORD patterns are redacted

**TUI Architecture**:

- All TUI apps follow same pattern: event loop with async event stream, state machine for screens, ScrollViewState for
  scrolling
- Event handlers are async to support I/O operations (file reads, approval saves)
- State machines use enums for screens with explicit transitions

**MCP Command Patterns**:

- Two architectural approaches for exposing commands via MCP tools:
  - **Separate tools per command** (git_read_only): Each command (status, diff, log, show) is a separate MCP tool with
    its own parameter struct. Better discoverability in Claude Code's tool picker, matches rmcp examples, more
    boilerplate.
  - **Enum-based single tool** (jj_read_only): Single MCP tool with `JjCommand` enum parameter to select the command.
    Less boilerplate, cleaner code, single handler, but Claude Code sees only one tool.
- Trade-offs:
  - **Separate tools**: More verbose but each tool is independently discoverable and documented in MCP's tool list
  - **Enum-based**: More concise and maintainable, but requires understanding the enum variants (still type-safe via
    JSON schema)
- Choice depends on: number of commands, similarity of parameter structures, and whether command discoverability is
  critical

**Configuration** (XDG-compliant):

- `~/.config/moriarty/tool_rules.toml` - Tool and Bash validation rules
- `<project>/.config/tools.toml` - Project commands and checks
- `~/.config/moriarty/project_approvals.toml` - Per-command approval sets (argv + resolved binary paths + binary
  SHA-256, match-any versions keyed by repository root)
- `~/.local/state/moriarty/hooks/` - Hook execution logs (JSON lines, daily-rotated)

**Repository Root Detection**:

- Approvals are keyed by **repository root**, not workspace directory, so identical content across jujutsu workspaces
  and git worktrees shares one approval. The configuration that verification parses (and the binaries it resolves) comes
  from the caller's **workspace root** (`detect_workspace_root`), which is what gives each worktree tooling
  independence: a worktree on a branch with different tooling runs its own `tools.toml`. Per-command approvals bind the
  argv, resolved binary paths, and binary hash as match-any versions, so only a new name, an argument change, or a
  binary change triggers re-approval
- Detection order: resolving `.jj/repo` (store directory or pointer file) → `git rev-parse --git-common-dir` →
  canonicalized path
- This allows approval sharing across jujutsu workspaces and git worktrees
- `repository.rs` also provides `detect_workspace_root()`, the working-copy counterpart (nearest directory containing
  `.jj/repo` → `git rev-parse --show-toplevel` → canonicalized path); the Stop hook and `run_checks` MCP tool
  (`checks::run_configured_checks`) load the config from and run checks in the workspace root, so a secondary workspace
  or worktree runs its own config and validates its own working copy while sharing the main repository's approvals.
  `moriarty test checks` (via `project_config::runner`) instead executes at the caller's canonicalized project
  directory, without the workspace-root walk
- Stored binary paths are normalized relative to the workspace root when inside it (`normalize_path_for_storage`), so an
  approval made in worktree A matches byte-identical content in worktree B; PATH-resolved binaries (`which cargo`) live
  outside the workspace and stay absolute
- For jj: `.jj/repo` is the store directory itself in the main workspace and a pointer file in a secondary workspace.
  Absolute pointers are used as-is; a relative pointer is resolved against the `.jj` directory (jj 0.41+) with a
  fallback to the workspace directory (older jj), so both layouts share one repository root
- For git: uses `--git-common-dir` which returns the shared `.git` directory for all worktrees
- Module: `repository.rs` provides `detect_repository_root()` function

## Development Notes

**Workspace Optimization**: The `my-workspace-hack` crate is managed by cargo-hakari to unify dependencies.

**Shared Test Utilities**: Test helpers used across multiple modules (`setup_isolated_xdg_config`,
`setup_isolated_xdg_state`, `setup_project_dir_with_config`, `write_tools_config`, `create_executable_script`,
`run_git_command`, `setup_git_repo_with_commit`, `setup_jj_main_and_secondary_workspace`,
`assert_workspace_local_copy_ran`, `set_test_env_var`, `remove_test_env_var`, `TestEnvVarGuard`, `redirect_rule`,
`directional_redirect_rule`, `deny_redirect_rule`, `SUBAGENT_EXECUTION_RULES`) live in
`crates/moriarty/src/test_helpers.rs`. This module is compiled only in test builds (`#[cfg(test)]`). All test
environment mutations go through `apply_test_env_var()` — the module's single `unsafe` block — with process isolation
guaranteed by `cargo nextest`. New test-only helpers needed in more than one module belong here rather than being
duplicated.

**Logging**: Hook execution is logged via tracing as JSON lines to `~/.local/state/moriarty/hooks/` (daily-rotated); the
`hooks report` command consumes these. Cost-report commands log to stderr instead. Sensitive env vars (TOKEN, SECRET,
KEY, PASSWORD) are redacted.

### Doc Comments

Doc comments (`///`) and inline comments (`//`) on Rust items must explain WHY, not WHAT. The function name, signature,
and body already say what the code does; comments should add information that is not visible from the code itself.

**Delete** doc comments that:

- Restate the function name (e.g. `/// Format duration in a readable way` on `fn format_duration`).
- Narrate the body line-by-line (e.g. `/// Appends one row per non-zero-cost model in display order` on a function that
  does exactly that and nothing else).
- Re-describe parameter names (e.g. ``/// `grand_total` is the footer total.`` on a parameter named `grand_total`).

**Keep or write** doc comments that:

- Explain a non-obvious choice or trade-off (e.g. why an enum arm must come before another to avoid misclassification).
- Document an invariant a caller must uphold (e.g. that two parameters are produced together and the indices are only
  valid against the matching vector).
- Capture context that is not obvious from the surrounding code (e.g. why a sentinel timestamp is safe because the
  variant is never billable).

Applies to source files only. CLAUDE.md and other docs use ordinary prose.

### Error Handling

This project uses the `miette` crate for rich error reporting throughout:

```rust
use miette::{IntoDiagnostic, Result, WrapErr};

fn example() -> miette::Result<()> {
    std::fs::read_to_string("file.txt")
        .into_diagnostic()
        .wrap_err("Failed to read configuration")?;
    Ok(())
}
```

**Conventions**:

- Use `miette::Result` as the return type (qualified usage to avoid shadowing std::Result)
- Use `.into_diagnostic()` to convert std errors
- Use `.wrap_err()` or `.context()` to add contextual error messages
- Use `#[derive(Debug, miette::Diagnostic, thiserror::Error)]` for custom error types

### Imports

#### Import grouping

This project has a particular convention for imports. There should be 3 groups of imports:

- std library,
- 3rd party crates,
- local and workspace crates

You should always use the compact import form.

This looks something like this:

```rust
// standard library imports
use std::{collections::{HashSet, HashMap}, fmt::Display};

// 3rd party crates
use chrono::{Datelike, NaiveDate, TimeZone, Utc};

// local / workspace deps
use super::{analyzer::*, pricing::{ModelMetricsMap, ModelType}, time_filter::TimeRangeFilter};
```

#### Avoid qualified usages

Additionally, you should avoid qualified usages inside of code blocks.

Instead of:

```rust
fn new_hashset() -> std::collections::HashSet<String> {
    std::collections::HashSet::new()
}
```

You should write:

```rust
use std::collections::HashSet;

fn new_hashset() -> HashSet<String> {
    HashSet::new();
}
```

There are two exceptions to this:

1. **Clarity through qualification**: Use qualified references when they make the code more clear. Examples:
   - `mpsc::channel()` vs `oneshot::channel()` - clarifies which channel type
   - `tokio::spawn()` vs `rayon::spawn()` - clarifies which runtime
2. **Avoiding prelude shadowing**: Use qualified references for types that would shadow std prelude items:
   - `miette::Result` - Never shadow `std::prelude::Result`
   - `miette::Error` - Never shadow `std::error::Error`
   - Custom `Result` type aliases should be avoided in favor of explicit `miette::Result`

#### Always do imports at the top of the module

Import go at the top of the file not in individual code blocks. The only exception to this is something like diesel's
generated table functions, they would all collide with each other making the code difficult to understand. Diesel is not
currently in use in the code base.

### Serde Conventions

The codebase uses specific serde attributes for protocol compatibility:

```rust
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]  // Fail on unexpected fields
#[serde(tag = "type")]          // Tagged enum variants
#[serde(rename_all = "camelCase")]  // Match JSON conventions
struct Example {
    #[serde(skip_serializing_if = "Option::is_none")]  // Omit None values
    optional_field: Option<String>,
}
```

**Important**: Always use `#[serde(deny_unknown_fields)]` when deserializing Claude Code protocol messages (hooks, log
parsing) to catch when Claude Code updates have added new fields that this codebase doesn't yet handle.

User-config compatibility is a separate exception: legacy `BashRuleAction` variants continue to ignore unknown
action-table keys because existing configs historically allowed metadata there. `AllowRedirect` and `DenyRedirect` use
dedicated strict payloads so misspelled locality or direction fields cannot silently widen path scope.

**Exceptions**: in `pi_logs`, three categories of struct legitimately omit `deny_unknown_fields`:

Also, do not force `rename_all = "camelCase"` onto parser structs whose upstream wire schema is not camelCase. Preserve
the on-disk protocol exactly, even when that means snake_case fields like `GitReadOnlyArgs.project_dir`.

1. **`serde(flatten)` of an internally-tagged enum**: when a struct flattens an enum that uses `#[serde(tag = "...")]`
   without a `content` field, the inner tag appears at the same JSON level as the outer struct's fields and serde's
   flatten codegen does not register it as claimed; a strict outer struct then rejects it as unknown at runtime.
   `WebSearchResultsData` is the only struct in this category. It keeps the flattened internally tagged wire shape, but
   restores strict outer-key validation with a manual deserializer. _Adjacently_ tagged flatten targets (those with both
   `tag` and `content`) do not hit this collision, so structs like `CustomLine` and `CustomMessageLine` keep derived
   `deny_unknown_fields` handling. Each exception must carry an inline comment naming the limitation.
2. **Corrupt-stream tolerance**: tool-argument structs (e.g. `EditArgs`, `EditReplacement`, `GrepArgs`) deliberately
   omit it to tolerate completed-but-corrupted or hallucinated assistant streams that emit malformed sibling keys. The
   same goal is also met at finer granularity by field-level aliases (for example `FindArgs.limit` accepting malformed
   `.limit` while keeping the rest of the struct strict) and untagged fallback enums (`EditEntry::Fragment` absorbs raw
   JSON tokens in an `edits` array; `MaybeU32::Garbage` absorbs string-typed corruption of numeric tool-call arguments).
   Each such exception must carry an inline comment naming the observed failure mode.
3. **Forward-compatible protocol schemas**: structs representing server-defined or runtime-defined protocol envelopes
   whose field sets evolve independently of the parser (e.g. `McpCallResult` for MCP tool-call results, which pi's
   runtime regularly extends with new metadata fields like `contentBlocks`, `outputGuard`, and `omitted`). Every such
   exception must carry a struct-level doc comment explaining why strict rejection is omitted and citing example fields
   that motivated the relaxation.

## Suggesting Updates to CLAUDE.md

When you make significant changes to the codebase that introduce new patterns, conventions, or architectural decisions,
you MUST suggest updates to this file.

**CRITICAL**: You MUST make the change with the real edit tool, not just propose prose. When suggesting a CLAUDE.md
update in your response, clearly name the topic you think should be documented, then immediately apply the matching edit
to `CLAUDE.md` with the actual tool call.

**Examples of significant changes that warrant CLAUDE.md updates**:

- New architectural patterns or design decisions
- New conventions for code organization or style
- Changes to the build system or testing strategy
- New security considerations or validation approaches
- Changes to configuration file formats or locations
- New error handling patterns or async patterns

**What NOT to document**:

- Implementation details of specific features
- Temporary workarounds
- Details that are better suited for code comments
- Information that will become stale quickly

The goal is to keep CLAUDE.md focused on information that helps understand how to work with the codebase effectively
across sessions.

## Finishing

After you have modified code, you are not allowed to stop until all of the quality checks have passed. If you need to
ask the user a question, use the dedicated user-question tool rather than writing the question in plain text and then
waiting for the user's next input.
