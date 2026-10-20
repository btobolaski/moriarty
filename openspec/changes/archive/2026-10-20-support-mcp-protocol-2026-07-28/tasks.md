# Tasks

## 1. Upgrade the SDK without widening protocol support

- [x] 1.1 Update `crates/moriarty/Cargo.toml` and `Cargo.lock` to resolve rmcp/rmcp-macros `3.6.0` with the existing
      feature selection, preserving the explicit four-revision list and `2025-11-25` legacy fallback. Update generated
      workspace-hack metadata with existing tooling only if required. Verify the resolved versions, configured build,
      and existing Nextest suite pass before enabling modern support.
- [x] 1.2 Replace `DiscoveryFallbackTransport` with upstream lifecycle-neutral discovery and remove only
      wrapper-specific tests. Preserve all three servers' end-to-end unsupported-discovery/legacy-fallback regressions,
      add recoverable malformed-bootstrap coverage, and verify these tests pass through real `serve_until` without the
      wrapper. Stop if upstream does not satisfy the fallback contract; do not widen the supported list to hide a
      failure.
- [x] 1.3 Update `doc/architecture.md` to explain upstream-owned discovery recovery while retaining the existing cap and
      application-owned settlement/backpressure boundary at this checkpoint. Verify the prose matches the upgraded paths
      and passes the configured Markdown formatter.

## 2. Enable and verify the modern core lifecycle

- [x] 2.1 Add `2026-07-28` to the shared explicit supported set while retaining an explicit `2025-11-25` initialization
      fallback in all handlers; disambiguate the fallback constant's name if necessary. Update per-handler and transport
      assertions, use an unsupported revision such as `2099-01-01` for rejection cases, and verify exact-list discovery,
      all four legacy initializations, and modern/unknown initialization fallback with targeted Nextest tests.
- [x] 2.2 Extend existing `mcp/tests.rs` helpers to send valid modern per-request metadata without initializing. Verify
      successful and rejected discovery followed by either valid lifecycle, direct first-message tool calls, omission of
      optional client info, missing/malformed required fields on first and later requests, and `-32022`
      requested/supported error data. Assert rejected tool calls admit no tool work; run the focused transport tests
      with Nextest.
- [x] 2.3 Extend existing per-server contract assertions to both lifecycles. Verify tool names, argument schemas,
      omitted `project_dir`, Git prompt names/content/roles, modern `resultType: "complete"`, and valid result-level
      cache hints on discovery/tool lists and Git prompt lists without requiring hints on tool calls or Git prompt
      lookups. Preserve jj-read-only/project-tools as tools-only. Verify legacy tool/prompt results omit the modern
      fields; run the affected handler and transport tests with Nextest.
- [x] 2.4 Keep Tasks unadvertised and ordinary tools request/response-only, using SDK capability gating rather than a
      new task manager. Add transport regressions for a task-capable client receiving an ordinary complete tool result
      and for valid `tasks/get`, `tasks/update`, and `tasks/cancel` requests returning `-32601`. Also verify modern
      `ping` is rejected and legacy ping succeeds; run these tests with Nextest.
- [x] 2.5 Preserve the `OwnedService`/`Scope` request-token bridge and admission/drain policy. Extend `PendingTools` and
      existing execution ownership seams to verify cancellation of first and later modern calls, isolation of unrelated
      requests, no new admission after cancellation, awaited settlement on disconnect, and retained cleanup failures
      after response loss. Rendezvous with admitted owners rather than sleeping, and do not wait for a cancelled-request
      response the SDK may suppress. Verify the focused MCP/execution Nextest tests pass without new OS-level cleanup
      harnesses.
- [x] 2.6 Extend existing `tests/host_shutdown_cli.rs` cases with the modern lifecycle. Verify graceful signal shutdown
      with input held open and with response output blocked, alongside the existing legacy cases; keep each child
      process's HOME/XDG environment isolated and run the host-shutdown integration tests with Nextest.
- [x] 2.7 Update `README.md`, `doc/architecture.md`, and `doc/man/moriarty.1` to describe the five supported revisions,
      handshake-free modern metadata versus legacy initialization fallback, and the explicit absence of Tasks. Include
      the protocol contract in architecture's "Contracts and related references", linking the existing change delta
      until main-spec synchronization. At sync/archive, retarget that link to the durable main spec and verify it
      resolves before the change directory is moved. Remove stale ceiling/workaround statements, preserve execution
      limitations, and verify documentation agrees with transport tests and passes applicable formatting checks.

- [x] 2.8 Repair the shared ping-rejection transport boundary so blocked output cannot prevent input EOF detection,
      admission closure, cancellation, or owned-work settlement. Retain consumed rejections across cancelled receive
      futures using bounded, host-owned delivery that shutdown awaits; preserve failure reporting and existing ping
      classification. Add deterministic existing-file regressions for EOF under backpressure and exactly-once delivery
      after receive cancellation, proving both fail before the repair. Keep approved SDK mirrors and the deferred module
      split unchanged.
