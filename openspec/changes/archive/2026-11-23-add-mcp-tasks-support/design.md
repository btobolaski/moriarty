# Design

## Context

`ToolRunner` routes five tools through two implementations: `run_command` for lint, build, formatter and tests, and `run_checks_impl` for the check batch. `OwnedService` currently wraps each `tools/call` in `execution::Scope::run(context.ct, ...)`, then combines the result and cleanup with `settle_tool_response`. `serve_until` owns the enclosing scope through disconnect and signal shutdown. A task must outlive its creating response, not this enclosing scope.

## Goals / Non-Goals

Deliver the [July 2026 Tasks wire contract](https://tasks.extensions.modelcontextprotocol.io/specification/2026-07-28/tasks) without introducing a second command runner. Nothing is written to disk or shared between stdio connections. No subscriptions, interactive task inputs, legacy experimental Tasks protocol, active-work cap, new execution deadlines, configuration or client-side changes.

## Decisions

1. **Negotiate on the wire; admit at the router.** Leave revision and required-metadata validation, including error precedence, to rmcp. Project-tools discovery under MCP `2026-07-28` advertises Tasks; legacy initialization omits it even for declaring clients. Valid legacy management calls get `-32601` whether or not initialization declared Tasks. Valid `2026-07-28` management calls without Tasks get `-32021` before task lookup. In rmcp 3.6 the management gate consults the handler's `get_info()` as well as client capabilities, so changing discovery alone cannot enable the methods. Moriarty’s outer service sees the request before rmcp validates it; the task handler runs after the capability gate. Neither is a validated pre-gate hook. Do not track a second lifecycle state machine.

   Only a `2026-07-28` call declaring Tasks and passing the existing router's name and argument validation is eligible. Unknown names return the router's immediate JSON-RPC error. Arguments that fail `RunArgs` deserialization return its immediate `isError: true` tool result. Neither path admits work or returns a handle. Do not add a second validator, a latency heuristic or a request-level `task` switch.
2. **Transfer ownership at admission.** Check the initiating request is still live. Register the connection-local ID, task token and host-tracked owner before returning the creation response. Registration is not a first-poll retention guarantee: ordinary expiry and capacity eviction apply to terminal records, not active work. Failed admission starts no work and exposes no handle. Once admitted, the owner runs approval, configuration checks and execution exactly once through the existing tool path inside the host scope, using the task token rather than the creating request's token. A bare spawn does not inherit the task-local submission scope. The existing host scope must retain admitted work through cleanup and terminal publication; an owner must never wait on the tracker that tracks it. The task owner must preserve the outcome it observes for Decision 3. With confirmed cleanup, a returned tool result stays `completed` and a returned JSON-RPC error stays `failed` with that error, even if cancellation was requested concurrently. The current `Scope::run` result path can discard a returned value when it rechecks cancellation; a later publisher cannot recover it. Host shutdown closes admission, signals all owners and drains them without a poll or successful response write.
3. **Classify at the task execution boundary.** Direct calls keep `settle_tool_response`. For tasks, use the outcome observed by the execution owner:

   | Observation | Terminal status and payload |
   | --- | --- |
   | Tool returned a result; cleanup confirmed | `completed` with that result, including `isError: true` |
   | Tool returned a JSON-RPC error; cleanup confirmed | `failed` with the original error |
   | Owner interrupted the work; cleanup confirmed | `cancelled` with neither result nor error |
   | Supervision reports that cleanup failed or cannot be confirmed | `failed` with the reported cleanup diagnostic and any initiating diagnostic |

   A panic unwinds or aborts; it is not an outcome in this table. Task execution must not catch the unwind, infer a panic from a closed channel, or synthesize a terminal payload from `Drop`. Ordinary RAII cleanup during unwind is fine. A panic guarantees neither a terminal record nor a response. This does not change host drain on graceful shutdown. The pre-existing unwind handling in shared `Scope::run` and the main spec’s “Contained work panic still settles admitted operations” scenario are a separate CLI and direct-request concern.

   `tasks/cancel` signals the task token and acknowledges intent, not outcome. A flag alone cannot relabel a returned result or independent failure. Shared checks, hooks and commands need not reconstruct why they returned a result. After rmcp decodes `tasks/update`, acknowledge a known ID without inspecting input responses; none of these tools request input.
4. **Store active tasks separately from a small terminal FIFO.** Use rmcp's `Task`, `DetailedTask` and acknowledgement types, not its 3.6 `TaskManager` as the store: its TTL sweep can abort live work, its shutdown aborts futures, and it lacks count-based eviction. Index active tasks by ID. On settlement, atomically move the record to a completion-ordered queue and prune expired records and any overflow. A linear ID lookup among at most 20 terminal records is sufficient. Use a monotonic clock for expiry age; FIFO push order is settlement order. Prune expired entries on the next task-state access or settlement, before lookup or insertion. An idle host may still hold up to 20 expired records; there is no expiry timer to wake or shut down. Removal drops payload and metadata. Polls, updates and cancellation acks do not reorder entries.
5. **Separate retention from the wire clock.** `ttlMs` is measured from creation, not settlement. Use `null` while working; on settlement publish elapsed lifetime plus one hour as the maximum age from creation. Capacity eviction can remove a result sooner. Use the SDK's wire timestamps and suggested poll interval, not wall-clock adjustments, to describe the task to clients; do not use result expiry to cancel work.

## Risks / Trade-offs

- Large outputs consume memory until eviction → store the already bounded tool result once. Twenty bounds terminal record count, not concurrent work or total process memory.
- Fast completion or cancel racing with shutdown → register before replying, serialize terminal publication with eviction, and never release process/approval completion owners in response to a poll, eviction or failed write.
- The SDK can fall back to initialization capabilities → test a legacy client explicitly declaring Tasks as well as `2026-07-28` requests changing capabilities on the same connection.

**SDK evidence:** In rmcp 3.6.0, `src/task_manager.rs` (`sweep_expired`, `shutdown`) shows the abort and retention behavior; `src/model/task.rs` defines the nullable TTL and flattened task result shapes.

## Migration Plan

No configuration or storage migration. Ship discovery and functioning handlers together. Existing clients that omit the extension keep direct semantics. Rolling back requires restarting the server; task IDs never survive that restart.
