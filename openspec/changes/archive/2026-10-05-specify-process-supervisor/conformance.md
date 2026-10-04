# Process-supervision conformance evidence

## Round 2 — current remediation

- Baseline implementation reviewed by the new verification report: `yvkwpspm` / `0e89982e`. Follow-up changes are in
  working change `qoulypwr`, based on that implementation. This section supersedes the round-1 handoff below.
- New supplied reviews: `reviews/process-supervisor-changeset-review.md` (direct structural review, REJECTED) and
  `reviews/process-supervisor-xhigh-review.md` (workflow `3cf61685-dff1-4e59-81cd-538772b805ab`). They were added after
  round 1; its old removal/acceptance statements did not describe this newer checkout. Tasks 2.1–3.2 were reopened.
- The verification report's critical host-group defect is confirmed. Passing historical checks did not establish the
  missing live host-group invariant. The supplied standalone run `bee38b17-09d4-4a8b-bbcc-2c0470a94bad` passed 44 cases
  with 22 ignored fixture skips; this was not an independent rerun by the remediation agent.

### Dispositions of supplied findings

1. **Critical host-group rejection** (verification report and xhigh #1): fixed in the shared `Owner::identity` boundary.
   Observation/probes and every destructive attempt, including EINTR retries, compare the saved identity to the current
   host group. A match rejects the operation and permanently revokes authority. TERM, final KILL, Drop and late cleanup
   use this boundary; no driver-specific bypass was added.
2. **Startup timing** (verification suggestion and xhigh #2): shared fault options now have maximal startup allowance,
   under a separate bounded outer wait. Only explicit admission/handoff/pipe timeout scenarios select `SHORT_STARTUP`.
   Cancellation and ordinary pipe-failure checks cannot expire under the former shared 100 ms startup budget.
3. **Interruption representation** (changeset #1, high): the user explicitly approved the structural refactor. Active
   cause owns the complete `Interruption`; `PartialState.interruption` is a projection at error construction, not a
   parallel live decision. Admission returns a typed reason; launch failures carry creation certainty and reason as
   explicit outcome fields; one startup constructor projects both, removing the repeated classification from mutable
   metadata.
4. **Capture lifecycle** (changeset #2, medium): user-approved private `Lifecycle` holds `Open(reader)`, Uninitialized,
   Eof or Failed. Reader presence and terminal state cannot contradict each other; public EOF is derived only when
   producing `Capture`. Existing capture/error checks continue to cover limits, bytes and truthful EOF.
5. **Placeholder phase** (changeset #3, low): pipes and final startup checks run before constructing `Driver`; the
   initial phase is real Running or Final. One shared final-transition helper preserves KILL metadata and pipe release.
6. **Associated constructor** (changeset #4, low): user-approved `RunningProcess::spawn` replaces the free constructor;
   standalone callers/tests and rustdoc were updated. No Moriarty production caller was migrated.
7. **Unit-fixture environment helper** (changeset #5, low): retain the recorded owner deferral. Existing unit-site
   isolation stays inline; integration helper reuse remains intact. No additional shared-test module was authorized.
8. **Locality timeout race** (xhigh #3, plausible/pre-existing): the user explicitly approved the out-of-scope test-only
   repair in `crates/moriarty/src/hooks/tool_rules.rs`. It now reuses `with_saturated_blocking_pool`, keeping the task
   queued until timeout observation instead of racing a 25 ms sleep. Production locality behavior is unchanged.
9. **Finalization delay** (xhigh #4, confirmed): `Driver::kill` immediately performs nonblocking finalization instead of
   waiting another process poll to reap an already exited leader. Existing post-KILL bounds and revocation still apply.
10. **Stale evidence** (verification warning): historical sections are explicitly round 1, not current acceptance. This
    section records the newer reviews, scope approvals, actual regressions and the current handoff status.

### Runnable evidence

- Contained host-group regression: existing wrapper (now `host_fixtures_exit_normally`) drives the exact ignored
  `process::platform::tests::host_group_change_revokes_signals` fixture. Only that host starts as a non-group-leader so
  it can join an owned child's group. Every child has independently exited with a checked body marker before the join;
  direct-host rescue uses std Child with bounded waits, independent of Owner under test. Before the fix, run
  `dbd0f2b9-cc47-4378-9f46-9bd077e982e7` failed because the isolated host received SIGTERM (15). After the fix,
  standalone run `2bbe6299-3191-4a2b-ab5e-79af7243a13a` passed 44 cases with 23 fixture skips. The strengthened fixture
  initially checked TERM, KILL, Drop, late cleanup and a real group join injected between observation and signaling;
  traces prove no destructive operation, and leaving the group does not restore authority. The subsequently approved
  retry extension is recorded below.
- `cleanup_uncertainty_retains_the_interrupted_phase` now covers both reasons in both phases using the primary cause;
  the existing confirmed pipe-failure regressions still pass. Startup classification remains creation-aware.
- `final_transition_reaps_without_another_poll` failed before the immediate-finalization fix in run
  `49c4f156-8869-4cb8-88bd-3f8fdefc0b2c`; it now proves synchronous status/group/EOF completion and signal ordering
  without a timing threshold. Standalone run `ffea9779-c988-48b6-96be-f6ae8de36a34`: **45 passed, 23 skipped**.
- Locality test targeted run `b53aade8-16b4-446f-bb30-79f688b6b740`: **1 passed**; filter-excluded cases were not run.

### Configured verification and cleanup

- Configured formatter, build, Clippy and full workspace Nextest passed. Run `b36300d2-6b21-411c-8498-4d3137850d60`:
  **1842 passed, 25 skipped**. Skips remain ignored fixtures; the added host fixture is exercised by its wrapper, not
  counted as an additional standalone passing test.
- `cargo doc --no-deps -p process_supervisor` and strict OpenSpec validation passed. Markdown is formatted with treefmt.
- The active LSP probe returned three inactive-cfg hints and nine inconclusive file checks; no clean LSP result is
  claimed. Build/Clippy/Nextest provide compiler evidence for both production and test configurations.
- Both new review documents were actually deleted after the required changes and configured verification; `reviews/` was
  checked empty. Their identifiers and all supplied dispositions are retained above.
- No dependency, workspace/lockfile, CI, MCP or Moriarty production caller changed. The sole Moriarty edit is the
  separately approved existing locality-timeout test.

### Round-2 auto-review cycle 1

Workflow `4815b933-45cf-4cb3-893d-44c92182d550`, range `@`: **FAIL**. Four reviewers completed, one failed review, eight
findings (one medium, seven low), no high/critical findings, no workflow errors. Code review confirmed the safety and
state fixes but rejected test-helper placement; tests, documentation and complexity passed.

All cycle-1 findings have dispositions before cycle 2:

- Medium capture-helper placement: `is_open` now lives in an impl inside the existing gated test module, not the
  production impl. No extra production API or test-support file was introduced.
- Low identity-error provenance: the retry closure retains the original identity error while preserving every live
  recheck. The existing between-observation-and-signal case now asserts the identity diagnostic; run
  `745e24f0-ad11-4602-bc9b-998569c806e7` failed before this repair with synthetic EINVAL, and the repaired standalone
  suite passed (`8ccf3c74-1cca-41db-af36-efcaa71048f8`, 45 cases, 23 fixture skips).
- Low duplicated startup-option choice: both launcher scenarios reuse one reason-specific options helper.
- Low direct startup-classification coverage: one result-module test covers all four typed outcomes and deliberately
  stale input metadata, requiring both correct error category and projected reason/creation certainty.
- Low phased-fixture observation: retain the expressly accepted five-trigger loop, with no additional trigger or phase.
- Low rustdoc wrapping: the host-reaping sentence is manually wrapped; Rustfmt does not reflow this raw doc string.
- Low startup-constructor complexity: one match extracts facts from `StartupFailure`; immutable facts select the error,
  never mutable PartialState. The closed certainty type is retained rather than replaced with a boolean policy input.
- Low fixture group-selection complexity: the group choice is a table column, not a comparison against marker text.

Configured formatter, build, Clippy and full workspace Nextest passed again after all feedback: run
`bab6c747-a869-4413-82b4-3df9bb0cc027`, **1843 passed, 25 skipped**. Rustdoc generation passed. At that checkpoint,
handoff was pending cycle 2 and task 3.2 remained open.

### Round-2 auto-review cycle 2 and approved polish

Workflow `d0fcb4e0-cd57-437e-bf24-9d49190a1c02`, range `@`: **PASS**. All four reviewers completed and passed, zero
failed reviews, four low findings, no medium/high/critical findings or workflow errors. Code and documentation reported
zero findings. Test review reran the full workspace successfully (1843 passed, 25 skipped).

After consuming cycle 2, the user explicitly chose to implement all optional follow-ups rather than retain the accepted
trade-offs. This supersedes the cycle-1 retention choices above. All four low findings are addressed:

- The existing Script seam now has a per-attempt signal-error queue. A sixth trigger injects EINTR on the first signal
  attempt and joins the real host to the supervised group before returning that error. The retry must reject identity,
  preserve the original diagnostic and permanently revoke authority. The single Term trace entry is the simulated failed
  attempt, not a successful destructive syscall; no second signal attempt occurs. The contained wrapper passed before
  simplifying the already-correct retry implementation (`26e08b73-4e47-48e6-aac5-9be66009fb13`). No new harness or test
  file was added.
- The wrapper is now named `host_fixtures_exit_normally`, covering all four host fixtures without a stale two-scenario
  name. Targeted run `0391bbf3-3bdc-40c1-b854-ffa94d5c1bd9` passed.
- `LaunchOutcome::Failure { unconfirmed, reason }` carries explicit facts directly; the extra `StartupFailure` enum and
  destructuring match are removed. Startup classification still uses those facts, not mutable PartialState. The direct
  mapping test was adapted and renamed `startup_failure_projects_outcome_facts_over_stale_metadata`.
- Signaling uses a direct loop with a fresh identity check before each attempt, at most two retries (three total
  attempts), and direct propagation of identity errors. The synthetic EINVAL protocol and side-channel Option are
  removed. ESRCH remains successful absence. Standalone post-polish run `85337221-b23e-4e3e-b04f-857a44ea6681`: **46
  passed, 23 fixture skips**.

Both authorized auto-review cycles have been used; no third batch was launched. The PASS describes the cycle-2 reviewed
snapshot, before these expressly approved low-priority follow-ups.

### Final round-2 verification and handoff

- After all approved polish, configured formatter, build, Clippy and full workspace Nextest passed. Run
  `7b2096fe-57ae-41d5-bc7c-943b1c82a9b2`: **1843 passed, 25 skipped** (expected ignored subprocess fixtures).
- Rustdoc generation and strict OpenSpec validation passed again. Final task/evidence edits are documentation-only; the
  configured formatter and strict validation were rerun for those edits. Code verification above covers the final Rust
  sources without another redundant test/build run for the bookkeeping.
- The fresh six-path LSP probe gave three inactive-cfg hints and four inconclusive checks, not clean-file confirmation.
  Build, Clippy and Nextest supply the compiler evidence; no current type/error diagnostic remained in that probe.
- `reviews/` is empty. Scope remains the standalone crate, its existing tests/docs and the expressly approved single
  Moriarty test repair; jj's final scope check showed no dependency, lockfile, CI or production-caller changes.
- All supplied findings and both reviewer cycles have explicit dispositions. Task 3.2 is complete and all eight tasks
  are checked. No known demonstrated violation from the supplied reports remains unresolved.
- Runtime evidence remains Linux-only; macOS is not verified. Socket EOF does not establish orphan-zombie reaping or
  group absence, and live host-group checks do not provide atomic exclusion against concurrent job-control changes.
- The OpenSpec change is deliberately **not archived**, per the user's instruction.

## Round 1 — historical baseline

- Pre-remediation assessment snapshot: working copy `yvkwpspm` / `7663007e`, parent `lmrpznwr` / `4a32ff3c`. Only the
  two supplied review documents differed from the parent before assessment.
- Platform: Linux x86_64, kernel `6.18.45-xanmod1`. macOS is not available on this host and is **not runtime-verified**.
- Command:
  `cargo nextest run -p process_supervisor --no-fail-fast --hide-progress-bar --success-output never --status-level fail --final-status-level flaky`.
- Nextest run `2a205857-a283-4036-9d1a-cfd7cf9456ba`: **41 passed, 19 skipped**. Ignored tests are subprocess fixtures
  invoked explicitly by wrapper tests, not independently executed Nextest cases.
- A passing baseline does not disprove the concurrent pipe-setup/interruption defect or the PID-1-dependent
  orphan-zombie test failure described below.

## Round 1 — initial scenario assessment

Paths below are relative to `crates/process_supervisor`. “Demonstrated” means the named runnable check exercises the
behavior on Linux; source evidence alone and simulated OS failures are identified separately. Initial gaps are retained
here so the remediation can be distinguished from baseline coverage.

| Spec scenario                                     | Source and existing check                                                                                                                                                                                                                                                                                                                         | Initial assessment                                                                                                                                                         |
| ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Cancel while another task waits                   | `driver.rs`: token plus independently spawned driver; `tests/supervision.rs`: `term_kill_descendants_and_continuous_output`, `dropping_owners_cancels_but_does_not_abort_cleanup_or_other_groups`                                                                                                                                                 | Cancellation demonstrated; add an explicit separate waiter task.                                                                                                           |
| Startup acknowledgement                           | `driver.rs`: `prepare` registers both pipes before sending acknowledgement; `driver/tests/launcher.rs`: `pipe_setup_failures_preserve_stage`                                                                                                                                                                                                      | Demonstrated: failed registration cannot return a running handle.                                                                                                          |
| Literal arguments and program identity            | `launch.rs`: std `Command` without shell/canonicalization; `tests/supervision.rs`: `literal_program_cwd_environment_and_live_host_stdin_are_preserved`; `driver/tests/launcher.rs`: `missing_program_preserves_spawn_stage`                                                                                                                       | Demonstrated symlink spelling, literal shell syntax, cwd/environment; raw non-UTF-8 argv checked at the command seam.                                                      |
| Host stdin and protocol output untouched          | `launch.rs`: null/piped streams; `tests/supervision.rs`: `real_statuses_and_null_stdin`, `host_contract`/`child_contract`                                                                                                                                                                                                                         | Stdin EOF and host sentinel demonstrated. Add an explicit host-output exclusion assertion.                                                                                 |
| Both streams exceed limits                        | `capture.rs`: fixed read buffer and discard; `tests/supervision.rs`: `both_streams_remain_bounded_while_draining`, flood mode of `term_kill_descendants_and_continuous_output`                                                                                                                                                                    | Demonstrated bounded retention, both truncation flags, cancellation under continuous output.                                                                               |
| Exact and zero limits                             | `capture.rs`: `byte_bounds_cover_zero_exact_and_discard_cases`                                                                                                                                                                                                                                                                                    | Demonstrated raw invalid-UTF-8/NUL retention, exact-limit nontruncation and zero-limit drain to EOF.                                                                       |
| Partial binary output fails before EOF            | `capture.rs`: `failed_read_disables_stream_without_inventing_eof`; `driver/tests/phases.rs`: `failed_capture_settles_without_inventing_eof_or_hiding_cleanup_uncertainty`                                                                                                                                                                         | Partial text and false EOF demonstrated; strengthen retained partial bytes to include invalid UTF-8/NUL.                                                                   |
| Invalid or unallocatable capture limit            | `options.rs`: `setters_validate_capture_limits_and_accept_independent_graces`; `driver/tests/launcher.rs`: `capture_allocations_fail_fast_before_launch`                                                                                                                                                                                          | Demonstrated oversized rejection and both pre-launch allocation-failure boundaries; add exact `isize::MAX` boundary assertions.                                            |
| Zero and maximal durations                        | `launch.rs`: elapsed-time startup deadline with capped sleeps; `driver.rs`: saturating phase budgets; `driver/tests/launcher.rs`: `interrupted_startup_never_launches`, `blocked_admission_is_cancellable`; `driver/tests/phases.rs`: `fresh_exit_observation_precedes_pending_control`, `term_phase_preserves_its_cause`, cleanup failure checks | Demonstrated zero startup/execution/TERM/post-KILL and non-overflowing maximal startup/cleanup budgets; no actual enormous wait is required.                               |
| Independent cleanup graces                        | `options.rs`: `setters_validate_capture_limits_and_accept_independent_graces`; `driver/tests/phases.rs`: independently selected phase graces                                                                                                                                                                                                      | Demonstrated independent `Duration::MAX` values and phase handling.                                                                                                        |
| Cancellation before waiter polls                  | `tests/supervision.rs`: `execution_timeout_and_pre_poll_cancellation`, repeated cancellation and token-clone drop in lifecycle test                                                                                                                                                                                                               | Demonstrated persistence/idempotence and clone-drop independence.                                                                                                          |
| Dropped owner/completion future                   | `tests/supervision.rs`: `dropping_owners_cancels_but_does_not_abort_cleanup_or_other_groups`                                                                                                                                                                                                                                                      | Demonstrated cancellation after owner drop/task abort and isolation of another live group.                                                                                 |
| Dropped startup future                            | `driver/tests/launcher.rs`: `dropped_spawn_cleans_late_children_during_creation_and_send`; `driver.rs`: async body and drop guard                                                                                                                                                                                                                 | Polled drop demonstrated with explicit handoff closure and launcher completion; add literal unpolled-drop check.                                                           |
| All creation capacity stalled                     | `launch.rs`: private process-wide four-slot admission; `driver/tests/launcher.rs`: `cancelled_empty_handoff_retains_launcher_ownership`, `blocked_admission_is_cancellable`; `driver/tests/hosts.rs`: stuck-launch host                                                                                                                           | Demonstrated using a private one-slot seam: capacity retained after interruption, another request times out, runtime remains responsive.                                   |
| Child created after caller stops waiting          | `driver/tests/launcher.rs`: `dropped_spawn_cleans_late_children_during_creation_and_send`                                                                                                                                                                                                                                                         | Demonstrated creation/send stalls, late cleanup, reaping and admission release. Children independently exit; no real unkillable fixture.                                   |
| Child queued at interruption                      | `driver/tests/launcher.rs`: `interrupted_handoffs_preserve_known_spawn_and_cleanup_outcomes`                                                                                                                                                                                                                                                      | Demonstrated queued success/failure with cancellation and startup timeout; no fabricated creation uncertainty.                                                             |
| Exit observable at control boundary               | `driver/tests/phases.rs`: `fresh_exit_observation_precedes_pending_control`                                                                                                                                                                                                                                                                       | Demonstrated with scripted running/exited/failed observation, cancellation and timeout; observation order, not historical exit time.                                       |
| Child exits during cancellation cleanup           | `driver/tests/phases.rs`: `term_phase_preserves_its_cause`; cooperative lifecycle mode                                                                                                                                                                                                                                                            | Demonstrated fixed selected cause despite exit and later cancellation.                                                                                                     |
| Execution clock excludes startup                  | `driver.rs`: clock reset after registration                                                                                                                                                                                                                                                                                                       | Source evidence only; add a clock-order assertion at the pipe-setup seam.                                                                                                  |
| Descendants ignore TERM                           | `tests/supervision.rs`: tree/ignore modes of `term_kill_descendants_and_continuous_output`                                                                                                                                                                                                                                                        | Demonstrated real TERM/KILL and grandchildren on this host. Existing absence assertions wrongly assume prompt orphan reaping.                                              |
| Leader exits leaving silent descendants           | `tests/supervision.rs`: `naturally_exited_root_does_not_leave_background_descendants`, root-exits-closed mode                                                                                                                                                                                                                                     | Demonstrated on this host; existing unconditional success/group-absence assertions contradict allowed orphan-zombie uncertainty. Add independent socket-liveness evidence. |
| Output ends before process                        | `driver/tests/phases.rs`: `fresh_exit_observation_precedes_pending_control` with EOF captures and scripted live observation                                                                                                                                                                                                                       | Demonstrated at the OS fault seam: EOF cannot complete a live job, which waits for execution timeout/cancellation. Not a real live-process EOF fixture.                    |
| Unkillable/unconfirmable child                    | `driver/tests/phases.rs`: scripted failures, stuck cleanup; `driver/tests/hosts.rs`: unconfirmed-kill host                                                                                                                                                                                                                                        | Demonstrated bounded abandonment through fault seams, not a real uninterruptible kernel process. Real D-state tests would violate containment.                             |
| Orphan zombies prevent group absence              | `process.rs`: disarm before reaping; `driver.rs`: shared post-KILL deadline                                                                                                                                                                                                                                                                       | Source evidence; add scripted reaped-child/present-group check and allow this truthful outcome in real descendant tests. PID-1 timing is host-dependent.                   |
| Capture cannot reach EOF                          | `driver/tests/phases.rs`: `failed_capture_settles_without_inventing_eof_or_hiding_cleanup_uncertainty` holds a writer open                                                                                                                                                                                                                        | Demonstrated shared cleanup budget, partial bytes, unchanged false EOF.                                                                                                    |
| Nonzero exit with confirmed cleanup               | `tests/supervision.rs`: `real_statuses_and_null_stdin`                                                                                                                                                                                                                                                                                            | Demonstrated actual code 101 and signal status as ordinary successful reports.                                                                                             |
| Reader failure after confirmed cleanup            | `driver/tests/phases.rs`: failed capture check; `capture.rs`: failed read check                                                                                                                                                                                                                                                                   | Demonstrated supervision error, retained bytes, closed reader, false EOF rather than cleanup uncertainty.                                                                  |
| Pipe setup fails during startup interruption      | `driver.rs`: `cause.is_none()` skips interruption after setup failure                                                                                                                                                                                                                                                                             | **Reproducible violation** via existing pipe hook cancelling and returning an error. Add combined cancellation/timeout regression and repair shared boundary.              |
| Lost driver or multiple failures                  | `driver/tests/hosts.rs`: runtime-loss host; `driver/tests/phases.rs`: `scripted_errors_preserve_initiating_failure_and_signal_order`, `cleanup_uncertainty_retains_the_interrupted_phase`                                                                                                                                                         | Demonstrated missing evidence on lost channel, original diagnostics and phase metadata preserved during additional failures.                                               |
| Ownership lost before signaling                   | `process.rs`: `late_cleanup_records_signal_failures`, scripted `ECHILD`; `driver/tests/phases.rs`: scripted errors                                                                                                                                                                                                                                | Demonstrated revocation with no TERM/KILL syscall after failed ownership observation.                                                                                      |
| Reaping/group identity reuse                      | `process.rs`: `reaping_view_permanently_revokes_signal_authority`, `invalid_identity_cannot_signal_even_in_drop`; `driver/tests/phases.rs`: `confirmed_group_absence_is_monotonic`                                                                                                                                                                | Demonstrated signal-order/revocation/monotonicity through traces; does not force kernel PID recycling. `exited_owner` needs body-marker evidence.                          |
| Unrelated child remains host-owned                | `tests/supervision.rs`: `supervision_preserves_host_group_and_unmanaged_child_waits`                                                                                                                                                                                                                                                              | Demonstrated host group and unrelated wait ownership. Indirect SIGCHLD evidence only; no direct inspection of signal disposition/subreaper flags.                          |
| Runtime destruction during stalled/failed cleanup | `driver/tests/hosts.rs`: `hosts_exit_normally_after_stuck_launch_and_unconfirmed_kill` and three exact host fixtures                                                                                                                                                                                                                              | Demonstrated bounded normal host exit and truthful lost-driver results. Stalls/unkillability injected; no required launcher join.                                          |
| Sensitive output on failure                       | `result.rs`: `diagnostic_display_keeps_metadata_but_excludes_captured_output`; `process.rs`: `late_diagnostic_event_excludes_captured_bytes`                                                                                                                                                                                                      | Demonstrated display/event privacy; add missing signal-attempt metadata to display.                                                                                        |

## Round 1 — supplied findings

All seven supplied findings are valid and initially unfixed:

1. Add the qualification rationale beside the exact rustix pin; do not change dependency versions or wiring.
2. Explain why the execution clock is reset after pipe setup, and check that ordering.
3. Include TERM/KILL path-entry flags in diagnostic display while retaining output privacy.
4. Preserve startup cancellation/timeout alongside pipe-setup failure at the shared startup boundary.
5. Accept only reaped, EOF-complete, diagnostic-free orphan-group uncertainty in real descendant scenarios;
   independently establish fixture death without requiring external zombie reaping. Rescue must remain identity-safe and
   bounded.
6. Require `fixture-body` output in `exited_owner` so zero-test libtest success cannot satisfy ownership checks.
7. Isolate explicit fixture `Command` HOME/XDG values, retaining temporary directories through child lifetime.

## Round 1 — remediation evidence

All seven supplied findings are addressed without API, dependency-version, caller or MCP changes:

- `prepare` now checks missing startup interruption metadata even after pipe-setup failure. New
  `failed_pipe_setup_preserves_startup_interruption` covers cancellation and timeout together with pipe failure. Before
  the repair, Nextest run `b1a57eae-ebe0-4597-bd4d-493e65108a3f` failed with `StartFailed` and `interruption=None`;
  after the repair, run `20894adf-0265-4bf4-8fdb-09d09118eda4` passed all three pipe-setup checks. The existing
  uncertain-cleanup phase check now preserves the pipe diagnostic and false EOF alongside startup cause.
- Real descendant and owner-drop scenarios run through the existing independent `Host` watchdog mechanism. Exact body
  markers and expected connection counts establish dispatch/registration; watchdog socket EOF establishes fixture death
  separately from zombie reaping. Tests only allow cleanup uncertainty with a reaped child, both EOFs, no stage
  failures, absent group-absence proof and the expected interruption. Forced rescue preserves the socket read half and
  still reaps the direct host within bounded waits. The scripted capture/cleanup check also covers a reaped child with a
  group that remains observable, requiring uncertainty and no destructive signals after reaping.
- `exited_owner` captures and asserts `fixture-body` before ownership fault injection. Explicit fixture commands have
  isolated HOME/XDG values; lifecycle directory subtrees remain under the outer guard until death checks/rescue finish.
  The leaf asserts its inherited isolation values.
- The rustix declaration records why its safe NOWAIT/group-signal behavior requires requalification on upgrade. The
  post-setup execution-clock reset has a WHY comment; `execution_clock_starts_after_pipe_setup` compares the running
  timestamp with the hook's recorded completion, using maximal startup/execution allowances.
- Display includes true/false TERM/KILL path-entry metadata. Existing diagnostic checks still exclude both captures.
- `dropping_unpolled_startup_never_launches` proves no allocation/launch work begins on an unpolled drop. Option checks
  now cover both streams at exactly `isize::MAX` and one byte above it. Partial failed capture includes invalid
  UTF-8/NUL. The live-host contract check captures host stdout and excludes the child's `stdin-eof` marker.
- Post-remediation standalone run `236572ab-16cf-4c42-a6c5-e9ffc7309169`: **44 passed, 22 skipped** on the Linux host
  above. Three additional ignored host fixtures wrap existing lifecycle scenarios; their bodies are checked by markers
  and expected watchdog registration counts. No new test harness, test file, dependency or CI wiring was added.

## Round 1 — configured verification

- Configured project-tools formatter (`cargo fmt`), build (`cargo build`), lint
  (`cargo clippy --all-targets -- --deny warnings`) and Nextest runner all succeeded.
- Full workspace Nextest run `8c4aa212-7d5c-4195-885e-a3ca38488d0f`: **1841 passed, 24 skipped** on Linux. This includes
  the strengthened unrelated-child wait check while a supervised job is cancelled.
- `treefmt --ci` on the changed Markdown and manifest: four files checked, zero changes.
- `cargo doc --no-deps -p process_supervisor`: succeeded; crate API links and usage documentation rendered without
  warnings.
- `openspec validate specify-process-supervisor --strict`: valid.
- Active LSP probe: two inactive-cfg hints and eight inconclusive file checks; no clean LSP result is claimed. The
  successful compiler/Clippy runs supply the type-check evidence.
- Changes remain confined to the standalone crate and its related guidance/evidence/tracking. No Moriarty launch site,
  workspace manifest, lockfile, dependency version, MCP configuration or CI wiring changed.

## Round 1 — auto-review dispositions

Auto-review pass 1 (`8f6c8fdf-c58b-492f-a1fa-546abbb71efa`, range `@`) completed all four reviewers without workflow
errors: code PASS, tests FAIL, documentation PASS, complexity PASS. Findings: one medium and six low; no high/critical.

- The medium test finding and duplicate low complexity finding about repeated HOME/XDG setup are addressed by the
  user-approved private `isolate_home` helper at all five integration-test sites. No isolation behavior/lifetime
  changed. Configured formatter, build, Clippy and full Nextest rerun succeeded: run
  `0c8ba70e-fb4d-437c-8237-40c4d6401603`, **1841 passed, 24 skipped**.
- The low-priority leaf-isolation assertion is retained as the reviewer explicitly permitted; it is a guard on fixture
  containment wiring, not claimed as additional production coverage. This pattern was not extended.
- The low-priority suggestion to combine oversized-limit probes is not taken, as the reviewer explicitly allowed
  retaining per-stream distinction. A combined oversized stdout/stderr input would still fail if stderr validation were
  accidentally removed; the independent probes protect both contractual bounds.
- The low-priority baseline-hash note is addressed by explicitly labelling the hash as the pre-remediation assessment
  snapshot, not the final working-copy hash.
- The low-priority temporary-directory suggestion is addressed by clarifying that cwd is the outer Host's isolated
  directory, not the repository. `new_in` is load-bearing: it puts the kept subtrees under that guard for removal after
  fixture-death checks/rescue. Using default temp directories with `keep` would leak them outside the guard.
- The two original round-1 documents (`changeset-review-process-supervisor.md` and
  `xhigh-review-lnxnytru-to-working-copy.md`) were removed before its final reviewer batch. `reviews/` was empty then,
  not after the newer round-2 reviews were added; their current dispositions are recorded above.

Auto-review pass 2 (`58a5d600-8ad0-4d4c-bb61-322b919a9b57`, range `@`) completed all four reviewers without workflow
errors: code PASS, tests PASS, documentation FAIL, complexity PASS. Findings: two medium and three low; no
high/critical. The user approved continuing beyond the two-review checkpoint.

- The two medium documentation findings and duplicate low code finding about pending review-record removal/stale
  REJECTED status are resolved by the actual deletion above; removal is no longer a future bookkeeping assertion.
- The low code finding about tree-mode group-probe skips is addressed with a site-level WHY comment naming orphan
  zombies and independent socket EOF evidence.
- The low test finding about a bare unexpected-variant panic is addressed by preserving the actual ProcessError Display
  diagnostic, which excludes captured output.
- Configured formatter, build, Clippy and full Nextest rerun succeeded after these fixes: run
  `b17a77b8-60a2-4715-b248-480e313bd387`, **1841 passed, 24 skipped**.

Auto-review pass 3 (`4bd6851e-af31-4462-b2df-ba10c9078b62`, range `@`) **PASS**: all four reviewers passed; four
completed, zero failed reviews, five low findings, no medium/high/critical findings, no workflow errors. Code review
confirmed the minimal startup fix and coherent ownership flow; test review confirmed coverage and passing Nextest;
documentation review confirmed consistency; complexity review found no unnecessary layers.

- The duplicate low code/test recommendations for a cross-module unit-fixture environment helper were explicitly
  deferred by the user. The integration helper remains; the two single-site unit-test blocks stay inline. This is an
  accepted maintainability choice, not an unresolved contract violation.
- The low code finding about unused piped host stdout is fixed: only `host_contract`, whose stdout is inspected, uses a
  pipe; other host fixtures use null stdout and cannot stall on an undrained output pipe.
- The low test finding about the host stdout marker scan is retained as explicitly permitted by the reviewer and user.
- The low documentation observation about the pre-fix scenario row requires no change per the reviewer: the table is
  explicitly initial assessment, followed by the completed remediation evidence.
- After the low-only stdout-pipe fix, configured formatter, build, Clippy and full Nextest all succeeded again: run
  `a8f5664f-2f1b-4e36-82e1-2770666cb74b`, **1841 passed, 24 skipped**. Those round-1 findings had dispositions, but that
  handoff missed the live host-group safety defect subsequently reported and repaired in round 2.

## Evidence boundaries

- Rustdoc generation is not doctest compilation or execution; no doctest result is claimed.
- macOS must run the same portable Nextest suite on a macOS host; Linux results do not establish Darwin runtime
  behavior.
- `/proc/self/fd` descriptor assertions are Linux-only. No equivalent macOS descriptor check is claimed.
- OS fault seams establish control-flow and evidence invariants, not the ability to kill a genuinely uninterruptible
  child.
- Socket EOF establishes that participating fixtures no longer hold their live watchdog connections, not that orphan
  zombies were reaped or their process groups disappeared. Group absence remains a stricter supervisor completion fact.
- Synchronous tracing subscribers may block and retain launcher capacity; this documented host-callback limitation is
  not a nonblocking logging promise. Host crash/exit before creation returns and intentional group escape remain
  excluded.