- [x] 2.9 Restore unsupported-discovery precedence: a parseable unsupported revision receives `-32022` with requested
      and supported data even when other required metadata is absent. Clarify the spec delta, design, and ownership
      documentation; prove error data and subsequent valid legacy/modern startup on all three servers. Delete both
      review documents before verification; do not synchronize specifications or archive this change.
- [x] 2.10 Apply the owner's explicit undeferral of the MCP test-module split. Separate transport, lifecycle,
      settlement, and server-contract tests while retaining shared private fixtures and all existing assertions. Replace
      copied expected-version lists in all three handler tests and wire assertions with one test-only constant
      independent of production policy. Verify unchanged test inventory/ignore flags and the configured Nextest suite.

## 3. Verify the integrated change

- [x] 3.1 Run the configured formatter, build, lint, Nextest suite, and project checks, plus the applicable Markdown
      formatting checks. Verify all succeed, no unrequested dependency/feature/transport changes remain, and every
      scenario in both delta specs has test or explicit verification evidence at the appropriate existing seam.
- [x] 3.2 Inspect the complete diff and affected startup/cancellation/error paths; verify existing approval, VCS,
      capture, deadline, and installer behavior remains intact. Record concrete readiness evidence, then run the
      mandated `openspec-auto-review` workflow for this change and changeset range. Address findings and verify the
      final review passes and `openspec validate support-mcp-protocol-2026-07-28 --strict` succeeds before declaring
      implementation complete.

### Review readiness and owner dispositions

- Before the original implementation's closeout review, the complete diff and startup, cancellation, admission, error,
  approval, VCS, capture, deadline, and installer paths were inspected, and the design's SDK-delegation claims were
  checked against the ping exception. Configured formatting, build, lint, all 2,033 Nextest tests (26 skipped), project
  checks, Markdown formatting, Hakari verification, and strict OpenSpec validation pass.
- Direct wrapper tests cover forwarding, legacy/context-free pass-through, the three rejection codes, and failed
  rejection delivery. All-server integration checks separately prove lifecycle recovery. A mutation that continued after
  a failed rejection write failed the queued-request regression; restoring closure passed it. Policy rejection retains
  both discovery positions and recovery lifecycles; context-only cases no longer repeat that cross-product.
- The owner approved retaining full metadata validation, first/later cancellation, argument-schema assertions,
  all-server signal coverage, and independent expected-version lists rather than the reviewers' proposed reductions.
  Documented SDK key/error mirrors, the man-page cross-reference, this readiness record, and panic-on-polled rejected
  work are also owner-approved. Test-module decomposition and specification synchronization/archive remain deferred. The
  owner also approved leaving task 3.2 unchecked during review, assessing substantive findings first, and checking it
  only after those findings clear and strict validation passes.
- Independent-review closeout: run `19e4aa64-2ce5-4dfc-83c9-4c882ab67e05` returned tests/docs/complexity PASS. Code
  review's sole medium finding requested the already owner-deferred module split; its low metadata-boundary cases were
  added and reverified. Minor test-reduction suggestions remain covered by the coverage decisions above. The verifier
  mapped all 10 requirements and 28 scenarios and flagged only the intentionally open task 3.2 at that snapshot. The raw
  aggregate FAIL is preserved; its remaining blocking findings are covered by those explicit owner dispositions, not
  represented as an unqualified passing reviewer verdict.

### Transport-repair readiness

- The prior checks covered rejection classification with writable output and completed receives, but not a dropped
  receive waiting for the writer; discovery rejection tests supplied complete metadata. New regressions reproduce all
  three follow-up defects against the unchanged guard (Nextest run `f7c28630-13eb-437a-87d5-23e0aa049bf4`): blocked
  output hides EOF/owner cancellation, cancelled receive loses reply ID 2, and incomplete discovery returns `-32602`
  instead of `-32022`.
- `guarded_ping_backpressure_does_not_hide_eof_or_owner_settlement` uses writer/owner gates, keeps response reads
  stopped, verifies closed host/request admission, and retains delayed cleanup uncertainty after owner release.
  `guarded_ping_rejection_survives_receive_cancellation_exactly_once` drops the receive while another write holds the
  lock; a subsequent writer is a delivery barrier without another receive, and the complete output contains the
  rejection exactly once. No sleeps synchronize either regression.
- `incomplete_unsupported_discovery_recovers_on_all_servers` pins revision error/data and both recovery lifecycles
  across all three servers; malformed revision metadata still returns `-32602`. Direct guard cases also cover
  unsupported discovery with malformed capabilities. Immediate/queued failure and queue-exhaustion tests verify retained
  diagnostics, closed admission, and awaited sender teardown.
