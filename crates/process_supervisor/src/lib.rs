#![forbid(unsafe_code)]
#![doc = r#"
Bounded asynchronous supervision of ordinary Linux/macOS process groups.

This is lifecycle management, not a sandbox. Each task starts a new process group;
children and grandchildren that remain in it are included in cleanup. Deliberate
`setsid`/`setpgid` escape, changed credentials preventing signals, and execution
delegated to another service are outside the guarantee. Other targets fail at
compile time. The crate and its tests forbid unsafe code; safe std/rustix APIs own
the OS boundary. Moriarty's existing callers have **not** been migrated.

# Usage and ownership

Call inside a Tokio runtime with I/O and time enabled. Program spelling (including
symlinks), raw OS arguments and optional cwd are passed unchanged to std Command;
the host environment is inherited. There is no implicit shell, environment override,
stdin option, or raw child handle. Stdin is always null.

```no_run
use process_supervisor::{spawn, Completion, ProcessOptions, ProcessSpec};

# async fn example() -> miette::Result<()> {
let spec = ProcessSpec {
    program: "printf".into(),
    args: vec!["%s".into(), "hello".into()],
    cwd: None,
};
let options = ProcessOptions::default().with_capture_limits(4096, 4096)?;
let process = spawn(spec, options).await?;
let cancellation = process.cancellation_handle();
// Move this token to another task if cancellation must race with wait().
let report = process.wait().await?;
match report.completion {
    Completion::Exited(status) => assert!(status.success()),
    Completion::Cancelled(_) | Completion::TimedOut(_) => {}
}
drop(cancellation);
# Ok(())
# }
```

For one-shot execution use `spawn(spec, options).await?.wait().await`. A
[`RunningProcess`] is not clonable. Its independently clonable [`CancellationToken`]
is the cancellation role: `cancel()` is persistent and idempotent; dropping a clone
does not cancel. Dropping the running handle, its `wait` future, or a polled `spawn`
future requests cancellation without aborting the driver. `cancel_and_wait` follows
the same result path. For application-wide shutdown, cancel all tokens first and
then await results concurrently. No task registry or application shutdown policy is
provided.

# Budgets and results

[`ProcessOptions`] has private fields, a valid Default, and consuming setters.
Capture limits are fallible; timeout and cleanup-grace setters accept any `Duration`
and return `Self`. The TERM and post-KILL budgets are independent, so even
`with_cleanup_graces(Duration::MAX, Duration::MAX)` is valid. Defaults are 1 MiB
retained **per stream**, 5 s startup, no execution timeout,
2 s TERM grace and 1 s post-KILL grace. Execution time starts after startup succeeds.
Zero limits discard while draining; zero durations perform an immediate transition
with one nonblocking state/readiness check. Capture sizes above `isize::MAX` are
rejected by [`OptionsError`] before execution.
Allocation, thread creation and OS failures are execution errors instead.

Captures retain raw bytes (possibly invalid UTF-8/NULs) and EOF/truncation flags.
Each stream uses at most its configured retention budget plus one fixed 8 KiB read
buffer, allocated fallibly before child creation. Overflow is discarded while both
streams continue draining fairly. Exact-limit output followed by EOF is not truncated.
Caller-side text conversion may expand memory. Concurrent tasks and retained results
consume the sum of their budgets, plus metadata, allocator and kernel-pipe overhead;
there is no completed-task history. Child output is never logged or forwarded.

Every successful [`ProcessReport`] has an actual **reaped** direct-child status in
[`Completion`], confirmed group absence, and EOF on both streams. A nonzero
`Exited(status)` is still `Ok`; cleaned cancellation/timeout is explicitly
`Cancelled(status)`/`TimedOut(status)`. EOF alone does not complete a live process.
[`ProcessError`] is downcastable from `miette::Report` and exposes bounded partial
captures and at most one source diagnostic per fixed stage via `partial()`. Its four
categories are start failure, interrupted startup, supervision failure and unconfirmed
cleanup. It never fabricates a completed report. Initiating failures and phase-tagged
startup/execution interruptions remain visible when cleanup also fails. Failed readers
are closed and settled for error finalization, retaining their bytes and `eof = false`.
Once the child is reaped, the group is absent and both readers are settled, capture
failure returns `SupervisionFailed`, not cleanup uncertainty. An uninitialized reader
is not EOF; failed pipe setup closes it and remains `StartFailed` after confirmed cleanup.
The partial `term_attempted` and `kill_attempted` flags record entry into the respective
signaling path, including identity/observation preflight. They do not establish that a
signal syscall ran, succeeded, or killed anything; a preflight failure can set a flag
without reaching the signal operation. False means no path entry was recorded, not
proof about cleanup performed outside the surviving driver evidence.

