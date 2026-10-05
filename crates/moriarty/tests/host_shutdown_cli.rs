#![forbid(unsafe_code)]

use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

#[cfg(target_os = "linux")]
use std::{
    env,
    ffi::OsStr,
    fs::{File, OpenOptions},
    os::unix::{ffi::OsStrExt, process::CommandExt},
};

#[cfg(target_os = "linux")]
use fs2::FileExt;
use rustix::process::{Pid, Signal, kill_process};
#[cfg(target_os = "linux")]
use rustix::{
    process::setsid,
    pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt},
    termios::{LocalModes, Winsize, tcgetattr, tcsetwinsize},
};
use tempfile::TempDir;

const LIMIT: Duration = Duration::from_secs(3);

#[test]
fn mcp_open_input_does_not_block_signal_shutdown() {
    for server in ["git-read-only", "jj-read-only", "project-tools"] {
        for (signal, label) in [(Signal::INT, "SIGINT"), (Signal::TERM, "SIGTERM")] {
            let root = TempDir::new().unwrap();
            let mut launch = command(root.path(), Path::new(env!("CARGO_BIN_EXE_moriarty")));
            launch
                .args(["mcp", server])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let mut host = Host(launch.spawn().unwrap());
            let stdout = output(host.0.stdout.take().unwrap());
            let stderr = output(host.0.stderr.take().unwrap());
            let mut input = host.0.stdin.take().unwrap();
            input.write_all(br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"host-test","version":"1"}}}
"#).unwrap();
            until_text(&stdout, "\"id\":1");
            input
                .write_all(
                    br#"{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"ping"}
"#,
                )
                .unwrap();
            until_text(&stdout, "\"id\":2");
            host.signal(signal);
            assert_eq!(host.wait().code(), Some(1), "{server} {label}");
            until_text(&stderr, label);
            drop(input);
        }
    }
}

#[test]
fn mcp_response_backpressure_does_not_block_signal_shutdown() {
    for (signal, label) in [(Signal::INT, "SIGINT"), (Signal::TERM, "SIGTERM")] {
        let root = TempDir::new().unwrap();
        fs::write(root.path().join("tracked"), "baseline\n").unwrap();
        for args in [["init", "--quiet"].as_slice(), &["add", "tracked"]] {
            let result = command(root.path(), Path::new("git"))
                .args(args)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        // The index supplies a baseline without a commit, and the response stays below the VCS capture cap.
        fs::write(
            root.path().join("tracked"),
            "unread-response-line\n".repeat(24_000),
        )
        .unwrap();
        let mut launch = command(root.path(), Path::new(env!("CARGO_BIN_EXE_moriarty")));
        launch
            .args(["mcp", "git-read-only"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut host = Host(launch.spawn().unwrap());
        let stdout = output(host.0.stdout.take().unwrap());
        let stderr = output(host.0.stderr.take().unwrap());
        let mut input = host.0.stdin.take().unwrap();
        input.write_all(br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"backpressure-test","version":"1"}}}
"#).unwrap();
        until_text(&stdout, "\"id\":1");
        input.write_all(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"diff","arguments":{"project_dir":".","args":[]}}}
"#).unwrap();
        until_text(&stdout, "diff --git");
        // A response prefix proves the owned Git operation settled. Keep both pipes open and stop consuming output.
        assert!(host.0.try_wait().unwrap().is_none());
        host.signal(signal);
        assert_eq!(
            host.wait().code(),
            Some(1),
            "{label}: a partially delivered response is not success"
        );
        until_text(&stderr, label);
        drop(input);
        drop(stdout);
    }
}

#[test]
fn hook_open_input_does_not_block_signal_shutdown() {
    for (signal, label) in [(Signal::INT, "SIGINT"), (Signal::TERM, "SIGTERM")] {
        let root = TempDir::new().unwrap();
        let mut launch = command(root.path(), Path::new(env!("CARGO_BIN_EXE_moriarty")));
        launch
            .args(["hooks", "exec"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut host = Host(launch.spawn().unwrap());
        let stderr = output(host.0.stderr.take().unwrap());
        let mut input = host.0.stdin.take().unwrap();
        let (sent, received) = mpsc::sync_channel(1);
        thread::spawn(move || {
            // More than normal pipe capacity proves the hook is consuming input, not merely starting.
            let result = input.write_all(&vec![b' '; 2 * 1024 * 1024]);
            let _ = sent.send((input, result));
        });
        let (input, result) = received
            .recv_timeout(LIMIT)
            .expect("hook must consume the gated input");
        result.unwrap();
        assert!(host.0.try_wait().unwrap().is_none());
        host.signal(signal);
        assert_eq!(host.wait().code(), Some(1), "{label}");
        until_text(&stderr, label);
        drop(input);
    }
}

fn command(root: &Path, program: &Path) -> Command {
    let mut command = Command::new(program);
    command.current_dir(root).env("TERM", "xterm-256color");
    for (name, directory) in [
        ("HOME", "home"),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_STATE_HOME", "state"),
        ("XDG_CACHE_HOME", "cache"),
    ] {
        fs::create_dir_all(root.join(directory)).unwrap();
        command.env(name, root.join(directory));
    }
    command
}

struct Host(Child);

impl Host {
    fn signal(&self, signal: Signal) {
        kill_process(Pid::from_raw(self.0.id() as i32).unwrap(), signal).unwrap();
    }

    fn wait(&mut self) -> ExitStatus {
        wait_for_child(&mut self.0).expect("host did not exit while its input/lock was held")
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        // Rescue only the owned fixture child, independently of the code under test.
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            if wait_for_child(&mut self.0).is_none() {
                eprintln!("fixture rescue could not reap owned child {}", self.0.id());
            }
        }
    }
}

fn wait_for_child(child: &mut Child) -> Option<ExitStatus> {
    let deadline = Instant::now() + LIMIT;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Some(status);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(2));
    }
}

