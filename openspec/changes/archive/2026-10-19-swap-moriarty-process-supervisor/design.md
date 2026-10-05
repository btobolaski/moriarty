# Design

## Context

See `proposal.md` for motivation and `specs/moriarty-process-execution/spec.md` for behavior. The existing
`openspec/specs/process-supervision/spec.md` and `crates/process_supervisor/src/lib.rs` define the trusted boundary.

Read-only discovery found these production launch seams:

| Seam                                                                 | Current behavior                                                                                                | Migration constraint                                                                         |
| -------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| `project_config/runner.rs::execute_command_static`                   | Tokio output collection for verified programs; no execution timeout or capture cap                              | Preserve approved program and caller-directory cwd; add bounded capture                      |
| `checks.rs::run_checks_with_limits`                                  | Four concurrent output futures under one 300-second timeout; output limits applied after collecting all results | Preserve the batch clock; cancel and settle active work instead of dropping the collector    |
| `mcp/read_only.rs::run_read_only_command`                            | Shared git/jj output collection after path/flag validation                                                      | Preserve complete `CommandResult` shape and reject truncated captures                        |
| `repository.rs::try_git_worktree_root` and `try_git_repository_root` | Blocking git probes inside otherwise async workflows                                                            | Make subprocess-backed discovery async through all callers; preserve jj filesystem discovery |
| `mcp/mod.rs::install_single_mcp_server`                              | `claude mcp remove/add` with inherited terminal streams                                                         | Leave unchanged and outside the execution scope                                              |

`which::which` in `project_config/approvals.rs` is filesystem/PATH lookup, not a process launcher. Production coverage
excludes test-only git/jj/CLI fixture setup.

The supervisor exposes `ProcessSpec { program, args, cwd }`, validated `ProcessOptions`, `RunningProcess::spawn`,
`cancellation_handle`, and consuming `wait`. Reports contain `Completion::{Exited, Cancelled, TimedOut}` and byte
captures. Downcastable `ProcessError` exposes partial evidence without granting a raw child handle. Its defaults are 1
MiB per stream, five-second startup, no execution timeout, two-second TERM grace, and one-second post-KILL grace.
Cancellation tokens become available only after successful startup; dropping a polled startup future is the contract's
startup-cancellation path.

The pinned rmcp 3.2.0 already exposes `RequestContext.ct`; its service loop cancels that token for
`notifications/cancelled`. Moriarty's current tool handlers do not consume it. This migration does not require new MCP
revisions, task-management capabilities, or an rmcp upgrade. Current `mcp/mod.rs` protocol constants and protocol tests
remain the compatibility reference.

## Goals / Non-Goals

**Goals:**

- One small shared execution boundary that consumes the trusted crate, plus explicit application ownership of active
  operations through normal completion, request loss, and shutdown.
- Preserve distinct project-command, check-batch, VCS, and root-discovery policies rather than flatten them into one
  generic result or timeout policy.
- Keep cancellation effective during queueing and startup as well as after a running handle is returned.

**Non-Goals:**

- A second supervisor, custom signals/reaping, raw-PID registries, detached subprocess wrappers, per-operation runtimes,
  shell wrappers, or a pluggable backend framework.
- Changes to supervisor guarantees, MCP protocol support, approval formats, tool schemas, installer execution, or
  non-process report/rule behavior.
- New configuration knobs, new execution timeouts for previously unlimited paths, streaming output, environment
  overrides, or task-management tools.

## Decisions

### 1. Use one private execution module, not another process manager

Add `crates/moriarty/src/execution.rs`, the existing `process_supervisor` path dependency, and a direct
`tokio-util = "=0.7.18"` dependency with its `rt` feature, reusing the version already pinned by the supervisor. The
user approved that dependency/feature reuse after review. Use `TaskTracker` for completion ownership and
`CancellationToken` only at owning operation seams; enable Tokio's existing `signal` feature as needed. Callers retain
authorization, result interpretation, and output policy.

Track independently owned request/process tasks before startup. A process task owns startup and the running wait,
returning its report to the caller without making caller lifetime equal supervision lifetime. `TaskTracker` releases
completed task storage, survives receiver loss without aborting owners, and provides close/drain; use it rather than a
custom live-operation map, token list, or completion registry. Per-request/batch tracking can isolate settlement from
unrelated requests, while host tracking also retains all owners for disconnect/shutdown. No backend trait or raw-child
registry is needed.

