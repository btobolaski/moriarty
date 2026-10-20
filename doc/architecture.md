# Architecture

Workspace crates live under `crates/`. `claude_logs`, `pi_logs`, `cost_analyzer`, and `process_supervisor` are separate
crates; the reporting, hooks, MCP, approval, configuration, and TUI modules below live under `crates/moriarty/src/`. The
binary entry point is `crates/moriarty/src/main.rs`; `my-workspace-hack` is managed by cargo-hakari.

## Contracts and related references

- [Process-supervision specification](../openspec/specs/process-supervision/spec.md): standalone OS lifecycle contract.
- [Moriarty process-execution specification](../openspec/specs/moriarty-process-execution/spec.md): integration,
  cancellation, and authorization requirements.
- [MCP protocol-support specification](../openspec/specs/mcp-protocol-support/spec.md): explicit revisions,
  modern/legacy stdio interoperability, and exclusion of Tasks.
- [Supervisor crate rustdoc source](../crates/process_supervisor/src/lib.rs): public API contract and assurance limits.
- [Project-tools configuration manual](man/moriarty-project-tools.5): command/check configuration and approvals.

The implementation notes below complement these contracts; do not substitute them for the specifications.

## Module organization

### Claude Code logs

See the [crate overview](../crates/claude_logs/src/lib.rs),
[log schema and compatibility rationale](../crates/claude_logs/src/parser.rs), and
[model parsing](../crates/claude_logs/src/model.rs).

### `cost_report/`

Shared cost report rendering and filtering.

- Holds shared time filtering, grouped-table rendering, stacked-chart rendering, `ReportMode`, `CostComponents`,
  `TokenCounts`, `MetricComponents`, and report warning helpers used by both cost-report backends. `MetricComponents`
  carries agent-turn counts for detail rows alongside a cost/token payload; `ReportTotals` pairs the two only while
  rendering subtotal/summary rows, and `sum_report_totals` keeps backend grand-total aggregation aligned. Chart-facing
  `MetricTotal` remains cost/token-only
- `FormattedMetricColumns` and `GrandTotalRow` render agent turns with integer separators; cost mode formats dollars and
  token mode formats integer token counts while preserving the same table shape for both backends
- `display_summary` renders a consolidated "Summary" section (optional provider table for `pi cost`, model table for
  both backends, and grand total) called by each backend after its inline grouped-table rendering
- `charts.rs` renders deterministic horizontal stacked bars for both time-series and share views, including top-N plus
  `Other`, stable glyph/color assignment, and narrow-terminal truncation without changing the table-report path
- `series.rs` carries typed date/session chart handoff data so multiple sources can be merged before rendering
- Keeps the output behavior for `api-pricing`, `pi cost`, and the graph commands aligned without forcing the backends
  into a dynamic-column abstraction

### `api_pricing/`

Claude API usage cost analysis.

- Aggregates either pre-priced `LlmCost` values or raw token counts from `cost_analyzer` into daily buckets (keyed by
  timezone-adjusted date) or per-conversation buckets (keyed by session ID)
- Per-model aggregation uses `ModelMetricsMap` keyed by `claude_logs::Model` (family + parsed version) so report rows
  and chart legend distinguish e.g. "Sonnet 4" from "Sonnet 4.5"; row/legend ordering is family-first (Fable → Opus →
  Sonnet → Haiku) then version-desc via the local `model_sort_key` helper, so within-family Opus 4.x rows sit above Opus
  3 rows automatically. Token mode stays integer-exact end-to-end instead of passing through floating-point helpers
- Agent turns count deduplicated billable assistant responses, so copied records and multi-line content blocks sharing
  the same preferred response identifier contribute once
- Unknown Claude models surface as stderr tracing errors via `cost_analyzer`; they are not rendered in the report
- Also prepares `ChartBucket` data for `graphs claude`, reusing the same analyzer output while keeping the existing
  detailed table report unchanged
- Entry points: `api-pricing` and `graphs claude` subcommands in `main.rs`

### `pi_cost/`

Pi session cost analysis.

