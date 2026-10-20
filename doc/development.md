# Development reference

See [AGENTS.md](../AGENTS.md) for agent behavior guidelines and the [architecture reference](architecture.md) for
subsystem contracts. Commands below run from the repository root; the complete CLI reference is
[moriarty(1)](man/moriarty.1).

## Building

```bash
cargo build
```

## Running

The examples select local time; use `--timezone utc` instead for UTC reports. Replace angle-bracket placeholders with
actual paths before running the commands.

```bash
# Run Claude API pricing analyzer (--dir defaults to ~/.claude/projects)
cargo run -- api-pricing --timezone local
cargo run -- api-pricing --tokens
cargo run -- api-pricing --dir <directory>
cargo run -- api-pricing --last-days 7

# Run pi cost analyzer (--dir defaults to ~/.pi/agent/sessions)
cargo run -- pi cost --timezone local
cargo run -- pi cost --conversations
cargo run -- pi cost --tokens
cargo run -- pi cost --dir <pi-sessions-directory>
cargo run -- pi cost --last-days 3

# Render chart-focused usage graphs
# (graphs all uses both defaults; graphs claude --dir defaults to ~/.claude/projects;
# graphs pi --dir defaults to ~/.pi/agent/sessions)
cargo run -- graphs all --last-days 7
cargo run -- graphs all --claude-dir <claude-projects-directory> --pi-dir <pi-sessions-directory>
cargo run -- graphs claude --timezone local
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

# Evaluate a caller-owned rule regression suite against an explicit candidate policy
cargo run -- test rules <suite.toml> --config <tool_rules.toml> --json
```

## Testing

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

## Formatting

The configured project formatter runs `cargo fmt` for Rust. Markdown formatting is defined in
[`treefmt.toml`](../treefmt.toml): treefmt invokes Prettier with prose wrapping and a 120-column print width. The
command below is limited to `AGENTS.md`, `README.md`, and top-level `doc/` Markdown files:

```bash
treefmt --formatters md AGENTS.md README.md doc/*.md
```

## Benchmarking

`cargo bench -p moriarty --bench bash_rules` runs self-contained Criterion benchmarks against a committed sanitized
policy fixture. Cases call Rust functions directly; they do not spawn the CLI or read user configuration. The harness
reuses the binary's private modules, with test-only entry-point dispatch so production behavior stays unchanged. Use
Nextest for tests, and `cargo bench -p moriarty --bench bash_rules -- --test` for a one-iteration benchmark smoke check.
Linux Nix deployment uses static musl with mimalloc: add `--target x86_64-unknown-linux-musl --features mimalloc` when
measuring that deployment, and keep target/allocator baselines separate. The flake enables `moriarty/mimalloc` for the
package, dependency artifacts, and Linux checks; it sets target-qualified `CC_<target_with_underscores>` to the musl C
compiler so mimalloc never picks up host glibc headers while native build scripts retain their host compiler. Ad-hoc
Cargo musl builds need that C compiler configured too. Native Cargo and Darwin builds retain the system allocator unless
explicitly built with the optional feature; the global allocator declaration in `main.rs` is shared with the included
benchmark. See [the profiling report](benchmarks/bash-rules.md) for timing boundaries, baseline data, and reproduction
commands.

## Development support

**Workspace Optimization**: The `my-workspace-hack` crate is managed by cargo-hakari to unify dependencies.

**Shared Test Utilities**: Test helpers used across multiple modules (`setup_isolated_xdg_config`,
`setup_isolated_xdg_state`, `setup_project_dir_with_config`, `write_tools_config`, `create_executable_script`,
`run_git_command`, `setup_git_repo_with_commit`, `setup_jj_main_and_secondary_workspace`,
`assert_workspace_local_copy_ran`, `set_test_env_var`, `remove_test_env_var`, `TestEnvVarGuard`, `test_report`,
`redirect_rule`, `directional_redirect_rule`, `deny_redirect_rule`, `SUBAGENT_EXECUTION_RULES`) live in
`crates/moriarty/src/test_helpers.rs`. This module is compiled only in test builds (`#[cfg(test)]`). All test
environment mutations go through `apply_test_env_var()` — the module's single `unsafe` block — with process isolation
guaranteed by `cargo nextest`. New test-only helpers needed in more than one module belong here rather than being
duplicated. Executable integration tests cannot import this binary-only module; each child `Command` must receive its
own HOME and XDG config/state/cache values, with temporary directories kept alive until the child exits.

**Logging**: Hook execution is logged via tracing as JSON lines to `~/.local/state/moriarty/hooks/` (daily-rotated); the
`hooks report` command consumes these. Cost-report commands log to stderr instead. Sensitive env vars (TOKEN, SECRET,
KEY, PASSWORD) are redacted.

## Coding conventions and examples

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

Applies to source files only. AGENTS.md and other docs use ordinary prose.

### Error Handling

This project uses the `miette` crate for rich error reporting throughout:

```rust
use miette::{IntoDiagnostic, WrapErr};

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
    HashSet::new()
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

Imports go at the top of the module, not in individual code blocks. The only exception to this is something like
diesel's generated table functions, they would all collide with each other making the code difficult to understand.
Diesel is not currently in use in the code base.

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

Do not force `rename_all = "camelCase"` onto parser structs whose upstream wire schema is not camelCase. Preserve the
on-disk protocol exactly, even when that means snake_case fields like `GitReadOnlyArgs.project_dir`.

**Exceptions**: in `pi_logs`, three categories of struct legitimately omit `deny_unknown_fields`:

1. **`serde(flatten)` limitations**: `deny_unknown_fields` cannot be combined with a flattened internally-tagged enum
   because the inner tag appears at the outer level and serde does not register it as claimed; a strict outer struct
   then rejects it at runtime. `WebSearchResultsData` keeps that wire shape but restores strict outer-key validation
   with a manual deserializer. The same limitation applies to `McpSearchDetails`' flattened `McpPagination`: its custom
   deserializer rejects every flattened key except `hasMore` and `nextOffset`. The directory diagnostic envelopes
   similarly flatten `DirectoryOutcomes`, which rejects unknown keys and requires `outcomes`/`outcomeCounts` together
   while retaining the legacy state with neither field. _Adjacently_ tagged flatten targets (those with both `tag` and
   `content`) do not hit this collision, so structs like `CustomLine` and `CustomMessageLine` keep derived
   `deny_unknown_fields` handling. Each exception must carry an inline comment naming the limitation.
2. **Corrupt-stream tolerance**: tool-argument structs (e.g. `EditArgs`, `EditReplacement`, `GrepArgs`, and now
   `TodoArgs`) deliberately omit it to tolerate completed-but-corrupted or hallucinated assistant streams that emit
   malformed sibling keys. The same goal is also met at finer granularity by field-level aliases (for example
   `FindArgs.limit` accepting malformed `.limit` while keeping the rest of the struct strict) and untagged fallback
   enums (`EditEntry::Fragment` absorbs raw JSON tokens in an `edits` array; `MaybeU32::Garbage` absorbs string-typed
   corruption of numeric tool-call arguments). Each such exception must carry an inline comment naming the observed
   failure mode.
3. **Forward-compatible protocol schemas**: structs representing server-defined or runtime-defined protocol envelopes
   whose field sets evolve independently of the parser (e.g. `McpCallResult` for MCP tool-call results, which pi's
   runtime regularly extends with new metadata fields like `contentBlocks`, `outputGuard`, and `omitted`). Every such
   exception must carry a struct-level doc comment explaining why strict rejection is omitted and citing example fields
   that motivated the relaxation.