`TaskTracker::close` does not prevent new spawns. Keep only a small shared mutex-protected admission flag: under that
same gate, reject closed admission or register tracked ownership before releasing the lock; shutdown closes admission
before cancelling owning operations and closing/waiting on trackers. This gate supplies the one primitive TaskTracker
does not provide, not another task registry. Submission access for ordinary helpers is token-free; host/request tokens
stay with their respective owning tasks.

A dropped request/work future drops its process-result receivers, requesting cancellation at the process seam while
tracked process owners continue to await `wait`. Closing/cancelling an owning scope precedes concurrent tracker drain;
never abort owners or rely on tracker drop to settle them. Choose TaskTracker over `JoinSet` because completed return
values need not accumulate and accidental collection drop must not abort the completion owners.

### 2. Respect the startup ownership boundary

Before launch, check operation cancellation and the closed admission state. While `RunningProcess::spawn` is pending,
race it against the operation's cancellation. If cancellation wins, drop that startup future and return an explicit
startup-cancelled diagnostic; the trusted crate retains responsibility for queued/late children. Do not claim a child
was never created, synthesize a `ProcessReport`, or wait for a stalled launcher. This is a bounded terminal operation
outcome, not proof of completed child cleanup.

After startup succeeds, bridge cancellation to the running handle's own token and continue awaiting `wait`; do not
select cancellation by discarding that wait future. A startup/cancellation race must transfer any returned handle into
that same cancel-and-await path rather than lose it. Preserve `ProcessError`'s startup/execution interruption and
uncertainty when available. A cancelled operation cannot start another probe or check afterward.

Changing the trusted crate to accept a startup token or expose launcher joins is unnecessary and outside this change.
Its documented late-launch/host-exit limitation remains explicit.

### 3. Thread async discovery through its real callers

Make the public root-detection functions and their git probe helpers async, without cancellation-token parameters or an
operation context carrying a token. Keep jj pointer resolution and pure filesystem/PATH lookup unchanged. Update callers
in `checks.rs`, `project_config/runner.rs`, approval persistence in `project_config/approvals.rs`, and
`approval_tui/mod.rs`, plus their existing tests. Do not use a nested runtime, `block_on`, or `spawn_blocking` to
conceal asynchronous process execution.

Distinguish an ordinary probe miss from interrupted/incomplete execution before the outer detection function falls back.
Missing git and normal non-repository results retain today's canonical-path fallback. Cancellation, startup expiry,
supervision/cleanup failure, and truncated capture propagate as execution errors. The checks runner must not route these
errors through its existing broad root-error-to-`NoChecks` branch; preserve that branch only for the ordinary setup
failures it already permits. Never approve or run under a guessed alternative key after an interrupted probe.

This preserves shared-repository approvals and workspace-local execution while removing both blocking probes. Leaving
these helpers synchronous would leave production launch sites outside the requested migration.

### 4. Map supervisor evidence into existing caller results

Follow the spec's "Bounded captures have caller-specific presentation" and "Outcomes and discovery failures remain
truthful" requirements. Construct `ProcessOptions::default()` and inspect `Completion` before constructing existing
output DTOs: only `Exited` reaches `status.code()` (`None` for project output, `-1` for VCS signal exits). Use existing
error envelopes for interruption/incomplete execution, not new `CommandResult` fields or argument schemas.

Presentation consumes capture bytes and truncation flags, then uses existing lossy UTF-8 conversion. The old formatter's
exact discarded-byte count cannot be retained because the supervisor does not provide it; use a flag-based notice. Keep
the existing checks' post-conversion text bound before aggregate accounting. Avoid `Debug` formatting of supervisor
reports, partial state, or errors because their debug representation can contain child output; use safe display plus
explicit phase/reason/uncertainty metadata.

### 5. Apply the existing check policy incrementally

Implement "Check deadlines remain whole-batch limits" with one caller-owned clock and separate queue/completion-owner
bookkeeping. Consume completions incrementally, format before applying the aggregate gate, and retain only permitted
aggregate text plus bounded active captures/one incoming result rather than a collect-all vector.

Record the batch stop reason before cancelling and draining owners. A deadline-triggered cancellation can legitimately
produce supervisor `Cancelled`; the batch still maps to `Blocked` for its deadline, without rewriting the report cause.
Retain that stop reason alongside any cleanup uncertainty. Ordinary nonzero exits still collect remaining check results.
Use the normal tracking drain for settlement, not a second future-dropping timeout or per-check replacement clocks.

