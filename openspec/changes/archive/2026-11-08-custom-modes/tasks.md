# Tasks

## 1. Open the shared mode representation

- [x] 1.1 Extend existing `user_config/mod.rs` tests with one rule listing all six built-in modes in scrambled order
      with duplicates, asserting one fixed pre-change canonical serialization and effective hash rather than per-mode
      tests. Keep the existing omitted-mode baseline; verify both baselines pass under Nextest before changing the
      representation.
- [x] 1.2 Extend `permission_mode.rs` with `Custom(CustomModeName)`, whose private string payload has no unchecked
      construction, mutation, or deserialization access; the shared parser maps built-ins to named variants and
      constructs only valid custom payloads. Replace rename-based enum Serde with `try_from = "String"` /
      `into = "String"` conversion attributes using the shared validated parser and exact display spelling; replace
      closed clap enumeration and adapt callers for loss of `Copy` without changing evaluation flow. Replace type
      rustdoc with the identity invariant (exact, case-sensitive non-blank names without trimming), retain Claude
      Manual's `default` wire mapping, and remove obsolete clap-metadata comments. Verify the payload's
      construction/privacy boundary, `cargo check`, and existing module tests for built-in/custom Serde and CLI round
      trips, exact wire strings, `manual != default`, case/padded-name distinction, eligibility, and overlap. Test blank
      rejection once in the parser and non-string rejection once in its Serde adapter; do not add audit-only tests or
      repeat the invalid-name matrix at each boundary.
- [x] 1.3 Extend existing configuration tests for custom restrictions on both tool and Bash rules, deterministic mixed
      ordering, rule-list de-duplication, omitted/empty restrictions, and hash changes; verify with Nextest that the
      fixed built-in and omitted-mode baselines from 1.1 remain unchanged.
- [x] 1.4 Update the permission-mode sections in `BASH_RULES.md`, `doc/man/moriarty-tool-rules.5`, and
      `doc/man/moriarty-bash-rules.5`; verify they describe exact free-form names, blank rejection, `manual` as distinct
      from `default`, canonical ordering, and the intentional loss of closed-list typo detection without promising a
      registry. Review `rules/mod.rs` schema/starter examples and verify their existing round-trip tests still pass: the
      schema uses valid built-in names and the starter omits mode restrictions, so neither needs new example text.

## 2. Verify live and offline mode continuity

- [x] 2.1 Extend existing hook parser and `hooks/tests.rs` cases to accept a custom `permission_mode`, select a
      restricted rule, and log the exact value; retain the boundary-specific missing-mode rejection. Verify the focused
      tests under Nextest using existing isolated environment helpers, relying on 1.2 for the shared invalid-name
      contract.
- [x] 2.2 Adapt existing mode-threading tests in `hooks/tests.rs` and `hooks/bash_rules_tests/` to exercise custom names
      through tool precedence/fallback, locality eligibility, compound/bail evaluation, redirect rules, Modify redirect
      validation, and ArgumentFilter rechecks; verify focused Nextest cases preserve decisions and mode-independent
      path-alias denial without duplicating the complete existing rule-engine suite.
- [x] 2.3 Extend `test_runner/mod.rs` and `test_runner/rules_suite_tests/mod.rs` tests for custom normal/JSON/explain
      CLI evaluation, per-mode suite expansion and exact report labels. Replace the obsolete unknown-name rejection with
      acceptance; verify duplicate/empty suite lists and unrelated strict fields still reject, while omitted modes
      remain mode-less, without repeating the shared parser's invalid-name cases. Compare CLI and live Bash decisions
      only where no tool rule decides first.
- [x] 2.4 Update `README.md`, `doc/hooks.md`, `doc/getting-started-rules.md`, `doc/man/moriarty.1`,
      `doc/man/moriarty-test-rules.1`, and the rule-suite guidance in `BASH_RULES.md`; update README's "exact Claude
      Code modes" wording to include custom names. Explicitly update `moriarty(1)`'s SYNOPSIS mode alternation and
      Hooks-section `modes` description, and verify CLI help and every closed mode list describe the free-form contract.
      Distinguish pi-modal mode selection from `pi-permission-system` hook transport, and describe the connecting
      pi-side wiring as separate work, not implemented here.

## 3. Preserve recorded modes and rule-authoring behavior

- [x] 3.1 Use the shared parser in `hooks/report.rs` so unfamiliar valid names remain modes; extend existing tests for
      custom and case-distinct aggregation plus missing/blank historical modes. Update `HookRecord.permission_mode`
      rustdoc to explain that missing or blank, not unfamiliar valid, modes restrict replay to unrestricted rules.
      Verify focused Nextest coverage preserves public `hooks report` grouping/JSON and existing malformed-record
      behavior.
- [x] 3.2 Extend `rules/mod.rs` tests for custom-mode replay eligibility and divergence labels, suggestion unions of
      built-in/custom names, and the mode-less contributor fallback; verify Nextest assertions retain exact names
      without adding a second evaluator or changing historical cwd/hash filtering.
- [x] 3.3 Extend existing lint tests for exact custom-mode overlap/disjointness, redirect coverage, and empty
      restrictions; verify strict lint accepts unobserved valid names and retains existing diagnostics without a
      registry or new lint kind.
- [x] 3.4 Update recorded-mode, replay/suggestion, and lint guidance in `BASH_RULES.md`, `doc/hooks.md`, and the
      relevant sections of the manuals already touched; verify no guidance still says unfamiliar non-blank hook/rule/log
      modes are rejected or degraded to mode-less. Leave the separate `claude_logs` transcript contract unchanged.

## 4. Integrated acceptance

- [x] 4.1 Run the configured formatter, build, lint, and checks from `.config/tools.toml`, including Cargo check,
      Semgrep, Clippy, and Nextest; verify all succeed and inspect the final diff for accidental schema, precedence,
      hashing, or unrelated changes. Use Nextest rather than `cargo test` and add no dependencies, test files, or
      build/CI wiring.
- [x] 4.2 Run `openspec validate custom-modes --strict`, self-review the implementation against every delta requirement,
      and complete the required `openspec-auto-review` workflow for this change; verify findings are resolved according
      to project review rules before requesting archive or reporting implementation complete.

## Workflow follow-up

- After an explicitly requested spec sync/archive creates `openspec/specs/permission-mode-policy/spec.md`, add its
  navigation link to the `AGENTS.md` "Where to read" table, the spec-pointer paragraph in `doc/hooks.md`, and the
  "Contracts and related references" list in `doc/architecture.md`; verify all three links resolve. Do not link the
  nonexistent main spec during implementation or sync/archive without a user request.
