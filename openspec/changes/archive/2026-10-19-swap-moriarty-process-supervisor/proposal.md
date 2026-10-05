# Proposal

## Archive closure

Archived on 2026-10-19 with 52/52 tasks complete after owner authorization. Earlier pending-work and no-sync/no-archive
statements describe their planning revisions, not the archive's current status.

At the owner's request, strict-mode cleanup regrouped long requirements in both `moriarty-process-execution` and
`process-supervision` without changing behavior, Purpose text or scenarios. The owner chose this closure record instead
of an additional `process-supervision` delta.

Historical references to "Bounded captures have caller-specific presentation" correspond to "Bounded project captures
and exit decisions" and "Complete VCS output and compatible presentation". References to "Outcomes and discovery
failures remain truthful" correspond to the execution-outcome, discovery and diagnostic requirements. The original
completed-task evidence and historical citations remain unchanged.

## Why

Moriarty previously launched external tools through several independent, mostly uncancellable execution paths; the
checks runner dropped process futures on timeout without supervising child cleanup. The migration uses the existing
trusted `process_supervisor` crate to own those lifecycles. The current follow-up addresses remaining application-level
ownership and failure-reporting gaps without changing that trusted contract.

## What Changes

- Route every production external-process launch through `process_supervisor`, except `moriarty mcp install`. This
  includes approved project commands/checks, git/jj read-only MCP tools, and git repository/workspace-root probes.
- Make subprocess-backed root detection asynchronous so its owning operation can cancel it, including during approval
  loading and approval TUI initialization. Keep cancellation at operation boundaries, not threaded through helper
  signatures. Preserve approval identity, verified program spelling, literal arguments, cwd rules, environment
  inheritance, read-only flags, and response shapes.
- Connect existing MCP request cancellation, client disconnect, and graceful SIGINT/SIGTERM shutdown to supervisor
  cancellation. Stop admitting work and await active operation completion before destroying the runtime.
- Keep the current MCP protocol ceiling and supported versions unchanged; introduce no task-management APIs or new
  tools.
- **BREAKING**: Previously unlimited project-command and VCS captures become bounded at the supervisor default of 1 MiB
  per stream. Command/check output discloses truncation; VCS tools and root probes reject incomplete captures.
- Preserve existing execution-deadline policy: no new execution timeout for project commands or VCS probes/tools; the
  Stop-hook/MCP checks retain their five-minute whole-batch deadline, four-way concurrency, and 10 MiB aggregate output
  policy. All migrated launches use the supervisor's default startup and cleanup budgets.
- Treat `process_supervisor` as trusted. Moriarty tests verify integration, policy, and result mapping, not the
  supervisor's process-group, reaping, signal, or capture implementation. Any missing internal assurance belongs in the
  supervisor's own tests, not a duplicate Moriarty harness.

## Post-implementation hardening

The migration and sections 7–10 follow-ups are implemented through `ce42bdca`, with their point-in-time evidence and
owner dispositions preserved in `tasks.md`. Section 10 addressed the executable-path identity, late-discovery
classification, approval-load and diagnostic-composition defects reported against `9140ef44`; some mechanisms predated
the migration. Section 11 records the current ownership and discovery-diagnostic follow-up. This planning revision
refines the same capability without changing the trusted supervisor contract or authorizing code implementation.

- Preserve filesystem identity through root discovery and approval keys; limit lossy text conversion to presentation.
  **BREAKING**: unexpected Git/root-resolution failures and identities the existing approval format cannot represent
  losslessly now require errors rather than successful fallback or approval-key aliasing.
- Make graceful shutdown independent of idle MCP stdin, unfinished hook input, and approval-lock contention. Retain
  ownership of admitted approval persistence and its lock through actual I/O completion rather than dropping a save
  future while detached filesystem work continues.
- Reject empty approval argv with actionable errors and preserve safe diagnostic source chains. Add discriminating
  caller-policy, persistence, and real host-input shutdown evidence; do not duplicate supervisor-internal assurance.

The implemented section 8 follow-up preserves native Git lookup under the probe cwd and unset PATH, and ordinary absence
at filesystem boundaries; prevents additional admission/startup after request cancellation; makes MCP signal shutdown
independent of response backpressure; and retains persistence error causes after caller loss. Representability
validation is consolidated in the existing root identities without changing the approval format.

The implemented section 9 root-pair repair keeps shared Stop-hook/MCP check discovery anchored to one canonical project
input so a retargeted alias cannot mix A's approval identity with unrelated B's configuration and execution directory.
That pairing repair does not cover a later failure to canonicalize a disappeared input directory.

The owner accepts that blocked stdout writes from `moriarty hooks exec`, `moriarty test checks`, and
`moriarty test project-tools` can require SIGKILL: no output-write timeout or graceful exit is promised for that
condition. This narrow exception does not waive MCP shutdown, admission/cancellation, owned-work settlement, or the
existing host-crash limits.

## Section 10 follow-up (implemented)

