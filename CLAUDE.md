# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Moriarty is a Rust CLI tool for analyzing Claude Code logs and API usage. It provides:
- **Logs viewer**: An interactive TUI for viewing and navigating Claude Code conversation logs
- **API pricing analyzer**: Analyzes Claude API usage from log directories and generates detailed cost reports
- **MCP servers**: Provides Model Context Protocol servers for git operations and project tools
- **Hooks system**: Security integration for validating commands before execution (bash rules, project checks)
- **Project approval TUI**: Interactive interface for approving project tools before execution

## Essential Commands

**Building:**
```bash
cargo build
```

**Running:**
```bash
# Run the logs viewer TUI
cargo run -- logs -f <path-to-log-file>

# Run the logs validator (no TUI)
cargo run -- logs -f <path-to-log-file> --validate

# Run API pricing analyzer
cargo run -- api-pricing -d <directory> --timezone local|utc

# Run MCP servers
cargo run -- mcp git-read-only
cargo run -- mcp project-tools
cargo run -- mcp install  # Install both servers to Claude Code

# Run project approval TUI
cargo run -- approve-project <project-dir>

# Execute hooks (for debugging)
cargo run -- hooks exec
```

**Testing:**
```bash
# Run tests (MUST use cargo nextest, NOT cargo test)
cargo nextest run

# Run tests for specific package
cargo nextest run -p moriarty

# Save yourself a great deal of repeated output
cargo nextest run --no-fail-fast --hide-progress-bar --success-output never --status-level fail --final-status-level flaky
```

**⚠️ CRITICAL**: Tests MUST be run using `cargo nextest`, never `cargo test`. Tests use `std::env::set_var` to set up isolated XDG config directories, which is only safe when each test runs in its own process. `cargo nextest` runs each test in a separate process, making this safe and preventing tests from clobbering real config files.

## Architecture

### High-Level Module Organization

**`logs/`** - Log file parsing and TUI viewer:
- Deserializes Claude Code log files (JSON lines format) into typed structs using serde with tagged enums
- The `LogLine` enum represents different message types (User, Assistant, FileHistorySnapshot, Summary, System)
- Provides hierarchical thread view with message selection and rendering
- Modal dialog for viewing detailed message content
- Entry point: `logs` subcommand in `main.rs`

**`api_pricing/`** - API usage cost analysis:
- Two-pass architecture: aggregates token usage into daily/conversation buckets (by timezone), then calculates costs
- Two-level deduplication for streaming responses and forked conversations
- Handles unknown models gracefully by tracking them separately
- Line counter tracks code changes from file history snapshots
- Entry point: `api-pricing` subcommand in `main.rs`

**`tui/`** - Terminal UI infrastructure:
- Event-driven async architecture where `App` owns all state and orchestrates rendering/event handling
- Uses ratatui for rendering, crossterm for terminal control, and tui-scrollview for scrolling
- Async event stream combining keyboard input and other UI events

**`mcp/`** - Model Context Protocol servers:
- Two MCP servers: `git_read_only` (status, diff, log, show) and `tool_runner` (lint, test, build, format)
- Uses rmcp library with stdio transport for Claude Code integration
- Both servers run as stdin/stdout servers that Claude Code can invoke
- `install` command configures both servers in Claude Code's MCP registry

**`hooks/`** - Security hook system for Claude Code integration:
- **PreToolUse hook**: Validates Bash commands using user-configured rules from `~/.config/moriarty/tool_rules.toml`
- **Stop hook**: Runs project checks before allowing execution
- Rule engine supports Allow, Deny, Modify, and Ask decisions
- Structured logging with tracing crate for debugging hook execution
- Security model: Defaults to "Ask" when unconfigured, fail-closed once configured (verification failures block execution)

**`approval_tui/`** - Interactive approval interface:
- Multi-screen TUI flow: ProjectOverview → CommandReview → InProjectWarning → Summary → Approved/Cancelled
- Reviews both commands and checks, showing security details (binary path, hash, writability, in-project status)
- Script contents preview for writable in-project scripts
- Atomic file I/O with locking during final approval save

**`project_config/`** - Project configuration and security:
- Three submodules: `config` (loads `.config/tools.toml`), `approvals` (SHA-256 verification), `runner` (verified execution)
- **Design asymmetry**: Commands are fixed struct (lint/test/build/format) for MCP, Checks are dynamic `Vec<Check>` for user validations
- Tracks config and binary hashes to detect changes; uses file locking for atomic persistence

### Key Design Patterns

**Security Model**:
- **Default to Ask when unconfigured**: If no rules/checks configured, defaults to "Ask" which prompts the user in Claude Code UI
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
- All TUI apps follow same pattern: event loop with async event stream, state machine for screens, ScrollViewState for scrolling
- Event handlers are async to support I/O operations (file reads, approval saves)
- State machines use enums for screens with explicit transitions

**Configuration** (XDG-compliant):
- `~/.config/moriarty/tool_rules.toml` - Bash validation rules
- `<project>/.config/tools.toml` - Project commands and checks
- `~/.config/moriarty/project_approvals.toml` - SHA-256 approval hashes
- `~/.local/state/moriarty/logs/` - Structured logs

## Development Notes

**Workspace Optimization**: The `my-workspace-hack` crate is managed by cargo-hakari to unify dependencies.

**Logging**: Structured logging via tracing to `~/.local/state/moriarty/logs/` (auto-rotated). Sensitive env vars (TOKEN, SECRET, KEY, PASSWORD) are redacted.

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
use super::{analyzer::*, pricing::{ModelType, TokenCosts, TokenCounts}, time_filter::TimeRangeFilter};
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

Import go at the top of the file not in individual code blocks. The only exception to this is something like diesel's generated table functions, they would all collide with each other making the code difficult to understand. Diesel is not currently in use in the code base.

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

**Important**: Always use `#[serde(deny_unknown_fields)]` when deserializing Claude Code protocol messages (hooks, log parsing) to catch when Claude Code updates have added new fields that this codebase doesn't yet handle.

## Suggesting Updates to CLAUDE.md

When you make significant changes to the codebase that introduce new patterns, conventions, or architectural decisions, you MUST suggest updates to this file.

**CRITICAL**: You MUST use the actual Edit TOOL to make the changes. Do NOT just suggest text - actually invoke the Edit tool with the old and new strings. Format your suggestion like this:

```
> I think we should add/update information about [topic]:
>
> Edit(file_path="CLAUDE.md", old_string="...", new_string="...")
```

Then immediately follow it by actually calling the Edit tool with those exact parameters.

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

The goal is to keep CLAUDE.md focused on information that helps understand how to work with the codebase effectively across sessions.

## Finishing

After you have modified code, you are not allowed to stop until all of the quality checks have passed. If you need to ask the user a question, you must use the tool to do so, instead of writing the question and then awaiting the user's next input.