### 6. Wire current MCP requests and normal host shutdown

Consume `RequestContext.ct` only at the owning MCP request seam. That owner races cancellation against its ordinary work
future and drops the work future if cancellation wins; nested repository, approval, and verification helpers take no
token or token-carrying operation context. Loss of a process-result receiver requests cancellation at the process seam,
where the independent completion owner still awaits supervision. At batch/scheduling seams, stop before launching the
next unit of work. Do not propagate cancellation policy through every async helper. A narrow execution-submission
capability, if needed by helpers, carries no cancellation token or cancellation authority.

Cancellation of one request must not cancel host shutdown or unrelated requests. Keep protocol constants, advertised
capabilities, schemas, and rejection of newer metadata unchanged; do not use `tasks/cancel` or other newer task APIs.

The three MCP server branches own an execution scope independently of the transport request handlers. On disconnect,
service failure, or graceful signal, close admission, cancel that scope's operations, then drain operation completion
before `McpServers::run` returns. A detached/dropped handler cannot remove its operation from that drain. Keep transport
errors and execution cleanup errors truthful even when no client remains to receive a tool response.

For migrated CLI/hook workflows, handle SIGINT and SIGTERM using the same close/cancel/drain ordering, while maintaining
the runtime. Enable signal handling only for those workflows, not `mcp install`. Ordinary CLI errors also drain owned
work before returning. Preserve command failure exit behavior only after settlement; do not take an early
`process::exit` path with live execution. For approval initialization/cancellation, ensure terminal restoration still
runs when a root probe fails or a signal interrupts it. Do not redesign TUI event handling or unrelated CLI commands.

### 7. Verify Moriarty glue, not the trusted implementation

Extend existing test modules and CLI/MCP fixtures for spec-to-policy mapping: approved program/cwd, forced flags, output
notices/incomplete-output errors, fail-closed discovery, request-local cancellation, stop-admission, deadline scope,
aggregate gates, and waiting before shutdown. Use pure mapping tests or a small private test seam at the operation
boundary for rare outcome/error branches and ordering; no public trait or alternative production backend.

Use existing ordinary-process fixtures only when they demonstrate real integration, with isolated child HOME/XDG values
and their existing bounded test containment. MCP duplex protocol tests can validate existing cancellation and unchanged
version negotiation. Moriarty tests must not construct descendant trees, inject signal/reaper faults, prove PGID
identity, or reproduce the supervisor's watchdog/capture fairness harness. Supervisor internal verification remains in
its own Nextest suite; any newly discovered internal gap is addressed there separately, not hidden in Moriarty tests.

## Risks / Trade-offs

- [New bounded output changes large responses] → User approved 1 MiB per stream; mark the compatibility change, disclose
  project output truncation, and reject incomplete VCS/probe results.
- [Async root detection reaches approval/TUI paths] → Update all callers and keep existing workspace/approval regression
  tests; no sync wrapper that blocks Tokio or escapes supervision.
- [A blanket root-probe error fallback could bypass checks] → Distinguish ordinary absence/setup errors from execution
  interruption and reject the latter before configuration loading or approval persistence.
- [Transport/request ownership can disappear before cleanup] → Register independent operation owners before startup;
  shutdown drains them rather than assuming rmcp handler lifetime equals process lifetime.
- [Five-minute deadline is not a five-minute response bound] → Document execution versus cleanup budgets; cancel active
  operations together and await concurrently, not serially or through a future-dropping timeout.
- [Stalled startup or cleanup can remain uncertain] → Preserve the supervisor's explicit uncertainty and no-retry
  policy; do not promise late cleanup after host exit, eventual reaping, or hard-real-time shutdown.
- [Platform scope narrows Moriarty builds] → Inherit the crate's Linux/macOS support; never provide a silent
  unsupervised fallback on unsupported targets.

## Migration Plan

Apply only after a separate explicit request. Use the implementation groups in `tasks.md`, verifying each small
changeset before moving on: shared execution ownership, async discovery, project commands, check batches, MCP/host
lifecycle, then documentation and full regression verification. Do not claim full migration while any production
launcher remains outside the trusted crate except the installer.

