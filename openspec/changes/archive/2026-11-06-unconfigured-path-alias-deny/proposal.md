# Proposal

## Why

Models invent path-alias variable names instead of using the configured `bash_path_aliases`, causing avoidable manual
approvals. An opt-in deny with the supported names gives the model actionable feedback so it can retry using approved
names.

## What Changes

- Add the top-level boolean `bash_path_aliases_deny_unconfigured` alongside `bash_path_aliases`, defaulting to `false`.
  Omission and explicit `false` preserve existing behavior and effective configuration hashes.
- When enabled, deny an unapproved name in the leading declaration prefix if the declaration otherwise meets the
  existing path-alias grammar. Reuse the current absolute and cwd-relative literal rules, including single-component
  relative values such as `src` or `1`; a later reference is not required to trigger the deny.
- Include the rejected name and the complete, sorted supported-name list in the deny reason. An empty allow-list must
  explicitly say that no names are supported.
- Preserve tool-rule precedence. Enforce the setting whenever Bash analysis is reached, even without `bash_rules`;
  commands without a qualifying violation continue through existing policy, including the empty-rule Ask default.
- Leave ordinary variable references, command-scoped environment assignments, unsupported declaration forms, and
  declarations after the leading prefix unchanged. This is not a general shell-variable allow-list.
- Keep live evaluation and offline Bash-policy consumers consistent, including explain, replay, suggestions, and rewrite
  checks that inspect a replacement command.

## Capabilities

### New Capabilities

- `bash-path-alias-policy`: Configuration and enforcement of approved variable names for leading Bash path-alias
  declarations, including actionable denial feedback and compatibility with existing policy evaluation.

### Modified Capabilities

None. The current active capabilities concern MCP protocol support and process execution/supervision, not Bash
path-alias policy.

## Impact

- Configuration: `crates/moriarty/src/user_config/mod.rs`, its serialization/hash tests, and existing `UserConfig`
  construction sites.
- Analysis and decisions: `crates/moriarty/src/hooks/command_split.rs`, `hooks/bash_rules.rs`, `hooks/mod.rs`, and their
  existing test modules. Preserve the current parser and shared evaluation pipeline rather than add a separate scan.
- Offline diagnostics and policy tooling: `crates/moriarty/src/test_runner/mod.rs` and `rules/mod.rs`, including the
  generated configuration example and existing regression coverage.
- Documentation: `BASH_RULES.md`, `doc/hooks.md`, and `doc/man/moriarty-bash-rules.5`.
- No dependencies, subprocess execution changes, main-spec edits, or implementation code are part of this proposal.
