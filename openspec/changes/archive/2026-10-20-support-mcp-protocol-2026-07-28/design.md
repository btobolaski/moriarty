# Design

## Context

See [proposal.md](proposal.md) for motivation and scope. This design is needed because the dependency upgrade changes
shared startup and cancellation behavior across three servers, with compatibility and process-ownership implications.

The pre-change implementation has three useful boundaries:

- `crates/moriarty/src/mcp/mod.rs` owns the shared protocol constants, `DiscoveryFallbackTransport`, `OwnedService`,
  `serve_until`, and `ObservedIo`. Each concrete handler explicitly returns the shared version list and fallback.
- `OwnedService` wraps `tools/call` in `execution::Scope::run(context.ct, ...)`. `Scope` retains process completion
  owners; `serve_until` closes admission and awaits shutdown. `ObservedIo` observes EOF/errors and makes blocked output
  wake on service cancellation. These responsibilities are broader than the SDK's request-token bookkeeping.
- `mcp/tests.rs` drives `serve_until` over a newline-delimited in-memory Tokio duplex with `TestClient`, per-server
  contracts, `PendingTools`, discovery fallback, and transport failure/backpressure. `execution/tests.rs` and
  `execution/test_seams.rs` cover ownership policy; `tests/host_shutdown_cli.rs` covers process-level stdio and host
  signal shutdown. Only git-read-only advertises prompts; jj-read-only and project-tools advertise tools only.

The existing `moriarty-process-execution` specification freezes MCP support at `2025-11-25`. Its delta in this change
explicitly replaces that freeze; the standalone `process-supervision` contract is unchanged.

### Upstream evidence

The upgrade target is published rmcp `3.6.0`, not an unreleased branch. Relevant shipped fixes are:

| Version | Evidence                                                                                                        | Relevance                                                                                                          |
| ------- | --------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| 3.4.0   | [#1263](https://github.com/modelcontextprotocol/rust-sdk/pull/1263)                                             | Dispatch the first ordinary handshake-free request from the service loop, including request cancellation tracking. |
| 3.5.1   | [#1248](https://github.com/modelcontextprotocol/rust-sdk/pull/1248)                                             | Keep discovery bootstrap-neutral and allow recovery after invalid/unsupported bootstrap requests.                  |
| 3.5.1   | [#1308](https://github.com/modelcontextprotocol/rust-sdk/pull/1308)                                             | Supply missing cache hints for modern cacheable results.                                                           |
| 3.6.0   | [release changelog](https://github.com/modelcontextprotocol/rust-sdk/blob/rmcp-v3.6.0/crates/rmcp/CHANGELOG.md) | Cancel pending request tokens after transport EOF (#1333) and bound the incoming transport line buffer (#1049).    |

[The 2026-07-28 changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog) separates the modern core
lifecycle from the optional `io.modelcontextprotocol/tasks` extension. Upstream `TaskManager` still aborts operation
futures on TTL expiry/shutdown; it is neither needed nor enabled here.

## Goals / Non-Goals

**Goals:**

- Use the SDK's repaired lifecycle machinery instead of introducing another protocol implementation.
- Keep protocol selection separate from legacy initialization fallback, and retain application-owned settlement.
- Prove modern interoperability at Moriarty's real transport boundary, not only by changing version-list assertions.

**Non-Goals:**

- No task manager, durable task state, polling, background tool execution, or task-related configuration.
- No new interaction patterns, transports, caching implementation, positive cache TTL policy, or timeout knobs.
- No supervisor redesign or duplicate OS-level cleanup tests.

## Decisions

### 1. Upgrade before widening the explicit supported set

First update `crates/moriarty/Cargo.toml` and `Cargo.lock` to resolve rmcp/rmcp-macros `3.6.0` with the existing feature
selection. Keep the four-revision policy and existing compatibility tests intact for this verification checkpoint.
Resolve generated workspace-hack changes through the existing cargo-hakari tooling only if required by the dependency
update; do not introduce new build wiring or dependencies.

Then add `2026-07-28` to the shared explicit list. Keep the legacy fallback explicitly `2025-11-25`; rename the current
`MCP_PROTOCOL_VERSION` constant if needed to make that role unambiguous. rmcp `3.5.0` changed `ProtocolVersion::LATEST`
and its default to `2026-07-28`, so neither is an acceptable substitute for the explicit fallback or supported list. A
modern revision passed to legacy `initialize` still negotiates the newest legacy revision, not the modern lifecycle.

Alternative: update the dependency and replace the fallback with `LATEST` in one step. Rejected because it conflates two
different lifecycles, weakens the explicit policy, and makes regressions harder to isolate. Replacing the existing
whitelist with a new abstraction is unnecessary.

### 2. Remove only the discovery workaround superseded upstream

After the version-preserving upgrade, use upstream lifecycle-neutral discovery and remove `DiscoveryFallbackTransport`
and its wrapper-specific tests once the existing end-to-end fallback regressions pass without it. Retain those
end-to-end assertions for each server. Add accepted modern discovery followed by legacy initialization and rejected
unsupported discovery followed by either valid lifecycle.

Modern requests can start directly with `tools/list` or `tools/call`; neither discovery nor initialization is mandatory.
The first ordinary request must enter the same service-loop cancellation path as later requests. Invalid bootstrap
metadata must produce a recoverable protocol error rather than close or commit the connection prematurely.

Alternative: retain the wrapper indefinitely or implement a custom startup loop. Rejected because upstream owns
lifecycle selection and recovery; Moriarty should not duplicate that router. The initial complete-metadata regressions
passed, but follow-up tests exposed an unsupported discovery revision with missing capabilities receiving `-32602`
instead of the prior `-32022` with revision data. The owner-requested repair extends the narrow transport guard to
restore that precedence; successful discovery, other metadata validation, and recovery remain upstream-owned.

### 3. Delegate versioned wire behavior to rmcp

Retain the existing handler macros and `ServerHandler` defaults. Modern requests supply
`io.modelcontextprotocol/protocolVersion` and `io.modelcontextprotocol/clientCapabilities` under `_meta`; client info is
optional. Validate every request with the SDK rather than carrying capability context from an earlier request.

Use the SDK's ordinary complete-result discriminator and cache hints. Cache hints are required on `server/discover` and
`tools/list` for all three servers, plus `prompts/list` for git-read-only: `ttlMs >= 0` and `cacheScope` of `public` or
`private` are result-level fields. The macro/default values are sufficient; no positive TTL or custom cache is needed.
`tools/call` and Git's `prompts/get` do not require cache hints. Legacy tool/prompt responses must not acquire the
modern fields. Neither tools-only server gains a prompt capability.

Keep protocol errors in the existing JSON-RPC envelopes. Tests must distinguish malformed/missing required metadata
(`-32602`), unsupported revision (`-32022` plus requested/supported data), and methods unavailable in the modern core
(such as `ping`, `-32601`). Check these at the transport boundary before any tool work is admitted. The current tools do
not expose resource operations or server-to-client interactions, so no new resource/MRTR handlers are required.

Alternative: manually stamp metadata/result fields or hand-maintain parallel modern routers. Rejected because the SDK
already implements these differences; only a demonstrated integration gap warrants a local adaptation.

**Demonstrated exception:** rmcp `3.6.0` answers bootstrap ping before validating metadata. A regression reproduced
modern ping returning success, so the owner approved `ProtocolGuardTransport` at the shared transport boundary. It
rejects unsupported ping revisions with `-32022` before metadata-completeness validation, missing/malformed modern
context with `-32602`, and complete modern ping with `-32601`. SDK `RequestMetaObject::DRAFT_REQUIRED_KEYS` and
`missing_required_keys` supply validation; the guard mirrors private `clientInfo`/`logLevel` key names and the private
metadata-error wording, with source comments identifying those mirrors. Metadata-free and supported legacy-version ping
pass through, including supported legacy-version ping with context keys and empty/extension-only context-free ping. The
same guard rejects a parseable unsupported discovery revision with `-32022` before checking other metadata, even when
required capabilities are missing or malformed. Other discovery validation, lifecycle recovery, and ordinary dispatch
remain SDK-owned. The guard never selects a lifecycle.

### 4. Preserve request ownership and backpressure protection

Keep `OwnedService`, `Scope`, `serve_until`, and `ObservedIo`. Both lifecycles route ordinary `tools/call` through the
existing `context.ct` bridge; no new task scope or cancellation registry is introduced. A client can still cancel its
own modern tool request using `notifications/cancelled` with `params.requestId`.

Upstream EOF cancellation signals intent; it does not prove subprocess settlement, close Moriarty's admission gate,
retain cleanup failures, or interrupt Moriarty's blocked output writes. Therefore the upgrade is not permission to
remove the application boundary. Preserve current authorization, subprocess capture, deadline, and cleanup policies.

The SDK suppresses responses for cancelled requests after removing their request IDs. Tests must assert cancellation,
isolation, and awaited ownership directly, not wait for a cancellation response that need not be delivered. Cleanup
uncertainty must remain retained by the owning host even when request delivery disappears.

A rejection cannot await a blocked writer inside `receive()`: that prevents EOF detection, and rmcp may cancel the
receive future when a different service-loop event wins. Poll each rejection send once to preserve immediate failure
handling; transfer a still-pinned pending send synchronously to one transport sender before another await. Its bounded
channel holds at most 64 queued sends plus one active send. There is no task per ping and no detached send. Queue
exhaustion or send failure closes admission, cancels the service, and retains a transport error.

The sender is tracked with `TaskTracker`. Transport close closes admission, cancels blocked `ObservedIo` output, closes
the queue, and awaits the sender before closing the SDK transport. `serve_until` also owns and awaits its join handle,
including on cancelled/failed startup, preserving task failures before awaiting process settlement. Writable replies may
drain; blocked delivery on shutdown fails truthfully. On a live connection, cancelling receive does not cancel accepted
rejections, which complete exactly once when output resumes without another receive driving them.

Alternative: trust SDK request cancellation or dropped handler futures as proof of process termination. Rejected because
it violates the retained-owner and truthful-settlement contracts.

### 5. Keep Tasks disabled even for task-capable clients

Do not enable the Tasks extension or return task-backed results. Verify discovery capabilities omit it and valid
`tasks/get`, `tasks/update`, and `tasks/cancel` requests remain method-not-found, even when the client advertises Tasks.
This avoids both unnecessary task state and upstream abort-versus-settlement concerns.

Alternative: implement Tasks as part of upgrading the core revision. Rejected by the confirmed scope; Tasks would need a
separate lifetime, retention, cancellation, and settlement design.

### 6. Extend the existing verification seams

Shared private fixtures remain in `crates/moriarty/src/mcp/tests.rs`; its `tests/` child modules separate transport unit
tests (`transport.rs`), protocol lifecycle policy (`lifecycle.rs`), cancellation and settlement (`settlement.rs`), and
server tool/prompt contracts (`contracts.rs`). The owner superseded the earlier module-split deferral without reducing
coverage. Handler metadata and wire assertions share one test-only `EXPECTED_PROTOCOL_VERSIONS` constant, independent of
the production list so accidental policy changes still fail tests.

Use the following acceptance matrix rather than duplicating every test for every protocol/version combination:

| Area                        | Existing seam                                           | Required coverage                                                                                                                                                                                                                             |
| --------------------------- | ------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Version policy and fallback | Per-handler tests and `tests/lifecycle.rs`              | Exact shared list on all servers; all four legacy revisions; modern/unknown initialization falls back to `2025-11-25`; use a still-unsupported revision such as `2099-01-01` for rejection tests.                                             |
| Startup and discovery       | `TestClient` with real `serve_until`                    | Accepted/rejected discovery followed by legacy or modern work; first ordinary modern tool call without discovery; recoverable malformed bootstrap; parseable unsupported discovery keeps `-32022` revision data ahead of incomplete metadata. |
| Validation                  | Existing protocol-meta helpers and execution seams      | Missing/malformed fields on first and later requests; optional client info omitted; unsupported revision; rejected calls admit no tool work.                                                                                                  |
| Tool/prompt compatibility   | Existing per-server contract and prompt tests           | Names, schemas, omitted `project_dir`, prompt roles/content; modern discriminator and cache hints versus legacy field absence.                                                                                                                |
| Cancellation and settlement | `PendingTools` plus existing execution ownership seams  | First/later modern call cancellation; unrelated request survives; disconnect cancels and awaits admitted operations; retained cleanup failure.                                                                                                |
| Guarded rejection ownership | `GatedWrite`, transport unit tests, and retained owners | Blocked rejection cannot hide EOF/settlement; consumed rejection survives receive cancellation exactly once; immediate/queued failures and queue exhaustion fail closed; sender is awaited on shutdown and failed/cancelled startup.          |
| Host shutdown               | Existing `host_shutdown_cli.rs`                         | Modern startup with input held open and modern blocked response output under graceful signals, alongside existing legacy cases.                                                                                                               |
| Excluded features           | Transport-level requests/discovery                      | No Tasks capability or task-backed result; valid task methods rejected; modern ping rejected and legacy ping preserved.                                                                                                                       |

Use rendezvous notifications and existing retained-owner seams to prove ordering. No sleeps as cleanup proof, no new
subprocess descendant trees, and no independent PGID/reaping assertions in Moriarty tests. Build fixtures from existing
helpers and run tests with Nextest for process-isolated environment handling.

## Risks / Trade-offs

- **New SDK defaults widen support accidentally**: retain explicit fallback/list policy and exact-list regressions.
- **An upstream fix is narrower than its release note**: exercise real first-request cancellation, discovery recovery,
  and EOF settlement against the resolved dependency before relying on it.
- **Modern validation runs after admission or reuses stale context**: assert malformed/unsupported requests admit no
  tool work and validate later requests independently.
- **SDK cancellation hides a cleanup error by suppressing a response**: retain host-owned failures and test settlement
  independently of response delivery.
- **Cache policy grows beyond compatibility needs**: accept valid SDK hints, including zero TTL; defer cache tuning.
- **SDK bootstrap/private metadata drift**: exercise every ping-guard branch on upgrades and keep the documented
  key/error mirrors aligned; remove the guard once upstream passes these regressions without it.
- **Guarded rejection backpressure or receive cancellation**: keep input independent of queued writes, bound the sender
  queue, fail closed on exhaustion, and await its owner on every startup/shutdown path. Use deterministic writer and
  process-owner gates to test EOF settlement and exactly-once delivery after a dropped receive.
- **Two lifecycles increase test surface**: share helpers and contract assertions, retaining per-server coverage only
  where each server advertises its own tools/prompts or policy.

## Migration Plan

1. Upgrade and verify with the existing protocol cap. No approval/configuration migration or installer change is needed.
2. Replace the redundant discovery wrapper after its transport regressions pass against upstream.
3. Enable the explicit modern revision and complete the modern/legacy acceptance matrix. Update `README.md`,
   `doc/architecture.md`, and `doc/man/moriarty.1` in the same implementation changeset so they distinguish modern
   request metadata from legacy initialization and explicitly exclude Tasks.
4. Run configured formatting, lint/build, Nextest, and checks; inspect the complete diff and run the mandated
   independent review before declaring implementation complete.

Rollback can remove `2026-07-28` from the explicit list while retaining the upgraded SDK and verified legacy behavior.
There is no persisted task state or configuration format to migrate. A dependency rollback must restore its manifest,
lockfile, and generated workspace-hack metadata together, remove `ProtocolGuardTransport` and its SDK
metadata-validation references, restore the handlers' SDK-compatible `ServerInfo` return types in place of
`ServerConfig`, and re-establish the legacy discovery fallback behavior. Do not pair the older SDK with newer handler
types or a removed discovery workaround.
