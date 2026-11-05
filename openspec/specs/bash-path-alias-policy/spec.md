# bash-path-alias-policy Specification

## Purpose

Control which variable names models use for leading Bash path aliases and return actionable denial feedback without
turning ordinary shell-variable use into a general allow-list policy.

## Requirements

### Requirement: Opt-in unconfigured alias denial

The configuration SHALL accept the top-level boolean `bash_path_aliases_deny_unconfigured`, defaulting to `false`.
Omitted and explicitly false values SHALL preserve existing alias behavior and the pre-change effective configuration
hash. Enabling the setting SHALL change that hash.

#### Scenario: Existing configuration remains compatible

- **WHEN** an existing configuration omits the new setting
- **THEN** command decisions, alias expansion, and the effective configuration hash remain unchanged

#### Scenario: Explicit false is equivalent to omission

- **WHEN** the setting is explicitly `false`
- **THEN** behavior and the effective configuration hash match the same configuration with the setting omitted

#### Scenario: Enabling the policy changes configuration identity

- **WHEN** the setting changes from omitted or false to true
- **THEN** the effective configuration hash changes deterministically

#### Scenario: Non-boolean setting is invalid

- **WHEN** the setting contains a string, number, array, or table instead of a boolean
- **THEN** configuration loading fails through the existing configuration-error path

### Requirement: Reuse the leading path-alias declaration boundary

A candidate SHALL meet the existing leading path-alias declaration rules except approved-name membership: a synchronous,
explicitly semicolon- or newline-terminated, single scalar non-append assignment, without a command, redirect, pipeline
peer, negation, timing, or conditional/background separator. Values SHALL use the existing unquoted literal grammar;
relative values require cwd and exclude complete `..` components. Unsupported prefix items end eligibility as before.

#### Scenario: Leading absolute declaration qualifies

- **WHEN** Bash analysis receives `Q=/work/project; rg x $Q/file`
- **THEN** the leading declaration of Q qualifies as a candidate

#### Scenario: Single-component relative values qualify

- **WHEN** cwd is `/work/project` and Bash analysis receives `Q=src; rg x $Q/file` or `DEBUG=1; rg x file`
- **THEN** the leading assignment qualifies as a candidate in each command

#### Scenario: Relative declaration requires recorded cwd

- **WHEN** Bash analysis receives `Q=src; rg x $Q/file` without a recorded cwd
- **THEN** the declaration does not qualify under the new policy and existing evaluation behavior is retained

#### Scenario: Approved leading declarations do not hide a later candidate

- **WHEN** P is approved and Bash analysis receives `P=/work/project; Q=/work/other; rg x $P/file $Q/file`
- **THEN** the declaration of Q also qualifies as a candidate

#### Scenario: Unsupported declaration shapes retain existing treatment

- **WHEN** an assignment is command-scoped, quoted, dynamic, append-style, one of multiple simultaneous assignments,
  redirected, pipelined, conditional, backgrounded, or lacks an explicit sequence terminator
- **THEN** that assignment does not qualify under the new policy and retains existing evaluation behavior

#### Scenario: Later assignment is not a leading alias

- **WHEN** Bash analysis receives `rg x file; Q=/work/project; rg x $Q/file`
- **THEN** the later declaration of Q does not qualify under the new policy

### Requirement: Reject unapproved candidate names

When enabled and Bash analysis recognizes a candidate whose name is absent from `bash_path_aliases`, the policy SHALL
Deny the command rather than Ask, Allow, Modify, or ArgumentFilter. A supported reference consuming the declaration
SHALL NOT be required. A detected violation SHALL retain its Deny and alias-feedback reason despite later analysis
bailouts or competing Bash-rule decisions. Invalid syntax that prevents recognition retains existing behavior.

#### Scenario: Unknown alias cannot be allowed or reduced to Ask

- **WHEN** the setting is true, only P is approved, and Bash analysis receives `Q=/work/project; rg x $Q/file`
- **THEN** the result is Deny with alias feedback, regardless of matching Bash-rule actions

#### Scenario: Unused declaration still rejects the name

- **WHEN** the setting is true, Q is unapproved, and Bash analysis receives `Q=/work/project; rg x file`
- **THEN** the result is Deny with alias feedback even though Q is not subsequently referenced

#### Scenario: Empty allow-list rejects candidates

- **WHEN** the setting is true, no names are approved, and Bash analysis receives `Q=/work/project; rg x $Q/file`
- **THEN** the result is Deny

#### Scenario: Later unsupported syntax does not erase a recognized violation

- **WHEN** the setting is true, Q is unapproved, and Bash analysis receives `Q=/work/project; echo $(pwd)`
- **THEN** the recognized declaration still produces Deny with alias feedback despite the later analysis bailout