Rollback is a normal revert of the integration changeset; no approval/config data migration is needed. A rollback
restores the old cancellation weakness and unlimited-output behavior, so disclose that limitation. Do not add a runtime
feature flag or parallel legacy backend merely to enable rollback.

## Post-implementation hardening addendum

The preceding design remains the historical migration baseline under the owner's review dispositions. The updated spec's
identity/discovery, persistence, host-input shutdown, and configuration-error requirements specified the completed
section 7 hardening plan and evidence. Section 10 records completed approval/diagnostic and required M1/L1/L2 work;
section 11 records the current additional obligations. Earlier cleanup/style exclusions remain in force.

- Classify Git absence at the shared probe boundary using the canonical cwd captured at entry; revalidating it must not
  conceal a launch failure.
- Keep acquisition/admission cancellation at the owning approval boundary, without threading tokens through helper
  chains. After admission, an independent completion owner retains the load-modify-save transaction, lock, and failure
  evidence after caller loss and participates in graceful host drain. Reuse existing ownership primitives, not a generic
  backend or second process manager.
- Unbounded input/lock syscalls inside a future's poll cannot be cooperatively interrupted. Dropping a blocking task
  does not cancel its I/O or prevent Tokio shutdown from joining it. Exercise actual production host paths, not only
  duplex fixtures; the historical binary/PTY inspection waiver does not verify the new scenarios.
- Validate argv on the direct approval-TUI configuration path before metadata indexing. Use miette's alternate Display
  (`{error:#}`) for safe cause chains rather than Debug-rendering supervisor state; reuse existing MCP error envelopes.
- Keep installer streams, MCP support, supervisor guarantees, and execution deadlines unchanged.

## Section 8 review follow-up (implemented)

Section 8 of `tasks.md` records this completed follow-up and its review dispositions. Its behavioral scenarios and
hook-stdout waiver remain historical context; section 9 records the completed root-pair consistency and CLI-output
follow-up, section 10 records completed approval/diagnostic repairs, and section 11 owns the remaining work.

- Resolve Git using the recorded probe cwd and native PATH semantics, including relative/empty entries and the unset
  PATH default. Preserve inherited environment and program spelling. Recognize known ordinary filesystem-boundary
  absence without broadening all failed probes into absence; unknown diagnostics remain fail-closed.
- Request cancellation must be visible at admission/startup even while the work future is being polled. Keep that policy
  at owning seams and retain independent waits; a later outer-select poll is insufficient. Do not thread tokens through
  ordinary discovery, approval, or verification helpers.
- Settle MCP-owned work without making host exit depend on a client resuming response reads. Address transport/host
  output ownership without aborting process or persistence completion owners. Do not replace the writer wait with an
  output timeout or abandon owned work to force runtime destruction. The hook stdout waiver does not apply to MCP.
- Validate representability when constructing the existing root types, retain their distinct types through approval
  interfaces, and remove redundant preflights. Reuse the current representation and diagnostics, not a new key format or
  generic root framework. Those root-construction paths already failed closed; this consolidated their invariant, not
  the executable-path identity issue now covered by section 10.
- Preserve safe contextual causes before retaining a persistence failure as text; alternate Display can retain causes
  but cannot recover ones already discarded. Keep output-excluding diagnostics and existing first-failure ownership.

Blocked hook response stdout remains an owner-accepted SIGKILL limitation. No hook-output timeout or cancellation
rewrite is required. Document it without promising cooperative cleanup after forceful host termination.

## Root-pair consistency and CLI stdout follow-up (implemented)

Section 9 of `tasks.md` records this completed repair. The shared Stop-hook/MCP checks path captures the canonical
project input once before either awaited discovery, reusing the project-runner pattern. Its synchronized alias-retarget
regression distinguishes approval A/workspace B even when check names, argv and the executable match. This closes that
pairing race, not filesystem TOCTOU generally or the late missing-directory failure covered by section 10.

The owner extended the stdout/SIGKILL limitation to `moriarty test checks` and `moriarty test project-tools`; runtime
documentation records it without an output rewrite or timeout. MCP transport shutdown remains outside the exception, and
forceful termination still proves neither owned-work cleanup nor completed persistence.

## Section 10 follow-up (implemented)

