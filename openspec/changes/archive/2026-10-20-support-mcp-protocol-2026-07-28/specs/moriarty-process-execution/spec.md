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

#### Scenario: First modern tool request is cancellable

- **WHEN** a valid `2026-07-28` tool call is the first request on a stdio connection and the client sends
  `notifications/cancelled` with that request's `requestId` while it is running
- **THEN** the operation stops admitting work and its owned operations settle without requiring a discovery request or
  initialization handshake; cancellation does not affect an unrelated request

#### Scenario: Modern disconnect and backpressure retain settlement

- **WHEN** a modern client disconnects during its first tool call, or graceful shutdown occurs while response delivery
  is blocked
- **THEN** the server cancels and awaits owned work before host return without requiring more client input or resumed
  response reads, and retains transport failure or cleanup uncertainty rather than claiming successful delivery

### Requirement: Cancellation does not change MCP protocol support

Moriarty SHALL explicitly support `2024-11-05`, `2025-03-26`, `2025-06-18`, `2025-11-25`, and `2026-07-28`, as defined
by `mcp-protocol-support`. Cancellation integration and dependency defaults SHALL NOT expand this set or introduce
task-management tools or capabilities. Request cancellation SHALL use the supported lifecycle's protocol facilities;
modern per-request metadata SHALL be accepted only for supported revisions.

#### Scenario: Existing request cancellation

- **WHEN** a client sends `notifications/cancelled` for an in-flight request under a supported revision
- **THEN** Moriarty connects that request cancellation to execution without changing the declared supported revisions or
  exposing task-management APIs

#### Scenario: Unsupported protocol remains unsupported

- **WHEN** a client requests an unsupported MCP revision through initialization or per-request metadata
- **THEN** legacy initialization falls back to `2025-11-25`, while unsupported per-request metadata is rejected with
  `-32022`; neither path implicitly enables the unsupported revision

#### Scenario: Modern protocol support does not enable Tasks

- **WHEN** a client uses supported `2026-07-28` request metadata
- **THEN** Moriarty accepts the modern core lifecycle while keeping tool work request-owned and task-management
  capabilities unavailable
