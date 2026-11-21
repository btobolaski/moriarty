# mcp-protocol-support Specification

## Purpose

Define the explicit MCP revision policy and interoperable modern and legacy stdio behavior of Moriarty's git-read-only,
jj-read-only, and project-tools servers without adding task-backed tool execution.

## Requirements

### Requirement: Explicit supported revisions

All three MCP servers SHALL advertise and support exactly `2024-11-05`, `2025-03-26`, `2025-06-18`, `2025-11-25`, and
`2026-07-28`. Dependency upgrades SHALL NOT implicitly expand this set. Unsupported per-request revisions SHALL receive
`-32022` with the requested revision and supported set rather than execute the request.

#### Scenario: Discovery agrees across servers

- **WHEN** a client discovers any of the three servers using valid `2026-07-28` request metadata
- **THEN** discovery advertises exactly the five supported revisions and that server's actual capabilities

#### Scenario: Unsupported ordinary request does not execute

- **WHEN** a client sends an ordinary request with an unsupported per-request protocol revision
- **THEN** the server returns `-32022` with `requested` and `supported` data, and does not execute the operation

### Requirement: Legacy initialization remains available

Legacy clients SHALL retain the `initialize` and `notifications/initialized` lifecycle. Initialization SHALL echo a
supported legacy revision; unknown revision strings and revisions without an initialization handshake SHALL fall back to
`2025-11-25`. Adding modern support SHALL NOT make an `initialize` response select `2026-07-28`.

#### Scenario: Each supported legacy client initializes

- **WHEN** a client initializes with any supported revision from `2024-11-05` through `2025-11-25`
- **THEN** the server selects that revision and accepts that server's existing metadata-free legacy operations

#### Scenario: Modern or unknown version over the legacy handshake

- **WHEN** a client initializes with `2026-07-28`, an unknown newer revision, or an unknown older revision
- **THEN** the server selects `2025-11-25`, retaining legacy wire behavior

### Requirement: Discovery is lifecycle-neutral

`server/discover` SHALL NOT commit a stdio connection to either lifecycle. After discovery or recoverable
invalid/unsupported bootstrap requests, valid legacy initialization or modern work SHALL remain possible on the same
connection. Discovery declaring a parseable unsupported revision SHALL receive `-32022` with `requested`/`supported`
data before validation of other required metadata.

#### Scenario: Successful discovery followed by legacy fallback

- **WHEN** a client discovers using `2026-07-28` and subsequently initializes with `2025-11-25` on the same connection
- **THEN** the legacy initialization and a subsequent tool request succeed with legacy wire shapes

#### Scenario: Rejected discovery followed by legacy fallback

- **WHEN** discovery uses an unsupported revision and the client subsequently initializes on the same connection
- **THEN** discovery returns `-32022` and the subsequent legacy initialization and tool request still succeed

#### Scenario: Incomplete unsupported discovery preserves version-error precedence

- **WHEN** discovery declares a parseable unsupported revision but omits required client-capability metadata
- **THEN** each server returns `-32022` with `requested` and `supported` data rather than `-32602`, and permits
  subsequent valid legacy initialization or a modern ordinary request on the same connection

#### Scenario: Discovery followed by a modern request

- **WHEN** a client sends a valid modern tool request after discovery on the same connection
- **THEN** the request succeeds without an intervening initialization handshake

#### Scenario: Malformed bootstrap request remains recoverable

- **WHEN** a non-initialize bootstrap request has missing or malformed required modern metadata, except discovery
  declaring a parseable unsupported revision
- **THEN** the server returns `-32602` without executing it and accepts a subsequent valid legacy initialization or
  modern request on the same connection

### Requirement: Modern requests carry their own context

Modern ordinary requests SHALL work without prior discovery or initialization. Every such request SHALL supply valid
`_meta` entries `io.modelcontextprotocol/protocolVersion` and `io.modelcontextprotocol/clientCapabilities`; optional
client information SHALL remain optional. Missing or malformed required entries SHALL produce `-32602` before tool
execution. Context SHALL be validated per request rather than borrowed from a previous request.

#### Scenario: Tool call is the first request

- **WHEN** a client sends a valid `2026-07-28` `tools/call` as its first message, with no client-info entry
- **THEN** the server executes the ordinary tool operation and returns its modern result without requiring discovery or
  an initialization handshake

#### Scenario: Later request cannot reuse earlier metadata

- **WHEN** a valid modern request is followed by another ordinary request missing either required metadata entry
- **THEN** the later request receives `-32602` and launches no tool work

#### Scenario: Malformed capability metadata does not execute

- **WHEN** a modern tool request supplies a malformed protocol-version value or non-object client-capabilities value
- **THEN** the server returns `-32602` and does not execute the tool

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

### Requirement: Removed legacy methods are not modern features

Methods removed by the `2026-07-28` core protocol SHALL NOT be exposed as modern capabilities or accepted merely because
an older lifecycle supported them. A modern `ping` request SHALL return method-not-found, while legacy ping behavior
SHALL remain compatible. Supported legacy-version and context-free ping (including empty/extension-only `_meta`) SHALL
retain upstream behavior.

#### Scenario: Ping differs by lifecycle

- **WHEN** clients send `ping` under a valid modern lifecycle and an initialized legacy lifecycle
- **THEN** the modern request returns `-32601` and the legacy request retains its successful empty-result behavior

### Requirement: Protocol rejections retain bounded transport ownership

Rejections generated by Moriarty's transport guard SHALL remain bounded and host-owned, survive receive cancellation,
and SHALL NOT block input EOF detection, admission closure, cancellation, or awaited owned-work settlement. Delivery
failure or queue exhaustion SHALL close admission and remain a host-visible failure, not successful response delivery.

#### Scenario: Blocked ping rejection cannot hide input EOF

- **WHEN** response output is blocked, a guarded ping is consumed, and input closes while tool work remains owned
- **THEN** the host detects EOF, closes admission, cancels and awaits the owned work without resumed response reads, and
  retains output failure and any cleanup uncertainty

#### Scenario: Receive cancellation does not lose a consumed ping rejection

- **WHEN** rmcp cancels a receive after consuming a guarded ping while its rejection is blocked behind another write
- **THEN** the rejection remains owned and arrives exactly once when output resumes on the live connection, without
  needing another receive to drive delivery

#### Scenario: Rejection delivery failure remains host-owned

- **WHEN** an immediate or queued rejection write fails
- **THEN** the host closes admission, cancels affected work, awaits the sender and owned operations, and retains the
  failure even if no response reaches the client

#### Scenario: Rejection queue exhaustion fails closed

- **WHEN** blocked output exhausts the bounded rejection queue
- **THEN** the host closes admission, cancels the service, awaits its sender, and retains a transport failure rather
  than allocating an unbounded queue or detaching sends

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