- Aggregates recorded `LlmCost` values or raw token counts from `cost_analyzer` into daily buckets or per-conversation
  buckets keyed by normalized session ID. Cost mode estimates only all-zero recorded usage from pi's
  `models-store.json`, then applies `models.json` price overrides (`PI_CODING_AGENT_DIR`, else `~/.pi/agent`); nonzero
  recorded charges remain unchanged. The catalog is loaded lazily once per report, without network requests or
  credential execution. Decimal estimates follow pi's prompt-size tiers and 1h cache-write pricing. Missing prices warn
  and retain zero; invalid rates fail the report. These are current catalog estimates, not historical prices; token mode
  never reads pricing files
- Uses raw pi `(provider, model)` pairs for row grouping, with deterministic ordering from a
  `BTreeMap<PiModel, MetricComponents>` accumulator inside `PiModelMetricsMap`
- Conversation mode depends on `cost_analyzer::LineWithCost.session_id`, which is attached during the single-pass parse
  from either Claude assistant lines or pi `SessionLine` headers
- Agent turns count assistant messages only; billable compaction and branch-summary summarization calls contribute cost
  and tokens under the active model but zero turns. Auxiliary `usage` records contribute under their explicit model,
  with zero turns, and do not change the active model used by subsequent summaries
- Also prepares provider/model `ChartBucket` data for `graphs pi`, reusing the same analyzer output while keeping the
  existing detailed table report unchanged
- Entry points: `pi cost` and `graphs pi` subcommands in `main.rs`

### `combined_graphs.rs`

Concurrently analyzes Claude/API and pi logs, merges typed date/session series, and prefixes segment labels by source.
Cost mode mixes Claude's local pricing with pi's recorded prices plus zero-cost catalog estimates; missing defaults are
skipped with warnings, while explicit missing paths and having no available source are errors.

### Pi logs

See the [crate overview](../crates/pi_logs/src/lib.rs) and
[parser schema and compatibility rationale](../crates/pi_logs/src/parser.rs).

### `cost_analyzer/`

Generic cost-analysis library.

- Workspace crate for recursively scanning JSONL directories, parsing logs in parallel, and deduplicating billable model
  responses. It skips Claude's well-known non-transcript `history.jsonl` and workflow `journal.jsonl` files by basename
  wherever they occur because neither schema contains billable model responses, and pi's pi-subagents
  `<runId>_<agent>_transcript.jsonl` artifact archives because they use a separate `recordType` envelope while
  duplicating the billable turns already logged to each child run's own `session.jsonl`
- Core abstractions: `AnalyzableLog` for pluggable log formats, `LlmCost` for input/cache/output cost breakdowns,
  `TokenType` plus `AnalyzableLog::token_count(...) -> Option<u64>` for raw token extraction, `LineWithCost` for
  normalized billable entries, and `AnalysisResult` for returning those deduplicated lines alongside a partial-failure
  flag
- Concrete implementations currently support `pi_logs::PiLogLine` and `claude_logs::LogLine`. Claude log costs are
  calculated in `cost_analyzer` with local Decimal-based Claude pricing helpers (`ClaudeModelPricing::for_model`) that
  consume `&claude_logs::Model`; the family enum itself lives in `claude_logs` so the parser and pricing layer agree on
  classification without depending on `moriarty::api_pricing` internals. Opus 3 vs Opus 4.x share `ModelFamily::Opus`
  and the pricing dispatch reads the parsed `version.major` to pick the OPUS or OPUS_4 tier; `ModelFamily::Fable`
  compares the full parsed `ModelVersion` against 5.1 to pick FABLE (Fable 5 and versionless ids) or FABLE_5_1 (5.1 and
  later, which differs only in its reduced cache-read rate).
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
- `parse_json_line` (the shared typed JSON-line funnel for both log formats) retries a failed line once after rewriting
  lone UTF-16 surrogate `\uXXXX` escapes to U+FFFD: pi logs legitimately contain them (JSON.stringify serializes
  ill-formed provider strings that way, and the grammar is valid per RFC 8259) but serde_json's well-formed-UTF-8
  validation rejects them; a failure whose sanitized form still does not parse surfaces the original error so reported
  positions refer to the raw line. The retry helper itself (`pi_logs::parser::parse_json_with_surrogate_retry`) lives in
  `pi_logs`, so `pi_logs`' own public entry points (`parse_line`/`parse_file`) apply the same retry;
  `cost_analyzer::parse_json_line` delegates to it
- Public entry point: `cost_analyzer::analyze_directory(path)`

### `process_supervisor/`

Trusted asynchronous process-group supervision for Moriarty's non-installer subprocesses.

