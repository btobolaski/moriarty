# Moriarty, an assistant for an assistant

A Rust CLI tool for analyzing Claude Code and pi session logs, reporting API costs or token usage, and enforcing
security hooks for command validation.

## Features

- **API pricing analyzer**: Analyzes Claude API usage from log directories and generates detailed cost or token reports
- **Pi cost analyzer**: Analyzes pi session logs and generates daily or per-conversation cost or token reports grouped
  by provider and model. Nonzero recorded charges are preserved; zero-cost usage (including subscription calls) is
  estimated from `~/.pi/agent/models-store.json`, with `models.json` price overrides. `PI_CODING_AGENT_DIR` overrides
  that directory. Estimates use current cached dollars-per-million-token rates, including prompt-size tiers and 1h cache
  writes, not historical prices. No network requests are made; missing prices warn and leave usage at zero. Configure
  explicit prices in `models.json` or refresh pi's catalog when rates are missing. Auxiliary `usage` records contribute
  cost/tokens but no agent turns.
- **Terminal graphs**: Renders chart-focused stacked summaries for Claude/API and pi usage via `moriarty graphs all`,
  `moriarty graphs claude`, and `moriarty graphs pi`. For `moriarty graphs all`, cost mode uses local pricing tables for
  Claude values and the same recorded-price/zero-cost-estimate policy for pi values; token mode combines homogeneous
  token counts without consulting pricing.
- **MCP servers**: Provides Model Context Protocol servers for read-only git operations, read-only jj operations, and
  project tools
- **Tool call permissioning**: Security hooks that control which tools and commands Claude Code can execute
  - **Tool rules**: Permission any tool call (Read, Write, Edit, Bash, etc.) with ordered, conjunctive conditions for
    top-level key presence, absence, typed equality, and scalar regex matching, plus optional `allow_local` checks for
    `path` / `file_path` under the session's working directory. Legacy single-field regex rules remain supported; an
    unexpandable or invalid condition regex drops the whole rule rather than broadening it.
  - **Permission-mode restrictions**: Both tool and Bash rules can opt into exact Claude Code modes with `modes`; an
    omitted restriction is a permanently supported unrestricted form that applies everywhere, while `modes = []`
    disables the rule.
  - **Bash rules**: Fine-grained command validation with pattern matching, modification, and argument filtering
  - See [BASH_RULES.md](./BASH_RULES.md) for complete configuration guide

## Security

While tool call permissioning is labeled as a security control, it should be treated as a very porous control. As long
as you don't add a rule that allows, it will prevent an agent from doing something like `rm -rf /` or executing a binary
that it downloads from somewhere. It will not protect you from actively hostile agent. An example of this is that you're
very likely to have a rule allow the agent to write tests without manual approval. All the agent needs to do is right a
test to do the thing that it wants to do and then the rules do not provide protection. It is better to think of the
rules as guidance on what the agent is allowed to do.

## Process execution

On Linux/macOS, project commands, checks, read-only git/jj tools, and git root-discovery probes use the trusted
`process_supervisor` lifecycle. They inherit the environment, pass literal arguments, and use null stdin. Approved
program spelling and each entry point's existing cwd policy are preserved. This is not a security sandbox.

- **Bounded captures (breaking change):** at most 1 MiB of raw bytes per stdout/stderr stream. Project/check reports
  disclose truncation; VCS tools and root discovery reject incomplete output. Lossy UTF-8 conversion can expand retained
  text. Truncation alone does not fail an otherwise successful project command/check.
- **Budgets:** 5 s startup, no new execution deadline for project commands, CLI checks, or VCS tools, and separate 2 s
  TERM / 1 s post-KILL cleanup budgets. Stop-hook/MCP checks keep four-way concurrency, a five-minute whole-batch
  deadline including queueing/startup after verification, and a 10 MiB aggregate formatted-output gate. Rejection stops
  queued work and cancels/awaits active work; cleanup may extend past the five-minute execution allowance.
