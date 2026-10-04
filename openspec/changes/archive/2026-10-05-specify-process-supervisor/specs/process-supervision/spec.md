# Spec Delta

## Purpose

Define the standalone process supervisor's contract for native asynchronous Rust job execution, bounded output capture,
cancellation, descendant cleanup, and explicit abandonment when cleanup cannot be confirmed. This contract covers
ordinary Linux/macOS process groups, not hostile-process containment or migration of Moriarty callers.

## ADDED Requirements

### Requirement: Native asynchronous process ownership

The supervisor SHALL expose native Rust asynchronous startup, completion waiting, and cancellation-with-completion
operations. Successful startup SHALL return a single owning, non-clonable running-process handle and acknowledge
ownership of the direct child and registration of both capture streams. A separately clonable cancellation handle SHALL
support cancelling a job while another task owns its completion future. Callers SHALL NOT need a shell wrapper, an
external kill utility, a per-job runtime, or access to a raw child handle.

#### Scenario: Cancel while another task waits

- **WHEN** one async task awaits completion and another uses the job's cancellation handle
- **THEN** cancellation initiates supervised cleanup and the waiting task receives the same final report or supervision
  error as a cancellation-with-completion operation

#### Scenario: Startup acknowledgement

- **WHEN** asynchronous startup succeeds
- **THEN** the returned handle represents an owned child with both output streams registered for capture, not merely an
  accepted launch request

### Requirement: Literal execution and isolated standard streams

The supervisor SHALL preserve the supplied program spelling, raw operating-system arguments, and optional working
directory without implicit shell evaluation or symlink canonicalization of the program. The child SHALL inherit the host
environment. Stdin SHALL be null and produce EOF on reads; stdout and stderr SHALL be captured separately rather than
inherited from, or forwarded to, the host. Providing stdin input, environment overrides, and live output streaming are
outside this contract.

#### Scenario: Literal arguments and program identity

- **WHEN** a job is started with arguments containing shell syntax and a program path whose spelling matters to
  executable dispatch
- **THEN** the child receives the arguments literally and retains the supplied program spelling; no shell interprets the
  arguments

#### Scenario: Host stdin and protocol output remain untouched

- **WHEN** a child reads stdin and writes to stdout and stderr while the host uses its own standard streams
- **THEN** the child reads EOF without consuming host input, and its output appears only in the respective captures,
  never on the host's standard streams

### Requirement: Independently bounded byte capture

Each stream SHALL retain at most its independently configured byte limit, preserve raw bytes including invalid UTF-8 and
NUL, and expose whether bytes were discarded and whether EOF was actually observed. Reaching the limit exactly SHALL NOT
imply truncation. Overflow SHALL be discarded while both streams continue draining so output pressure does not deadlock
the child or starve cancellation and deadline handling. Capture memory SHALL be bounded by the configured retention
limits plus fixed-size read buffers and bounded metadata, excluding allocator, kernel-pipe, and caller-side conversion
overhead. No completed-job history SHALL retain additional captures internally.

#### Scenario: Both streams exceed their limits

- **WHEN** a child continuously emits output exceeding each stream's limit
- **THEN** both retained byte lengths remain within their separate limits, both truncation indicators report discarded
  output, and cancellation and cleanup deadlines remain serviced

#### Scenario: Exact-limit and zero-limit capture

- **WHEN** a stream emits exactly its limit and then EOF, or emits data with a zero retention limit
- **THEN** exact-limit output is retained without truncation, while zero-limit output is discarded with truncation and
  continues draining to genuine EOF

#### Scenario: Partial binary output

- **WHEN** a stream emits invalid UTF-8 or NUL bytes and later fails before EOF
- **THEN** retained bytes remain unchanged, the failure preserves the partial capture, and EOF is not invented

### Requirement: Validated options and independent time budgets

Options SHALL have valid defaults: 1 MiB retained per stream, a five-second startup budget, no execution timeout, a
two-second TERM grace, and a one-second post-KILL grace. Capture sizes greater than `isize::MAX` SHALL be rejected
before launch; allocation failure SHALL produce an execution error rather than launch an uncapturable child. Startup,
execution, TERM, and post-KILL budgets SHALL have distinct meanings; TERM and post-KILL durations SHALL be independently
configurable. Every representable nonnegative duration SHALL be accepted without deadline arithmetic overflow. Zero
durations SHALL trigger immediate transitions with a nonblocking state/readiness check, not introduce an unbounded wait.

#### Scenario: Invalid or unallocatable capture limit

