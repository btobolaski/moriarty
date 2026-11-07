# Design

## Context

See `proposal.md` for motivation and `specs/permission-mode-policy/spec.md` for the behavior contract.

The current shared `PermissionMode` in `crates/moriarty/src/permission_mode.rs` is a six-variant, `Copy` enum deriving
Serde and clap `ValueEnum`. `HookInput.permission_mode` is required. Tool and Bash configurations store restrictions as
`Option<BTreeSet<PermissionMode>>`, and matching already uses shared eligibility and overlap helpers. Mode eligibility
is applied within the existing ordered tool, command, and redirect domains rather than selecting a separate ruleset.

The same type reaches the Bash evaluation context, tool locality preflight, rewrites, offline tests, suite reports,
internal recorded-row aggregation, suggestions, and replay. `hooks/report.rs::parse_recorded_permission_mode` currently
turns unfamiliar recorded names into mode-less rows. `main.rs` restricts `test bash-rules --mode` using `value_enum`.
`user_config/mod.rs` tests canonical mode ordering and effective-hash behavior. The separate enum in
`crates/claude_logs/src/parser.rs` parses Claude transcripts, not permission-hook requests.

No existing spec owns this mode contract. The related `bash-path-alias-policy` capability already applies its denial
independently of permission mode and needs no delta.

Pi-modal supplies mode selection; the existing pi setup guide documents `pi-permission-system` as the hook transport.
The intended separate pi-side integration will pass the selected name through that transport's `permission_mode` field;
this change does not establish that the external wiring already exists.

## Goals / Non-Goals

**Goals:**

- Open the shared value type rather than add a pi-only evaluator or parallel rule representation.
- Keep the current control flow and JSON/TOML shapes; make ownership changes mechanical and local.
- Preserve hashes for existing mode-bearing as well as unrestricted configurations.

**Non-Goals:**

- Owning pi-modal state, switching modes, or changing the external pi permission extension's payload construction.
- Inferring modes from logs, registering names, warning based on observed history, or adding mode-specific defaults.
- Widening Claude transcript schemas, changing hook envelope strictness, or refactoring unrelated evaluators.

## Decisions

### 1. Extend the existing enum with a validated custom-name payload

Retain the six variants in their current order and append `Custom(CustomModeName)`. `CustomModeName` wraps a private
`String`, exposes no mutable or unchecked construction access, and can be created only through the shared mode parser.
That parser maps the six exact built-in spellings to their existing variants, rejects blank names, and constructs the
custom payload only for other names. Do not derive a separate unchecked deserializer for the payload. Callers cannot
construct `Custom` from a raw string or manufacture a custom value containing a built-in spelling.

Use `trim().is_empty()` only to reject blank names, never to transform stored values. Derive equality, ordering, and
hashing on the enum and payload: built-in variant order stays unchanged and custom names use lexicographic UTF-8 byte
order. This preserves existing constructors without a compatibility comparator or a wrapper around the entire enum.

Replace `serde(rename_all)` with Serde's string conversion attributes `#[serde(try_from = "String", into = "String")]`,
retaining derived `Serialize`/`Deserialize`. `TryFrom<String>` is the shared validated parser; conversion back to
`String` uses the same exact spelling as `Display`. Use that parser from CLI parsing too. This avoids hand-written
serializer machinery and externally tagged payload objects while keeping JSON/TOML/CLI interpretation identical. Remove
clap `ValueEnum` metadata and its Display dependency; `manual` remains an ordinary custom name.

A raw `Custom(String)` plus a constructor audit would leave invalid states constructible; the small validated payload
removes that responsibility instead. A pure-string representation would require compatibility ordering; a registry or
interner adds state without serving the approved contract. No dependency is needed.

### 2. Replace closed CLI enumeration, not its wire contract

Remove `ValueEnum` and the CLI's `value_enum` constraint in favor of the shared string parser. Keep the field name
`permission_mode` and all existing `modes` arrays. Help must describe free-form names rather than present a closed
possible-values list. Keep suite-specific validation of empty mode arrays and exact duplicates; rule arrays still
canonicalize duplicates as sets.

A separate pi field would need precedence and compatibility policy that the user explicitly chose not to introduce.

### 3. Adapt ownership without changing evaluation responsibility

The owned custom payload removes `Copy`. Borrow the mode for eligibility/overlap comparisons and synchronous evaluation
where practical, and clone only where an owned context must cross a blocking/async boundary or outlive its source.
Preserve one mode through `hooks/mod.rs`, tool rules, Bash contexts, continuations, explain, and offline suite
expansion.

