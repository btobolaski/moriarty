# Proposal

## Why

Moriarty deliberately limits MCP support to `2025-11-25`, even though its process-supervisor integration now provides
request-scoped cancellation and awaited subprocess settlement. Published rmcp releases since the current `3.2.0` fix
modern startup, discovery fallback, and disconnect cancellation, making an upgrade to `3.6.0` a practical basis for
supporting the `2026-07-28` core protocol without losing legacy compatibility.

## What Changes

- Upgrade rmcp and its resolved macro dependency to `3.6.0`, retaining the existing `server`, `schemars`, and
  `transport-io` features.
- Explicitly support `2026-07-28` alongside `2024-11-05`, `2025-03-26`, `2025-06-18`, and `2025-11-25`; future
  dependency upgrades must not silently widen that list.
- Support modern discovery and handshake-free requests with per-request protocol/capability metadata, modern result
  discriminators and cache hints, and protocol-appropriate validation and errors.
- Preserve legacy `initialize` negotiation, tool names and argument schemas, prompt behavior, and legacy wire shapes.
- Route modern tool calls, including a tool call sent as the first request, through the existing request ownership and
  cancellation/drain boundary. Preserve approval checks, VCS restrictions, capture limits, and truthful cleanup
  outcomes.
- Replace the discovery-fallback workaround with upstream lifecycle-neutral discovery once equivalent transport
  regressions pass; retain application-owned execution and backpressure handling.
- Do not advertise or implement the optional Tasks extension. Tool calls remain ordinary request/response operations;
  there are no background task IDs, polling endpoints, or new process deadlines.

## Capabilities

### New Capabilities

- `mcp-protocol-support`: Explicit protocol-version policy and modern/legacy stdio interoperability for all three MCP
  servers, including discovery, metadata validation, versioned wire shapes, and exclusion of Tasks.

### Modified Capabilities

- `moriarty-process-execution`: Replace the requirement that freezes protocol support at `2025-11-25` with the explicit
  modern-and-legacy support policy, while retaining request-local cancellation and awaited owned-work settlement.

## Impact

- Dependency manifests: `crates/moriarty/Cargo.toml` and `Cargo.lock`; update generated
  `crates/my-workspace-hack/Cargo.toml` only if the project's existing cargo-hakari workflow requires it.
- MCP implementation: `crates/moriarty/src/mcp/mod.rs` and the `git_read_only`, `jj_read_only`, and `tool_runner`
  handler contracts. Reuse `execution.rs` and the standalone `process_supervisor` without redesigning either.
- Tests: extend existing MCP transport, handler-contract, execution-seam, and `host_shutdown_cli.rs` coverage; do not
  add a second process-group/signal/reaping test harness.
- Documentation: update the protocol statements in `README.md`, `doc/architecture.md`, and `doc/man/moriarty.1` during
  implementation, including architecture's contract-reference list for the new protocol capability. No changes to the
  interactive MCP installer, hook policy, project approvals, CLI surface, or transports.
- Compatibility is additive: legacy clients retain their existing behavior. HTTP/SSE, Tasks, subscriptions, and new
  server-to-client interactions are outside this change.