- Supports ordinary Linux/macOS process trees, not a security sandbox or deliberately escaped descendants. Both the
  crate and test roots forbid unsafe code; std and pinned rustix safe APIs own the OS boundary. `lib.rs` contains only
  crate attributes (including rustdoc), private module declarations, and API reexports.
- Detached, admission-bounded std launch threads own stalled creation; one Tokio driver owns each handed-off child and
  both bounded captures. Non-reaping `waitid(EXITED | NOHANG | NOWAIT)` pins the leader until final signaling, and
  reaping requires an exclusive disarmed view, including error/Drop paths. Recheck the current host process group before
  each destructive attempt, including retries; matching it permanently revokes authority even if the host later leaves.
  Keep the raw child and semaphore-closing authority private; preserve revocation when changing the backend. Startup
  outcomes must encode ownership, creation certainty, and interruption reason explicitly rather than require
  classification from mutable evidence fields.
- `RunningProcess::spawn` is the owning async constructor. Before handoff, admission/failure outcomes carry explicit
  interruption and creation certainty; after handoff, the phase's cause owns interruption and error metadata is derived
  only at finalization. Capture lifecycle is a private enum holding the reader only while open; public EOF evidence is
  derived from that lifecycle rather than synchronized flags. Register pipes before constructing the initial phase.
- At a Running control boundary, a fresh non-reaping exit observation takes precedence over cancellation/timeout;
  selected TERM causes remain fixed. Confirmed group absence is monotonic. Failed/closed capture settles error cleanup
  without inventing EOF, and phase-tagged interruption metadata survives cleanup uncertainty. Lost driver reports cannot
  establish successful cleanup.
- Late-launch failures use output-excluding tracing diagnostics. Subscriber dispatch can block while retaining the
  launcher's admission slot; this is an accepted risk, not a nonblocking logging guarantee.
- Crate rustdoc is the contract reference for validated options, direct `CancellationToken` ownership, null stdin,
  startup uncertainty, descendant cleanup, completed versus partial outcomes, and host-lifetime/reaping limits.
- Tests use exact ignored libtest subprocess fixtures and private module-selected fault seams (`launch::hooks` and the
  process-operations alias); production functions contain no `#[cfg(test)]` logic. Fixture dispatch needs body evidence,
  and rescue must remain independent, identity-safe, and bounded. See crate rustdoc for watchdog mechanics and the
  limits of indirect SIGCHLD-isolation evidence. Descendant tests independently require watchdog-socket EOF, not prompt
  external orphan-zombie reaping; socket EOF is not group absence, which remains mandatory for successful supervisor
  reports. Rescue must preserve the socket's read half so local shutdown cannot fabricate EOF. Never replace safe
  dependency operations with local FFI/pre-exec hooks or change MCP protocol support as part of this crate. Moriarty's
  integration tests verify policy and ownership, not a second set of signal/reaping/descendant guarantees.

### `execution.rs`

Private Moriarty execution ownership.

- A token-free task-local submission handle routes project commands, checks, read-only VCS commands, and async git root
  probes through `process_supervisor`. Preserve the exact verified program `PathBuf`, including dispatch-sensitive
  symlinks; inherit the environment and pass literal argv with null stdin. Moriarty now requires Linux/macOS.
- Unscoped library/test calls use a single process-completion owner, not a one-operation scope/dispatcher. Dropping the
  operation requests cancellation while that owner retains the wait; runtime destruction still cannot guarantee drain.
- Register scoped owners directly with `TaskTracker` under the admission gate; there is no dispatcher or job queue.
  Operation tokens are children of the host-linked scope token. Request scopes also retain the actual request token:
  admission/startup inspect it directly, even during work polling, and nested batch scopes inherit it. Each process
  owner bridges that request cancellation to its operation token while retaining startup/awaited wait ownership;
  cancellation wins before re-polling startup. No relay task or token is threaded through ordinary helper chains. Typed
  cancellation before startup is polled proves no launch and cannot become a settlement failure, even when result
  delivery is lost; it remains an execution failure for the caller. After polling, retain the supervisor's uncertainty
  and cancel-and-await contract.
