# Tool & Bash Rules Configuration Guide

Moriarty provides a powerful tool call validation system that allows you to control which tools and commands Claude Code
can execute. **Tool rules** permission any tool call (Read, Write, Edit, Bash, etc.), while **bash rules** provide
command-level validation specifically for Bash tool calls.

## Table of Contents

- [Quick Start](#quick-start)
- [Tool Rules](#tool-rules)
- [Configuration File](#configuration-file)
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
pattern = "^ls( -[a-zA-Z0-9]+)?( [^|&;$`]+)?$"
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
field = "field_name"        # Optional: field in tool_input to match
pattern = "regex-pattern"   # Optional: regex pattern for the field value
action = { type = "ActionType", ... }
```

- **name**: A descriptive name for the rule (used in logs)
- **tool**: Exact tool name to match (e.g., `"Read"`, `"Write"`, `"Edit"`, `"Bash"`, `"Glob"`, `"Grep"`), or `"*"` to
  match any tool
- **field** + **pattern**: Optional pair. When both present, the regex `pattern` matches against the named field's value
  in `tool_input`. When absent, the rule applies to any invocation of the tool. If only one is present, the rule is
  skipped (configuration error, logged).
- **action**: `Allow`, `Deny`, or `Ask` (see [Rule Actions](#rule-actions)). Note: `Modify` and `ArgumentFilter` are
  Bash-specific and not available for tool rules.

### Field Pattern Matching

When `field` and `pattern` are specified, Moriarty extracts the field value from the tool input:

- **Strings**: matched directly (e.g., `file_path`, `content`)
- **Numbers**: converted to string (e.g., `42` → `"42"`)
- **Booleans**: converted to string (`true`/`false`)
- **Arrays/Objects/Null**: cannot be matched (rule doesn't match)

**CWD prefix stripping**: Claude Code sends absolute paths in tool inputs (e.g., `/home/user/project/src/main.rs`).
Before regex matching, Moriarty strips the hook input's `cwd` prefix from field values, so rules can use relative paths.
For example, with `cwd = "/home/user/project"`, a field value of `/home/user/project/src/main.rs` becomes `src/main.rs`
for matching purposes. If the value doesn't start with `cwd`, it's matched as-is.

### Evaluation Order

```
PreToolUse event (any tool)
  |
  +-> tool_rules engine (first-match-wins)
  |     Match found? -> return Allow/Deny/Ask
  |     NoMatch? -> continue
  |
  +-> tool_name == "Bash"?
  |     Yes -> bash_rules engine (existing behavior)
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

## Configuration File

Bash rules are configured in `~/.config/moriarty/tool_rules.toml`. Rules are evaluated in order with
**first-match-wins** semantics - the first rule that matches a command determines the action.

### Basic Structure

```toml
[[bash_rules]]
name = "descriptive-name"
pattern = "regex-pattern"
action = { type = "ActionType", ... }
```

- **name**: A descriptive name for the rule (used in logs)
- **pattern**: A regular expression pattern to match commands
- **action**: What to do when the pattern matches (see [Rule Actions](#rule-actions))

### Rule Evaluation Order

Rules are evaluated top-to-bottom. The first matching rule determines the action:

```toml
# This rule is checked first
[[bash_rules]]
name = "deny-dangerous-docker"
pattern = "^docker\\s+system\\s+prune"
action = { type = "Deny", value = "Docker system prune is dangerous" }

# This rule is only reached if the command doesn't match the first rule
[[bash_rules]]
name = "allow-other-docker"
pattern = "^docker"
action = { type = "Allow" }
```

**Important**: Place more specific rules before general ones!

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

Replace dangerous flags with safer alternatives:

```toml
[[bash_rules]]
name = "rm-force-interactive"
pattern = "^rm .*-f"
action = {
  type = "ArgumentFilter",
  remove = ["-f", "--force"],
  add = ["-i"],
  reason = "Replaced force mode with interactive"
}

[[bash_rules]]
name = "allow-rm-interactive"
pattern = "^rm .* -i$"
action = { type = "Allow" }
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

| Fragment        | Expansion                                      | Description              |
| --------------- | ---------------------------------------------- | ------------------------ | ------------------------------------------- | -------------- |
| `safe_chars`    | `[^                                            | &;$\`()<>{}]`            | Characters that don't allow shell injection |
| `identifier`    | `[a-zA-Z_][a-zA-Z0-9_-]*`                      | Valid identifier pattern |
| `number`        | `[0-9]+`                                       | Numeric values           |
| `safe_arg`      | `( [^                                          | &;$\`()<>{}]+)`          | Safe command argument                       |
| `safe_flag`     | `( -[a-zA-Z_][a-zA-Z0-9_-]*)`                  | Safe command flag        |
| `safe_path`     | `( [^                                          | &;$\`()<>{}]+/[^         | &;$\`()<>{}]\*)`                            | Safe file path |
| `safe_pipe_cmd` | `(head\|tail\|grep\|wc\|sort\|uniq)`           | Safe pipe commands       |
| `safe_pipe`     | `( \\\| (head\|tail\|grep\|wc\|sort\|uniq)( [^ | &;$\`()<>{}]+)\*)`       | Safe command piping                         |

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

## Security Best Practices

### 1. Block Shell Injection Characters

Always exclude shell metacharacters (`|`, `&`, `;`, `$`, `` ` ``, `(`, `)`, `<`, `>`, `{`, `}`) unless you explicitly
intend to allow them:

```toml
# Good: Blocks shell injection
pattern = "^ls( [^|&;$`()<>{}]+)?$"

# Bad: Allows arbitrary commands via injection
pattern = "^ls (.*)$"
```

### 2. Use Anchors

Always anchor patterns with `^` (start) and `$` (end) to prevent partial matching:

```toml
# Good: Only matches exact pattern
pattern = "^git status$"

# Bad: Would match "git status && rm -rf /"
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

### 4. Place Specific Rules Before General Ones

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

### 5. Use Fragments for Security Patterns

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

### Example 1: Safe Cargo Commands

```toml
[pattern_fragments]
safe_chars = "[^|&;$`()<>{}]"

# Filter annoying --open flag from cargo doc
[[bash_rules]]
name = "cargo-doc-no-browser"
pattern = "^cargo doc.*--open"
action = { type = "ArgumentFilter", remove = ["--open", "-o"], reason = "Browser not useful for Claude" }

# Allow cargo commands after filtering
[[bash_rules]]
name = "cargo-safe-commands"
pattern = "^cargo (build|check|test|clippy|fmt|doc)( {{safe_chars}}+)*$"
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
[pattern_fragments]
safe = "[^|&;$`()<>{}]"
safe_arg = "( {{safe}}+)"

[[bash_rules]]
name = "deny-rm-rf-root"
pattern = "^rm\\s+-rf\\s+/"
action = { type = "Deny", value = "Cannot delete from root" }

[[bash_rules]]
name = "deny-sudo"
pattern = "^sudo\\b"
action = { type = "Deny", value = "sudo not allowed" }

[[bash_rules]]
name = "allow-read-commands"
pattern = "^(ls|cat|head|tail|grep|wc|find){{safe_arg}}*$"
action = { type = "Allow" }

[[bash_rules]]
name = "allow-cargo"
pattern = "^cargo (build|check|test|clippy|fmt){{safe_arg}}*$"
action = { type = "Allow" }

[[bash_rules]]
name = "allow-git-read"
pattern = "^git (status|diff|log|show){{safe_arg}}*$"
action = { type = "Allow" }

# Default: ask for anything not explicitly allowed
```

## Troubleshooting

### Rule Not Matching

**Problem**: Your rule isn't matching commands you expect.

**Solution**: Check the logs at `~/.local/state/moriarty/logs/` to see which rule (if any) matched.

```bash
tail -f ~/.local/state/moriarty/logs/moriarty-*.log | grep "Bash rule matched"
```

### Pattern Expansion Errors

**Problem**: You see "Failed to expand pattern fragments" in the logs.

**Solutions**:

- Check for undefined fragments: `{{undefined}}`
- Check for circular dependencies: `a` → `b` → `a`
- Verify fragment names follow naming rules (start with letter/underscore)

### Unexpected Modifications

**Problem**: Commands are being modified in unexpected ways.

**Solution**: Check your Modify rules and their capture groups. Use logs to see the transformation:

```bash
tail -f ~/.local/state/moriarty/logs/moriarty-*.log | grep "Command modified"
```

### Rules Not Loading

**Problem**: Your rules don't seem to be taking effect.

**Solutions**:

- Verify config file location: `~/.config/moriarty/tool_rules.toml`
- Check TOML syntax: `cat ~/.config/moriarty/tool_rules.toml`
- Look for parse errors in logs: `~/.local/state/moriarty/logs/`

### Testing Patterns

Use online regex testers like [regex101.com](https://regex101.com/) to test your patterns. Remember:

- Moriarty uses Rust regex syntax
- Patterns are case-sensitive
- Use the "Rust" flavor in online testers

## Further Reading

- [Rust Regex Syntax](https://docs.rs/regex/latest/regex/#syntax) - Detailed regex syntax documentation
- [TOML Specification](https://toml.io/) - Configuration file format
- `~/.local/state/moriarty/logs/` - Moriarty logs showing rule evaluation
