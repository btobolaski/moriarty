use std::{env, future::Future, thread, time::Duration};

use tokio::time::timeout;

use super::*;

mod hosts;
mod launcher;
mod phases;

fn assert_contains(haystack: &[u8], needle: &[u8]) {
    assert!(haystack.windows(needle.len()).any(|bytes| bytes == needle));
}

fn failure(state: &PartialState, stage: Stage) -> &io::Error {
    let failure = state.failures().next().unwrap();
    assert_eq!(failure.stage, stage);
    &failure.source
}

fn stage_count(trace: &[Stage], stage: Stage) -> usize {
    trace.iter().filter(|seen| **seen == stage).count()
}

fn spec() -> ProcessSpec {
    ProcessSpec {
        program: env::current_exe().unwrap().into(),
        args: [
            "--ignored",
            "--exact",
            "driver::tests::fixture",
            "--nocapture",
            "--quiet",
        ]
        .map(Into::into)
        .to_vec(),
        cwd: None,
    }
}

const SHORT_STARTUP: Duration = Duration::from_millis(100);

fn options() -> ProcessOptions {
    ProcessOptions::default()
        .with_startup_timeout(Duration::MAX)
        .with_cleanup_graces(Duration::from_millis(30), Duration::from_secs(1))
}

async fn within<T>(future: impl Future<Output = T>) -> T {
    timeout(Duration::from_secs(5), future).await.unwrap()
}

async fn returned_error(options: ProcessOptions, hooks: Hooks) -> ProcessError {
    let error = within(spawn_with(spec(), options, hooks))
        .await
        .err()
        .expect("expected failure");
    error.downcast::<ProcessError>().unwrap()
}

// Real children get no startup deadline and exit before scripting unkillable/error states;
// the model never leaves a genuinely unkillable process behind.
async fn model() -> Driver {
    let options = ProcessOptions::default().with_startup_timeout(Duration::MAX);
    let token = CancellationToken::new();
    let mut state = PartialState::default();
    let mut hooks = Hooks::default();
    let start = Instant::now();
    let permit = launch::admit(&options, &token, start, &mut hooks)
        .await
        .unwrap();
    let mut owner =
        match launch::launch(spec(), &options, &token, start, &mut state, permit, hooks).await {
            LaunchOutcome::Ready(owner) => owner,
            _ => panic!("fixture launch failed"),
        };
    state.child_id = Some(owner.id());
    let mut stdout = Stream::allocate(32).unwrap();
    let mut stderr = Stream::allocate(32).unwrap();
    setup_pipes(&mut owner, &mut stdout, &mut stderr).unwrap();
    let mut driver = Driver {
        owner,
        state,
        stdout,
        stderr,
        phase: Phase::Running(Instant::now()),
    };
    within(async {
        while !driver.owner.observe().unwrap() {
            sleep(POLL).await;
        }
        while driver.stdout.is_open() {
            driver.stdout.read().await.unwrap();
        }
        while driver.stderr.is_open() {
            driver.stderr.read().await.unwrap();
        }
    })
    .await;
    assert_contains(&driver.stdout.bytes, b"fixture-body");
    driver.owner.ops().trace_handle().clear();
    driver
}

#[test]
#[ignore = "subprocess fixture"]
fn fixture() {
    thread::sleep(Duration::from_millis(50));
    println!("fixture-body");
}