fn output(reader: impl Read + Send + 'static) -> Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::sync_channel(4);
    thread::spawn(move || {
        let mut reader = reader;
        let mut bytes = [0; 4096];
        while let Ok(count) = reader.read(&mut bytes) {
            if count == 0 || sender.send(bytes[..count].to_vec()).is_err() {
                break;
            }
        }
    });
    receiver
}

fn until_text(receiver: &Receiver<Vec<u8>>, expected: &str) {
    let deadline = Instant::now() + LIMIT;
    let mut bytes = Vec::new();
    while !String::from_utf8_lossy(&bytes).contains(expected) {
        let chunk = receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|error| {
                panic!(
                    "waiting for {expected:?}: {error}; output: {}",
                    String::from_utf8_lossy(&bytes)
                )
            });
        bytes.extend(chunk);
        assert!(
            bytes.len() <= 256 * 1024,
            "fixture output exceeded its bound"
        );
    }
}

// Linux /proc provides an affirmative lock-acquisition boundary without a production test flag.
#[cfg(target_os = "linux")]
#[test]
fn contended_approval_exits_and_restores_terminal() {
    for signal in [Signal::INT, Signal::TERM] {
        let root = TempDir::new().unwrap();
        fs::create_dir_all(root.path().join(".config")).unwrap();
        fs::write(
            root.path().join(".config/tools.toml"),
            "[commands]\nlint = [\"echo\", \"approved\"]\n",
        )
        .unwrap();
        let master = File::from(
            openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC).unwrap(),
        );
        grantpt(&master).unwrap();
        unlockpt(&master).unwrap();
        let name = ptsname(&master, Vec::new()).unwrap();
        let slave_path = Path::new(OsStr::from_bytes(name.as_bytes()));
        let slave = OpenOptions::new()
            .read(true)
            .write(true)
            .open(slave_path)
            .unwrap();
        tcsetwinsize(
            &slave,
            Winsize {
                ws_row: 40,
                ws_col: 120,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )
        .unwrap();
        let original = tcgetattr(&slave).unwrap();
        let mut launch = command(root.path(), &env::current_exe().unwrap());
        launch
            .args([
                "--exact",
                "approval_pty_entry",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("MORIARTY_TEST_PTY", slave_path)
            .stdin(slave.try_clone().unwrap())
            .stdout(slave.try_clone().unwrap())
            .stderr(slave.try_clone().unwrap());
        let lock_path = root.path().join("config/moriarty/project_approvals.lock");
        fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .unwrap();
        lock.lock_exclusive().unwrap();
        let mut host = Host(launch.spawn().unwrap());
        let receiver = output(master.try_clone().unwrap());
        until_text(&receiver, "Project Tools Approval");
        assert!(
            !tcgetattr(&slave)
                .unwrap()
                .local_modes
                .contains(LocalModes::ICANON)
        );
        let mut input = &master;
        input.write_all(b"y").unwrap();
        until_text(&receiver, "Review");
        input.write_all(b"y").unwrap();
        until_text(&receiver, "Summary");
        input.write_all(b"y").unwrap();
        let deadline = Instant::now() + LIMIT;
        loop {
            let waiting = fs::read_dir(format!("/proc/{}/fd", host.0.id()))
                .unwrap()
                .filter_map(Result::ok)
                .any(|fd| fs::read_link(fd.path()).ok().as_deref() == Some(lock_path.as_path()));
            if waiting {
                break;
            }
            assert!(
                host.0.try_wait().unwrap().is_none(),
                "approval exited before lock acquisition"
            );
            assert!(
                Instant::now() < deadline,
                "approval did not attempt the held lock"
            );
            thread::sleep(Duration::from_millis(2));
        }
        host.signal(signal);
        assert!(
            !host.wait().success(),
            "signal cancellation must not claim approval success"
        );
        assert_eq!(
            format!("{:?}", tcgetattr(&slave).unwrap()),
            format!("{original:?}"),
            "all terminal attributes must be restored"
        );
        assert!(
            !root
                .path()
                .join("config/moriarty/project_approvals.toml")
                .exists()
        );
        drop(lock);
        assert!(
            !root
                .path()
                .join("config/moriarty/project_approvals.toml")
                .exists(),
            "no late save after release"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "exact PTY fixture only"]
fn approval_pty_entry() {
    let slave = env::var_os("MORIARTY_TEST_PTY").expect("fixture must be dispatched explicitly");
    setsid().unwrap();
    // Opening the slave after setsid gives this fixture its own controlling terminal.
    let _terminal = OpenOptions::new()
        .read(true)
        .write(true)
        .open(slave)
        .unwrap();
    let error = Command::new(env!("CARGO_BIN_EXE_moriarty"))
        .args(["approve-project", "."])
        .exec();
    panic!("approval fixture exec failed: {error}");
}
