# Spec Delta

## Purpose

Define permission-mode identity and rule eligibility consistently across live hooks, offline policy evaluation, and
recorded-rule workflows, including arbitrary modes supplied by pi integrations.

## ADDED Requirements

### Requirement: Exact free-form mode identity

Permission modes SHALL accept any string containing at least one non-whitespace character without prior declaration.
Names SHALL retain their exact spelling and compare case-sensitively, without trimming, normalization, or aliases. The
existing names `default`, `plan`, `acceptEdits`, `auto`, `dontAsk`, and `bypassPermissions` SHALL retain their identity.

#### Scenario: Previously unknown custom mode

- **WHEN** a caller supplies `review-only` as its permission mode
- **THEN** Moriarty accepts it as a mode without requiring a registry entry

#### Scenario: Similar spellings remain distinct

- **WHEN** modes are named `"review"`, `"Review"`, and `" review "`
- **THEN** all three are accepted and treated as distinct names with their spelling preserved

#### Scenario: Manual is not a default alias

- **WHEN** a caller supplies `manual`
- **THEN** it is a custom mode distinct from `default`
- **AND** Claude Code's Manual mode continues to arrive on the wire as `default`

### Requirement: Safe human-readable mode labels

Human-readable mode labels and mode diagnostics SHALL escape control characters. Ordinary mode labels SHALL remain
unchanged; JSON, logs, and matching SHALL retain exact mode names.

#### Scenario: Mode contains terminal controls

- **WHEN** a valid custom name contains an embedded newline or terminal escape character
- **THEN** human-readable replay labels, suite failure labels, and duplicate-mode diagnostics escape those characters
- **AND** JSON, logs, and rule matching retain the original name

### Requirement: Shared mode validation at input boundaries

Live hook input SHALL require the existing top-level `permission_mode` string. Empty or whitespace-only names and
non-string values SHALL be rejected through the existing input-error paths, including in configured rule mode lists,
offline mode arguments, and regression suite mode lists. Missing live modes SHALL NOT default to another mode.

#### Scenario: Custom hook value uses the existing transport

- **WHEN** a valid hook event contains `permission_mode = "review-only"`
- **THEN** it is evaluated under `review-only` without requiring a second mode field

#### Scenario: Invalid or missing live mode

- **WHEN** a hook event omits `permission_mode` or supplies null, a number, an array, an object, an empty string, or
  only whitespace
- **THEN** hook input is rejected rather than evaluated under an inferred mode

#### Scenario: Blank or non-string configured modes

- **WHEN** a rule or regression suite mode list contains an empty string, whitespace-only string, or non-string value
- **THEN** the input is rejected through its existing validation path

#### Scenario: Blank offline argument

- **WHEN** `test bash-rules --mode` receives an empty or whitespace-only argument
- **THEN** argument parsing rejects it

### Requirement: Mode restrictions preserve eligibility and ordering

Both tool and Bash rule `modes` lists SHALL use exact name membership. Omitted restrictions SHALL remain unrestricted;
empty restrictions SHALL remain disabled. A mode-less evaluation SHALL satisfy only unrestricted rules. Ineligible rules
SHALL be skipped in place, preserving first-match order among eligible rules within each existing rule domain.

#### Scenario: Matching custom restriction

- **WHEN** a tool or Bash rule has `modes = ["review-only"]` and the current mode is `review-only`
- **THEN** the rule participates subject to its other existing conditions

#### Scenario: No implicit fallback to a built-in mode

- **WHEN** the current mode is `unheard-of` and ordered rules restrict the same request to `default`, then `plan`, then
  omit `modes`
- **THEN** only the unrestricted rule is mode-eligible

#### Scenario: Empty and mode-less eligibility

- **WHEN** a rule has `modes = []`, or a restricted rule is evaluated without a mode
- **THEN** that rule is ineligible while otherwise matching unrestricted rules remain eligible

#### Scenario: First eligible rule wins independently per domain

- **WHEN** an earlier matching rule is restricted to another mode and a later matching rule includes the current custom
  mode
- **THEN** evaluation skips the earlier rule and uses the later rule in tool, Bash command, and redirect domains
  independently

### Requirement: One mode across the complete permission decision

The current exact mode SHALL be preserved through tool-to-Bash fallback, locality preflight, compound leaves, redirect
endpoints, bailout handling, Modify redirect validation, and ArgumentFilter rechecks. Supporting custom modes SHALL NOT
change tool-rule precedence, action precedence, existing fallback decisions, or mode-independent path-alias policy.

#### Scenario: Custom-mode tool rule short-circuits Bash

- **WHEN** an eligible custom-mode tool rule decides a Bash request
- **THEN** that decision retains its existing precedence over Bash rules

#### Scenario: Fallback and component checks retain the custom mode