Task 10.1 completed planning alignment against the reports at implementation `9140ef44`. Tasks 10.2–10.9 are complete;
fresh scenario-to-evidence mappings, original-code failures and final review dispositions are recorded in `tasks.md`,
without relabeling earlier evidence. The final five-reviewer batch passed; its three low findings were explicitly
owner-retained. The approval format and trusted supervisor contract remain unchanged. No sync or archive was performed.

- Reject unrepresentable original/canonical executable identities in the shared approval comparison/storage paths,
  before lossy text can define a match. Validated repository/workspace roots alone do not validate binary paths. Keep
  the original verified program `PathBuf` for execution, existing argv/hash binding and relative workspace sharing;
  neither invent a key format nor canonicalize the program used for spawning.
- After initial canonicalization succeeds, a later input-canonicalization failure must block shared Stop-hook/MCP
  discovery even if it is an ordinary filesystem error rather than a typed execution error. Fix the shared classifier,
  not each adapter, while preserving the accepted initial-missing-input result and affirmative ordinary-absence rules.
- Classify approval-file absence from typed filesystem evidence, before diagnostic rendering. Reuse an existing typed
  I/O cause or classify at the file-read boundary; never inspect error text for `NotFound`. Parsing/access failures
  propagate, and the existing locked update must not proceed to save after a failed load. Retain transaction and I/O
  ownership; no new persistence backend, storage migration, rollback or crash promise.
- Preserve safe source chains before composing initiating and cleanup failures into text. Reuse alternate Display at
  this composition seam rather than Debug-printing captures or adding a diagnostic framework. Keep existing error
  envelopes and first-failure retention; do not expand into the deferred duplicate composition/recording work.
- Move the existing inline MCP suite into the gated `mcp/tests.rs` module with test identities and assertions intact.
  Consolidate repeated `ObservedIo` construction without changing the shared execution scope, first-failure slot,
  service token or cancellation wake future across the two halves. Reuse the helper in the existing test wrapper where
  appropriate; introduce no new transport state or configuration. Fix only the scoped M1 imports alongside this work.

Use the existing approval/discovery/settlement tests and operation seams for discriminating regressions, including
unchanged hashes/argv, a synchronized late directory disappearance, malformed approval text containing `NotFound`, and
nested source causes combined with cleanup failure. Retain genuine-absence controls, MCP cancellation/backpressure/EOF
coverage and the trusted test boundary. L1/L2 are required maintenance, not new protocol or runtime behavior. All
earlier owner dispositions, deadline-first polling and the three blocked-stdout exceptions remain in force.

## Current section 11 follow-up

The reports named in section 11 of `tasks.md` concern implementation `ce42bdca`. Its completed section-10 evidence
remains historical; it does not verify the contained-panic scenario or the stronger discovery-cause assertion. These are
application ownership/presentation repairs under existing requirements, not changes to the trusted supervisor.

- Keep the child scope alive across an unwind from its work future. Contain that work panic at the owning seam, close
  admission, request cancellation and await the existing child tracker before publishing the failed outcome. Preserve
  the work failure alongside any cleanup failure or uncertainty, including after caller loss. An unsuccessful operation
  is not by itself evidence of failed cleanup; keep request-locality and unrelated requests unchanged. Reuse existing
  scope/future primitives, not another registry, backend or process manager.
- Test ordering with the existing execution-owner seams: synchronize admission before the panic, hold process or
  persistence completion, prove host drain remains pending, then release completion and inspect failure evidence. Use
  Moriarty-level ownership checks, not a duplicate signal/reaping harness. This covers contained unwinding while the
  runtime lives, not host crash, panic-abort or SIGKILL; existing startup/cleanup uncertainty and budgets still apply.
- Preserve source chains before the shared discovery classifier converts errors into presentation text. Use safe
  alternate Display rather than Debug output; strengthen the existing late-directory-disappearance regression so both
  adapters require the canonicalization context and actual I/O cause. Do not change root pairing, failure-versus-absence
  classification, initial-missing-input behavior, inherited environment or response envelopes.
- Add only the MCP WHY comment: early exits must stay inside the immediately-awaited async block so the following
  close/cancel/drain runs. Preserve the control flow rather than refactor it.

Keep the current review inputs untouched and all recorded owner dispositions in force. Do not expand into token/root
representation, timeout partial-output policy, the persistence seam, renderer work or duplicate settlement/recording
cleanup. Section 11's unchecked implementation and verification tasks require a separate apply request; no sync or
archive is performed during planning.
