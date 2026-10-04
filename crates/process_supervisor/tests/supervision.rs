#![forbid(unsafe_code)]

use std::{
    env, fs,
    io::{self, Read, Write},
    net::Shutdown,
    os::unix::{
        fs::symlink,
        net::{UnixListener, UnixStream},
        process::{CommandExt, ExitStatusExt},
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use rustix::{
    io::Errno,
    process::{
        Pid, Signal, getpgrp, getpid, kill_process, kill_process_group, test_kill_process_group,
    },
};
use tempfile::TempDir;
use tokio::{
    signal::unix::{SignalKind, signal},
    time::{sleep, timeout},
};

use process_supervisor::{
    Capture, Completion, Interruption, InterruptionReason, ProcessError, ProcessOptions,
    ProcessReport, ProcessSpec, RunningProcess,
};

fn isolate_home<'a>(command: &'a mut Command, home: &Path) -> &'a mut Command {
    command
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_STATE_HOME", home.join("state"))
        .env("XDG_CACHE_HOME", home.join("cache"))
}

fn fixture(name: &str) -> ProcessSpec {
    ProcessSpec {
        program: env::current_exe().unwrap().into(),
        args: ["--ignored", "--exact", name, "--nocapture"]
            .map(Into::into)
            .to_vec(),
        cwd: None,
    }
}

async fn run(name: &str, options: ProcessOptions) -> ProcessReport {
    timeout(Duration::from_secs(10), async {
        RunningProcess::spawn(fixture(name), options)
            .await
            .unwrap()
            .wait()
            .await
            .unwrap()
    })
    .await
    .unwrap()
}

fn assert_contains(haystack: &[u8], needle: &[u8]) {
    assert!(haystack.windows(needle.len()).any(|bytes| bytes == needle));
}

#[tokio::test]
async fn real_statuses_and_null_stdin() {
    for (name, code, signal) in [
        ("child_success", Some(0), None),
        ("child_nonzero", Some(101), None),
        ("child_signal", None, Some(9)),
        ("child_stdin", Some(0), None),
    ] {
        let report = run(name, ProcessOptions::default()).await;
        let Completion::Exited(status) = report.completion else {
            panic!("wrong completion")
        };
        assert_eq!(status.code(), code);
        assert_eq!(status.signal(), signal);
        assert!(report.stdout.eof && report.stderr.eof);
        if name == "child_success" {
            assert_contains(&report.stdout.bytes, b"literal output");
        }
        if name == "child_stdin" {
            assert_contains(&report.stdout.bytes, b"stdin-eof");
        }
    }
}

#[tokio::test]
async fn both_streams_remain_bounded_while_draining() {
    for limit in [0, 1000] {
        let report = run(
            "child_output",
            ProcessOptions::default()
                .with_capture_limits(limit, limit)
                .unwrap(),
        )
        .await;
        assert!(matches!(report.completion, Completion::Exited(status) if status.success()));
        for capture in [report.stdout, report.stderr] {
            assert_eq!(capture.bytes.len(), limit);
            assert!(capture.truncated && capture.eof);
        }
    }
}

fn lifecycle_options() -> ProcessOptions {
    ProcessOptions::default()
        .with_capture_limits(1000, 1000)
        .unwrap()
        .with_cleanup_graces(Duration::from_millis(80), Duration::from_secs(2))
}

