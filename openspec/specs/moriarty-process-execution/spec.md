# moriarty-process-execution Specification

## Purpose

Define Moriarty's externally observable execution and cancellation behavior when using the trusted process-supervision
contract, while preserving authorization policy and excluding the interactive MCP installer.

## Requirements

### Requirement: Supervised production execution

Moriarty SHALL apply the existing `process-supervision` contract to every production external-process execution other
than `moriarty mcp install`, including project commands, checks, read-only VCS tools, and repository/workspace
discovery. Migrated execution SHALL use null stdin, inherited environment, literal arguments, and separately captured
output. Test-fixture setup processes are outside the production migration scope.

#### Scenario: Production execution coverage

- **WHEN** Moriarty executes a project tool, configured check, read-only VCS command, or git root-discovery probe
- **THEN** its process lifecycle follows the existing supervision contract rather than an unsupervised execution path

### Requirement: Interactive installer exception

The `moriarty mcp install` installer SHALL retain its existing inherited terminal streams and execution behavior and
SHALL NOT use the `process-supervision` contract.

#### Scenario: Interactive MCP installation remains separate

- **WHEN** a user runs `moriarty mcp install`
- **THEN** Claude's remove/add commands retain inherited stdin/stdout/stderr and existing installer behavior without
  entering the migrated execution lifecycle

### Requirement: Existing authorization and execution identity are preserved

Moriarty SHALL retain current approval gates, argv/binary identity verification, workspace-local configuration loading,
and repository-keyed approval sharing. Execution SHALL use the verified program spelling rather than re-resolving the
configured program or canonicalizing a dispatch-sensitive symlink. Project commands and CLI checks SHALL execute in the
caller's canonical project directory; Stop-hook/MCP checks SHALL execute in the workspace root.

#### Scenario: Approved workspace-local program

- **WHEN** a secondary workspace executes its approved local program from a nested project directory
- **THEN** the exact program verified for that workspace runs with the entry point's existing cwd policy, and an
  unapproved local copy cannot shadow it

#### Scenario: Approval identity cannot cross a retargeted project alias

- **WHEN** a project-path alias is retargeted from approved repository A to unrelated unapproved repository B between
  repository and workspace discovery, with matching check names and argv referring to the same executable
- **THEN** the repository approval identity remains consistent with the workspace used for configuration and execution,
  or checks report an explicit blocked discovery failure; A's approvals never authorize B's checks, and that failure
  never becomes a successful no-checks result

### Requirement: Read-only execution restrictions

Read-only VCS tools SHALL preserve their path validation, argument restrictions, and forced read-only flags.

#### Scenario: Read-only tool validation still precedes execution

- **WHEN** a VCS tool receives a disallowed argument or invalid project directory
- **THEN** Moriarty rejects it before executing that VCS command, without weakening its existing restrictions

### Requirement: Lossless repository and workspace approval identities

Repository/workspace filesystem identities SHALL remain distinct through discovery, approval lookup, and approval
storage. Presentation text SHALL NOT define identity. Moriarty SHALL reject an identity that cannot be represented
losslessly by the existing approval format before approval lookup or storage, rather than substitute a replacement
string or change the storage format.

#### Scenario: Unrepresentable approval identities remain distinct

- **WHEN** distinct project directories have byte-valued names that produce the same lossy display string
- **THEN** an approval for one directory does not authorize the other, and an identity the existing approval format
  cannot represent losslessly is rejected before lookup or storage

### Requirement: Lossless root-output decoding

Root-output decoding SHALL preserve path characters and remove only the output protocol's terminator, not leading or
trailing path whitespace. Unsupported or invalid root identities SHALL produce an explicit discovery failure rather than
an alternative root or successful no-checks result.

#### Scenario: Whitespace belongs to the root identity

- **WHEN** Git reports a workspace root whose path contains leading or trailing whitespace
- **THEN** discovery preserves those path characters and selects that workspace's configuration rather than falling back
  to the nested caller directory

#### Scenario: Non-UTF-8 root output is not approximated

- **WHEN** Git reports a filesystem root containing non-UTF-8 path bytes
- **THEN** discovery either preserves the filesystem identity losslessly or reports an explicit unsupported-identity
  failure, without interpreting a replacement-character path or permitting successful no-checks fallback