- Tracked scope/request/process owners survive dropped callers. Close admission, broadcast cancellation, then await
  concurrent settlement before normal runtime exit; do not abort completion owners. Request cancellation stays local.
  With a live runtime, request owners contain work-future panics and still await child drain before delivery; synthetic
  panic/cleanup diagnostics omit the payload and survive caller loss. Completed work and cleanup evidence stay separate
  until the CLI/MCP boundary combines them; cleanup failure must not replace the initiating failure or blocked
  tool-result content. Shared settlement composition uses alternate Display to preserve available source causes for both
  errors without Debug-printing captures. Keep transport causes separate from execution's first cleanup error so both
  survive. The MCP `Service` wrapper consumes pinned rmcp `RequestContext.ct`; observed EOF/errors close the scope
  before rmcp drains pending handlers. Pending MCP write/flush/shutdown polls also register the service-cancellation
  waker: stalled output fails instead of retaining rmcp's writer mutex, without a new timeout or aborting execution
  owners. Writable EOF replies still drain; partial delivery is retained as a transport failure. Signals also settle
  cooperative migrated CLI/hook work, and approval's drop guard restores the terminal even during initialization.
  Unrelated CLI paths and inherited-stdio `mcp install` remain outside this scope.
- The `scoped` match in `main.rs::run_cli` is the single CLI registry: add any new non-MCP dispatch path reaching
  `execution::submit` there to retain signal-driven drain. MCP hosts own their scopes; the installer stays excluded.
  `input_host` separately selects only MCP servers and `hooks exec` for `Runtime::shutdown_background` after that drain:
  Tokio's uncancellable stdin syscall must not hold runtime destruction open. Hooks use asynchronous input reads so
  cancellation can reach the owner; other commands and installation retain ordinary runtime teardown. Stdout from
  `moriarty hooks exec`, `moriarty test checks`, and `moriarty test project-tools` remains blocking: an unread full pipe
  can prevent cooperative polling and require SIGKILL (owner-accepted), with no output-write timeout or graceful signal
  exit promised in that condition. This does not waive MCP backpressure shutdown, cancellation/admission, or owned-work
  settlement. Forceful termination proves neither owned-child cleanup nor completed approval persistence.
- Approval-lock acquisition uses nonblocking fs2 retries owned by the caller. After acquisition, `retain_persistence`
  admits the load-modify-save future under the same scope gate/tracker as process work. Caller loss cannot drop its
  transaction, lock or actual I/O wait; host drain includes it and retains failed writes. Retention uses alternate
  Display (`{error:#}`) so safe I/O context and source causes survive caller loss without Debug-printing captures. No
  rollback or crash guarantee.
- Supervisor defaults: 1 MiB raw bytes per stream, 5 s startup, no execution timeout, 2 s TERM and 1 s post-KILL
  cleanup. Project/check text discloses truncation; incomplete VCS/root-probe output fails closed. Lossy UTF-8
  conversion can expand text; limits are not a global memory bound. Never log captured output in lifecycle diagnostics.
- Stop-hook/MCP checks retain four-way concurrency, a five-minute whole-batch execution budget (including queueing and
  startup after verification), and a 10 MiB aggregate formatted-output gate. Use `buffer_unordered` for bounded,
  incremental scheduling. Dropping the pipeline cancels its operations before the owning batch scope drains them;
  cleanup may exceed the execution deadline. CLI checks have neither that deadline nor aggregate gate, but still have
  bounded captures. Only an ordinary exit qualifies for exit-code success; startup/cancellation/supervision uncertainty
  must not become a fallback approval key or successful no-checks result.
- Gated ownership tests and result-only injection seams live in `execution/tests.rs` and `execution/test_seams.rs`.
  Deadline-routing tests rendezvous with admitted owners before advancing paused Tokio time; `test-util` is declared in
  Moriarty's dev dependencies. Keep these tests at the integration boundary, not supervisor OS fault seams.
- Keep OS cleanup assurance in the supervisor crate. Moriarty tests cover authorization, output policy, cancellation
  routing, and awaited ownership. Graceful drain needs a live runtime; SIGKILL/crashes and stalled creation's
  late-cleanup uncertainty retain the supervisor contract's limits. Both legacy and modern MCP tool calls, including a
  first-message modern call, use the same request-token bridge and awaited ownership; Tasks remains disabled.

### `tui/`

Terminal UI event infrastructure.

- Provides an async event stream (`input_stream`) that maps crossterm terminal events (keys, resize, paste) into the
  internal `Event` / `UIEvent` enum
- Used by `approval_tui/` as its input source

### `mcp/`

Model Context Protocol servers.