async fn ready(path: &Path) {
    timeout(Duration::from_secs(5), async {
        while !path.exists() {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

async fn start(mode: &str) -> (PathBuf, RunningProcess, Pid) {
    // cwd is the outer Host's isolated directory, not the repo. Its TempDir guard
    // removes kept subtrees only after fixture socket EOF/rescue, including orphans.
    let dir = TempDir::new_in(env::current_dir().unwrap()).unwrap().keep();
    fs::write(dir.join("mode"), mode).unwrap();
    let mut spec = fixture("child_lifecycle");
    spec.cwd = Some(dir.clone());
    let process = timeout(
        Duration::from_secs(5),
        RunningProcess::spawn(spec, lifecycle_options()),
    )
    .await
    .unwrap()
    .unwrap();
    ready(&dir.join("ready")).await;
    let pid = Pid::from_raw(
        fs::read_to_string(dir.join("ready"))
            .unwrap()
            .parse()
            .unwrap(),
    )
    .unwrap();
    (dir, process, pid)
}

async fn gone(pid: Pid) {
    timeout(Duration::from_secs(5), async {
        loop {
            match test_kill_process_group(pid) {
                Err(Errno::SRCH) => break,
                Ok(()) => sleep(Duration::from_millis(5)).await,
                Err(error) => panic!("group probe: {error}"),
            }
        }
    })
    .await
    .unwrap();
}

// Orphan zombies may keep a group observable even after every fixture died.
// The outer Host independently checks watchdog-socket EOF and owns rescue on failure.
fn lifecycle_outcome(
    result: miette::Result<ProcessReport>,
    interruption: Option<Interruption>,
) -> (ExitStatus, Capture, Capture) {
    match result {
        Ok(report) => {
            let (status, actual) = match report.completion {
                Completion::Exited(status) => (status, None),
                Completion::Cancelled(status) => (
                    status,
                    Some(Interruption::Execution(InterruptionReason::Cancelled)),
                ),
                Completion::TimedOut(status) => (
                    status,
                    Some(Interruption::Execution(InterruptionReason::TimedOut)),
                ),
            };
            assert_eq!(actual, interruption);
            (status, report.stdout, report.stderr)
        }
        Err(error) => {
            let error = error.downcast::<ProcessError>().unwrap();
            let ProcessError::CleanupUnconfirmed { state } = error else {
                panic!("unexpected lifecycle failure: {error}")
            };
            assert_eq!(state.interruption, interruption);
            assert!(!state.group_absent && state.failures().next().is_none());
            assert!(state.stdout.eof && state.stderr.eof);
            (
                state.status.expect("direct child reaped"),
                state.stdout,
                state.stderr,
            )
        }
    }
}

fn lifecycle_host(name: &str, peers: usize) {
    let dir = TempDir::new().unwrap();
    let mut host = Host::spawn(name, dir.path());
    assert!(host.wait(Duration::from_secs(20)).unwrap().success());
    assert_eq!(
        fs::read_to_string(dir.path().join("lifecycle.completed")).unwrap(),
        name
    );
    assert_eq!(
        host.peers.len(),
        peers,
        "every lifecycle fixture must register rescue"
    );
    host.fixtures_stopped(Duration::from_secs(2)).unwrap();
    host.terminate().unwrap();
}

#[test]
fn term_kill_descendants_and_continuous_output() {
    lifecycle_host("host_term_kill_descendants", 9);
}

#[tokio::test]
#[ignore = "subprocess host fixture"]
async fn host_term_kill_descendants() {
    for mode in ["cooperative", "ignore", "tree", "root-term", "flood"] {
        let (dir, process, pid) = start(mode).await;
        let token = process.cancellation_handle();
        drop(token.clone());
        assert!(!token.is_cancelled(), "dropping a token clone cancelled");
        if mode == "flood" {
            ready(&dir.join("stdout.flooded")).await;
            ready(&dir.join("stderr.flooded")).await;
        }
        let start = Instant::now();
        let waiter = tokio::spawn(process.wait());
        token.cancel();
        token.cancel();
        let (status, stdout, stderr) = lifecycle_outcome(
            timeout(Duration::from_secs(5), waiter)
                .await
                .unwrap()
                .unwrap(),
            Some(Interruption::Execution(InterruptionReason::Cancelled)),
        );
        if matches!(mode, "cooperative" | "root-term") {
            assert!(status.success());
            assert!(dir.join("termed").exists());
        } else {
            assert_eq!(status.signal(), Some(9));
        }
        assert!(start.elapsed() >= Duration::from_millis(80));
        assert!(stdout.eof && stderr.eof);
        assert!(stdout.bytes.len() <= 1000 && stderr.bytes.len() <= 1000);
        if mode == "flood" {
            assert_eq!((stdout.bytes.len(), stderr.bytes.len()), (1000, 1000));
            assert!(stdout.truncated && stderr.truncated);
        }
        // Orphan zombies may keep these tree groups observable; outer socket EOF proves fixture death.
        if !matches!(mode, "tree" | "root-term") {
            gone(pid).await;
        }
        token.cancel();
    }
    fs::write("lifecycle.completed", "host_term_kill_descendants").unwrap();
}

#[test]
fn naturally_exited_root_does_not_leave_background_descendants() {
    lifecycle_host("host_naturally_exited_root", 6);
}

#[tokio::test]
#[ignore = "subprocess host fixture"]
async fn host_naturally_exited_root() {
    for mode in ["root-exits", "root-exits-closed"] {
        let (dir, process, _pid) = start(mode).await;
        fs::write(dir.join("go"), "").unwrap();
        let (status, stdout, stderr) = lifecycle_outcome(
            timeout(Duration::from_secs(5), process.wait())
                .await
                .unwrap(),
            None,
        );
        assert!(status.success() && stdout.eof && stderr.eof);
    }
    fs::write("lifecycle.completed", "host_naturally_exited_root").unwrap();
}

#[test]
fn dropping_owners_cancels_but_does_not_abort_cleanup_or_other_groups() {
    lifecycle_host("host_dropping_owners", 3);
}

#[tokio::test]
#[ignore = "subprocess host fixture"]
async fn host_dropping_owners() {
    let (_dir, survivor, survivor_pid) = start("ignore").await;
    for wait in [false, true] {
        let (_dir, process, pid) = start("ignore").await;
        if wait {
            let task = tokio::spawn(process.wait());
            tokio::task::yield_now().await;
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            drop(process);
        }
        gone(pid).await;
        assert!(test_kill_process_group(survivor_pid).is_ok());
    }
    assert!(matches!(
        survivor.cancel_and_wait().await.unwrap().completion,
        Completion::Cancelled(_)
    ));
    gone(survivor_pid).await;
    fs::write("lifecycle.completed", "host_dropping_owners").unwrap();
}

// Each fixture holds membership in the group it signals. Closing a socket asks that
// live member to kill its own group, never a potentially recycled externally saved PGID.
fn fixture_watchdog() {
    let sockets = env::var_os("SUPERVISOR_FIXTURE_WATCHDOGS").expect("fixture watchdog sockets");
    for path in env::split_paths(&sockets) {
        let group = getpgrp();
        assert!(group.as_raw_pid() > 1);
        let mut socket = match UnixStream::connect(path) {
            Ok(socket) => socket,
            Err(_) => {
                kill_process_group(group, Signal::KILL).unwrap();
                unreachable!()
            }
        };
        socket
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        socket.write_all(&group.as_raw_pid().to_ne_bytes()).unwrap();
        thread::spawn(move || {
            let _ = socket.read(&mut [0]);
            // The fixture never changes process groups after joining this watchdog.
            kill_process_group(getpgrp(), Signal::KILL).unwrap();
        });
    }
}

struct Host {
    child: Child,
    listener: Option<UnixListener>,
    peers: Vec<UnixStream>,
    groups: Vec<Pid>,
}

impl Host {
    fn spawn(name: &str, dir: &Path) -> Self {
        let socket = dir.join("watchdog.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut sockets = env::var_os("SUPERVISOR_FIXTURE_WATCHDOGS")
            .map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
            .unwrap_or_default();
        sockets.push(socket);
        let child = isolate_home(&mut Command::new(env::current_exe().unwrap()), dir)
            .args(["--ignored", "--exact", name, "--nocapture"])
            .current_dir(dir)
            .env(
                "SUPERVISOR_FIXTURE_WATCHDOGS",
                env::join_paths(sockets).unwrap(),
            )
            .env("SUPERVISOR_TEST_ENV", "inherited ;$ spaces")
            .stdin(Stdio::piped())
            .stdout(if name == "host_contract" {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stderr(Stdio::inherit())
            .process_group(0)
            .spawn()
            .unwrap();
        Self {
            child,
            listener: Some(listener),
            peers: Vec::new(),
            groups: Vec::new(),
        }
    }

    fn accept(&mut self) -> io::Result<()> {
        if let Some(listener) = &self.listener {
            loop {
                match listener.accept() {
                    Ok((mut peer, _)) => {
                        peer.set_read_timeout(Some(Duration::from_millis(100)))?;
                        let mut raw = [0; 4];
                        peer.read_exact(&mut raw)?;
                        let group = Pid::from_raw(i32::from_ne_bytes(raw)).expect("fixture group");
                        if !self.groups.contains(&group) {
                            self.groups.push(group);
                        }
                        self.peers.push(peer);
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(())
    }

    fn wait(&mut self, budget: Duration) -> io::Result<ExitStatus> {
        let start = Instant::now();
        loop {
            self.accept()?;
            if let Some(status) = self.child.try_wait()? {
                return Ok(status);
            }
            if start.elapsed() >= budget {
                return Err(io::ErrorKind::TimedOut.into());
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn fixtures_stopped(&mut self, budget: Duration) -> io::Result<()> {
        let start = Instant::now();
        for peer in &mut self.peers {
            let remaining = budget.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                return Err(io::ErrorKind::TimedOut.into());
            }
            peer.set_read_timeout(Some(remaining))?;
            if peer.read(&mut [0])? != 0 {
                return Err(io::Error::other("unexpected watchdog response"));
            }
        }
        Ok(())
    }

    fn terminate(&mut self) -> io::Result<ExitStatus> {
        let _ = self.accept();
        self.listener.take();
        for peer in &self.peers {
            // Keep the read half live: local read shutdown would fabricate fixture EOF.
            let _ = peer.shutdown(Shutdown::Write);
        }
        let _ = self.child.kill();
        let status = self.wait(Duration::from_secs(2))?;
        self.fixtures_stopped(Duration::from_secs(2))?;
        Ok(status)
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

#[test]
fn forced_watchdog_failure_contains_separate_groups_and_reaps_the_host() {
    let dir = TempDir::new().unwrap();
    let mut outer = Host::spawn("host_watchdog_failure", dir.path());
    assert!(outer.wait(Duration::from_secs(10)).unwrap().success());
    outer.terminate().unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("contained")).unwrap(),
        "host reaped; all fixture sockets closed"
    );
}

#[test]
#[ignore = "subprocess host fixture"]
fn host_watchdog_failure() {
    assert_eq!(getpgrp(), getpid());
    fixture_watchdog();
    let dir = TempDir::new().unwrap();
    let mut host = Host::spawn("host_watchdog_stall", dir.path());
    let start = Instant::now();
    while host.peers.len() < 3 {
        host.accept().unwrap();
        assert!(start.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        host.groups.len(),
        2,
        "host and supervised tree must be separate groups"
    );
    let error = host.wait(Duration::ZERO).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(!host.terminate().unwrap().success());
    fs::write("contained", "host reaped; all fixture sockets closed").unwrap();
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "subprocess host fixture"]
async fn host_watchdog_stall() {
    assert_eq!(getpgrp(), getpid());
    fixture_watchdog();
    let _process = RunningProcess::spawn(fixture("child_watchdog_tree"), ProcessOptions::default())
        .await
        .unwrap();
    // Deliberately stop polling the runtime: rescue cannot rely on the driver or Drop.
    loop {
        thread::park();
    }
}

#[test]
#[ignore = "subprocess fixture"]
#[allow(
    clippy::zombie_processes,
    reason = "watchdog rescue, not supervisor cleanup, must kill this descendant"
)]
fn child_watchdog_tree() {
    assert_eq!(getpgrp(), getpid());
    fixture_watchdog();
    let home = env::current_dir().unwrap();
    let _child = isolate_home(&mut Command::new(env::current_exe().unwrap()), &home)
        .args(["--ignored", "--exact", "child_watchdog_leaf", "--nocapture"])
        .env("SUPERVISOR_TREE_GROUP", getpgrp().as_raw_pid().to_string())
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    loop {
        thread::park();
    }
}

#[test]
#[ignore = "subprocess fixture"]
fn child_watchdog_leaf() {
    assert_eq!(
        getpgrp().as_raw_pid().to_string(),
        env::var("SUPERVISOR_TREE_GROUP").unwrap()
    );
    fixture_watchdog();
    loop {
        thread::park();
    }
}

#[test]
fn literal_program_cwd_environment_and_live_host_stdin_are_preserved() {
    let dir = TempDir::new().unwrap();
    symlink(
        env::current_exe().unwrap(),
        dir.path().join("executable spelling"),
    )
    .unwrap();
    fs::write(dir.path().join("cwd.marker"), "").unwrap();
    let mut host = Host::spawn("host_contract", dir.path());
    let mut input = host.child.stdin.take().unwrap();
    input.write_all(b"sentinel").unwrap();
    // Keep the writer live: an accidentally inherited stdin must not get EOF.
    assert!(host.wait(Duration::from_secs(10)).unwrap().success());
    host.terminate().unwrap();
    let mut output = Vec::new();
    host.child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut output)
        .unwrap();
    assert!(
        !output
            .windows(b"stdin-eof".len())
            .any(|bytes| bytes == b"stdin-eof")
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("host.completed")).unwrap(),
        "contract checked"
    );
    drop(input);
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "subprocess host fixture"]
async fn host_contract() {
    assert_eq!(getpgrp(), getpid());
    fixture_watchdog();
    let mut spec = fixture("child_contract");
    spec.program = "./executable spelling".into();
    spec.args
        .extend(["--skip".into(), "literal ;$ spaces".into()]);
    let report = RunningProcess::spawn(spec, ProcessOptions::default())
        .await
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert!(matches!(report.completion, Completion::Exited(status) if status.success()));
    let mut sentinel = [0; 8];
    io::stdin().read_exact(&mut sentinel).unwrap();
    assert_eq!(&sentinel, b"sentinel");
    assert_eq!(
        fs::read_to_string("child.completed").unwrap(),
        "contract checked"
    );
    fs::write("host.completed", "contract checked").unwrap();
}

#[test]
#[ignore = "subprocess fixture"]
fn child_contract() {
    assert_eq!(getpgrp(), getpid());
    fixture_watchdog();
    assert_eq!(env::args_os().next().unwrap(), "./executable spelling");
    assert!(env::args_os().any(|arg| arg == "literal ;$ spaces"));
    assert_eq!(
        env::var("SUPERVISOR_TEST_ENV").unwrap(),
        "inherited ;$ spaces"
    );
    assert!(Path::new("cwd.marker").exists());
    child_stdin();
    fs::write("child.completed", "contract checked").unwrap();
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn repeated_completions_release_pipe_descriptors() {
    assert_contains(
        &run("child_success", ProcessOptions::default())
            .await
            .stdout
            .bytes,
        b"literal output",
    );
    let descriptors = || fs::read_dir("/proc/self/fd").unwrap().count();
    let before = descriptors();
    for _ in 0..20 {
        assert_contains(
            &run("child_success", ProcessOptions::default())
                .await
                .stdout
                .bytes,
            b"literal output",
        );
    }
    assert_eq!(descriptors(), before);
}

#[tokio::test]
async fn supervision_preserves_host_group_and_unmanaged_child_waits() {
    let host_group = getpgrp();
    let dir = TempDir::new().unwrap();
    let mut unmanaged = isolate_home(&mut Command::new(env::current_exe().unwrap()), dir.path())
        .args(["--ignored", "--exact", "child_nonzero", "--nocapture"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let report = run("child_success", ProcessOptions::default()).await;
    assert_contains(&report.stdout.bytes, b"literal output");
    assert!(matches!(report.completion, Completion::Exited(status) if status.success()));
    let process = RunningProcess::spawn(fixture("child_lifecycle_timeout"), lifecycle_options())
        .await
        .unwrap();
    let cancelled = timeout(Duration::from_secs(5), process.cancel_and_wait())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(cancelled.completion, Completion::Cancelled(_)));
    assert_eq!(getpgrp(), host_group);
    // A broad wait/reaper or SIGCHLD auto-reaping would lose this unrelated status.
    let status = timeout(Duration::from_secs(2), async {
        loop {
            if let Some(status) = unmanaged.try_wait().unwrap() {
                break status;
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(status.code(), Some(101));
}

#[tokio::test]
async fn execution_timeout_and_pre_poll_cancellation() {
    let report = run(
        "child_lifecycle_timeout",
        lifecycle_options().with_execution_timeout(Some(Duration::from_millis(100))),
    )
    .await;
    assert!(matches!(report.completion, Completion::TimedOut(_)));
    for _ in 0..10 {
        let process =
            RunningProcess::spawn(fixture("child_lifecycle_timeout"), lifecycle_options())
                .await
                .unwrap();
        process.cancellation_handle().cancel();
        let wait = process.wait();
        assert!(matches!(
            timeout(Duration::from_secs(5), wait)
                .await
                .unwrap()
                .unwrap()
                .completion,
            Completion::Cancelled(_)
        ));
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "subprocess fixture"]
#[allow(
    clippy::zombie_processes,
    reason = "the supervisor must clean descendants after this fixture exits"
)]
async fn child_lifecycle() {
    fixture_watchdog();
    let mode = fs::read_to_string("mode").unwrap();
    let mut term = signal(SignalKind::terminate()).unwrap();
    if matches!(
        mode.as_str(),
        "tree" | "root-term" | "root-exits" | "root-exits-closed"
    ) {
        let home = env::current_dir().unwrap();
        let mut command = Command::new(env::current_exe().unwrap());
        isolate_home(&mut command, &home)
            .args(["--ignored", "--exact", "child_middle", "--nocapture"])
            .stdin(Stdio::null());
        if mode == "root-exits-closed" {
            command.stdout(Stdio::null()).stderr(Stdio::null());
        }
        // Intentionally left to the supervisor, including after this leader exits.
        let _child = command.spawn().unwrap();
        ready(Path::new("middle.ready")).await;
    }
    // Existence must imply a complete PID, not an empty file between create and write.
    fs::write("ready.pending", getpid().as_raw_pid().to_string()).unwrap();
    fs::rename("ready.pending", "ready").unwrap();
    match mode.as_str() {
        "cooperative" | "root-term" => {
            timeout(Duration::from_secs(10), term.recv()).await.unwrap();
            fs::write("termed", "").unwrap();
        }
        "root-exits" | "root-exits-closed" => ready(Path::new("go")).await,
        "flood" => {
            thread::spawn(|| {
                io::stdout().write_all(&[0xff; 128 * 1024]).unwrap();
                fs::write("stdout.flooded", "").unwrap();
                loop {
                    io::stdout().write_all(&[0xff; 8192]).unwrap();
                }
            });
            thread::spawn(|| {
                io::stderr().write_all(&[0; 128 * 1024]).unwrap();
                fs::write("stderr.flooded", "").unwrap();
                loop {
                    io::stderr().write_all(&[0; 8192]).unwrap();
                }
            });
            sleep(Duration::from_secs(10)).await;
        }
        _ => sleep(Duration::from_secs(10)).await,
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "subprocess fixture"]
#[allow(
    clippy::zombie_processes,
    reason = "ordinary grandchildren are intentionally owned by the supervisor"
)]
async fn child_middle() {
    fixture_watchdog();
    let _term = signal(SignalKind::terminate()).unwrap();
    let home = env::current_dir().unwrap();
    let _child = isolate_home(&mut Command::new(env::current_exe().unwrap()), &home)
        .args(["--ignored", "--exact", "child_leaf", "--nocapture"])
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    ready(Path::new("leaf.ready")).await;
    fs::write("middle.ready", "").unwrap();
    sleep(Duration::from_secs(10)).await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "subprocess fixture"]
async fn child_leaf() {
    fixture_watchdog();
    let home = env::current_dir().unwrap();
    for (key, expected) in [
        ("HOME", home.clone()),
        ("XDG_CONFIG_HOME", home.join("config")),
        ("XDG_STATE_HOME", home.join("state")),
        ("XDG_CACHE_HOME", home.join("cache")),
    ] {
        assert_eq!(env::var_os(key).unwrap(), expected);
    }
    let _term = signal(SignalKind::terminate()).unwrap();
    fs::write("leaf.ready", "").unwrap();
    sleep(Duration::from_secs(10)).await;
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "subprocess fixture"]
async fn child_lifecycle_timeout() {
    let _term = signal(SignalKind::terminate()).unwrap();
    sleep(Duration::from_secs(10)).await;
}

#[test]
#[ignore = "subprocess fixture"]
fn child_success() {
    println!("literal output");
}

#[test]
#[ignore = "subprocess fixture"]
fn child_nonzero() {
    panic!("intentional nonzero status");
}

#[test]
#[ignore = "subprocess fixture"]
fn child_signal() {
    kill_process(getpid(), Signal::KILL).unwrap();
}

#[test]
#[ignore = "subprocess fixture"]
fn child_stdin() {
    let mut byte = [0];
    assert_eq!(io::stdin().read(&mut byte).unwrap(), 0);
    println!("stdin-eof");
}

#[test]
#[ignore = "subprocess fixture"]
fn child_output() {
    let stdout = thread::spawn(|| {
        for _ in 0..128 {
            io::stdout().write_all(&[0xff; 8192]).unwrap();
        }
    });
    for _ in 0..128 {
        io::stderr().write_all(&[0; 8192]).unwrap();
    }
    stdout.join().unwrap();
}