### Requirement: Lossless executable approval identities

Original and canonical executable paths bound by approvals SHALL also retain lossless identity during approval
comparison and storage. Moriarty SHALL reject current executable paths the existing approval format cannot represent
losslessly rather than match or store their lossy display strings, even when argv and executable hashes match an
existing approval. This SHALL NOT change the approval format or the verified program spelling used for execution.

#### Scenario: Executable-path collisions cannot reuse an approval

- **WHEN** an executable symlink is retargeted between canonical paths with distinct non-UTF-8 bytes but identical lossy
  display strings, while argv and executable hashes remain unchanged
- **THEN** unrepresentable current executable-path identities are rejected during verification and before approval
  storage; an existing approval containing the lossy spelling does not authorize the other target

### Requirement: Bounded project captures and exit decisions

Migrated execution SHALL retain at most 1 MiB of raw bytes per output stream and use the supervision contract's default
startup and cleanup budgets. Project-command and check output SHALL disclose each truncated stream without inventing a
count of discarded bytes. Capture truncation alone SHALL NOT change an otherwise completed project command/check's
exit-based success decision; the checks' aggregate-output policy remains a separate gate.

#### Scenario: Large project-command output

- **WHEN** a completed project command exceeds the stdout retention limit
- **THEN** Moriarty presents the retained stdout with a truncation notice and uses the ordinary exit status for its
  success decision

### Requirement: Complete VCS output and compatible presentation

Read-only VCS tools and root discovery SHALL reject truncated captures rather than return or interpret incomplete
output. Existing complete-output response shapes and lossy text conversion for human-readable command/check/VCS
presentation SHALL remain compatible; that conversion SHALL NOT derive filesystem identities.

#### Scenario: Large VCS output

- **WHEN** a read-only VCS command truncates either capture
- **THEN** Moriarty returns an explicit incomplete-output error, not a successful partial diff, log, or file response

### Requirement: Check deadlines remain whole-batch limits

Stop-hook/MCP checks SHALL retain a five-minute whole-execution-batch deadline, at most four concurrent checks, and the
existing 10 MiB aggregate formatted-output gate. The deadline SHALL include queueing and startup after pre-execution
verification, rather than grant each queued check a fresh five minutes.

#### Scenario: Queued check at deadline

- **WHEN** the batch deadline expires while four checks are active and other checks remain queued
- **THEN** no queued check starts, the active operations are cancelled and awaited, and the Stop hook and MCP checks
  both report the batch as blocked by timeout

### Requirement: Stopped check batches settle active operations

When the deadline or aggregate gate stops a batch, Moriarty SHALL stop launching queued checks, cancel active
operations, and await their terminal outcomes before returning a blocked result. Cleanup time SHALL follow the
supervision contract and SHALL NOT be claimed to fit inside the five-minute execution allowance. Project commands, CLI
checks, and VCS execution SHALL receive no new execution deadline.

#### Scenario: Aggregate output gate

- **WHEN** retained, formatted check output exceeds the aggregate gate
- **THEN** Moriarty blocks the batch, stops queued work, and settles active operations without collecting an unbounded
  history of completed results first

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

### Requirement: Cleanup ownership survives caller loss

Cleanup ownership SHALL survive a dropped request handler, and Moriarty SHALL NOT rely solely on dropping futures or
destroying the runtime for normal shutdown. Cancellation during startup SHALL retain the supervision contract's
late-launch ownership and uncertainty limitations without an unbounded join. Moriarty's cancellation and shutdown
integration SHALL NOT add subprocess execution deadlines or strengthen the supervisor's documented startup, cleanup, and
host-lifetime guarantees.

#### Scenario: Contained work panic still settles admitted operations

- **WHEN** scoped work panics after admitting a process or approval-persistence transaction, while the async runtime
  remains available
- **THEN** the operation remains unsuccessful, its owning scope retains admitted work and initiates cancellation, and
  request/host drain awaits those operations' reports or errors before completing; cleanup uncertainty remains visible
  without aborting completion owners

#### Scenario: Cancellation during startup

