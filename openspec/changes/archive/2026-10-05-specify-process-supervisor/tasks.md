# Tasks

These tasks are for later assessment/apply agents, not proposal authoring. No Moriarty caller migration is authorized.
Repairs are conditional on demonstrated violations; conformance does not require manufacturing a Rust change.

## 1. Establish scenario-level conformance evidence

- [x] 1.1 Map the native async API, literal execution, null stdin, bounded binary capture, option validation, and
      cancellation/drop scenarios to the standalone crate's existing source and tests; verify completion with a
      scenario-by-scenario evidence table distinguishing demonstrated behavior, missing checks, and reproducible
      violations.
- [x] 1.2 Map startup stalls/late ownership, execution precedence, TERM/KILL descendant cleanup, abandonment, truthful
      errors, identity safety, host isolation, and output privacy scenarios to the existing launch/driver/process fault
      seams and subprocess tests; verify each scenario has evidence or an explicit unresolved gap, without treating
      Linux-only or indirect checks as stronger evidence.
- [x] 1.3 Run the existing standalone baseline using `cargo nextest run -p process_supervisor` on available supported
      platforms; verify recorded results identify the tested revision, OS, failures, and any untested Linux/macOS
      scenarios rather than claiming cross-platform conformance from one host.

## 2. Repair only demonstrated standalone deficiencies

- [x] 2.1 For each confirmed contract violation, make the smallest focused change within `crates/process_supervisor` and
      pair it with a failing-before/passing-after regression check using existing test seams and independent
      rescue/watchdogs; verify the affected Nextest checks pass and unrelated callers remain untouched. If no violations
      exist, close this task with evidence that no production repair is needed.
- [x] 2.2 For each unsupported scenario, add or strengthen the smallest check in the existing standalone tests, or
      explicitly document why it needs another supported platform or indirect evidence; verify fixture body execution,
      bounded host termination, and identity-safe rescue for subprocess checks. Do not add new harnesses, dependencies,
      or CI wiring without separate approval.
- [x] 2.3 Align the standalone crate's rustdoc with any confirmed repair or clarified boundary; verify the examples and
      API documentation with `cargo doc --no-deps -p process_supervisor` and review that cleanup uncertainty,
      runtime/host limitations, and stream semantics are accurately stated.

## 3. Verify and hand off the standalone contract

- [x] 3.1 Run the configured formatter, build, lint, and Nextest runner after any Rust remediation, plus
      `openspec validate specify-process-supervisor --strict`; verify successful outputs and an unchanged Moriarty
      caller/dependency configuration. For a conformance-only result, report why production build/lint reruns are
      unnecessary and retain the baseline evidence.
- [x] 3.2 Deliver the final scenario evidence and any standalone repair through the required reviewer workflow; verify
      all findings have explicit dispositions, remaining platform/evidence limitations are disclosed, and no unresolved
      contract violation is claimed as complete.
