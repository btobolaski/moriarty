# Proposal

## Why

A project-tools client currently has to hold a `tools/call` request open for an entire build, test, or check run. The July 2026 MCP Tasks extension lets it receive a handle immediately, poll for the same result, or cancel that work without changing how the project command runs.

## What Changes

- Advertise `io.modelcontextprotocol/tasks` in project-tools discovery under MCP `2026-07-28`. Return a task for each eligible call to any of its five tools when the client declares Tasks on that request.
- Support polling, cooperative cancellation and update acknowledgements. Keep execution supervised by the stdio host after the creating request returns.
- Keep at most 20 terminal results per server instance, evicting the oldest settled result first. Results expire one hour after settlement; the next access or settlement removes expired records. Keep active work separate from this limit.
- Keep legacy clients, `2026-07-28` clients without Tasks, git-read-only and jj-read-only on their existing direct-result paths. No new tools, transports, subscriptions or persistent jobs.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `mcp-protocol-support`: replace the blanket Tasks exclusion with capability negotiation, method behavior and result retention limited to project-tools under MCP `2026-07-28`.
- `moriarty-process-execution`: distinguish request cancellation from task cancellation and preserve awaited host settlement for both.

## Impact

Affects the project-tools handler, the MCP service's execution ownership boundary, stdio contract and lifecycle tests, and the Tasks statements in the README, CLI manual and architecture reference (including its capability overview and execution ownership section). Use the existing rmcp 3.6 SDK's wire types for protocol revision `2026-07-28`; no changes to project configuration, approvals or the standalone supervisor contract.