- **WHEN** an operation is cancelled while process creation or handoff is still pending
- **THEN** Moriarty relinquishes startup through the supervision contract's cancellation-safe ownership path, preserves
  that creation or cleanup can remain uncertain, and never waits indefinitely for a stalled launcher

### Requirement: Interruptible host boundaries and approval settlement

Host input reads and approval-lock acquisition SHALL remain interruptible at their owning boundaries. Graceful
SIGINT/SIGTERM shutdown SHALL allow the affected CLI/MCP host to terminate after settling owned work without requiring
additional stdin bytes, input EOF, or another process to release an approval lock. Application settlement SHALL include
admitted approval persistence, not just subprocess drain. Interrupted approval workflows SHALL restore the terminal
before exit.

#### Scenario: Idle MCP stdin does not prevent signal shutdown

- **WHEN** an MCP server receives SIGINT/SIGTERM while its production stdin is awaiting another message and the client
  keeps the input pipe open
- **THEN** the server settles admitted work and exits without requiring another message or input EOF, and a pending
  input read does not indefinitely block normal runtime destruction

#### Scenario: MCP response backpressure does not prevent signal shutdown

- **WHEN** an MCP client stops reading a large response while keeping its pipes open and the server receives
  SIGINT/SIGTERM
- **THEN** pending response output does not prevent host exit after admitted work has settled, without requiring the
  client to resume reading or abandoning completion owners, and incomplete response delivery is not claimed successful

#### Scenario: Incomplete hook input remains interruptible

- **WHEN** `moriarty hooks exec` receives SIGINT/SIGTERM while waiting for its input writer to finish or close the pipe
- **THEN** the hook invocation terminates after owned-work settlement without waiting for input EOF or reporting an
  unfinished hook evaluation as successful

#### Scenario: Approval-lock contention remains interruptible

- **WHEN** `moriarty approve-project` receives SIGINT/SIGTERM while another process holds the approvals lock
- **THEN** the cancelled invocation restores the terminal and exits without requiring that process to release the lock,
  and its cancelled acquisition cannot later start an approval save

### Requirement: Accepted blocked-stdout shutdown limitation

The graceful host-termination guarantee SHALL exclude blocked stdout writes from `moriarty hooks exec`,
`moriarty test checks`, and `moriarty test project-tools`. No output-write timeout is required; SIGKILL can be needed in
that condition. This exception SHALL NOT apply to MCP transport shutdown or relax cancellation/admission and owned-work
settlement obligations. SIGKILL remains outside cooperative cleanup and persistence guarantees.

#### Scenario: Blocked hook response stdout is an accepted shutdown limitation

- **WHEN** `moriarty hooks exec` is blocked writing its response to stdout because the reader keeps the pipe open
  without consuming output
- **THEN** no output-write timeout or graceful SIGINT/SIGTERM exit is guaranteed, SIGKILL can be required, and forceful
  host termination does not establish completed cleanup or persistence

#### Scenario: Blocked CLI test stdout is an accepted shutdown limitation

- **WHEN** `moriarty test checks` or `moriarty test project-tools` is blocked writing to stdout because the reader keeps
  the pipe open without consuming output
- **THEN** no output-write timeout or graceful SIGINT/SIGTERM exit is guaranteed, SIGKILL can be required, and forceful
  host termination does not establish completed cleanup or persistence

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

### Requirement: Truthful execution outcomes

Only a completed ordinary exit SHALL be eligible for existing exit-code-based success handling. Cancellation, timeout,
startup failure, supervision failure, and unconfirmed cleanup SHALL remain unsuccessful and distinguishable from an
ordinary exit, even if a known child status is zero. Diagnostics SHALL retain the available interruption phase/reason
and creation/cleanup uncertainty without claiming successful termination or automatically retrying an uncertain launch.

#### Scenario: Cancelled operation has a zero child status

- **WHEN** supervision reports cancellation with a known zero exit status
- **THEN** Moriarty reports cancellation rather than successful command/check/VCS completion

### Requirement: Interrupted discovery fails closed

Repository/workspace discovery SHALL remain cancellation-responsive and preserve ordinary non-repository/missing-Git
fallback. Cancellation, startup timeout, truncated discovery output, supervision failure or cleanup uncertainty SHALL
stop the operation, not yield successful canonical-path fallback, an alternative approval key or successful no-checks
result. Stop-hook checks SHALL fail closed for interrupted or incomplete discovery.

