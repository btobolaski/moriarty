# Tasks

## 1. Configuration and compatibility

- [x] 1.1 Add the default-false, false-omitted `bash_path_aliases_deny_unconfigured` boolean to `UserConfig`, thread it
      into configured engines, and update existing struct fixtures as required; verify existing construction sites
      compile and Nextest tests in `user_config` cover omitted/false equivalence, a pinned pre-change hash, true
      changing the hash, and invalid non-boolean values.
- [x] 1.2 Show the opt-in setting beside `bash_path_aliases` in the existing `rules schema` example and document its
      default, empty-list behavior, scalar-relative-value consequence, and rollback in `BASH_RULES.md`, including its
      Trusted Policy and Best Practice 4 guidance; verify the existing schema round-trip test passes and the example
      deserializes with the intended enabled value.

## 2. Shared declaration rejection and decision propagation

- [x] 2.1 Reuse the declaration predicate in `hooks/command_split.rs` to recognize unapproved candidates at eligible
      prefix positions without changing disabled behavior; extend existing splitter tests to verify absolute, relative,
      single-component, newline-terminated, approved-prefix-then-unknown, unused, and later-bailout cases, plus the
      excluded forms and unchanged ordinary references described in the spec.
- [x] 2.2 Carry a distinct policy Deny through `hooks/bash_rules.rs`, including ordinary analysis and
      Modify/ArgumentFilter continuation checks; extend existing engine tests to verify precedence over competing
      actions, deterministic complete feedback, empty aliases, approved alias behavior, disabled-policy behavior, and
      rejection in replacements. Verify result/contributor conversions expose the stable synthetic policy attribution
      without a fabricated matched rule.
- [x] 2.3 Update the configured path-alias and canonical-evaluation notes in `doc/hooks.md`; verify they distinguish
      policy Deny from confirmation caps, describe prefix/bailout limits and shared continuation handling, and preserve
      the documented single-task live-evaluation deadline and existing Modify rematching boundary. Update the Rule-set
      provenance bullet to name the new boolean; verify it states that false is omitted and enabling it changes the
      hash.

## 3. Hook integration and offline presentation

- [x] 3.1 In `hooks/mod.rs`, allow the enabled policy to run when Bash rules are absent or empty while preserving the
      tool-rule short-circuit; extend existing hook tests to verify supported-name Deny output, no-rule Ask when no
      violation exists, tool-level Allow/Ask/Deny precedence, and all supported permission modes. Verify offline
      mode-less analysis also enforces the policy.
- [x] 3.2 Adapt existing diagnostic/result projections in `test_runner/mod.rs` and rule tooling only where needed;
      extend existing normal/explain, replay, and suggestion tests to verify the shared alias Deny and reason rather
      than a separate scan, and confirm replacement checks use the same feedback. Verify live and offline results agree
      for the same cwd/configuration/mode and preserve tool-first versus Bash-only entry-point boundaries.
- [x] 3.3 Update `BASH_RULES.md` usage examples and `doc/man/moriarty-bash-rules.5` with the setting, denied-name
      feedback, leading-only grammar, unchanged ordinary references, and tool-rule precedence. Reconcile the guide's
      Best Practice 4 “unsupported forms prompt” wording and the manual's “unused declaration” Allow-to-Ask paragraph
      with the unchanged configured-alias cap versus the new unconfigured-name Deny; verify documented examples against
      the completed implementation using `moriarty test bash-rules --cwd <dir> --explain` with an isolated policy
      configuration.

## 4. Integration verification

- [x] 4.1 Run the configured formatter, build, lint, and tests through project tools, including the configured checks
      (`fmt`, `check`, `semgrep`, `clippy`, `tests`); verify every result succeeds and tests use `cargo nextest`, never
      `cargo test`. Format/check only touched Markdown with the existing Prettier/treefmt configuration.
- [x] 4.2 Inspect the complete implementation diff and affected execution paths, confirm every scenario in
      `specs/bash-path-alias-policy/spec.md` has suitable existing-module regression coverage, and record concrete
      readiness evidence; verify `openspec validate unconfigured-path-alias-deny --strict` passes and run the mandated
      `openspec-auto-review` workflow for this change before reporting implementation complete.

## 5. Review corrections

- [x] 5.1 Preserve the alias-policy Deny when Modify redirect attachment fails, for direct and ArgumentFilter-to-Modify
      continuations. Extend the existing continuation tests with static trailing-semicolon replacements, retained
      redirects, cwd-dependent relative aliases, exact feedback and policy attribution, and invalid-attachment Ask
      controls without an alias violation.
- [x] 5.2 Share the redirect-check denial/Modify/Ask decision between both continuation paths without changing
      attribution or adding full command-rule rematching to Modify.
- [x] 5.3 Replace the replay label snapshot with semantic content checks while retaining exact reason, typed
      attribution, and JSON assertions.
- [x] 5.4 Qualify the attachment-failure and confirmation-bearing rewrite prompt claims in `doc/hooks.md` and
      `doc/man/moriarty-bash-rules.5` with the enabled alias-policy Deny exception.
- [x] 5.5 Retain the documented private redirect-analyzer name and explain why the missing-cwd relative replacement and
      multi-leaf controls preserve Ask.
