# Spec Delta

## MODIFIED Requirements

### Requirement: Result envelopes follow the request revision

`2026-07-28` complete results SHALL carry `resultType: "complete"`. `2026-07-28` complete `server/discover`, `tools/list`, and, for
servers advertising prompts, `prompts/list` results SHALL also carry nonnegative `ttlMs` and a valid `cacheScope`.
Legacy tool and prompt results SHALL retain their existing wire shapes without `resultType` or newly introduced
cache-hint fields. Cache hints SHALL NOT be required on `tools/call` or `prompts/get` results.

#### Scenario: `2026-07-28` discovery and lists include cache hints

- **WHEN** a `2026-07-28` client discovers a server, lists its tools, or lists its supported prompts
- **THEN** each complete result contains `resultType`, nonnegative `ttlMs`, and `cacheScope` equal to `public` or
  `private`

#### Scenario: `2026-07-28` tool and prompt results remain ordinary results

- **WHEN** a `2026-07-28` direct (non-task) tool call or supported prompt lookup completes
- **THEN** its result uses `resultType: "complete"`, retains its tool content or prompt messages, and is not a task
  result

#### Scenario: Legacy results do not gain `2026-07-28` envelope fields

- **WHEN** a legacy client lists tools, calls a tool, lists supported prompts, or gets a supported prompt
- **THEN** its result retains the existing content, names, and roles without `2026-07-28` discriminator or cache-hint fields

### Requirement: Existing tool and prompt contracts remain stable

Both lifecycles SHALL preserve each server's tool names, argument schemas, optional `project_dir` default, and tool
success/error content. Only git-read-only SHALL advertise prompts; its prompt names and message roles SHALL remain
stable. Jj-read-only and project-tools SHALL remain tools-only: the Tasks extension adds no prompts or other primitives.
Approval and read-only restrictions SHALL remain unchanged. HTTP/SSE, subscriptions, and new server-to-client interactions SHALL NOT be introduced.

#### Scenario: Equivalent tool surfaces

- **WHEN** clients using MCP `2026-07-28` and clients using the initialization handshake list the same server's tools
- **THEN** names, argument properties, and required arguments match the existing tool contract, including an optional
  `project_dir` that resolves to the server's current working directory when omitted

#### Scenario: Existing Git prompt behavior survives both lifecycles

- **WHEN** clients using MCP `2026-07-28` and clients using the initialization handshake list and get a supported git-read-only prompt
- **THEN** prompt names and message roles remain compatible, with only revision-required envelope differences

#### Scenario: Tools-only servers do not acquire prompts

- **WHEN** clients using MCP `2026-07-28` and clients using the initialization handshake discover jj-read-only or project-tools
- **THEN** each server advertises tools but no prompt capability

## REMOVED Requirements

### Requirement: Tasks remain unsupported

**Reason**: Project-tools now offers task-backed execution under MCP `2026-07-28`.
**Migration**: Clients that want direct results omit the extension from their per-request capabilities. Other servers and legacy clients need no changes.

## ADDED Requirements

### Requirement: Project-tools task negotiation

Only project-tools SHALL advertise `capabilities.extensions["io.modelcontextprotocol/tasks"]: {}` in `2026-07-28` discovery. On every valid, admitted `2026-07-28` call to one of its five tools declaring this extension in per-request client capabilities, it SHALL return a task handle without waiting for command completion. Other calls SHALL retain direct behavior. The old request-level `task` field SHALL NOT select task execution.

#### Scenario: First call and subsequent opt-out

- **WHEN** a capable client first calls project-tools without discovery, then calls the same tool without declaring Tasks
- **THEN** the first response has `resultType: "task"` and a top-level `taskId` registered before the creation response is returned; once the task settles, expiry or capacity eviction may remove its record before the first poll. The second call returns its ordinary result without creating a task.

#### Scenario: Invalid protocol context retains existing error precedence

