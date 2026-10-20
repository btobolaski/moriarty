# Hook and rule-engine internals

The [Tool & Bash Rules Configuration Guide](../BASH_RULES.md) owns the user-facing policy format and examples;
[Getting Started with rules](getting-started-rules.md) covers installation and hook wiring. This reference describes
`crates/moriarty/src/hooks/` and its rule-authoring/reporting integrations. For process ownership and approval
boundaries, read the [architecture reference](architecture.md) and the
[Moriarty process-execution specification](../openspec/specs/moriarty-process-execution/spec.md).

- **PreToolUse hook**: Two-tier permission system from `~/.config/moriarty/tool_rules.toml`:
  - **Permission-mode eligibility**: both rule kinds accept `modes`, an optional set of the six exact hook values
    (`default`, `plan`, `acceptEdits`, `auto`, `dontAsk`, `bypassPermissions`; Manual arrives as `default`). Omitted
    means the rule is permanently unrestricted in all modes and remains omitted from serialization so existing effective
    hashes stay stable; `[]` means no modes. Hook events require `permission_mode`; a restricted rule cannot match a
    mode-less replay or test row. Ineligible rules stay in declaration order and are skipped before other gates, so
    first-match-wins applies among eligible rules; the same mode threads through tool-to-Bash fallback, locality
    preflight, compound/bail evaluation, ArgumentFilter recheck, explain, replay, and suggestion de-duplication. Public
    `hooks report` grouping/JSON remains mode-agnostic, but cwd-aware internal rows split by mode; suggestions union
    known contributing modes and become unrestricted if any contributor is mode-less.
  - `tool_rules`: Permission any tool call (Read, Write, Edit, Bash, etc.) with optional legacy field-level regex
    matching and an optional ANDed `conditions` list over literal top-level input keys: `Present`, `Absent`, typed raw
    JSON `Equals`, and scalar-regex `Matches`. Separate ordered rules provide OR/fallback behavior. Actions: Allow,
    Deny, Ask. Checked first. Regex field values that start with the hook input's `cwd` have that prefix stripped so
    rules can use relative paths (e.g., `^src/` instead of absolute paths). `allow_local = true` canonicalizes every
    `path` / `file_path` selected by presence-requiring conditions or the legacy field; `~` and `~/...` first expand
    against `HOME`, while other tilde forms cannot satisfy locality. For non-existent targets it canonicalizes the
    deepest existing ancestor and safely rebuilds the missing suffix so `..` cannot escape above that ancestor.
  - `bash_rules`: Bash-specific command and redirect-endpoint validation with regex patterns. Ordinary actions are
    Allow, Deny, Modify, Ask, and ArgumentFilter; directional `AllowRedirect`/`DenyRedirect` actions form a target-only
    domain that never grants command execution. Existing `AllowRedirect` rules default to output-only. Checked when no
    tool_rule matches a Bash call.
  - **Canonical Bash evaluation** (`hooks/bash_rules.rs` + `hooks/command_split.rs`): `evaluate_sync` invokes the
    `brush-parser` splitter and builds one completed `Evaluation` containing source command decisions, resolved endpoint
    facts, and a closed continuation: none, a Modify redirect check, an ArgumentFilter recheck, or that recheck followed
    by one Modify redirect check. Leaf, whole-policy, rewrite, and final outcomes plus ordered contributors are derived
    from those facts rather than stored. Command and redirect rules are compiled into separate first-match domains, and
    parsed redirect targets are closed descriptor/filesystem/unresolvable states carrying input/output/both direction. A
    simple `^ls` rule still matches the `ls` leaf of a compound, while merge precedence remains Deny, Ask,
    NoMatch/Modify/ArgumentFilter, then Allow with first-leaf attribution on ties; multi-leaf rewrites are never
    stitched together. Replay, normal testing, explain, and the hook consume the same completed evaluation. Suggestions
    call the same canonical original-policy analyzer but deliberately skip unused Modify and ArgumentFilter
    continuations. Explain alone requests diagnostic endpoint facts, and `test_runner` projects each endpoint's
    direction plus whether its matched redirect rule allowed or denied it into the compatibility trace. Compiled rules
    own shared match metadata so repeated evaluations do not rebuild explanation strings.
  - **Configured path-alias preprocessing**: the optional top-level `bash_path_aliases` ordered set names trusted shell
    variables that may be bound by an exact leading absolute or cwd-relative `NAME=literal/path;` declaration or
    newline-terminated equivalent. Relative values must be non-empty, require a recorded cwd, and cannot contain a
    complete `..` component; `command_split` joins accepted values lexically to cwd without filesystem inspection, then
    expands parser-identified plain `$NAME`/`${NAME}` references in either unquoted or ordinary double-quoted words,
    including `"$P/runtime/mocks.js"` and `"$P"/runtime/mocks.js`, then applies quote-aware cwd stripping. Single-quoted
    `'$P'` remains literal; localized `$"…"`, indirect, dynamic, and other quoted or escaped compositions stay
    unsupported and cap Allow at Ask. The declaration is analysis metadata only after a later supported reference
    consumes it. Unsupported declarations/references and command-position expansion remain leaves with an independent
    Allow-to-Ask confirmation cap, so later Denies still win. Rather than model per-builtin mutation targets, an
    assignment invalidates all aliases only when a binding is active or it targets a configured alias; unrelated
    standalone environment assignments remain eligible for normal rules. Dynamic command names, wrappers, `exec`, and
    recognized shell-state builtins (including `shopt`, `alias`, and `unalias`) invalidate all active aliases and become
    redirect path-context barriers. This trades extra prompts for a small fail-closed implementation without retaining
    stale bindings. Config deserialization validates shell identifiers and rejects the fixed shell-control names before
    any tool rule can short-circuit Bash analysis. No aliases is the omitted/hash-compatible default; configured aliases
    are trusted policy and must not be application/tool behavior variables.
  - **Component-scoped redirect policy**: command rules match a cwd-normalized view with every parsed input and output
    redirect removed, with first-match-wins among ordinary actions. Redirect-only leaves retain their full syntax so
    endpoint policy alone cannot authorize shell execution. Every endpoint (`<`, `<&`, `>`, `>>`, `>|`, `&>`, `&>>`,
    `>&file`, `<>`, devices, and fd duplication/closure) independently uses the first direction-eligible redirect rule:
    `DenyRedirect` blocks it and `AllowRedirect` authorizes it. Existing allows default to output; input is explicit and
    a read-write endpoint requires `both`. Strict lint reports a redirect rule as shadowed only when the earlier rule
    covers every endpoint class the later rule could match. All command leaves and endpoints must pass. Redirect rules
    are a separate domain, so ordinary rules cannot shadow them or authorize a target. Each endpoint is retained in
    source order with static, alias-expanded, tilde, or descriptor metadata; unresolved parameters, ordinary globs,
    extglobs, braces, invalid descriptor-duplication words, and unsupported forms prompt. The redirect-free normalized
    command match and alias-expanded rewrite source remain separate for `ArgumentFilter`, which preserves prefix,
    interleaved, and suffix redirects. `Modify` expands redirect-free command-domain captures, retains consumed
    path-alias declarations used by redirects, and appends the original redirect operations in source order. Analyzable
    replacements must reparse with those redirects on the same leaf. Direct `Modify` output passes redirect-only
    authorization inside the shared engine evaluation before it is returned; a matching redirect deny still blocks a
    confidently parsed static endpoint on bailout, while any other unanalyzable or confirmation-bearing rewrite prompts.
    Synchronous test/replay/explain and the live hook therefore apply the same policy. The live hook runs the entire
    synchronous evaluation in one blocking task under one two-second deadline; timeout or join failure prompts with no
    deciding rule or contributors, including for plain commands delayed in the blocking pool. `ArgumentFilter` consumes
    normalized argument values but rewrites alias-expanded source spans, preserving unchanged quoting, escapes, literal
    globs, redirect operators, leading `time`/`time -p` and `!` pipeline prefixes, and a trailing background `&` while
    shell-quoting replacement and added values. The result then passes the full command-plus-endpoint evaluation again
    in live, replay, and explain paths. A relative or home-relative filesystem redirect in a path-context barrier
    command, or any later command, asks because changed shell path context is deliberately not modeled. Assignments,
    dynamic command names, `cd`/`pushd`/`popd`, recognized shell-state builtins, and non-isolated compound constructs
    such as brace groups and `if`/`for`/`while`/`case` clauses are barriers; absolute paths and descriptors remain
    evaluable.
  - **Shared hook path resolution** (`hooks/path_resolution.rs`): tool-rule locality and redirect policy share the same
    symlink-safe missing-path implementation, with redirect resolution additionally rejecting non-directory ancestors
    and unsafe virtual paths (`/proc/self`, `/proc/thread-self`, `/dev/fd`, `/dev/std*`, `/dev/tcp`, `/dev/udp`) —
    including components introduced by nested symlink targets — that would resolve differently in the hook or perform
    Bash-specific network I/O. Redirect contexts capture cwd/HOME text without filesystem I/O and initialize canonical
    roots only when an eligible filesystem endpoint needs them; invalid HOME does not block unrelated targets. Paths
    render cwd-relative, `~/...`, or canonical absolute, while devices always remain canonical absolute. Cwd-relative
    names beginning with `~` or `&` gain `./` to keep home and descriptor namespaces distinct, and non-UTF-8 resolved
    paths fail closed instead of using lossy text. Local redirect rules additionally require containment under cwd and
    cannot authorize devices, special files, or fd tokens. Live Bash evaluation always uses one `spawn_blocking` call
    and one two-second deadline for parsing, matching, resolution, and continuations; offline evaluation has no
    deadline. Tool-rule `allow_local` retains its separate conditional blocking preflight, where failure or timeout
    skips all local rules and falls through. Canonicalization is pre-execution and retains the unavoidable filesystem
    TOCTOU limitation.
  - **Bail ⇒ fail safe**: a command containing command substitution `$(...)`, backticks, an uninspectable value-carrying
    parameter expansion (`${x:-$(…)}`), a subshell, process substitution, a here-doc/here-string, a compound construct
    (`if`/`for`/`while`/`case`/`[[ ]]`/`((…))`/brace group/function), or that fails to parse is un-analyzable. The
    splitter retains confidently parsed static redirect facts while deferring the first bailout, so a raw whole-command
    Deny or a matching DenyRedirect is honored; every other outcome becomes a prompt.
  - **Bash cwd stripping**: static paths—including ordinary quoted values, escaped literals, safe glob patterns, and
    known alias expansions—are normalized only when their in-cwd relative remainder contains no `..` component.
    Parent-containing paths, unquoted brace syntax, and glob paths with dot-prefixed components remain unchanged; quoted
    or escaped braces stay literal. An exact-cwd operand becomes `.` rather than disappearing. Thus `^cat src/` matches
    `cat "/abs/cwd/src/x"`. `evaluate_sync(command, context, purpose)` is the completed synchronous runtime evaluator;
    suggestion mining uses its canonical `evaluate_original` source-fact stage directly, and `evaluate_live` supplies
    the live deadline. `RuleResult` is only the compatibility DTO projected at hook, replay, normal-test, and
    explain-trace boundaries. Explain includes the final primary rule plus ordered contributors; matched redirect
    endpoints carry a typed allowed/denied decision, while failure text is reserved for resolution or unmatched-policy
    failures. An unanalyzable Modify rewrite reports its rewritten command, bail reason, and any retained endpoint that
    caused a redirect denial without a second evaluation path.
  - Evaluation order: tool_rules → bash_rules (for Bash) → passthrough (for non-Bash, defers to Claude Code)
