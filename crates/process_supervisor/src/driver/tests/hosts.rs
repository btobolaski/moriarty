use std::{
    fs,
    os::unix::process::CommandExt,
    process::{Child, Command, ExitStatus, Stdio},
    thread,
};

use rustix::{
    io::Errno,
    process::{Pid, WaitId, WaitIdOptions, getpgrp, waitid},
};
use tempfile::TempDir;
use tokio::runtime::Builder;

use super::*;

fn wait_for_exit(child: &mut Child, budget: Duration) -> io::Result<ExitStatus> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if start.elapsed() >= budget {
            return Err(io::ErrorKind::TimedOut.into());
        }
        thread::sleep(POLL);
    }
}

#[test]
fn host_fixtures_exit_normally() {
    for (name, evidence, group) in [
        ("driver::tests::hosts::host_stuck_launch", "stuck-launch", 0),
        (
            "driver::tests::hosts::host_unconfirmed_kill",
            "unconfirmed-kill",
            0,
        ),
        ("driver::tests::hosts::host_runtime_loss", "runtime-loss", 0),
        // A group leader cannot join its child's group; this fixture must not lead one.
        (
            "process::platform::tests::host_group_change_revokes_signals",
            "host-group-rejected",
            getpgrp().as_raw_pid(),
        ),
    ] {
        let dir = TempDir::new().unwrap();
        let marker = dir.path().join("completed");
        let mut command = Command::new(env::current_exe().unwrap());
        command
            .args(["--ignored", "--exact", name, "--nocapture"])
            .env("SUPERVISOR_HOST_COMPLETED", &marker)
            .env("HOME", dir.path())
            .env("XDG_CONFIG_HOME", dir.path().join("config"))
            .env("XDG_STATE_HOME", dir.path().join("state"))
            .env("XDG_CACHE_HOME", dir.path().join("cache"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .process_group(group);
        // These fixtures create only independently exiting 50 ms children (or stall
        // before creation). Rescue uses std directly, not the Owner under test.
        let mut host = command.spawn().unwrap();
        let (natural, status) = match wait_for_exit(&mut host, Duration::from_secs(10)) {
            Ok(status) => (true, Ok(status)),
            Err(_) => {
                let _ = host.kill();
                (false, wait_for_exit(&mut host, Duration::from_secs(2)))
            }
        };
        assert!(
            natural && matches!(status, Ok(status) if status.success()),
            "host failed: {name}; {status:?}"
        );
        // A stale --exact selector exits successfully after running zero tests.
        assert_eq!(fs::read_to_string(marker).unwrap(), evidence);
    }
}

fn completed(evidence: &str) {
    fs::write(env::var_os("SUPERVISOR_HOST_COMPLETED").unwrap(), evidence).unwrap();
}

#[test]
#[ignore = "subprocess host fixture"]
fn host_runtime_loss() {
    for during_startup in [false, true] {
        let runtime = Builder::new_current_thread().enable_all().build().unwrap();
        let (created_tx, created_rx) = oneshot::channel();
        let (_resume_tx, resume_rx) = oneshot::channel();
        let mut spawning = Box::pin(spawn_with(
            spec(),
            ProcessOptions::default(),
            Hooks {
                child_created: Some(created_tx),
                before_receive: during_startup.then_some(resume_rx),
                ..Hooks::default()
            },
        ));
        let (pid, process) = runtime.block_on(async {
            if during_startup {
                tokio::select! {
                    pid = created_rx => (pid.unwrap(), None),
                    _ = &mut spawning => panic!("startup must be waiting for handoff"),
                }
            } else {
                let process = (&mut spawning).await.unwrap();
                (created_rx.await.unwrap(), Some(process))
            }
        });
        drop(runtime);
        let replacement = Builder::new_current_thread().enable_all().build().unwrap();
        let error = replacement.block_on(async {
            if let Some(process) = process {
                process.wait().await.unwrap_err()
            } else {
                spawning.await.err().expect("startup driver was destroyed")
            }
        });
        let error = error.downcast_ref::<ProcessError>().unwrap();
        assert!(matches!(error, ProcessError::CleanupUnconfirmed { .. }));
        let state = error.partial();
        assert_eq!(state.launch_unconfirmed, during_startup);
        assert!(state.child_id.is_none() && state.status.is_none());
        assert!(!state.group_absent && !state.stdout.eof && !state.stderr.eof);
        assert!(
            state
                .failures()
                .any(|failure| failure.stage == Stage::Driver)
        );
        // This fixture's child exits independently after 50 ms; reap it even if
        // runtime Drop's single nonblocking attempt preceded the actual exit.
        let start = Instant::now();
        loop {
            match waitid(
                WaitId::Pid(Pid::from_raw(pid as i32).unwrap()),
                WaitIdOptions::EXITED | WaitIdOptions::NOHANG,
            ) {
                Ok(Some(_)) | Err(Errno::CHILD) => break,
                Ok(None) => {
                    assert!(start.elapsed() < Duration::from_secs(2));
                    thread::sleep(POLL);
                }
                Err(error) => panic!("fixture reap failed: {error}"),
            }
        }
    }
    completed("runtime-loss");
}

#[test]
#[ignore = "subprocess host fixture"]
fn host_stuck_launch() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let error = returned_error(
            options().with_startup_timeout(SHORT_STARTUP),
            Hooks {
                before_spawn: Some(Box::new(|_| {
                    loop {
                        thread::park();
                    }
                })),
                ..Hooks::default()
            },
        )
        .await;
        assert!(error.partial().launch_unconfirmed);
    });
    drop(runtime);
    completed("stuck-launch");
}

#[test]
#[ignore = "subprocess host fixture"]
fn host_unconfirmed_kill() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let mut driver = model().await;
        driver.owner.ops().stall_cleanup();
        let error = driver
            .run(
                options().with_cleanup_graces(Duration::ZERO, Duration::from_millis(20)),
                CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, ProcessError::CleanupUnconfirmed { .. }));
    });
    drop(runtime);
    completed("unconfirmed-kill");
}