- **WHEN** no tool rule decides a compound Bash request under a custom mode
- **THEN** Bash command and redirect rules, including local rules and bailout decisions, use that same custom mode

#### Scenario: Rewrite checks retain the custom mode

- **WHEN** Modify or ArgumentFilter produces a replacement under a custom mode
- **THEN** its existing redirect-only or full-policy revalidation uses that same mode without widening rewrite authority

#### Scenario: Custom modes cannot bypass alias-name policy

- **WHEN** Bash analysis under a custom mode detects a violation of enabled unconfigured path-alias denial
- **THEN** the mode-independent Deny and supported-name feedback remain unchanged

### Requirement: Offline evaluation accepts the same modes

`test bash-rules --mode` SHALL accept the same names as live hooks in normal, JSON, and explain output modes. Rule
regression suite mode lists SHALL accept those names and retain existing per-mode expansion, empty-list rejection, and
duplicate-name rejection. Omitted offline modes SHALL remain mode-less rather than implying `default`.

#### Scenario: Custom-mode CLI evaluation agrees with live evaluation

- **WHEN** no tool rule decides the request and the live hook and `test bash-rules --mode review-only` evaluate the same
  command, cwd, and configuration
- **THEN** normal, JSON, and explain evaluation agree on the Bash policy decision under `review-only`

#### Scenario: Suite expands custom and built-in names

- **WHEN** a regression case lists `modes = ["review-only", "plan"]`
- **THEN** it produces separate evaluations preserving each mode name in its existing report format

#### Scenario: Suite validation remains strict

- **WHEN** a suite case lists no modes in an explicit empty array or repeats an exact custom name
- **THEN** suite validation rejects it, while differently cased names are not duplicates

### Requirement: Stable rule serialization and configuration identity

Rule mode lists SHALL remove exact duplicates and serialize deterministically. Existing valid configurations SHALL
retain their serialized mode spellings, ordering, and effective hashes, including omitted restrictions. Existing names
SHALL sort in their previous order before custom names; custom names SHALL sort lexicographically by UTF-8 bytes.
Changing an effective mode restriction SHALL change configuration identity.

#### Scenario: Existing built-in order remains stable

- **WHEN** a rule lists the six existing names in any order with duplicates
- **THEN** serialization emits `default`, `plan`, `acceptEdits`, `auto`, `dontAsk`, `bypassPermissions` once each in
  that order
- **AND** its effective hash matches the same configuration before custom-mode support

#### Scenario: Mixed names have stable canonical order

- **WHEN** equivalent configurations list `zeta`, `plan`, `alpha`, and a duplicate `alpha` in different orders
- **THEN** both serialize the restriction as `plan`, `alpha`, `zeta` and have identical effective hashes

#### Scenario: Omission and changes retain their meaning

- **WHEN** a configuration omits a rule's `modes`
- **THEN** serialization still omits that field and retains its previous effective hash
- **AND** adding or changing a mode restriction changes the effective hash

### Requirement: Recorded workflows preserve custom-mode provenance

Completion logs and mode-aware internal aggregation SHALL preserve exact custom names. Replay SHALL evaluate a record
under its recorded valid mode rather than degrade unfamiliar names to mode-less. Missing or blank historical modes SHALL
remain mode-less. Public `hooks report` grouping and JSON shape SHALL remain unchanged.

#### Scenario: Previously unfamiliar historical name remains usable

- **WHEN** a reproducible historical row records `review-only` and a candidate rule restricts itself to that name
- **THEN** replay evaluates that rule as mode-eligible and includes the exact mode in any divergence label

#### Scenario: Custom modes remain separate in internal rows

- **WHEN** otherwise identical recorded requests use `review` and `Review`
- **THEN** mode-aware internal aggregation keeps their provenance distinct

#### Scenario: Historical absence does not enable restricted rules

- **WHEN** a historical row has no mode or a blank mode value
- **THEN** replay treats it as mode-less and does not enable restricted rules

### Requirement: Suggestions and diagnostics use exact custom names

Suggested rule restrictions SHALL union contributing valid names, including custom names, and remain unrestricted if any
contributor is mode-less. Existing mode-overlap lint checks SHALL handle custom names by exact membership, without
requiring declarations or observation in logs. Existing empty-restriction diagnostics SHALL remain available.

#### Scenario: Suggestions retain the custom-mode union

- **WHEN** a suggestion combines rows from `review-only` and `plan`, with no mode-less contributor
- **THEN** it emits a deterministic restriction containing exactly those two names
- **AND** adding a mode-less contributor instead makes the suggestion unrestricted

#### Scenario: Lint compares exact names without a closed list

- **WHEN** strict lint checks otherwise comparable rule restrictions containing `review` and `Review`
- **THEN** it treats them as disjoint rather than invalid or overlapping
- **AND** matching exact custom names overlap for existing shadowing and redirect-coverage checks