- **WHEN** a caller supplies an unsupported capture size, or capture allocation fails
- **THEN** the request fails before child creation with a diagnostic identifying the invalid option or allocation
  failure

#### Scenario: Zero and maximal durations

- **WHEN** a caller selects zero or the largest representable duration for a time budget
- **THEN** zero gets immediate boundary handling, and the largest duration remains valid without overflow or premature
  expiry

#### Scenario: Independent cleanup graces

- **WHEN** a caller configures TERM and post-KILL graces independently
- **THEN** each grace governs only its own phase, rather than being rejected or shortened because of the other grace's
  magnitude

### Requirement: Persistent cancellation and cancellation-safe ownership loss

Cancellation SHALL be persistent and idempotent. Dropping a cancellation-handle clone SHALL NOT itself cancel the job.
Dropping the owning process handle, its completion future, or a previously polled startup future SHALL request
cancellation without aborting the cleanup owner. Dropping an unpolled startup future SHALL NOT create a child. Cleanup
SHALL remain owned despite loss of the caller's result receiver, subject to the documented host-lifetime boundary.

#### Scenario: Cancellation before the waiter polls

- **WHEN** cancellation is requested repeatedly before the completion future is polled
- **THEN** the request is retained and the job follows one cancellation lifecycle rather than losing or duplicating the
  request

#### Scenario: Dropped owner or completion future

- **WHEN** a caller drops the running handle or drops a pending completion future
- **THEN** drop returns without waiting and the independent cleanup owner continues cancellation and descendant cleanup

#### Scenario: Dropped startup future

- **WHEN** a caller drops startup before or after its first poll
- **THEN** an unpolled future launches nothing, and a polled future cancels outstanding startup while retaining
  responsibility for any created or late-created child

### Requirement: Bounded startup despite stalled process creation

Admission, process creation, handoff, and capture setup SHALL share one startup budget. Stalled creation SHALL NOT block
an async executor worker, require joining a stuck launcher, or make normal async-runtime destruction wait for that
creation call. Outstanding process-creation attempts SHALL have a finite process-wide bound; a stalled attempt SHALL
retain its admission capacity until it actually finishes, even after its caller stops waiting. Admission and handoff
SHALL respond to readiness, cancellation, and budget expiry.

At startup interruption, an already-created child available for handoff SHALL remain owned and receive bounded cleanup.
If creation or ownership transfer is uncertain, startup SHALL return an explicit interrupted/uncertain result without
claiming that no child exists. Any late-created child SHALL remain owned and receive a final group-KILL attempt and
cleanup bounded by the post-KILL budget. Waiting for known-child cleanup SHALL add at most that cleanup budget to
startup interruption handling, excluding scheduler and syscall latency.

#### Scenario: All creation capacity is stalled

- **WHEN** earlier creation attempts remain stuck and a new request exhausts its startup budget waiting for admission
- **THEN** the request returns a startup-timeout result, starts no additional unbounded launcher, and leaves the async
  executor responsive

#### Scenario: Child created after the caller stops waiting

- **WHEN** process creation completes after startup cancellation or timeout has closed the handoff
- **THEN** the late child still has a cleanup owner and receives bounded group cleanup; the earlier caller result does
  not fabricate the late outcome

#### Scenario: Child queued at interruption

- **WHEN** a created child is already available for transfer as startup is interrupted
- **THEN** the supervisor claims that child and attempts bounded cleanup instead of treating the closed result channel
  as proof that no child exists

### Requirement: Execution deadlines and cause precedence

The execution budget SHALL start only after successful startup. When no execution timeout is configured, the job SHALL
be allowed to run until exit, cancellation, or supervision failure. At a running-state cancellation or timeout boundary,
the supervisor SHALL freshly check for observable direct-child exit without reaping it before selecting the interruption
cause. An already observable exit SHALL take precedence; this is observation-order precedence, not reconstruction of the
kernel's historical exit timestamp. Once execution cancellation or timeout is selected, subsequent exit during cleanup
SHALL NOT replace that cause.

#### Scenario: Exit observable at the control boundary

- **WHEN** cancellation or execution timeout is pending and a fresh check observes that the direct child has already
  exited
- **THEN** the final result retains ordinary exit as its cause while still cleaning up leftover descendants

#### Scenario: Child exits during cancellation cleanup

- **WHEN** cancellation has been selected and the child exits during TERM grace
- **THEN** the result remains cancellation with the child's actual status rather than becoming ordinary exit

#### Scenario: Execution clock excludes startup

- **WHEN** startup consumes part of its own budget and the child then becomes ready
- **THEN** the child's configured execution allowance begins at readiness rather than at the initial launch request