#### Scenario: Uncertain discovery cannot bypass checks

- **WHEN** a git root probe reports timeout, incomplete output, or unconfirmed cleanup
- **THEN** approval loading or checks stop with an execution failure instead of treating the directory as a successfully
  discovered non-repository project or allowing the Stop hook without checks

### Requirement: Evidence of ordinary repository absence

Ordinary repository absence SHALL require evidence that Git is unavailable independently of a subprocess spawn's
`NotFound` result, or that completed discovery establishes a non-repository directory. A generic nonzero probe exit,
spawn `NotFound`, invalid returned identity, or failure to canonicalize a reported root SHALL NOT by itself establish
ordinary absence.

#### Scenario: Ordinary non-repository directory

- **WHEN** complete root discovery ordinarily establishes that a directory is not a repository
- **THEN** Moriarty retains its existing canonical-directory fallback and approval/configuration semantics

#### Scenario: Git lookup follows the probe directory

- **WHEN** PATH contains relative or empty entries, Git is available through those entries under the probe's canonical
  cwd, and Moriarty's host cwd differs
- **THEN** discovery resolves Git using the probe's cwd and retains the repository/workspace root rather than falsely
  treating Git as unavailable or permitting successful no-checks fallback

#### Scenario: Unset PATH preserves native Git lookup

- **WHEN** Moriarty starts with PATH unset and Git is available through the platform's native default executable search
  path
- **THEN** discovery retains that lookup without changing the inherited environment or failing solely because PATH is
  unset

#### Scenario: Filesystem-boundary absence remains ordinary absence

- **WHEN** a complete Git root probe establishes a non-repository directory after stopping at a filesystem boundary
- **THEN** discovery retains the canonical-directory fallback and existing approval/configuration semantics without
  treating unrelated probe failures as ordinary absence

#### Scenario: Spawn NotFound does not establish missing Git

- **WHEN** Git is available but its root probe cannot start because the recorded cwd disappeared before launch
- **THEN** discovery reports an execution failure rather than treating Git as missing or permitting canonical-path or
  successful no-checks fallback

### Requirement: Unexpected discovery failures block checks

Unexpected Git-probe or returned-root failures SHALL stop discovery and approval/check processing rather than invent a
root or permit successful no-checks fallback. Both Stop-hook and MCP checks SHALL report those failures as blocked or
unsuccessful.

#### Scenario: Git configuration failure is not repository absence

- **WHEN** a Git root probe fails because an existing repository has invalid or unreadable Git configuration
- **THEN** discovery reports failure rather than selecting the nested caller directory's configuration or returning
  successful no-checks results

#### Scenario: A reported root cannot be canonicalized

- **WHEN** a completed Git probe reports a root that cannot be canonicalized
- **THEN** discovery reports the root-resolution failure rather than treating the caller as a successfully discovered
  non-repository project or choosing another approval/configuration root

#### Scenario: Late input canonicalization failure cannot skip checks

- **WHEN** the project input was initially canonicalized successfully and its repository probe succeeded, but the
  canonical nested project directory disappears before workspace discovery canonicalizes that input again
- **THEN** both Stop-hook and MCP checks report a blocked or unsuccessful discovery outcome, not successful `NoChecks`,
  and their diagnostics retain the canonicalization context and underlying filesystem cause without disclosing captured
  child output; the existing initial-missing-input behavior remains unchanged and does not apply to this later failure

### Requirement: Approval persistence survives cooperative cancellation

Once an approval load-modify-save transaction acquires its lock and is admitted to persistence, Moriarty SHALL retain
exclusive lock and transaction ownership until all filesystem operations it started have completed or returned failure,
even if its caller is cancelled or dropped. Graceful shutdown SHALL await admitted persistence before normal runtime
exit. Another approval update SHALL NOT overlap an unfinished write from the earlier transaction.

#### Scenario: Cancellation during an approval write

- **WHEN** an approval save is cancelled while its file write is still outstanding
- **THEN** its transaction retains the lock until the actual write completes or fails, a competing updater cannot enter
  that transaction meanwhile, and graceful shutdown awaits the outstanding persistence without claiming save success