- **Stop hook**: Runs the project's configured checks before allowing execution, delegating to the shared
  `crate::checks::run_configured_checks` routine (see [MCP servers](architecture.md#mcp)); it maps the routine's
  `CheckRunOutcome` onto allow/deny. Checks execute with the workspace root as their working directory, following the
  [configuration and approval root policy](architecture.md#repository-root-detection). Each check's program is spawned
  as the original resolved path verification hashed — not the canonical path, since canonicalizing breaks symlinked
  multi-call binaries (nix coreutils, busybox) that dispatch on argv[0], and not the raw `command[0]`, so an unapproved
  local copy cannot shadow the approved one (a workspace-local `./check.sh` runs only if that exact argv+binary was
  approved)
- Structured logging with tracing crate for debugging hook execution. The "PreToolUse hook completed" log event records
  a clean `result` field (`allow`/`deny`/`ask`/`modify`/`passthrough`) classified from the typed `HookOutput` by
  `hooks::result::pretool_result`, alongside `tool_name`, `tool_args`, `cwd`, `permission_mode`, `rules_hash`, `rule`,
  and `rules`. `rule` remains the primary/deciding compatibility attribution (empty when none); `rules` is a
  JSON-encoded ordered contributor list covering compound command rules, endpoint rules, and filter revalidation.
  Contributors are de-duplicated by matched rule identity rather than display name, so distinct command and redirect
  rules with the same configured name remain visible as repeated names. A bailed command retains a deciding command or
  redirect Deny contributor and otherwise has none.
- **`hooks report` subcommand**: `hooks/report.rs` reads the JSON-lines hook logs, keeps completed PreToolUse records
  that carry the clean `result` field (historical lines lacking it are skipped), and aggregates them by the exact
  `(tool name, arguments, result, rule, ordered contributors)` key into a JSON report with counts; `rule` remains for
  compatibility and `rules` carries complete provenance. Historical lines without `rules` fall back to a non-empty
  singular `rule`; both are omitted when no rule contributed. Reuses `cost_report::TimeRangeFilter` for
  `--start-time`/`--end-time` and supports `--tool` and `--result` filters. `report::aggregate` (used by `hooks report`)
  and `ReportRow` are `pub(crate)`. `report.rs` parses the completion line's `cwd` back into `HookRecord` and folds
  filtered records directly into each caller's accumulator while reading rotated log files concurrently, so no vector
  retains every matching event. `rules suggest`/`rules replay` call `report::aggregate_with_cwd`, which joins `cwd` into
  the grouping key and populates `ReportRow.cwd` (a `#[serde(skip)]` field) so each command is re-normalized with the
  directory it actually ran under. `rules report` calls `report::fold_outcomes`, preserving each event's timestamp only
  until it has been folded into a timezone-aware daily or exact-directory bucket while sharing the same time/hash
  filtering and skip accounting. `aggregate` keeps `cwd` out of its key and serialization, so cwd differences do not
  split `hooks report` rows; contributor differences still do. Rows recorded before `cwd` was logged are excluded (with
  disclosed counts) from replay/suggest because component path policy cannot be reproduced, while the effectiveness
  report retains them in an explicit `Unknown` bucket.
- **Bash pattern validation**: Non-Modify patterns use `validate_rule_pattern`, which compiles forward and reverse
  Thompson NFAs with the string `Regex::new` defaults, including UTF-8 checks, capture-state accounting, and the 10 MiB
  NFA size limit. Failed probes fall back to `Regex::new` for canonical diagnostics and literal-fast-path acceptance.
  Keep `REGEX_NFA_SIZE_LIMIT` and the validation parity tests aligned when upgrading `regex` or `regex-automata`. Only
  `CommandAction::Modify` owns a full per-rule capture regex; ordinary command and redirect matching still use the
  separate RegexSets, whose indexed patterns are checked against their metadata in debug builds.
- **Compile diagnostics & `rules` authoring tooling**: `BashRuleEngine::compile_with_diagnostics` and
  `ToolRuleEngine::compile_with_diagnostics` return the engine plus a `RuleDiagnostic` for every dropped rule
  (undefined/circular/over-depth/over-count fragment, invalid regex, or — tool rules only — a `field`/`pattern` given
  without its partner); an invalid `Matches` condition drops the whole tool rule rather than broadening it by retaining
  only the valid predicates. `from_config` delegates to these compilers and logs each diagnostic, preserving the
  fail-open-per-rule hot path. `moriarty test rules` instead delegates to both diagnostic compilers and rejects any
  dropped candidate rule before evaluating its strict v1 suite; its full format and status contract live in the
  [rules guide](../BASH_RULES.md#rule-regression-suites) and [test-rules manual](man/moriarty-test-rules.1). The
  `crate::rules` command group surfaces diagnostics and helps author safe rules: `report` (daily outcome
  counts/percentages by default, exact recorded-cwd buckets with `--directories`, all recorded rule sets by default, and
  opt-in active-hash filtering with `--current-rules`), `lint` (errors when a rule the user wrote is silently dropped;
  `--strict` additionally warns on permanently disabled `modes = []`, missing mode-overlapping redirect policy,
  same-domain rules fully shadowed across their direction/locality scope, over-broad command Allows, and broad
  local/non-local redirect authority), `list-fragments`, `schema` (round-tripped against `UserConfig` in tests),
  `starter` (paste-ready read-only command rules plus `/dev/null`, output-descriptor, and stdin `&0` input rules),
  `suggest` (anchored rules mined from hook logs; each recorded command is split into the leaf simple-commands the hook
  actually evaluates — normalized with the recorded cwd — before pattern generation, so compounds yield per-leaf
  candidates with summed counts, consumed alias declarations omitted, and confirmation-required alias leaves excluded; a
  bailed command stays whole. `--match exact|prefix|fuzzy` picks the shape, where prefix/fuzzy clustering reuses
  brush-derived program and argument metadata rather than reparsing leaf text; leaves whose normalized match text does
  not start with the literal program stay exact-only. Fuzzy generalizes simple-identifier subcommands into a closed
  alternation like `^cargo (build|check)(\s|$)`, falling back to a program prefix; Allow is emitted only with
  `--match exact`). Explicit `--action allow-redirect` is also exact-only and switches to target mining: active redirect
  policy is filtered out; leaves containing dynamic or shell-context-dependent endpoints and command-blocked leaves are
  omitted; project targets preserve each endpoint's direction and emit `allow_local = true`. Local target patterns are
  portable across project cwds. Historical tilde targets use the current process HOME and live filesystem because HOME
  is not recorded. Ordinary Allow suggestions omit leaves already covered by active command policy; Ask and Deny
  suggestions retain command-allowed leaves when redirect policy still prompts. `replay` re-evaluates the full candidate
  component policy, including one `ArgumentFilter` recheck; its migration gate fails on lost auto-approvals or when
  every in-scope row lacks a reproducible cwd. `test bash-rules --explain [--cwd <dir>] [--mode <mode>]` prints consumed
  bindings, original/alias-expanded/normalized leaf text, command matches, endpoint resolution/locality/rules,
  contributors, and the merged decision; normal mode uses the same compound path. Omitted `--mode` runs a mode-less
  evaluation. Alias coverage spans `user_config`, splitter, engine, authoring commands, CLI, and hook tests and must run
  under Nextest like every XDG-mutating test.
- **Rule-set provenance**: each `PreToolUse hook completed` log line records `rules_hash`, a stable hash of the
  effective config (`UserConfig::effective_hash`, computed once per hook invocation). The hash covers the parsed config
  — tool rules, bash rules, fragments, and the deterministically ordered path-alias policy — re-serialized via
  `serde_json::to_value` so map keys (`pattern_fragments`, an ArgumentFilter `replace` table) sort deterministically
  while rule order (significant for first-match-wins) is preserved; cosmetic edits don't change it but any semantic
  change does. The empty alias set is omitted so pre-feature hashes remain unchanged. `rules suggest`/`rules replay`
  default to only the records whose `rules_hash` matches the rule set currently installed at the default config path
  (for `replay` this is the migration source, independent of the `--config` candidate); `--rules-hash <hash>` pins a
  specific set and `--all-rules` disables the filter. Both commands report the active hash and how many records the
  filter excluded (`report::RulesHashFilter`/`HashSkipStats`/`CwdAggregation`); excluded counts are never hidden.
  `rules report` follows the same disclosure rule under `--current-rules`, but defaults to all recorded history.
