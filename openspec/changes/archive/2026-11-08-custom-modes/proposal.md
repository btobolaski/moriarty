# Proposal

## Why

Pi-modal supports arbitrary mode names, but Moriarty's permission hooks and rules currently reject every name outside
six Claude Code modes. Moriarty must accept the mode supplied by the pi integration without bypassing its existing
mode-scoped permission decisions.

## What Changes

- Accept custom names in the existing required hook `permission_mode` string, tool/Bash rule `modes` arrays, offline
  `--mode` arguments, and rule regression suite mode lists. Separately owned pi-side wiring will carry pi-modal's
  selected name through the `pi-permission-system` hook transport documented in the setup guide.
- Accept any string containing at least one non-whitespace character; match names exactly and case-sensitively without
  trimming, normalization, aliases, or a custom-mode registry. `manual` is a custom name, not an alias for `default`.
- Preserve the six existing spellings and behavior, omitted/empty rule restriction semantics, eligible-rule ordering,
  and mode propagation through the existing evaluation paths.
- Preserve custom names in completion logs, replay, suggestion mode unions, and mode-aware rule diagnostics instead of
  treating unfamiliar recorded names as mode-less.
- Preserve effective hashes and serialized mode ordering for all previously valid configurations. Keep missing,
  non-string, empty, and whitespace-only live mode values invalid.
- Update mode documentation and existing tests. Unknown non-blank names cease to be validation errors; this
  intentionally removes typo detection based on the old closed list.

## Capabilities

### New Capabilities

- `permission-mode-policy`: Mode identity, validation, rule eligibility, compatibility, and propagation across
  permission hooks, offline evaluation, and recorded-rule workflows.

### Modified Capabilities

None. `bash-path-alias-policy` already requires its policy to apply independently of permission mode and remains
unchanged for custom modes.

## Impact

- Shared `crates/moriarty/src/permission_mode.rs` representation and its callers in hooks, rule configuration,
  `test_runner`, `rules`, and the `test bash-rules --mode` CLI.
- Hook parsing/logging, tool and Bash eligibility, redirect and rewrite evaluation, recorded-mode parsing, replay,
  suggestions, and mode-overlap lint retain one shared interpretation.
- `README.md`, `BASH_RULES.md`, `doc/hooks.md`, pi hook setup guidance, the shared mode type's rustdoc, and the
  CLI/tool-rule/Bash-rule/test-rule manuals need matching custom-mode guidance. After the main spec exists, add
  navigation pointers in `AGENTS.md`, `doc/hooks.md`, and `doc/architecture.md`.
- No dependencies, alternate hook fields, pi-modal implementation, mode registry, new lint categories, or changes to the
  separate closed `claude_logs` transcript mode parser.
