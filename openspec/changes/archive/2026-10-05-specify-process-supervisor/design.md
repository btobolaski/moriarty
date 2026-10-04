# Design

## Context

See [proposal.md](proposal.md) for motivation and [the capability spec](specs/process-supervision/spec.md) for the
normative contract. This artifact records boundaries for later conformance work, not a verdict about the current
implementation.

The existing standalone crate documents its public contract in `crates/process_supervisor/src/lib.rs`. Its public
surface now provides `RunningProcess::spawn`, `RunningProcess::wait`, `RunningProcess::cancel_and_wait`, a clonable
cancellation token, validated `ProcessOptions`, and report/error types. The implementation separates launch, capture,
process identity/reaping, and the async lifecycle driver. Tests already include subprocess fixtures, fault seams, and
independent rescue/watchdog mechanisms. These observations ground the proposal; they do not establish compliance.

## Goals / Non-Goals

**Goals:**

- Give later assessors independently testable requirements, including ownership loss, stalled launch, identity reuse,
  reader failure, and abandonment—not just the successful execution path.
- Preserve the native Tokio-compatible semantics and safe OS boundary. The user explicitly approved the round-2
  constructor/state refactors described below; no Moriarty production caller migration is authorized.
- Permit the smallest standalone repair and regression check for each confirmed deficiency, without prescribing
  speculative changes now.

**Non-Goals:**

- New caller adapters, task registries, runtime services, environment/stdin controls, or migration of Moriarty's
  existing launch sites.
- Stronger containment for escaped descendants, host crashes, or platforms other than Linux/macOS.

## Decisions

### Recover requirements without treating code as proof

The user clarified null stdin with bounded stdout/stderr capture, accepted ordinary process-group containment, excluded
caller migration, and assigned conformance assessment to later agents. Existing crate documentation supplies the
additional ownership, timing, output privacy, identity-safety, and host-lifetime requirements. The spec states those as
behavioral obligations; it does not mark them implemented or deficient.

Alternative: infer suitability from the crate's tests or change code immediately. Rejected because those are later
assessment/apply responsibilities.

### Reuse the established lifecycle architecture where it satisfies the contract

The current design uses detached, admission-bounded launcher threads; one Tokio driver per handed-off child; bounded
stream retention with continuing drain; non-reaping exit observation; and permanent signal revocation before reaping.
Keep the crate/test `forbid(unsafe_code)` boundary and safe dependency APIs; no local FFI or pre-exec hooks are
proposed.

The finite launcher bound is contractual; its current value of four and the current 8 KiB read buffer are implementation
choices, not new API guarantees. By contrast, documented defaults and public completion/error semantics are
caller-visible and are recorded in the spec.

Alternative: replace the crate with direct async command spawning, unbounded blocking-pool launches, or an external kill
command. These choices do not remove the need for startup ownership, bounded abandonment, and identity-safe descendant
cleanup.

### Round-2 explicitly approved structural remediation

The live host-group defect requires a fresh comparison before destructive attempts and permanent rejection of unsafe
signal authority. The user separately approved the review's structural suggestions, which were not themselves behavioral
violations: use `RunningProcess::spawn` instead of the free constructor; carry interruption in the active cause and
project it into error evidence; encode capture lifecycle with an enum; and register pipes before constructing an initial
phase. Typed admission/launch failures retain creation certainty and reason without reconstructing decisions from
mutable evidence fields. Reaping begins immediately after the final transition rather than waiting for another poll. The
user also approved repairing the unrelated Moriarty locality-timeout **test** with its existing blocking-pool gate; this
does not migrate callers or change production tool-rule behavior.

### Bound waiting instead of promising impossible cleanup

Maintain separate startup, execution, TERM, and post-KILL budgets. A normal completion proves reaping, group absence,
and EOF; an uncertain result preserves bounded evidence without claiming those facts. The shared post-KILL budget
prevents separate reaping and pipe-draining waits from extending abandonment indefinitely. Cancellation handles request
cleanup; dropping an owning future must not abort the cleanup owner.

Alternative: wait until every descendant is provably dead. Rejected because an unkillable child or stalled creation
syscall must not prevent Moriarty from returning or terminating normally.

### Keep validation independent and contained

Later assessors should map spec scenarios to existing unit/integration checks, then identify only unsupported scenarios
or reproducible contradictions. Existing Nextest fixture selection, body markers, independent watchdog rescue, private
fault seams, and identity-safe cleanup should be reused. Never test unkillability by leaving real uncontrolled processes
behind; simulate stuck cleanup through the existing fault seams under an independent host watchdog.

Alternative: add broad new harnesses or CI wiring up front. Rejected until a concrete verification gap requires them.

## Risks / Trade-offs

- Process-group escape or delegated execution → Explicit exclusion; no claim of sandbox containment.
- Uninterruptible creation or unsuccessful KILL → Bound caller waiting, retain uncertain ownership evidence, and accept
  that a process can remain alive or unreaped after abandonment.
- Runtime destruction or host exit before late creation completes → Document best-effort nonwaiting cleanup and the
  limit of in-process guarantees; normal cleanup requires a live, polled Tokio runtime with I/O and time enabled.
- External reapers or SIGCHLD auto-reaping → Document host preconditions and revoke signaling rather than risk
  identifier reuse.
- Blocking synchronous tracing subscribers → Disclose that a late diagnostic can retain a launcher admission slot; do
  not promise nonblocking host callbacks or release the slot while its launcher remains active.
- Linux-only descriptor tests mistaken for macOS evidence → Record platform coverage separately; do not infer Darwin
  verification from `/proc` assertions or infer direct SIGCHLD inspection from unrelated-child wait tests.
- Very large caller-selected budgets → Treat bounds as configured allowances, not hard-real-time promises; document
  scheduler/syscall latency and aggregate capture-memory costs.

## Migration Plan

There is no Moriarty caller migration or deployment change. After an explicit apply request, other agents assess the
standalone crate against this specification and repair only demonstrated violations. If it already conforms, no
production Rust change is required. Any repair remains confined to the standalone crate and directly related
tests/documentation, is independently verifiable, and can be reverted without changing existing Moriarty callers.