# Signaling and cleanup

Before committing cancellation or an expired execution budget while Running, a fresh
non-reaping observation checks for leader exit. An already observable exit wins and
immediately KILLs leftover descendants, even if they closed both pipes. Otherwise the
cancellation/timeout cause is fixed, group TERM is sent once, captures drain through
the grace, then a final group KILL is attempted. Exit during TERM does not change that
cause; observation failure remains an error. This is observation-order precedence,
not recovery of the kernel's exit timestamp: delayed polling can observe an exit that
occurred after the nominal deadline. A successful
signal is not proof of death. Finalization shares **one** post-KILL budget across
reaping, signal-0 group-absence probes and settling both pipes (genuine EOF for success,
or a closed failed reader for errors). Total cancellation latency
is TERM grace plus post-KILL grace, plus scheduler/syscall latency, not hard real time.

Non-reaping rustix `waitid(EXITED | NOHANG | NOWAIT)` reserves the leader's identity
until the final destructive signal. Signal authority is permanently revoked before
std `try_wait` may reap, including on error/Drop paths. `ECHILD` revokes authority
rather than risking a recycled PID. The host must not concurrently reap these
children or configure SIGCHLD auto-reaping. After reaping, zombies or PGID recycling
may prevent proving group absence: the conservative result is an error, never
another destructive signal. Once absence has been observed it is retained, not
re-probed against a potentially recycled PGID while reaping or capture settles.
No external kill tool, SIGCHLD handler, subreaper or blocking wait is installed by
this crate.

# Stalled startup and host lifetime

Process creation runs only on detached std launcher threads, never an async worker
or Tokio's blocking pool. Four process-wide admission slots bound outstanding
launchers. Admission, creation, handoff and pipe setup share the startup budget. A
stalled launcher retains its slot even after startup times out; subsequent requests
can only wait within their own startup budgets. Admission and handoff wake on readiness,
cancellation or deadline, not the process-exit polling cadence. Timer sleeps are
capped at a day so every Duration is safe; elapsed time, not an overflowing absolute
deadline or intermediate wakeup, decides expiry. Startup cancellation closes the
handoff and claims any already-queued child. A late child stays owned by the launcher,
which KILLs, revokes signaling, closes pipes and polls cleanup within the post-KILL
budget. There are no required joins or per-process runtimes.

A timed-out creation syscall cannot be cancelled in-process. `launch_unconfirmed`
means a child may not yet be known or cleanable, **not** that none exists. Late cleanup
failures emit a metadata-only tracing event without captured output. Cleanup polling
is bounded, but a synchronous tracing subscriber can block diagnostic dispatch and
retain that launcher's admission slot indefinitely. This risk is accepted; releasing
the slot early would allow unbounded detached launchers. If the host exits before
creation returns, no in-process design can perform that later cleanup. Launch threads do not prevent
normal Tokio runtime destruction or host exit.

A lost driver channel reports `CleanupUnconfirmed`, not successful cleanup, with
no invented child status, group absence or EOF. Losing the startup acknowledgement
also leaves launch existence uncertain.

Normal cleanup requires a live, polled runtime. Runtime/task destruction gets only
the nonwaiting ownership guard (one final KILL attempt, then one nonblocking reap)
for handed-off children, and launcher-side late cleanup otherwise. Unconfirmed cleanup
may abandon a still-live child; eventual reaping is not guaranteed and it may become
a zombie until host exit. Host SIGKILL/crash containment is not guaranteed. Callers
should await completions, but need neither `shutdown_timeout` nor `process::exit`.

# Test containment and evidence

Nextest wrapper tests drive exact ignored libtest fixtures; body markers prevent a
stale selector's zero-test success from masquerading as execution. Independent fixture
watchdogs use Unix sockets so a live group member signals its own group, not a saved,
potentially recycled PGID. Outer guards bound direct-host reaping, and the forced
watchdog-failure scenario runs under another independent process. Rescue must not
rely solely on the supervisor under test.

`supervision_preserves_host_group_and_unmanaged_child_waits` supplies **indirect**
SIGCHLD/reaper-isolation evidence: the host group is unchanged and an unrelated child
remains waitable. It does not inspect the runtime SIGCHLD disposition. Descriptor-leak
assertions using `/proc/self/fd` are Linux-only, not Darwin evidence.
"#]

mod capture;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod driver;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod launch;
mod options;
mod process;
mod result;

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use driver::{RunningProcess, spawn};
pub use options::{OptionsError, ProcessOptions, ProcessSpec};
pub use result::{
    Capture, Completion, Interruption, InterruptionReason, PartialState, ProcessError,
    ProcessReport, Stage, StageFailure,
};
pub use tokio_util::sync::CancellationToken;