- Inspected the complete repair diff, every guard branch, SDK bootstrap/select/writer-lock/drain paths, sender transfer
  before a yielding await, transport-close ordering, and host settlement after failed/cancelled startup. The sender is
  bounded (64 queued plus one active), tracked, and joined; no per-rejection task is detached. `OwnedService`/`Scope`,
  approvals, VCS restrictions, captures, deadlines, and the unsupervised installer remain unchanged.
- Configured formatter/build/lint/checks pass, including 2,038 Nextest tests with 26 skipped; focused MCP/execution/host
  shutdown tests passed 71/71. The seven guard regressions passed five Nextest `--stress-count` iterations. Hakari,
  applicable Markdown formatting, and strict OpenSpec validation pass. LSP probes were inconclusive, so compiler and
  test results supply verification. Both review documents were deleted before any verification. Task 3.2 was reopened
  for the repair review, with the existing owner dispositions preserved.
- Repair review `833093a2-fa02-4127-9351-953ba5bdc1b2` completed all five reviewers with no workflow errors. Code, test,
  documentation, and complexity reviewers PASS; the verifier mapped all 11 requirements and 33 scenarios and
  independently passed the full gate. Its only high finding was the intentionally open task 3.2. The raw aggregate FAIL
  is retained under the approved review-time checkbox sequence, not rewritten as an unconditional passing verdict.
- With explicit owner approval, all six low findings are resolved: renamed the broader boundary to
  `ProtocolGuardTransport` and anchored its rationale, imported the poll macro, extracted shared teardown/parking/wire
  helpers without changing cancellation ordering or assertions, extended the design coverage matrix, retained deferred
  main-spec synchronization, and recorded ping pass-through in the delta. Final self-review checked the full diff,
  helper drop timing, and related documentation tables. All 2,038 tests and the configured build/lint/checks pass again;
  the active strict Clippy run supersedes historical cached status. The seven focused tests also pass five stress
  iterations after helper extraction. No further review is required for low-only fixes; task 3.2 is closed after
  substantive findings and strict validation clear. SDK mirrors and the test-module split remain as owner-approved;
  synchronization/archive are still deferred.

### SDK mirror-drift verification

- `guarded_ping_optional_context_errors_match_sdk` uses SDK setters for clientInfo-only/logLevel-only metadata and raw
  bootstrap tools/list/ping requests; both must return `-32602` with SDK-derived matching messages. Temporary wording
  and both optional-key classification mutations each failed; production mirrors were restored. Focused coverage passes
  72/72; this test, `protocol_guard_transport_classifies_requests`, and
  `bootstrap_ping_obeys_metadata_and_preserves_lifecycle_recovery_on_all_servers` pass five stress iterations. All 2,039
  Nextest tests pass (26 skipped), as do configured formatting/build/lint/checks, Hakari, and strict validation. Scope
  remains ping-guard SDK key/error-mirror drift coverage only; all owner waivers are unchanged.

### Test-organization follow-up

- The owner subsequently undefers the module split and requires a shared expected-version constant. This supersedes only
  the earlier split deferral and approval of repeated expected-version lists; other owner dispositions and raw
  historical review outcomes remain unchanged.
- `mcp/tests.rs` now retains shared private fixtures and the independent test-only expected revision list. Its
  `tests/transport.rs`, `tests/lifecycle.rs`, `tests/settlement.rs`, and `tests/contracts.rs` contain the existing 41
  tests. Test inventory and ignore attributes match the baseline after accounting for module paths; moved declarations
  retain their original assertions and rendezvous ordering. All 2,039 Nextest tests pass with 26 skipped. Configured
  formatting, build, all-target lint, and project checks (fmt/check/Semgrep/Clippy/Nextest) pass, as do Markdown treefmt
  idempotency and strict OpenSpec validation.
- Test-refactor review pass 1 (`234ecfeb-5a75-4a10-bef9-6fbad504de8b`) passes all five reviewers with no errors or
  medium-or-higher findings. The verifier maps all 15 tasks, 11 requirements, and 33 scenarios and independently passes
  Nextest, build/check/Clippy/rustfmt/Hakari/Semgrep. Its supplemental LSP probes are inconclusive, and its Markdown
  formatting rerun is permission-blocked; it relies on the parent's successful treefmt evidence, not a claimed rerun.
- The owner explicitly defers the two low advisories to parameterize the existing large tests and reconcile crate-wide
  shared-helper guidance with the MCP-local expected-version constant. The remaining low advisory is resolved by
  recording the full verification gate above. No production or normative specification changes were made;
  synchronization/archive remain deferred.