- **Cancellation:** existing MCP `notifications/cancelled` affects only its request. Admission and startup inspect
  cancellation even inside a work poll; already-started work retains its awaited owner. Disconnect, transport failure,
  and graceful SIGINT/SIGTERM close admission, cancel affected work, and await settlement before normal runtime exit,
  subject to the blocked-stdout limitation below. MCP/hook hosts exit after that drain even with stdin held open; hook
  reads yield to cancellation, and only these hosts use Tokio background runtime shutdown to avoid joining uncancellable
  input reads. MCP stalled response writes, flushes, and transport closure wake on cancellation without resumed peer
  reads; owned work still drains and partial delivery is a host failure. Approval-lock waiting is cancellable; admitted
  load-modify-save transactions retain their lock and actual I/O through caller loss and host drain (not rollback). With
  a live runtime, contained work-future panics still cancel and await admitted work. Retained failures keep safe write
  context and available I/O causes; combined initiating/cleanup errors preserve their contextual sources without
  exposing captured child output. Only genuine approval-file absence initializes an empty store; malformed/unreadable
  stores remain errors and are not overwritten by an update. Approval initialization restores the terminal on errors or
  interruption. Startup creation/late cleanup can remain uncertain; SIGKILL/crash containment is not guaranteed.
  Git/root-resolution failures after the canonical-input snapshot fail closed, never becoming an alternate approval key
  or successful no-checks result; initial missing-input setup keeps its existing no-checks behavior. Git PATH entries
  (including relative/empty entries) use the probe cwd; unset PATH uses native executable-search defaults without an
  environment override. Known English non-repository and filesystem-boundary diagnostics allow ordinary fallback;
  translated or unknown wording fails closed even for a standalone project. Root whitespace is preserved; non-UTF-8
  repository/workspace and original/canonical executable approval identities are rejected before lookup/save without
  changing the approval format or verified program spelling. Named empty-argv errors and safe diagnostic cause chains
  exclude captured child output.
- **Blocked-stdout limitation (owner-accepted):** stdout from `moriarty hooks exec`, `moriarty test checks`, and
  `moriarty test project-tools` remains blocking. A full, unread output pipe can prevent cooperative signal shutdown and
  require SIGKILL; no output-write timeout or graceful signal exit is promised in that condition. This exception does
  not apply to MCP transport shutdown or relax cancellation/admission and owned-work settlement obligations. Forceful
  termination proves neither owned-child cleanup nor completed approval persistence.
- **Compatibility:** MCP remains capped at 2025-11-25, supporting 2024-11-05, 2025-03-26, 2025-06-18, and 2025-11-25
  without task-management APIs. `moriarty mcp install` is excluded and retains inherited terminal streams and behavior.

## Requirements

Moriarty requires Linux or macOS; the entire binary depends on the platform-specific process supervisor.

### Testing

**⚠️ Important**: Tests MUST be run using `cargo nextest`:

```bash
cargo nextest run
```

Do **NOT** use `cargo test` as tests use `std::env::set_var` to set up isolated XDG config directories, which is only
safe when each test runs in its own process. `cargo nextest` runs each test in a separate process, making this safe and
preventing tests from clobbering your real config files.

To install `cargo nextest`:

```bash
cargo install cargo-nextest
```

Process-group safety, signal escalation, reaping, and pipe-draining guarantees are tested in `process_supervisor`;
Moriarty tests verify authorization, caller output policies, cancellation routing, and awaited ownership at that
boundary.

## Development

See the [development reference](doc/development.md) for build, run, test, and benchmark commands. [AGENTS.md](AGENTS.md)
contains agent guidelines and a topic-based documentation map.

# GitHub Mirror

The GitHub mirror exists as a push mirror for my copy of the repository. I do monitor this repository however, my
development activity will not be visible in the repository.