- Three MCP servers: `git_read_only` (status, diff, log, show), `jj_read_only` (status, diff, log, show, op log, file
  show, file list), and `tool_runner` (lint, test, build, format, checks)
- All three servers explicitly support exactly `2024-11-05`, `2025-03-26`, `2025-06-18`, `2025-11-25`, and `2026-07-28`.
  Each handler sets its legacy initialization fallback to `2025-11-25` separately from the supported list; dependency
  defaults must not widen either policy. Legacy `initialize` echoes supported legacy revisions, but modern or unknown
  revisions fall back to `2025-11-25`, followed by `notifications/initialized`.
- Modern `2026-07-28` requests need neither initialization nor discovery. Each modern request carries `_meta` entries
  `io.modelcontextprotocol/protocolVersion` and `io.modelcontextprotocol/clientCapabilities`; client info is optional.
  For ordinary modern requests, rmcp validates context per request: missing/malformed required metadata returns
  `-32602`; unsupported revisions return `-32022` with `requested`/`supported` data.
- rmcp 3.6 owns lifecycle-neutral `server/discover` routing and recovery, with the local revision-precedence guard
  below. Successful discovery, unsupported discovery, and malformed opening requests do not prevent subsequent legacy
  initialization or modern work on the same connection. Modern complete results carry `resultType: "complete"`;
  discovery, tool lists, and Git prompt lists also carry result-level `ttlMs` and `cacheScope` (SDK defaults suffice; no
  application cache). Legacy tool/prompt wire shapes stay unchanged. Only Git advertises prompts; jj and project-tools
  remain tools-only.
- Tasks (`io.modelcontextprotocol/tasks`) is neither advertised nor implemented, even for clients advertising it.
  `tasks/get`, `tasks/update`, and `tasks/cancel` return `-32601`; tools remain ordinary request/response operations,
  not background task results. No HTTP/SSE, subscriptions, or new server-to-client interactions are introduced.
- `ProtocolGuardTransport` rejects unsupported-version or modern-context ping before rmcp's unconditional bootstrap fast
  path. Supported legacy-version ping passes through even with context keys, as does context-free ping (including
  empty/extension-only metadata). Modern ping returns `-32601`; missing/malformed modern context returns `-32602`.
  Parseable unsupported ping and discovery revisions return `-32022` with revision data before checking other required
  metadata, even when those fields are missing or malformed. Other discovery validation and lifecycle recovery remain
  upstream-owned; the guard uses SDK metadata validation and never selects a lifecycle.
- Guarded rejections that cannot finish immediately move to one tracked sender with at most 64 queued sends plus one
  active send, rather than blocking input or living in a cancellable receive future. Send failure or queue exhaustion
  closes admission and retains a host-visible transport error. Transport close cancels blocked output and awaits the
  sender before closing the SDK transport; `serve_until` also awaits its join handle on every exit, including failed or
  cancelled startup. No per-rejection tasks are detached.
- Moriarty retains `OwnedService`/`Scope` for request-owned execution and `ObservedIo` for admission closure, retained
  transport failures, and cancellation of blocked response writes. SDK EOF cancellation does not prove owned-process
  settlement or replace this application boundary.
- All MCP tools accept an optional `project_dir`. Omitted values default to `.` and resolve against the server's current
  working directory at use time.
- `read_only`: Shared infrastructure used by both `git_read_only` and `jj_read_only`. Provides `CommandResult`,
  `validate_project_dir`, and the generic `run_read_only_command`. It rejects parent-traversal and non-directory targets
  before canonicalizing the working directory, while the per-server wrappers add command-specific flag restrictions
  (`git` forces `--no-optional-locks`, `--no-ext-diff`, and `--no-textconv` while rejecting output-file / no-index
  escape flags; `jj` forces `--ignore-working-copy` and rejects external-tool, config-injection, and repository-override
  flags). Neither server consults `.config/tools.toml` approvals; only `tool_runner` does.
