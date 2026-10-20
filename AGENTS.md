# Agent guide

## Project overview

Moriarty is a Rust CLI for analyzing Claude Code and pi session logs, reporting API costs and token usage, and rendering
terminal graphs. It also provides tool/command permission hooks, read-only git/jj and project-tool MCP servers, and a
project-approval TUI. It requires Linux or macOS.

## Agent guidelines

### Workflow and verification

- Read the relevant references below before changing a subsystem; preserve its security and execution contracts.
- Run tests with **`cargo nextest`, never `cargo test`**. Tests mutate process-global environment variables to isolate
  HOME/XDG configuration; Nextest runs each test in its own process so tests cannot race or clobber real configuration.
- Use the project's configured quality tools in [`.config/tools.toml`](.config/tools.toml). After modifying code, do not
  stop until all quality checks pass. See the [development reference](doc/development.md) for commands and examples.
- Reuse shared test helpers in [`crates/moriarty/src/test_helpers.rs`](crates/moriarty/src/test_helpers.rs); put new
  helpers used by multiple modules there. Route test environment changes through the existing helpers and their single
  `apply_test_env_var()` unsafe boundary. Executable integration tests cannot import this binary-only module: set HOME
  and XDG config/state/cache on each child `Command`, keeping temporary directories alive until it exits.
- Keep OpenSpec proposals, designs, deltas, and tasks in `openspec/changes/<change>/`; main capabilities live in
  `openspec/specs/`, and archives in `openspec/changes/archive/`. Proposal authoring is planning-only: implement only
  after an explicit apply request. Do not mix these artifacts with ad-hoc plans.
- The specs (`openspec/specs/`) are authoritative. You must read them and comply when making changes that are related.
  If you something you're doing violates on of the specs, then the full spec change process is required.
- Archived changes (`openspec/changes/archive/`) are historical evidence only. They may help to explain the current code
  but do not provide authorization to continue following their patterns. They must be treated as historical only and are
  not to be modified. Modifications to them will prompt full deletion of the archived change.

### Rust conventions

- Use `miette::Result` explicitly; do not shadow the standard `Result` or `Error` names. Convert standard errors with
  `.into_diagnostic()` and add context with `.wrap_err()` or `.context()`. Custom errors derive `Debug`,
  `miette::Diagnostic`, and `thiserror::Error`.
- Group imports in this order: standard library, third-party crates, then local/workspace crates. Use compact imports at
  module scope, not inside functions. Prefer imported names over qualified paths except when qualification clarifies the
  API (`mpsc::channel`, `tokio::spawn`) or avoids prelude shadowing (`miette::Result`, `miette::Error`). Generated
  imports that would collide are the exception to module-scope placement.
- Rust doc comments and inline comments explain **why**, invariants, or non-obvious trade-offs—not names, parameters, or
  the body's steps. Delete comments that merely restate code. This restriction does not apply to documentation prose.
- Use `#[serde(deny_unknown_fields)]` for Claude Code protocol messages so upstream additions surface as parse errors.
  Preserve the upstream wire naming; do not force camelCase onto snake_case schemas.
- Preserve the documented `pi_logs` strictness exceptions: flattening limitations require explicit unknown-key and
  co-occurrence validation; corrupt-stream tolerance must name the observed failure in an inline comment;
  forward-compatible runtime/server envelopes require a struct-level rationale with motivating fields. Read the
  [full Serde conventions](doc/development.md#serde-conventions) before changing these boundaries.
- Keep legacy `BashRuleAction` action-table unknown-key compatibility; `AllowRedirect` and `DenyRedirect` payloads stay
  strict so misspelled direction/locality cannot broaden authority.

### Documentation maintenance

- Update the appropriate reference when significant changes introduce architecture, organization, build/test, security,
  configuration, error-handling, or async patterns. Name the topic and apply the actual documentation edit rather than
  merely suggesting prose in the final response.
- Keep this file limited to agent behavior, the brief project overview, and documentation pointers. Put subsystem
  explanations in `doc/`, or link the existing capability spec instead of duplicating its contract here.
- Do not add temporary workarounds or feature implementation details to this guide; use the relevant reference or a
  why-focused source comment. Avoid documentation that will quickly become stale.

## Where to read

| Working on                                                                          | Read first                                                                                                                                            |
| ----------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| Building, running, testing, benchmarking, or coding conventions                     | [Development reference](doc/development.md)                                                                                                           |
| Workspace layout, cost/graph pipelines, MCP/TUI, approvals, or root detection       | [Architecture reference](doc/architecture.md)                                                                                                         |
| Hook evaluation, Bash splitting/redirects, rule diagnostics, or reporting internals | [Hook and rule-engine internals](doc/hooks.md)                                                                                                        |
| Claude Code JSONL schemas or model identities                                       | [Crate overview](crates/claude_logs/src/lib.rs), [log schema](crates/claude_logs/src/parser.rs), and [model parsing](crates/claude_logs/src/model.rs) |
| Pi JSONL schemas, tool results, or extension-owned metadata                         | [Crate overview](crates/pi_logs/src/lib.rs) and [parser schema/compatibility notes](crates/pi_logs/src/parser.rs)                                     |
| Standalone process lifecycle, cancellation, captures, or descendant cleanup         | [Process-supervision specification](openspec/specs/process-supervision/spec.md) and [crate rustdoc](crates/process_supervisor/src/lib.rs)             |
| Moriarty execution ownership, host shutdown, or cancellation integration            | [Moriarty process-execution specification](openspec/specs/moriarty-process-execution/spec.md)                                                         |
| Tool/Bash policy configuration and regression suites                                | [Tool & Bash Rules Configuration Guide](BASH_RULES.md)                                                                                                |
| Installing rules or wiring Claude Code/pi hooks                                     | [Getting Started with rules](doc/getting-started-rules.md)                                                                                            |
| CLI options or project-tools configuration                                          | [CLI manual](doc/man/moriarty.1) and [project-tools manual](doc/man/moriarty-project-tools.5)                                                         |
| Bash-rule profiling evidence and reproduction                                       | [Benchmark report](doc/benchmarks/bash-rules.md)                                                                                                      |

[README.md](README.md) covers user-facing features, security limitations, and platform requirements.
