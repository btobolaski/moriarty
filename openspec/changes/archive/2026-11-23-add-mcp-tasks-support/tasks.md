# Tasks

## 1. Bound connection-local task state

- [x] 1.1 Implement connection-local records with unguessable IDs. At the store, verify registration and instance isolation. Normal eviction may remove even an unpolled terminal record.
- [x] 1.2 Move settled records to a completion-ordered queue with lazy expiry. Using controlled time, verify access at the one-hour boundary and settlement after an idle period both prune expired entries, no expiry of running work, creation-relative wire `ttlMs`, and eviction of the first settled record on the 21st result across mixed outcomes.

## 2. Own execution beyond the creating call

- [x] 2.1 Admit the validated tool body under a host-owned scope; keep approval, configuration, cwd and execution limits on the existing paths. With one representative held command, observe a handle before release, then release the command and verify it executed once and returned its original result.
- [x] 2.2 Remove panic-to-error handling introduced by Tasks, including synthetic failure publication from `Drop`. Verify the ordinary mappings: returned tool result (including `isError`), returned JSON-RPC error, and cooperative interruption, each with confirmed cleanup. Exercise a cancellation flag alongside a returned outcome. Use a supervisor-reported cleanup failure, not an injected panic, to verify `failed` preserves both the initiating and cleanup diagnostics. With cleanup held, verify `tasks/cancel` acknowledges without cancelling unrelated work or publishing a terminal outcome; release cleanup and poll for the settled status. Verify cancelling a poll or the finished creation request does not cancel the task. Keep direct-call settlement unchanged.
- [x] 2.3 Use the existing host shutdown paths for task owners. With the representative owner held, trigger shutdown without another poll; verify admission closes and the host waits for that owner's settlement before returning, including under response backpressure. Document ownership in `doc/architecture.md`.

## 3. Expose the MCP `2026-07-28` Tasks extension and verify compatibility

- [x] 3.1 At the wire, check task selection for all five tools and per-request opt-out, without repeating the lifecycle suite per tool. Verify `2026-07-28` discovery and first-message calls, legacy initialization declaring Tasks after discovery, `-32601` for valid legacy management with and without Tasks declared at initialization, `-32021` before lookup for valid `2026-07-28` management without Tasks, and rmcp's existing revision/metadata error precedence. For task-capable calls assert the router's immediate JSON-RPC error for an unknown tool versus its immediate `isError: true` result for invalid `RunArgs`, neither with a handle or work started. Check known-ID reply shapes and `-32602` for unknown or foreign IDs on get/update/cancel. Keep the VCS contract checks: neither server advertises Tasks, both return direct results, and their task methods return `-32601`.
- [x] 3.2 Update Tasks statements in `README.md`, `doc/architecture.md` (including the spec pointer and execution ownership section) and `doc/man/moriarty.1`. Check the documented discovery, polling, cancellation and retention behavior against stdio tests. Leave `moriarty-project-tools(5)` unchanged: it documents configuration, which does not change.
- [x] 3.3 Run the configured formatter, lint, build and full Nextest suite; verify both the new task lifecycle and the pre-existing direct request, approval, revision and shutdown contracts pass.

## Workflow follow-up

- At spec sync, update the existing `mcp-protocol-support` Purpose to: “Define the explicit MCP revision policy and stdio behavior across supported revisions of Moriarty’s three MCP servers, including task-backed execution on project-tools under MCP `2026-07-28`.” OpenSpec does not apply Purpose deltas to an existing capability.
