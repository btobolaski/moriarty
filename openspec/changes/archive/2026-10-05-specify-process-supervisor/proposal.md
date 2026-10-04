# Proposal

## Why

Moriarty needs an explicit contract for asynchronously running external jobs without leaving ordinary descendants alive
or hanging on stalled startup or unconfirmed cleanup. The existing standalone `process_supervisor` crate is the starting
point, but its suitability must be judged against requirements rather than inferred from its implementation.

## What Changes

- Establish the `process-supervision` capability: a native async Rust API with owned process lifetimes, persistent
  cancellation, process-group cleanup, TERM-to-KILL escalation, and bounded abandonment.
- Require null stdin and independently bounded, raw-byte stdout/stderr capture with truthful truncation and EOF
  reporting; no standard-stream inheritance.
- Specify startup, execution, cleanup, identity-safety, error-reporting, and host-runtime boundaries, preserving
  relevant requirements documented by the existing crate.
- Provide scenario-based acceptance criteria for later agents to assess the standalone supervisor and repair confirmed
  deficiencies. This proposal does not perform that assessment or presume that repairs are necessary.
- Exclude migration of all existing Moriarty process callers. This workflow writes planning artifacts only.

## Capabilities

### New Capabilities

- `process-supervision`: Bounded asynchronous supervision of ordinary Linux/macOS process groups, with explicit
  ownership, cancellation, capture, cleanup, and uncertainty guarantees.

### Modified Capabilities

None; this project has no existing OpenSpec capabilities.

## Impact

- Planning artifacts define the contract for `crates/process_supervisor` and its public Rust API. Any later remediation
  is limited to that standalone crate, its tests, and directly related documentation.
- Existing Moriarty launch sites, command approvals, MCP behavior, dependency wiring, and application-wide shutdown
  policy remain unchanged.
- No new dependencies, runtime services, shell wrappers, stdin-input API, or cross-platform containment mechanism are
  proposed.
