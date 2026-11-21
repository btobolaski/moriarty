# Spec Delta

## MODIFIED Requirements

### Requirement: Existing MCP cancellation and graceful shutdown settle owned work

For every supported MCP lifecycle, Moriarty SHALL connect request cancellation to the affected operation only. Client
disconnect and graceful SIGINT/SIGTERM shutdown SHALL stop admission of new migrated work, cancel all affected active
operations before awaiting any one operation, and await their terminal outcomes concurrently while the async runtime
remains available. An already cancelled operation SHALL NOT launch additional work.

#### Scenario: Cancel one request without stopping another

- **WHEN** an MCP client cancels one in-flight tool request while another request is running
- **THEN** the cancelled request stops admitting work and initiates supervised cancellation, while the other request
  remains independently owned and is not cancelled by that request

#### Scenario: Cancellation is visible at admission and startup

- **WHEN** a request is cancelled while its work future is being polled and that work attempts another probe or check's
  admission or startup before yielding
- **THEN** the additional work is rejected at its owning admission/startup boundary, while already-started operations
  retain the supervision contract's cancellation and uncertainty semantics

#### Scenario: Disconnect or graceful signal

- **WHEN** the MCP transport disconnects or Moriarty receives SIGINT/SIGTERM with active migrated operations
- **THEN** Moriarty stops admitting work, requests cancellation for all affected operations, and waits for their reports
  or errors before normal runtime destruction

#### Scenario: First `2026-07-28` tool request is cancellable

- **WHEN** a valid direct (non-task) `2026-07-28` tool call is the first request on a stdio connection and the client sends
  `notifications/cancelled` with that request's `requestId` while it is running
- **THEN** the operation stops admitting work and its owned operations settle without requiring a discovery request or
  initialization handshake; cancellation does not affect an unrelated request

#### Scenario: `2026-07-28` disconnect and backpressure retain settlement

- **WHEN** a `2026-07-28` client disconnects during its first tool call, or graceful shutdown occurs while response delivery
  is blocked
- **THEN** the server cancels and awaits owned work before host return without requiring more client input or resumed
  response reads, and retains transport failure or cleanup uncertainty rather than claiming successful delivery

### Requirement: Cancellation does not change MCP protocol support

Moriarty SHALL explicitly support `2024-11-05`, `2025-03-26`, `2025-06-18`, `2025-11-25`, and `2026-07-28` as defined by `mcp-protocol-support`. Cancellation integration and dependency defaults SHALL NOT expand this set. Request cancellation SHALL use the supported lifecycle's protocol facilities. Direct calls SHALL remain request-owned; once project-tools creates a negotiated task, its execution SHALL instead be owned by the host and cancellable by task ID. Per-request metadata defined by MCP `2026-07-28` SHALL be accepted only for supported revisions.

#### Scenario: Existing request cancellation

- **WHEN** a client sends `notifications/cancelled` for an in-flight direct request under a supported revision
- **THEN** Moriarty connects that cancellation to the affected request's execution without changing the supported revisions

#### Scenario: Unsupported protocol remains unsupported

- **WHEN** a client requests an unsupported MCP revision through initialization or per-request metadata
- **THEN** legacy initialization falls back to `2025-11-25`, while unsupported per-request metadata is rejected with `-32022`; neither path implicitly enables the unsupported revision

#### Scenario: `2026-07-28` protocol support does not enable Tasks

- **WHEN** a client declares revision `2026-07-28` but does not declare the Tasks extension on a project-tools call
- **THEN** revision selection alone leaves the call request-owned and does not create a task

#### Scenario: Task survives the creating response

- **WHEN** project-tools has returned a handle and the client cancels the original request ID or a poll
- **THEN** that does not cancel the task; its owned execution continues subject to task cancellation and host shutdown

#### Scenario: Cancel one active task

- **WHEN** the client calls `tasks/cancel` for one task while another operation is active
- **THEN** Moriarty acknowledges the request without waiting for cleanup, stops admission for the targeted work and requests its cancellation without cancelling unrelated work; the acknowledgement does not determine the terminal status

#### Scenario: Host shuts down with tasks in flight

- **WHEN** stdin closes, transport fails or SIGINT/SIGTERM arrives while tasks and direct calls coexist
- **THEN** Moriarty closes admission, signals all active owners before awaiting any one, and drains them before host return even under output backpressure; it retains settlement failures without requiring further polling

## ADDED Requirements

### Requirement: Publish the task owner's observed outcome

Project-tools SHALL classify a task at its execution boundary, not from a cancellation flag or an inferred cause inside a shared command, check or hook. Only confirmed cleanup permits `completed` or `cancelled`. A panic is not a task outcome and guarantees neither a terminal task record nor a response. Direct-call settlement SHALL remain unchanged.

#### Scenario: Cleanup confirmed

- **WHEN** the task owner settles with confirmed cleanup
- **THEN** a returned tool result publishes `completed` with that result (including `isError: true`); a returned JSON-RPC error publishes `failed` with that error; an owner interruption of the work publishes `cancelled` with neither result nor error
- **AND** a cancellation request alone does not relabel a returned result or independent failure

#### Scenario: Supervision reports cleanup failure or uncertainty

- **WHEN** supervision reports that cleanup failed or cannot be confirmed
- **THEN** the task publishes `failed` with the reported cleanup diagnostic and preserves any initiating diagnostic