### Requirement: Approval cancellation and persistence limits

Cancellation before transaction admission SHALL prevent a later save. Persistence failures SHALL remain errors, and
cancellation SHALL NOT be reported as a successful approval save. Approval persistence guarantees apply to cooperative
cancellation and graceful shutdown, not host crash or SIGKILL, and do not promise rollback of an already admitted
transaction.

#### Scenario: Cancellation before approval transaction admission

- **WHEN** an approval invocation is cancelled before acquiring the lock and admitting its transaction
- **THEN** it does not later load-modify-save approvals after the caller has returned, even if the lock becomes
  available

### Requirement: Approval load errors do not overwrite stores

Only genuine filesystem absence of the approval file MAY produce a successful empty-store fallback from a load error.
Unreadable or malformed approval stores SHALL remain errors, regardless of words in their diagnostics. An update whose
approval load fails SHALL NOT subsequently write a default or replacement store over the existing file.

#### Scenario: Malformed approval stores are not missing files

- **WHEN** an existing approval file contains malformed TOML whose diagnostic includes `NotFound` and an approval
  load-modify-save update is attempted
- **THEN** the update reports the load/parse error and leaves the original file bytes unchanged rather than replacing it
  with a default store; genuine missing-file initialization remains allowed

### Requirement: Configuration argv errors remain actionable

Moriarty SHALL reject empty configured command/check argv before approval metadata indexing or execution, using an error
that identifies the affected item and configuration problem rather than a panic or lost-owner diagnostic.

#### Scenario: Empty approval command or check argv

- **WHEN** approval initialization encounters a configured command or check with an empty argv array
- **THEN** it returns a diagnostic identifying that item and its empty command array without panicking or reporting
  supervision ownership uncertainty

### Requirement: Contextual diagnostics exclude captured output

Lifecycle diagnostics SHALL NOT interpolate captured child output. Configuration, verification, and execution
diagnostics SHALL preserve available contextual source causes using output-excluding presentation. Safe error-chain
presentation SHALL NOT interpolate captured child output or debug-print supervisor reports or partial captures.
Diagnostic changes SHALL NOT alter MCP error envelopes or tool schemas.

#### Scenario: Malformed configuration retains its source diagnostic

- **WHEN** an MCP project tool fails to parse malformed project configuration
- **THEN** its error retains both the project-configuration context and the underlying parse cause, including location
  when available, rather than only the outer context, without disclosing captured child output

#### Scenario: Retained approval failures keep their I/O cause

- **WHEN** an admitted approval filesystem operation fails after its caller is cancelled or dropped
- **THEN** the retained failure preserves both persistence context and the available I/O cause after owned-work drain,
  without claiming save success or disclosing captured child output

#### Scenario: Combined failures retain their source diagnostics

- **WHEN** an initiating configuration, verification or execution failure carries contextual source causes and an
  independent cleanup failure is combined with it
- **THEN** the composed diagnostic retains the initiating context and available source causes of both failures, not only
  their outer messages, without disclosing captured child output or changing the existing error envelope

### Requirement: Verification respects the trusted supervision boundary

Moriarty's verification SHALL cover its authorization, caller policies, cancellation routing, shutdown ordering, output
presentation, and mapping of supervision outcomes. Coverage SHALL demonstrate that Moriarty requests cancellation and
awaits owned outcomes, not reconstruct operating-system cleanup proof.

#### Scenario: Integration cancellation regression

- **WHEN** Moriarty's cancellation integration is verified
- **THEN** the check asserts request scope, stop-admission behavior, and completion waiting through the trusted boundary
  without introducing descendant trees, signal-operation fault injection, or independent PGID/reaping proofs in Moriarty

### Requirement: Operating-system assurance belongs to supervisor tests

Verification of process-group identity safety, signal escalation, reaping, pipe-draining fairness, stalled-launch
internals, and descendant-cleanup guarantees SHALL belong to the standalone supervisor's tests rather than duplicate
Moriarty test harnesses.

#### Scenario: Missing internal assurance

- **WHEN** verification exposes a missing guarantee inside the trusted supervision implementation
- **THEN** that concern is covered by the supervisor's own tests rather than adding a second implementation or
  duplicated internal verification to Moriarty