### Requirement: Group-wide cleanup with TERM-to-KILL escalation

Each job SHALL have a process group distinct from the host's group. Cleanup SHALL target the direct child and ordinary
descendants that remain in that group, including grandchildren and descendants that no longer hold capture pipes open.
For execution cancellation, timeout, or supervision failure requiring termination, the supervisor SHALL attempt group
SIGTERM, service captures during the configured grace, and attempt a final group SIGKILL no later than grace expiry
unless cleanup is already confirmed or signaling authority has been revoked for identity safety. Direct-child exit SHALL
also initiate cleanup of remaining group members; EOF alone SHALL NOT complete a live job. Startup cleanup of an
already-created or late child SHALL be permitted to use immediate group SIGKILL rather than TERM grace.

#### Scenario: Descendants ignore TERM

- **WHEN** a cancelled job has a child and grandchild in its group that ignore SIGTERM
- **THEN** the supervisor escalates to group SIGKILL after the configured TERM grace and attempts to confirm cleanup
  within the post-KILL budget

#### Scenario: Leader exits leaving silent descendants

- **WHEN** the direct child exits but an ordinary descendant remains alive after closing stdout and stderr
- **THEN** the supervisor still attempts group cleanup and does not report success solely because the leader exited or
  the pipes reached EOF

#### Scenario: Output ends before the process

- **WHEN** the job closes both capture streams but remains alive
- **THEN** completion remains pending until exit or a termination trigger, rather than treating EOF as process
  completion

### Requirement: Bounded cleanup and explicit abandonment

After final SIGKILL handling, one shared post-KILL budget SHALL cover direct-child reaping, confirmation of group
absence, and settlement of both captures. Signal success SHALL NOT be treated as proof of death. If required cleanup
evidence cannot be established by budget expiry, the supervisor SHALL close its remaining capture resources, stop
waiting, and return an explicit cleanup-unconfirmed error with available bounded partial evidence. It SHALL NOT
indefinitely join a process, launcher, or cleanup task. Execution cancellation/timeout handling SHALL consume at most
TERM grace plus post-KILL grace, excluding scheduler and syscall latency; caller-selected enormous budgets intentionally
permit correspondingly long waits.

Abandonment SHALL NOT promise that the child or its descendants are dead or eventually reaped. Stalled creation calls
SHALL remain bounded outstanding attempts, not require the caller to await them. These guarantees assume a live, polled
async runtime and eventual return of the nonblocking OS operations used for observation/signaling; they are not
hard-real-time guarantees.

#### Scenario: Unkillable or unconfirmable child

- **WHEN** SIGKILL fails, a child remains uninterruptible, or direct-child reaping/group absence cannot be confirmed by
  the post-KILL deadline
- **THEN** the await returns cleanup uncertainty with the initiating cause and available status/captures, without
  blocking indefinitely or reporting a completed job

#### Scenario: Orphan zombies prevent group-absence confirmation

- **WHEN** the direct child has been reaped but killed orphan descendants remain as zombies because their reaper has not
  collected them, and group absence cannot be confirmed before the shared post-KILL budget expires
- **THEN** the supervisor returns a cleanup-unconfirmed error with available status and captures, without claiming
  completed cleanup, waiting indefinitely for external reaping, or sending another destructive signal after reaping

#### Scenario: Capture cannot reach EOF

- **WHEN** the child has been reaped but either stream cannot reach EOF within the shared cleanup budget
- **THEN** the supervisor returns an error with that stream's EOF evidence unchanged rather than adding another
  unbounded drain wait

### Requirement: Truthful completion and bounded failure evidence

A completed report SHALL require an actual reaped direct-child exit status, confirmed group absence, and genuine EOF on
both captures. Nonzero child exit SHALL remain a completed ordinary-exit report, not a supervision failure. Cleaned
execution cancellation and timeout SHALL be distinguishable from ordinary exit and carry the actual status.

Errors SHALL distinguish start failure, interrupted startup, supervision failure, and unconfirmed cleanup. They SHALL
expose bounded partial captures, known status/identity, startup-creation uncertainty, confirmed group absence,
phase-tagged interruption, and a bounded set of stage-specific failure diagnostics. Initiating failures and interruption
reasons SHALL survive additional cleanup failures. A failed reader SHALL close and settle for error finalization without
fabricating EOF; confirmed cleanup with a read failure SHALL be a supervision failure rather than falsely reporting
cleanup uncertainty. A lost supervision channel SHALL report uncertainty, not invent success evidence. Signal-attempt
indicators SHALL describe entry into a signaling path, including preflight, not prove that a syscall ran, succeeded, or
killed a process.