- `tool_runner`'s four command tools (`run_lint`/`run_build`/`run_formatter`/`run_tests`) run a single `[commands]`
  entry via `verify_and_load_project` + `run_command`; its `run_checks` tool runs the project's `[[checks]]` via the
  shared `crate::checks::run_configured_checks` routine — the same approval verification (checks only, not commands),
  resource limits (5-min batch deadline, 1 MiB/stream capture + 10 MiB aggregate formatted-output gate), and fail-closed
  semantics as the Stop hook. Cleanup has separate budgets and may extend past that deadline. That shared routine is
  intentionally distinct from `project_config::runner::run_all_checks` (no batch deadline or aggregate gate, bounded
  captures, verifies all commands too), which the `moriarty test checks` CLI uses. Both runner paths follow the
  [configuration and approval root policy](#repository-root-detection). Both spawn the program path
  `VerificationResult::Approved` carried back from verification (never the raw `command[0]`), so an unapproved local
  copy cannot shadow the approved binary. Their execution cwd differs: `project_config::runner` keeps the caller's
  canonicalized project dir, while the checks-runner uses `detect_workspace_root(project_dir)`, which can walk up past a
  nested project dir to the enclosing workspace root.
- Project-tool errors use miette alternate Display (`{error:#}`) to retain contextual causes and TOML locations without
  debug-printing supervisor captures; existing error codes/envelopes stay unchanged.
- Uses rmcp library with stdio transport for Claude Code integration
- All servers run as stdin/stdout servers that Claude Code can invoke
- `install` command configures all servers in Claude Code's MCP registry
- **Architectural patterns**: git_read_only uses separate MCP tools per command; jj_read_only uses enum-based single
  tool (see [MCP Command Patterns](#mcp-command-patterns))

### Hooks and rules

See [Hook and rule-engine internals](hooks.md) for permission evaluation, redirects, reporting, and rule provenance. Use
the [Tool & Bash Rules Configuration Guide](../BASH_RULES.md) for user-facing policy configuration.

### `approval_tui/`

Interactive approval interface.

- Multi-screen TUI flow: ProjectOverview → CommandReview → InProjectWarning → Summary → Approved/Cancelled
- Reviews both commands and checks, showing security details (binary path, hash, writability, in-project status)
- Script contents preview for writable in-project scripts
- Empty configured command/check argv is rejected before metadata indexing with the item name, not a lost-owner panic
- Admitted approval persistence retains its lock and actual I/O through cancellation; waiting for the lock is
  cancellable

### `project_config/`

Project configuration and security.

- Three submodules: `config` (loads `.config/tools.toml`), `approvals` (SHA-256 verification), `runner` (verified
  execution)
- **Design asymmetry**: Commands are fixed struct (lint/test/build/format) for MCP, Checks are dynamic `Vec<Check>` for
  user validations. Check names must be unique: config loading rejects duplicates because approvals and resolved
  programs are keyed by name, preventing one check's approved argv from authorizing another duplicate's arguments
- Binds each command's argv plus resolved binary paths and binary hash as match-any approved versions; uses file locking
  for atomic persistence. See [Repository Root Detection](#repository-root-detection) for configuration loading and
  approval sharing across worktrees.

## Cross-cutting design patterns

### Security Model

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
  binary fails closed instead of restoring its wildcard approval. See
  [Repository Root Detection](#repository-root-detection) for configuration/approval roots, stored-path normalization,
  and cross-worktree matching.
- **SHA-256 verification**: All binaries hashed, symlinks resolved before hashing
- **Dual path tracking**: Stores both original and canonical paths to detect symlink changes. Approval lookup/storage
  rejects non-UTF-8 executable identities instead of matching lossy spellings; verified program spelling stays intact
- **Atomic updates**: File locking (fs2 crate) prevents race conditions during approval saves. Only `NotFound` at the
  approval-file read initializes an empty store; setup, parse and other access errors propagate without running the
  update. `FileType::load` and `load_or_default` share that read/parse pipeline; only the latter permits actual-read
  `NotFound` to initialize defaults. Never classify absence from diagnostic text
- **Resource limits**: Stop-hook/MCP checks have a whole-batch deadline, concurrency limits, and output caps. CLI checks
  retain bounded captures and concurrency but no batch deadline or aggregate gate.
- **Sensitive data protection**: Environment variables matching TOKEN|SECRET|KEY|PASSWORD patterns are redacted

### TUI Architecture

- All TUI apps follow same pattern: event loop with async event stream, state machine for screens, ScrollViewState for
  scrolling
- Event handlers are async to support I/O operations (file reads, approval saves)
- State machines use enums for screens with explicit transitions

### MCP Command Patterns

- Two architectural approaches for exposing commands via MCP tools:
  - **Separate tools per command** (git_read_only): Each command (status, diff, log, show) is a separate MCP tool with
    its own parameter struct. Better discoverability in Claude Code's tool picker, matches rmcp examples, more
    boilerplate.
  - **Enum-based single tool** (jj_read_only): Single MCP tool with `JjCommand` enum parameter to select the command.
    Less boilerplate, cleaner code, single handler, but Claude Code sees only one tool.
- Choice depends on: number of commands, similarity of parameter structures, and whether command discoverability is
  critical

### Configuration (XDG-compliant)

- `~/.config/moriarty/tool_rules.toml` - Tool and Bash validation rules
- `<project>/.config/tools.toml` - Project commands and checks
- `~/.config/moriarty/project_approvals.toml` - Per-command approval sets (argv + resolved binary paths + binary
  SHA-256, match-any versions keyed by repository root)
- `~/.local/state/moriarty/hooks/` - Hook execution logs (JSON lines, daily-rotated)

### Repository Root Detection

- Async discovery uses fallible constructors for distinct `RepositoryRoot` and `WorkspaceRoot` newtypes with private
  UTF-8 string fields: unrepresentable identities cannot reach approval lookup/save, and no repeated preflight is
  needed. Keep these identities through verification and saving; those interfaces require both types so swapping them
  fails to compile. Use `as_path()` for path-based operations; git probes reuse the entry point's canonical cwd rather
  than canonicalizing it again at helper boundaries. Git absence requires independent executable lookup or a completed
  non-repository result; other probe/root-resolution failures stop discovery and block checks. Set-PATH entries are
  tested as absolute candidates under the recorded probe cwd (empty entries mean that cwd); unset PATH launches bare Git
  so libc owns native defaults without environment mutation. Spawn `NotFound` remains ambiguous, not absence. Only the
  known English non-repository/mount-boundary diagnostics allow fallback; translated or unknown wording fails closed.
  Strip only Git's output newline, never path whitespace. Construction enforces UTF-8 because the existing string-key
  format cannot represent arbitrary path bytes; never substitute lossy text or migrate the format implicitly.
- Before paired repository/workspace discovery, capture one canonical project input and pass it to both awaited
  detectors, as the project runner and shared Stop-hook/MCP checks do. Re-resolving a caller alias between them can
  authorize another workspace's tooling with the first repository's approvals; distinct root types alone do not
  establish that pairing. After this snapshot, even ordinary filesystem discovery failures block shared Stop/MCP checks;
  initial missing-input setup keeps its existing no-checks result. This closes alias retargeting and rejects observed
  late failures, not general filesystem TOCTOU.
- Approvals are keyed by **repository root**, not workspace directory, so identical content across jujutsu workspaces
  and git worktrees shares one approval. The configuration that verification parses (and the binaries it resolves) comes
  from the caller's **workspace root** (`detect_workspace_root`), which is what gives each worktree tooling
  independence: a worktree on a branch with different tooling runs its own `tools.toml`. Per-command approvals bind the
  argv, resolved binary paths, and binary hash as match-any versions, so only a new name, an argument change, or a
  binary change triggers re-approval
- Detection order: resolving `.jj/repo` (store directory or pointer file) → `git rev-parse --git-common-dir` →
  canonicalized path
- `repository.rs` also provides `detect_workspace_root()`, the working-copy counterpart (nearest directory containing
  `.jj/repo` → `git rev-parse --show-toplevel` → canonicalized path); the Stop hook and `run_checks` MCP tool
  (`checks::run_configured_checks`) load the config from and run checks in the workspace root, so a secondary workspace
  or worktree runs its own config and validates its own working copy while sharing the main repository's approvals.
  `moriarty test checks` (via `project_config::runner`) instead executes at the caller's canonicalized project
  directory, without the workspace-root walk
- Stored binary paths are normalized relative to the workspace root when inside it (`normalize_path_for_storage`), so an
  approval made in worktree A matches a byte-identical `./script.sh` in worktree B; PATH-resolved binaries
  (`which cargo`) live outside the workspace and stay absolute
- For jj: `.jj/repo` is the store directory itself in the main workspace and a pointer file in a secondary workspace.
  Absolute pointers are used as-is; a relative pointer is resolved against the `.jj` directory (jj 0.41+) with a
  fallback to the workspace directory (older jj), so both layouts share one repository root
- For git: uses `--git-common-dir` which returns the shared `.git` directory for all worktrees
- Module: `repository.rs` provides `detect_repository_root()` function