- **WHEN** a task-capable call or task management request has an unsupported revision or invalid required metadata
- **THEN** the server preserves the existing revision and metadata error precedence; task negotiation neither admits work nor substitutes a capability or lookup error

#### Scenario: Tool dispatch precedes admission

- **WHEN** a caller declares Tasks but requests an unknown project tool or passes arguments that fail `RunArgs` deserialization
- **THEN** an unknown name returns the router's existing immediate JSON-RPC error; invalid arguments to a known tool return the router's existing immediate `isError: true` tool result. Neither creates a task, returns a handle or starts work.
- **AND** for an admitted call, approval, configuration checks and execution still run through the existing tool path as task work

#### Scenario: `2026-07-28` discovery does not enable Tasks for legacy work

- **WHEN** a client discovers project-tools with `2026-07-28` metadata and then initializes with `2025-11-25` declaring Tasks in its initialization capabilities on the same connection
- **THEN** `2026-07-28` discovery advertises Tasks, but the subsequent initialization response and any legacy capability report omit it; legacy tool calls return direct results and valid legacy task management requests return `-32601` whether initialization declared Tasks or omitted it

#### Scenario: VCS servers do not support Tasks

- **WHEN** a declaring client discovers either VCS server, calls a tool or invokes a task management method
- **THEN** discovery omits Tasks, the tool returns its direct result, and task management returns `-32601`

#### Scenario: Management requires current capability

- **WHEN** a valid `2026-07-28` project-tools request invokes a task method without Tasks in its client capabilities
- **THEN** it receives `-32021` with the required extension in `data.requiredCapabilities`, without reading or changing task state

### Requirement: Task management by handle

Project-tools SHALL expose `tasks/get`, `tasks/update` and `tasks/cancel` only for the creating stdio connection using unguessable IDs. Management responses SHALL use the July 2026 extension shapes. Unknown, foreign, expired or evicted IDs SHALL produce invalid params (`-32602`) for all three methods. Updates SHALL NOT change command arguments, permissions or outcomes. No task enumeration, blocking result method or server-initiated input SHALL be added.

#### Scenario: Poll to settlement

- **WHEN** a declaring client polls a retained task before and after settlement
- **THEN** `tasks/get` returns `resultType: "complete"` with top-level task fields. While work or cleanup is pending the status is `working`. With confirmed cleanup, a returned tool result is `completed` with the original result (including `isError: true`), a returned JSON-RPC error is `failed` with that error, and an owner interruption is `cancelled` without result or error.
- **AND** if supervision reports that cleanup failed or cannot be confirmed, the polled status is `failed` and the error preserves the cleanup diagnostic and any initiating diagnostic. A panic is not a wire outcome; it does not guarantee a terminal record or response.

#### Scenario: Acknowledgements do not manufacture a terminal state

- **WHEN** a client supplies unsolicited input responses for a known task or cancels it
- **THEN** the server ignores unmatched inputs or signals cooperative cancellation respectively, returning an empty complete acknowledgement; polling observes the actual outcome, including `cancelled` without a result or error when cancellation settles cleanly

### Requirement: Bounded terminal result storage

Each live project-tools instance SHALL expire terminal records one hour after settlement, prune expired records on the next task-state access or settlement, and retain at most the 20 most recently settled. Both bounds SHALL apply to completed, failed and cancelled tasks together. Polls and acknowledgements SHALL NOT refresh retention. Active tasks SHALL NOT count against the limit, expire by result age or acquire a new execution deadline.

#### Scenario: Access or settlement after idle expiry

- **WHEN** the next task-state access or settlement occurs at least one hour after a result settled
- **THEN** the server removes the expired record and payload before processing the operation; access to that ID reports an unknown ID

#### Scenario: Twenty-first settlement

- **WHEN** a task settles with 20 unexpired terminal records already present
- **THEN** the earliest settled record is evicted even before its advertised `ttlMs` elapses, regardless of creation order, last access or outcome; the new record is retained and working tasks are unaffected
