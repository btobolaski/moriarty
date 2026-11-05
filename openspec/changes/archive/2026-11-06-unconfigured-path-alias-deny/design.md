# Design

## Context

See `proposal.md` for motivation and `specs/bash-path-alias-policy/spec.md` for the behavior contract. This is a
cross-module security-policy change, so it needs a design despite adding only one setting.

Observed implementation seams:

- `user_config/mod.rs`: `UserConfig.bash_path_aliases` is a validated `BTreeSet`; `effective_hash` hashes canonical
  serialization. Existing alias/default/hash tests cover backward compatibility.
- `hooks/command_split.rs`: `supported_declaration`, `sole_assignment`, and `resolve_literal_alias_value` already
  recognize the exact leading declaration form. `AliasState.declarations_allowed` closes the prefix at the first
  unsupported item. `split_command` parses once and preserves selected facts across later traversal bailouts.
- `hooks/bash_rules.rs`: `evaluate_original` and `analyze_redirect_check` both use the splitter. The former feeds live,
  test, replay, and suggestion decisions; `evaluate_sync` also owns ArgumentFilter and Modify continuations. Current
  Deny outcomes reference matched rules, whereas alias confirmation reasons only cap Allow at Ask.
- `hooks/mod.rs`: `handle_pretool_hook` applies tool rules first. The Bash handler currently returns Ask immediately
  when Bash rules are absent or empty, before constructing an engine. The engine itself accepts an empty rule list.
- `test_runner/mod.rs` projects the shared evaluation into normal/explain output; `rules/mod.rs` owns configuration
  examples and offline rule tooling. Existing tests already exercise alias expansion and denial rendering.

No active capability currently specifies Bash aliases. The similarly named project-path alias in the process-execution
spec concerns approval identity, not shell variables, and is unrelated.

## Goals / Non-Goals

**Goals:**

- Reuse declaration recognition, not parameter-reference scanning or textual heuristics.
- Represent a policy rejection once so downstream consumers cannot accidentally reduce it to a confirmation prompt.
- Preserve absent/false serialization and evaluation behavior, including old effective hashes.

**Non-Goals:**

- Execute Bash, infer environment-variable intent, broaden alias syntax, inspect hidden declarations in unsupported
  compound constructs, or inspect the filesystem to determine whether a literal is a path.
- Turn this into a general variable-reference prohibition, revise tool-rule ordering, or change ordinary Modify command
  rematching semantics.
- Add dependencies, a parallel parser, new test files, or unrelated configuration strictness changes.

## Decisions

### 1. A sibling top-level boolean with a hash-compatible default

Add `bash_path_aliases_deny_unconfigured: bool` to `UserConfig`, default false, omitted from serialization when false.
Store it beside the alias set in engines constructed from configuration. This keeps omission and false semantically and
hash-equivalent while true participates in the existing effective hash. Retain the current validated/sorted alias set.
Update existing configuration construction sites mechanically as needed.

Do not replace the alias array with a settings table or add a tri-state option: neither is needed, and a table would
break existing policy files. `rules schema` should show the opt-in setting and explain its default.

### 2. Check the declaration name at the existing leading-prefix seam

Separate declaration-shape/value recognition from approved-name membership only as far as needed to reuse the existing
predicate. When enabled, reject the first otherwise eligible declaration with an unapproved name at the point where a
supported declaration could occur. A configured declaration before it must not hide the violation. Do not activate,
expand, or consume an unapproved binding.

Keep the existing prefix termination, duplicate configured-binding treatment, literal character set, cwd requirement,
and explicit terminator checks. Relative literals such as `src` and `1` count; quoted values, command-scoped
assignments, later assignments, and unsupported shell forms do not. No reference/consumption test is added, so an unused
unapproved declaration also rejects the name. Shell-control names remain unconfigurable, not implicitly approved.

The rejection must be a terminal policy fact/result, separate from `requires_confirmation`. Once a candidate has been
recognized, a later traversal bailout cannot discard its denial; an immediate terminal rejection or retained fact is
sufficient. Tokenization/parse errors that prevent candidate recognition keep the current fail-safe path. A scan of all
parameter expansions was rejected because it would also prohibit ordinary `$HOME` and unrelated variable use.

### 3. Carry one policy-level Deny through the shared evaluation

Map the splitter's rejection into a distinct policy denial owned by the completed evaluation rather than invent a
matched user rule. Render the reason once from the rejected name and canonical alias set, for example:

`Unconfigured path alias 'Q'. Supported path-alias variable names: P, ROOT.`

For an empty set, say `No path-alias variable names are supported.` Use the stable synthetic attribution
`bash_path_aliases_deny_unconfigured` where current result/logging contracts require a rule name; do not describe it as
a matched configured rule. This avoids widening every rule-attribution API merely for one policy check.

A recognized violation takes precedence over other Bash outcomes, including configured Denies, so feedback always
contains the supported names. Wire it through ordinary policy analysis and the redirect-only Modify continuation;
ArgumentFilter already re-enters ordinary analysis. Enforce the name check on replacements without adding full
command-rule rematching to Modify. Live evaluation keeps its existing single blocking task and deadline.

Explain and rule tooling should project the same final denial/reason and attribution. Do not add a separate diagnostic
scan or reevaluate policy to display it. Update existing trace/result conversion matches only where needed.

Replay includes rule and reason attribution for every changed-to-Deny divergence, including ordinary configured-rule
denials.

`doc/man/moriarty.1` already describes explain output as including “ordered contributors, and the merged decision the
hook would make” and links to the Bash-rules manual. That wording covers the policy Deny and synthetic attribution
without an additional CLI-manual edit; document the new setting and its semantics in `moriarty-bash-rules.5` instead.

### 4. Preserve tool-rule authority; remove only the enabled-policy empty-rule bypass

Keep `handle_pretool_hook` unchanged in its tool-first ordering. In the Bash handler, retain the empty-rule early Ask
when the new setting is false. When it is true, enter the shared engine even if Bash rules are absent/empty, so a
candidate can be denied; with no violation the existing NoMatch-to-Ask behavior remains. Permission mode does not gate
this configuration-level check. Config-loading failures keep the existing Ask fallback.

A hook-only preflight was rejected because it would diverge from replay, explain, suggestions, and replacement checks.
Moving the check above tool rules was explicitly declined by the user.

## Risks / Trade-offs

- [A scalar can look like a relative path, e.g. `DEBUG=1; command`] → Reuse the existing grammar as approved by the
  user, document that consequence, and test it alongside unchanged `DEBUG=1 command` behavior.
- [Unsupported or non-leading forms can still prompt] → Describe this as an alias-declaration usability policy, not a
  complete shell-variable security boundary; keep existing fail-safe behavior instead of expanding the parser.
- [A broad tool-level Bash Allow bypasses the setting] → Preserve the user-approved precedence and document it plainly.
- [Reusing the confirmation channel would silently produce Ask] → Use a distinct denial outcome and test the full hook
  path, no-rule path, bailout path, and both replacement paths.
- [Serialization changes could fragment recorded history] → Pin the pre-change hash and test omission/false equivalence,
  true changing the hash, and deterministic sorted feedback.

## Migration Plan

Existing policy files require no migration. To opt in, add the setting next to the current aliases:

```toml
bash_path_aliases = ["P", "ROOT"]
bash_path_aliases_deny_unconfigured = true
```

Explain examples should demonstrate an approved declaration, a denied unapproved declaration with the supported-name
list, and the unchanged ordinary-variable/tool-rule boundaries. Remove the setting or set it to false to restore the
previous behavior and effective configuration hash. Implement and validate only after an explicit apply request.