The historical inputs were `reviews/xhigh-npkozszx-to-at.md` and
`reviews/swap-moriarty-process-supervisor-independent-review.md`, assessed against `9140ef44`. Under the approved apply
request, both were kept unedited/unformatted through implementation, then deleted before final review. Findings and
historical verdicts remain recorded in `tasks.md`.

- **BREAKING**: reject original/canonical executable-path approval identities the existing format cannot represent
  losslessly, at verification and storage, rather than authorize a lossy collision with unchanged argv and hash.
- Block late discovery/canonicalization failures after successful initial canonicalization through both Stop/MCP
  adapters; preserve the accepted initial-missing-input behavior and ordinary absence separately.
- **BREAKING**: use an empty-store fallback only for genuine file absence. Malformed/unreadable stores remain errors,
  and a failed load cannot lead to overwriting the file with a default store.
- Preserve available source causes when combining initiating and cleanup errors, not just when retaining a persistence
  failure after caller loss; continue excluding captured output.
- Fix the scoped import violations, extract existing MCP tests to `mcp/tests.rs`, and consolidate `ObservedIo`
  construction without changing transport behavior. These are required M1/L1/L2 tasks, not new behavioral requirements.

`tasks.md` preserves the 38 completed implementation tasks in sections 1–9 and their point-in-time evidence. Task 10.1
completed the planning alignment; tasks 10.2–10.9 are complete with fresh discriminating evidence, verification and
review dispositions. The final implementation review passed all five reviewers; its three low findings were explicitly
retained by the owner. Results and verification limits are recorded in `tasks.md`. Renderer overflow and duplicate
settlement composition/failure recording remain deferred; fixing source-chain loss does not reopen them. No sync or
archive was performed.

## Current section 11 follow-up

The inputs are `reviews/swap-moriarty-process-supervisor-changeset-review.md` and
`reviews/swap-moriarty-process-supervisor-xhigh-review.md`, assessed against implementation `ce42bdca`. Keep both
reports byte-identical, including formatting and retention. Their findings and REJECTED verdict remain historical
inputs, not passing evidence for this follow-up.

- Retain child-scope ownership across a contained work-future panic, cancel admitted operations and await their outcomes
  before request/host drain completes. Cover process and approval-persistence ownership while the runtime remains
  available; do not expand crash, panic-abort or SIGKILL guarantees.
- Preserve discovery context and its underlying filesystem cause through the shared Stop-hook/MCP failure conversion,
  without changing fail-closed classification, ordinary absence, initial-missing-input behavior or error envelopes.
- Correct the stale task introduction and require one WHY comment explaining how MCP's async-block early exits still
  reach the subsequent drain. The comment is maintenance, not new runtime behavior.

These repairs enforce existing settlement and diagnostic requirements. The delta adds one contained-panic scenario and
strengthens the existing late-canonicalization scenario, not new requirement paragraphs. Section 11 separates planning
alignment from pending repair, regression and verification tasks; earlier completed tasks/reviews do not verify these
new assertions. The change is not archive-ready until that work is complete. Existing owner dispositions, deferred
cleanup and the trusted-supervisor boundary remain unchanged; renderer work is not added to this scope. No sync or
archive is authorized.

## Capabilities

### New Capabilities

- `moriarty-process-execution`: Moriarty's use of trusted process supervision, including cancellation ownership,
  caller-specific policy, truthful result mapping, and the MCP-install exception.

### Modified Capabilities

The `process-supervision` main spec received the wording-only restructuring recorded above during archive closure. Its
standalone guarantees and scope remain unchanged; the migration consumes that contract rather than changing its
behavior.

## Impact

- Code: `crates/moriarty/src/{repository.rs,checks.rs,main.rs}`, `project_config/{runner.rs,approvals.rs}`,
  `approval_tui/mod.rs`, `test_runner/mod.rs`, and the MCP execution/hosting modules. Hardening also reaches
  `hooks/mod.rs` host input and `persistence.rs` approval I/O ownership. Add only the minimal shared execution/lifecycle
  glue needed by these callers; retain the existing approval format.
- Dependencies: add the existing workspace `process_supervisor` path dependency and the user-approved direct
  `tokio-util = "=0.7.18"` dependency with `rt` for TaskTracker, reusing the supervisor's pinned version. Update the
  lockfile as needed and enable existing Tokio signal facilities. No rmcp upgrade, new crate version, or supervision
  backend. Sections 10–11 require no additional dependencies.
- Tests: adapt existing Moriarty unit/CLI/MCP coverage at those seams; retain approval, workspace, path/flag validation,
  and ordinary exit-code regressions. Move the existing inline MCP suite to `crates/moriarty/src/mcp/tests.rs` without
  replacing its coverage. Test fixture setup processes are not production migration targets.
- Documentation: update execution limits/cancellation in `README.md`, `doc/man/moriarty.1`, affected module rustdoc, and
  architectural guidance in `CLAUDE.md`; remove stale claims that Moriarty callers are not migrated from the supervisor
  crate's rustdoc. Preserve the standalone spec and installer behavior.
- Platform/lifetime: inherit the supervisor's Linux/macOS boundary and documented cleanup uncertainty, startup
  uncertainty, deliberate group-escape exclusions, and host-crash limitations; this is not a sandbox.