Keep the existing eligibility predicates and first-match loops. Do not add fallback-to-default or unknown-mode branches:
an unfamiliar valid mode is just another exact key. An ineligible local tool rule must still avoid locality preflight,
and path-alias policy must remain mode-independent.

Leaking strings to preserve `Copy` is unsuitable because replay can ingest many names in one process; shared ownership
or interning is unnecessary for these small values.

### 4. Reuse recorded-mode and authoring workflows

Route recorded mode strings through the shared parser so valid custom names stop degrading to mode-less. Preserve
historical absence/empty handling; whitespace-only historical strings also remain mode-less through invalid-value
handling. Keep malformed-record handling and public `hooks report` grouping/output unchanged.

Existing aggregation keys, suggestion mode sets, replay labels, and mode-overlap diagnostics can keep their structure.
Update ownership and wording as needed, not the algorithms. Preserve suggestion unions and the sticky unrestricted
result when any contributing row is mode-less. Do not introduce a log-dependent lint requirement.

Human-readable mode labels and mode diagnostics escape control characters with the standard library so arbitrary names
cannot inject terminal controls or extra output lines. Ordinary labels stay unchanged; JSON, logs, and matching retain
exact names.

### 5. Prove compatibility in existing test seams

Extend existing module tests rather than create new suites or wiring:

- `permission_mode.rs`: built-in and custom Serde/CLI round trips, canonical variant selection, one focused parser
  blank-rejection check, one Serde non-string check, case/whitespace distinction, `manual` versus `default`,
  eligibility, and overlap. Other boundaries test their distinct envelope, omission, suite-list, and report behavior,
  not repeated copies of the shared parser's invalid-name matrix.
- `user_config/mod.rs`: mixed-name canonicalization, exact de-duplication, and fixed pre-change serialization/hash
  expectations for built-in-only and omitted restrictions. A round trip alone cannot prove hash compatibility.
- `hooks/tests.rs` and `hooks/bash_rules_tests/`: a custom-mode live hook decision plus exact logged value, tool/Bash
  fallback, redirect and locality gates, compound/bail decisions, and Modify/ArgumentFilter continuation checks. Reuse
  existing mode-threading cases and helpers; keep coverage focused on the paths affected by losing `Copy`.
- `test_runner/mod.rs` and `test_runner/rules_suite_tests/mod.rs`: custom CLI normal/JSON/explain paths and suite mode
  expansion/reporting, replacing the obsolete unknown-name rejection while preserving unrelated strictness tests.
- `hooks/report.rs` and `rules/mod.rs`: custom recorded rows, replay eligibility and labels, suggestion unions with and
  without mode-less contributors, and existing overlap/empty-mode diagnostics with custom names.

Use existing XDG/environment helpers and run tests with Nextest, never `cargo test`. Implementation verification must
use the configured formatter, build, lint, and checks in `.config/tools.toml`, including check, Semgrep, Clippy, and
Nextest. This planning-only change is checked with strict OpenSpec validation and artifact review, not Rust execution.

## Risks / Trade-offs

- Valid misspellings no longer fail closed-list validation → document exact matching and encourage explicit rule
  regression cases; no implicit aliases or log-dependent lint hide the behavior.
- Accidentally retaining externally tagged enum Serde changes wire/config shapes → use string conversion attributes and
  the fixed pre-change serialization/hash assertions; the payload has no unchecked deserialization path.
- Lost `Copy` can cause dropped mode propagation during mechanical edits → inspect every caller of the shared type and
  eligibility helper, then run existing mode-threading tests with custom names and targeted continuation coverage.
- Reordering built-ins changes hashes used to filter historical logs → retain enum order and assert fixed pre-change
  serialized values and hashes, not just equality between two new serializations.
- `manual` and padded names may surprise users → explicitly document `manual != default`, case sensitivity, and the
  absence of trimming; invalid blank values still fail at input boundaries.
- Older Moriarty binaries cannot read new custom names → deploy Moriarty support before enabling the pi-side plumbing.

## Migration Plan

1. Ship Moriarty's parser, ownership adaptations, tests, and mode documentation together. No existing valid policy or
   log format migration is required.
2. Separately wire pi-modal's selected name through `pi-permission-system` into the existing `permission_mode` field.
   Add desired custom restrictions to user policy; arbitrary names grant no built-in action by themselves.
3. Before rolling back Moriarty, stop sending custom names and remove custom names from active policy/suites. Older
   binaries will still treat custom historical log names as mode-less under their existing replay behavior.