### Requirement: Actionable supported-name feedback

The denial reason SHALL identify the first rejected candidate name in source order and include every configured
supported name exactly once in canonical sorted order. With an empty allow-list, it SHALL explicitly state that no
path-alias variable names are supported. Feedback SHALL NOT claim that an approved name itself grants command
permission.

#### Scenario: Supported names are complete and stable

- **WHEN** the configured aliases are `["ROOT", "P", "P"]` and the policy rejects Q
- **THEN** the reason identifies Q and lists P and ROOT once each, in that order

#### Scenario: No supported names is explicit

- **WHEN** the policy rejects Q with an empty alias list
- **THEN** the reason identifies Q and explicitly says no path-alias variable names are supported

### Requirement: Preserve ordinary shell and configured-alias behavior

The setting SHALL NOT independently deny ordinary variable references, command-scoped assignments, or assignments
outside the eligible prefix. Approved declarations SHALL retain existing expansion and authorization rules; approved
names in unsupported forms SHALL retain their existing confirmation treatment. The existing configured-name validation
SHALL remain unchanged.

#### Scenario: Ordinary references remain outside the deny policy

- **WHEN** only P is approved and the setting is true
- **AND** Bash analysis receives `rg x "$HOME/file"` or `rg x $Q/file` without a leading candidate declaration
- **THEN** the new policy introduces no denial and existing command/redirect rules determine the result

#### Scenario: Command-scoped environment assignment is unchanged

- **WHEN** the setting is true and Bash analysis receives `DEBUG=1 rg x file`
- **THEN** the new policy introduces no denial and existing evaluation behavior is retained

#### Scenario: Approved alias still requires command authorization

- **WHEN** P is approved, the setting is true, and Bash analysis receives `P=/work/project; rg x $P/file`
- **THEN** normal alias expansion and command/redirect policy determine the result without a new alias-name denial

#### Scenario: Unsupported approved alias remains confirmation-capped

- **WHEN** P is approved and the setting is true
- **AND** Bash analysis receives an otherwise allowed command with an unsupported P declaration or reference
- **THEN** the existing Allow-to-Ask confirmation cap remains in force

### Requirement: Preserve hook precedence and empty-rule defaults

Matching `tool_rules` decisions SHALL retain their existing precedence over Bash analysis. When Bash analysis is
reached, the enabled alias-name policy SHALL apply even if `bash_rules` is absent or empty and SHALL apply independently
of permission mode. Without a qualifying violation, absent or empty Bash rules SHALL retain the Ask default.

#### Scenario: Tool-level decision short-circuits alias analysis

- **WHEN** a tool rule matches a Bash call and returns Allow, Ask, or Deny
- **THEN** that decision and its attribution remain authoritative without running the new Bash alias-name policy

#### Scenario: Alias policy works without Bash rules

- **WHEN** no tool rule matches, Bash rules are absent or empty, the setting is true, and a candidate name is unapproved
- **THEN** the hook returns Deny with the supported-name feedback

#### Scenario: Empty Bash rules otherwise still Ask

- **WHEN** no tool rule matches, Bash rules are absent or empty, and there is no enabled candidate-name violation
- **THEN** the hook returns Ask as before

#### Scenario: Permission modes do not bypass the setting

- **WHEN** Bash analysis recognizes a violation in any supported permission mode, or in a mode-less offline evaluation
- **THEN** the alias-name policy returns Deny

### Requirement: Shared policy decisions across evaluation surfaces

Live hooks and offline Bash-policy evaluation SHALL agree on the alias-name denial and its reason for the same command,
cwd, configuration, and mode. Explain, replay, and suggestions SHALL reflect the denial rather than an auto-approval.
Existing checks of Modify and ArgumentFilter replacement commands SHALL enforce the same declaration-name policy without
changing their other revalidation semantics.

#### Scenario: Explain and replay agree with the hook

- **WHEN** the live hook, normal Bash-rule testing, explain, and replay evaluate the same qualifying violation
- **THEN** each reports Deny and the same supported-name feedback

#### Scenario: Suggestions do not treat denied declarations as approved commands

- **WHEN** suggestion generation evaluates a recorded command containing a qualifying violation under the enabled policy
- **THEN** its current-policy analysis reflects Deny, not Allow

#### Scenario: Replacement cannot introduce an unapproved alias

- **WHEN** an otherwise eligible Modify or ArgumentFilter replacement contains an unapproved candidate declaration
- **AND** the setting is true
- **THEN** replacement validation returns Deny with supported-name feedback rather than returning the replacement