#### Scenario: Nonzero exit with confirmed cleanup

- **WHEN** a child exits unsuccessfully but is reaped, its group is absent, and both captures reach EOF
- **THEN** the caller receives a completed ordinary-exit report with the actual nonzero status and captures

#### Scenario: Reader failure after otherwise confirmed cleanup

- **WHEN** a capture reader fails and closes, then the child is reaped and group absence is confirmed
- **THEN** the result is a supervision error retaining captured bytes and false EOF, not a successful report or invented
  cleanup uncertainty

#### Scenario: Pipe-setup failure coincides with startup interruption

- **WHEN** startup is cancelled or its budget expires while capture pipes are being registered, and pipe setup fails
- **THEN** the error retains both the pipe-setup diagnostic and the phase-tagged startup interruption reason, even if
  subsequent cleanup is unconfirmed

#### Scenario: Lost driver or multiple failures

- **WHEN** supervision communication is lost, or a startup/execution failure is followed by cleanup failures
- **THEN** the error preserves the surviving bounded evidence and initiating cause, and never invents a status, group
  absence, or EOF

### Requirement: Identity-safe destructive signaling

The supervisor SHALL retain exclusive ownership of direct-child reaping and preserve the child's identity until its
final destructive signal. It SHALL permanently revoke destructive signaling authority before reaping, including error
and drop paths. Lost child ownership, invalid identity, or an identity matching the host's process group SHALL prevent
destructive signaling rather than risk an unrelated process. No destructive signal SHALL target a saved identity after
reaping. Once group absence is confirmed, that fact SHALL remain monotonic rather than be invalidated by probing a
potentially recycled group identity. Inability to confirm identity or absence SHALL yield an error instead of weakening
these safety constraints.

#### Scenario: Ownership lost before signaling

- **WHEN** an observation establishes that the direct child is no longer owned or waitable by the supervisor
- **THEN** destructive signal authority is revoked and cleanup reports uncertainty rather than signaling a possibly
  recycled identity

#### Scenario: Reaping and group identity reuse

- **WHEN** the child is reaped and its former identifier could be reused
- **THEN** neither ordinary cleanup nor drop sends another destructive signal using that identifier, and
  already-confirmed group absence remains confirmed

### Requirement: Host isolation and supported lifetime boundaries

The contract SHALL support Linux and macOS and SHALL explicitly reject unsupported targets rather than silently offer
weaker cleanup. The supervisor SHALL leave the host process group, unrelated children, and unrelated child-wait
ownership intact. It SHALL NOT install process-wide SIGCHLD behavior or subreaper behavior to implement supervision. The
host SHALL NOT concurrently reap supervised children or enable automatic child reaping.

Normal cleanup SHALL require a live async runtime with the documented I/O and timer facilities. Runtime/task destruction
SHALL perform only nonwaiting best-effort final signaling/reaping for owned children; it SHALL NOT require forced host
exit or an unbounded runtime shutdown wait. Deliberate process-group escape, delegated execution, unsignalable
credential changes, host crash/SIGKILL, and host exit before stalled creation returns are outside the descendant-cleanup
guarantee. This is lifecycle management, not a sandbox. No global task registry or application shutdown policy is
required.

#### Scenario: Unrelated child remains owned by the host

- **WHEN** a supervised job is cancelled while the host has an unrelated child
- **THEN** the host group is unchanged and the unrelated child is neither signaled nor reaped by the supervisor

#### Scenario: Runtime destruction during stalled launch or failed cleanup

- **WHEN** the host destroys its async runtime while creation is stalled or cleanup is unconfirmed
- **THEN** supervisor-owned work does not require joining the stalled creation or an indefinitely live child to permit
  normal host termination, and no guarantee of completed cleanup is fabricated

### Requirement: Child-output privacy

The supervisor SHALL NOT log, forward, or interpolate captured child output into diagnostic display. Caller-accessible
report/error captures SHALL remain available separately. Late cleanup diagnostics without a remaining receiver SHALL
contain lifecycle metadata only. Caller-supplied synchronous diagnostic subscribers can block their dispatch and retain
an outstanding launcher slot; that host-callback limitation SHALL be documented rather than claimed to be nonblocking
logging.

#### Scenario: Sensitive output on a failing job

- **WHEN** a job emits sensitive stdout/stderr and supervision or late cleanup fails
- **THEN** diagnostics disclose lifecycle evidence without the captured bytes, while a surviving caller can inspect
  bounded captures through the error result
