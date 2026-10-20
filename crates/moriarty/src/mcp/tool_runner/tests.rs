use std::{
    env, fs,
    future::ready,
    io::ErrorKind,
    os::unix::{fs::symlink, process::ExitStatusExt},
    process::{Command, ExitStatus},
};

use serde_json::to_value;
use tempfile::TempDir;

use super::*;
use crate::{
    checks::{self, CheckRunOutcome},
    execution,
    mcp::tests::EXPECTED_PROTOCOL_VERSIONS,
    project_config::approvals,
    repository::{detect_repository_root, detect_workspace_root},
    test_helpers::{
        TestEnvVarGuard, assert_workspace_local_copy_ran, create_executable_script,
        setup_git_repo_with_commit, setup_isolated_xdg_config,
        setup_jj_main_and_secondary_workspace, setup_project_dir_with_config, test_report,
    },
};
use process_supervisor::{
    Completion, PartialState, ProcessError, ProcessOptions, ProcessSpec, RunningProcess,
};

async fn setup_project_dir_with_approvals(config_content: &str) -> (TempDir, TempDir) {
    let xdg_dir = setup_isolated_xdg_config();
    let temp_dir = setup_project_dir_with_config(config_content);
    approvals::approve_project_config(temp_dir.path(), config_content)
        .await
        .unwrap();
    (temp_dir, xdg_dir)
}

/// Test scaffolding for TOCTOU / binary-swap scenarios.
///
/// Creates a temp project dir with `.config/tools.toml` that maps the given
/// command `key` (lint/test/build/format) to a freshly-created executable
/// shell script whose body is `initial_body`. Returns:
///   (temp_dir, script_path, config_content)
///
/// Caller is expected to already have isolated XDG config set up.
fn create_script_project(
    key: &str,
    script_name: &str,
    initial_body: &str,
) -> (TempDir, std::path::PathBuf, String) {
    use std::io::Write;
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join(".config");
    std::fs::create_dir(&config_dir).unwrap();
    let script_path = temp_dir.path().join(script_name);
    let mut script = std::fs::File::create(&script_path).unwrap();
    writeln!(script, "#!/usr/bin/env bash").unwrap();
    writeln!(script, "{}", initial_body).unwrap();
    drop(script);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms).unwrap();
    }
    let config_content = format!("[commands]\n{key} = [\"{}\"]\n", script_path.display());
    std::fs::write(config_dir.join("tools.toml"), &config_content).unwrap();
    (temp_dir, script_path, config_content)
}

/// Overwrites `script_path` with a new bash body, preserving its executable bit.
fn overwrite_script(script_path: &std::path::Path, body: &str) {
    use std::io::Write;
    let mut script = std::fs::File::create(script_path).unwrap();
    writeln!(script, "#!/usr/bin/env bash").unwrap();
    writeln!(script, "{}", body).unwrap();
}

// load_project_settings success/error tests live in project_config::config::tests;
// duplicating them here just delayed test runs and produced jscpd noise.

/// Builds the `RunArgs` wrapper expected by `ToolRunner` for `project_dir`.
fn run_args(project_dir: &std::path::Path) -> RunArgs {
    RunArgs {
        project_dir: project_dir.to_path_buf(),
    }
}

/// Runs a single project command through the MCP tool runner against
/// `project_dir`, returning either the tool result or structured MCP error.
async fn run_project_cmd(
    command: ProjectCommand,
    project_dir: &std::path::Path,
) -> std::result::Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    ToolRunner::run_command(command, run_args(project_dir)).await
}

/// Builds `RunArgs` pointing at `temp_dir` and invokes `ToolRunner::run_command`
/// for `ProjectCommand::Test`.
async fn run_test_cmd(
    temp_dir: &TempDir,
) -> std::result::Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    run_project_cmd(ProjectCommand::Test, temp_dir.path()).await
}

/// Asserts an MCP error uses the INVALID_REQUEST code, which is how approval
/// and verification failures are surfaced to callers. The sentence wording is a static
/// template, so the error code is the structural contract.
fn assert_invalid_request(err: &rmcp::ErrorData) {
    assert_eq!(err.code, ErrorCode::INVALID_REQUEST);
}

#[tokio::test]
async fn probe_cwd_path_entries_keep_root_checks_in_both_adapters() {
    let _xdg = setup_isolated_xdg_config();
    let project = TempDir::new().unwrap();
    setup_git_repo_with_commit(project.path());
    let nested = project.path().join("nested");
    fs::create_dir_all(nested.join("tools")).unwrap();
    assert_ne!(env::current_dir().unwrap(), nested);
    let git = which::which("git").unwrap();
    symlink(&git, nested.join("tools/git")).unwrap();
    symlink(&git, nested.join("git")).unwrap();
    let script = project.path().join("check.sh");
    create_executable_script(&script, "printf 'probe-cwd-check\\n'");
    let config = format!(
        "[commands]\n[[checks]]\nname = \"root-only\"\ncommand = [{:?}]\n",
        script.to_str().unwrap()
    );
    fs::create_dir(project.path().join(".config")).unwrap();
    fs::write(project.path().join(".config/tools.toml"), &config).unwrap();
    approvals::approve_project_config(project.path(), &config)
        .await
        .unwrap();
    let expected = project.path().canonicalize().unwrap();
    for path in ["tools", ""] {
        let _path = TestEnvVarGuard::set("PATH", path);
        assert_eq!(
            detect_workspace_root(&nested).await.unwrap().as_path(),
            expected
        );
        assert_eq!(
            detect_repository_root(&nested).await.unwrap().as_path(),
            expected
        );
        let CheckRunOutcome::Ran { outputs, failures } =
            checks::run_configured_checks(&nested).await.unwrap()
        else {
            panic!("PATH {path:?} must not skip the root's checks")
        };
        assert!(failures.is_empty(), "{failures:?}");
        assert!(outputs.iter().any(|text| text.contains("probe-cwd-check")));
        let result = ToolRunner::run_checks_impl(run_args(&nested))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(false));
        assert!(
            to_value(result).unwrap()["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("probe-cwd-check")
        );
    }
}

#[tokio::test]
async fn unset_path_matches_native_git_lookup_without_inventing_absence() {
    let project = TempDir::new().unwrap();
    setup_git_repo_with_commit(project.path());
    let _path = TestEnvVarGuard::unset("PATH");
    let native = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(project.path())
        .output();
    match native {
        Ok(output) if output.status.success() => {
            assert_eq!(
                detect_workspace_root(project.path())
                    .await
                    .unwrap()
                    .as_path(),
                project.path().canonicalize().unwrap()
            );
        }
        Ok(output) => {
            let error = detect_workspace_root(project.path())
                .await
                .expect_err("native Git failure must not become repository absence");
            assert!(execution::is_execution_error(&error), "{error}");
            let expected = format!(
                "Git root discovery (--show-toplevel) failed with {}",
                output.status
            );
            assert!(error.to_string().contains(&expected), "{error}");
        }
        Err(error) => {
            assert_eq!(error.kind(), ErrorKind::NotFound);
            let error = detect_workspace_root(project.path())
                .await
                .expect_err("native lookup failure is not affirmative absence");
            assert!(error.downcast_ref::<ProcessError>().is_some(), "{error}");
            eprintln!(
                "Native-default Git unavailable: successful unset-PATH lookup not exercised; native failure remains fail-closed"
            );
        }
    }
    assert!(
        env::var_os("PATH").is_none(),
        "discovery must inherit, not override PATH"
    );
}

#[tokio::test]
async fn filesystem_boundary_absence_keeps_config_and_unknown_wording_blocks() {
    let _xdg = setup_isolated_xdg_config();
    let project = setup_project_dir_with_config(
        "[commands]\n[[checks]]\nname = \"boundary-check\"\ncommand = [\"echo\", \"ok\"]\n",
    );
    for (diagnostic, ordinary) in [
        (
            "fatal: not a git repository (or any parent up to mount point /fixture)\nStopping at filesystem boundary (GIT_DISCOVERY_ACROSS_FILESYSTEM not set).\n",
            true,
        ),
        ("fatal: bad config line 1 in file .git/config\n", false),
        (
            "fatal: not a git repository (or any parent up to mount point /fixture)\nunknown trailing error\n",
            false,
        ),
    ] {
        let handler = move |spec: ProcessSpec, _| {
            assert_eq!(spec.args.first().unwrap(), "rev-parse");
            let mut report = test_report(Completion::Exited(ExitStatus::from_raw(128 << 8)));
            report.stderr.bytes = diagnostic.as_bytes().to_vec();
            ready(Ok(report))
        };
        let work = async {
            let CheckRunOutcome::Blocked(reason) =
                checks::run_configured_checks(project.path()).await.unwrap()
            else {
                panic!("must process config or block discovery, never NoChecks")
            };
            let result = ToolRunner::run_checks_impl(run_args(project.path()))
                .await
                .unwrap();
            assert_eq!(result.is_error, Some(true));
            assert_eq!(to_value(result).unwrap()["content"][0]["text"], reason);
            assert!(
                reason.contains(if ordinary {
                    "boundary-check"
                } else {
                    "Git root discovery"
                }),
                "{reason}"
            );
        };
        execution::test_seams::with_operations(handler, work).await;
    }
}

#[tokio::test]
async fn malformed_toml_retains_parse_location_in_mcp_error() {
    let _xdg = setup_isolated_xdg_config();
    let project = setup_project_dir_with_config("[commands]\nlint = [\"echo\",\n");
    let error = run_project_cmd(ProjectCommand::Lint, project.path())
        .await
        .expect_err("invalid TOML");
    assert_invalid_request(&error);
    for expected in [
        "failed to parse project settings",
        "TOML parse error",
        "line 2",
        "column",
    ] {
        assert!(error.message.contains(expected), "{}", error.message);
    }
    assert!(error.data.is_none());
}

#[tokio::test]
async fn empty_previously_approved_argv_is_rejected_without_panic() {
    let valid = "[commands]\nlint = [\"echo\", \"ok\"]\n[[checks]]\nname = \"security\"\ncommand = [\"echo\", \"ok\"]\n";
    for check in [false, true] {
        let (project, _xdg) = setup_project_dir_with_approvals(valid).await;
        let invalid = if check {
            valid.replace("command = [\"echo\", \"ok\"]", "command = []")
        } else {
            valid.replace("lint = [\"echo\", \"ok\"]", "lint = []")
        };
        fs::write(project.path().join(".config/tools.toml"), invalid).unwrap();
        let (name, message) = if check {
            let result = run_checks_tool(project.path()).await.unwrap();
            assert_eq!(result.is_error, Some(true));
            (
                "security",
                to_value(result).unwrap()["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            )
        } else {
            let error = run_project_cmd(ProjectCommand::Lint, project.path())
                .await
                .expect_err("empty argv");
            assert_invalid_request(&error);
            ("lint", error.message.to_string())
        };
        assert!(
            message.contains(name) && message.contains("empty command array"),
            "{message}"
        );
    }
}

#[tokio::test]
async fn mcp_execution_diagnostic_never_includes_captured_child_bytes() {
    let (project, _xdg) =
        setup_project_dir_with_approvals("[commands]\nlint = [\"echo\", \"ok\"]\n").await;
    let result = execution::test_seams::with_operations(
        |spec, _| async move {
            if spec.args.first().is_some_and(|arg| arg == "rev-parse") {
                let mut report = test_report(Completion::Exited(ExitStatus::from_raw(128 << 8)));
                report.stderr.bytes =
                    b"fatal: not a git repository (or any of the parent directories): .git\n"
                        .to_vec();
                Ok(report)
            } else {
                let mut state = PartialState::default();
                state.stdout.bytes = b"PRIVATE_CHILD_BYTES".to_vec();
                state.stderr.bytes = b"PRIVATE_CHILD_BYTES".to_vec();
                Err(miette::Error::new(ProcessError::CleanupUnconfirmed {
                    state: Box::new(state),
                }))
            }
        },
        run_project_cmd(ProjectCommand::Lint, project.path()),
    )
    .await;
    let Err(error) = result else {
        panic!("uncertain cleanup must fail")
    };
    assert_eq!(error.code, ErrorCode::INTERNAL_ERROR);
    for cause in ["Failed to execute command 'lint'", "cleanup"] {
        assert!(error.message.contains(cause), "{}", error.message);
    }
    for private in [
        "PRIVATE_CHILD_BYTES".to_owned(),
        format!("{:?}", b"PRIVATE_CHILD_BYTES"),
    ] {
        assert!(!error.message.contains(&private), "{}", error.message);
    }
}

#[tokio::test]
async fn interrupted_discovery_keeps_stop_hook_blocking_reason_in_mcp_result() {
    let _xdg = setup_isolated_xdg_config();
    let project = setup_project_dir_with_config(
        "[commands]\n[[checks]]\nname = \"check\"\ncommand = [\"echo\", \"ok\"]\n",
    );
    let cause = "Startup cancelled; cleanup unconfirmed";
    let outcome = execution::test_seams::with_result(
        Err(cause),
        checks::run_configured_checks(project.path()),
    )
    .await
    .unwrap();
    let CheckRunOutcome::Blocked(reason) = outcome else {
        panic!("Stop hook must block")
    };
    let result = execution::test_seams::with_result(
        Err(cause),
        ToolRunner::run_checks_impl(run_args(project.path())),
    )
    .await
    .unwrap();
    assert_eq!(result.is_error, Some(true));
    let result = to_value(result).unwrap();
    assert_eq!(result["content"][0]["text"], reason);
    assert!(reason.contains(cause));
}

#[tokio::test]
async fn git_root_failures_block_checks_in_both_adapters() {
    let _xdg = setup_isolated_xdg_config();
    for (case, expected) in [
        ("corrupt config", "Git root discovery"),
        ("vanished cwd", "process start failed"),
        ("unresolvable root", "Failed to resolve Git"),
        ("late canonicalization", "Failed to canonicalize path"),
    ] {
        for mcp in [false, true] {
            let project = TempDir::new().unwrap();
            if case == "corrupt config" || case == "late canonicalization" {
                setup_git_repo_with_commit(project.path());
            }
            if case == "corrupt config" {
                fs::write(project.path().join(".git/config"), "[broken\n").unwrap();
            }
            let nested = project.path().join("nested");
            fs::create_dir(&nested).unwrap();
            let missing = project.path().join("missing-root");
            let work = async {
                let reason = if mcp {
                    let result = ToolRunner::run_checks_impl(run_args(&nested))
                        .await
                        .unwrap();
                    assert_eq!(result.is_error, Some(true), "{case}");
                    to_value(result).unwrap()["content"][0]["text"]
                        .as_str()
                        .unwrap()
                        .to_owned()
                } else {
                    let CheckRunOutcome::Blocked(reason) =
                        checks::run_configured_checks(&nested).await.unwrap()
                    else {
                        panic!("{case} must block Stop checks")
                    };
                    reason
                };
                assert!(reason.contains(expected), "{case}: {reason}");
                if case == "late canonicalization" {
                    let cause = nested.canonicalize().unwrap_err().to_string();
                    assert!(
                        reason.contains(&cause),
                        "{case}, MCP={mcp}: {reason}; missing I/O cause: {cause}"
                    );
                }
            };
            if case == "corrupt config" {
                work.await;
            } else {
                execution::test_seams::with_operations(
                    move |spec, _| {
                        let missing = missing.clone();
                        async move {
                            if case == "vanished cwd" {
                                fs::remove_dir(spec.cwd.as_ref().unwrap()).unwrap();
                                RunningProcess::spawn(spec, ProcessOptions::default())
                                    .await?
                                    .wait()
                                    .await
                            } else if case == "late canonicalization" {
                                assert_eq!(spec.args.last().unwrap(), "--git-common-dir");
                                let cwd = spec.cwd.clone().unwrap();
                                let common = cwd.parent().unwrap().join(".git").canonicalize().unwrap();
                                let mut report = RunningProcess::spawn(spec, ProcessOptions::default())
                                    .await?.wait().await?;
                                assert!(matches!(report.completion, Completion::Exited(status) if status.success()), "first probe must succeed: {:?}", report.completion);
                                // Preserve the successful probe's identity after deleting its relative-path base.
                                report.stdout.bytes = format!("{}\n", common.display()).into_bytes();
                                fs::remove_dir(cwd).unwrap();
                                Ok(report)
                            } else {
                                let mut report =
                                    test_report(Completion::Exited(ExitStatus::from_raw(0)));
                                report.stdout.bytes =
                                    format!("{}\n", missing.display()).into_bytes();
                                Ok(report)
                            }
                        }
                    },
                    work,
                )
                .await;
            }
        }
    }
}

#[tokio::test]
async fn root_identity_policy_reaches_stop_and_mcp_checks() {
    let _xdg = setup_isolated_xdg_config();
    for invalid in [false, true] {
        let project = TempDir::new().unwrap();
        let root = project.path().join(" whitespace root ");
        fs::create_dir(&root).unwrap();
        setup_git_repo_with_commit(&root);
        let nested = root.join("nested");
        fs::create_dir(&nested).unwrap();
        fs::create_dir(root.join(".config")).unwrap();
        fs::write(
            root.join(".config/tools.toml"),
            "[commands]\n[[checks]]\nname = \"root-only\"\ncommand = [\"echo\", \"ok\"]\n",
        )
        .unwrap();
        let work = async {
            let expected = if invalid { "Non-UTF-8" } else { "root-only" };
            let CheckRunOutcome::Blocked(reason) =
                checks::run_configured_checks(&nested).await.unwrap()
            else {
                panic!("root policy must never skip configured checks")
            };
            let result = ToolRunner::run_checks_impl(run_args(&nested))
                .await
                .unwrap();
            assert_eq!(result.is_error, Some(true));
            assert_eq!(to_value(result).unwrap()["content"][0]["text"], reason);
            assert!(reason.contains(expected), "{reason}");
        };
        if invalid {
            // Inject Git's raw root bytes so filesystems that reject these names
            // still exercise the policy through both callers, rather than skip it.
            execution::test_seams::with_operations(
                |spec, _| {
                    assert_eq!(spec.args.last().unwrap(), "--git-common-dir");
                    let mut report = test_report(Completion::Exited(ExitStatus::from_raw(0)));
                    report.stdout.bytes = b"/root\xff/.git\n".to_vec();
                    ready(Ok(report))
                },
                work,
            )
            .await;
        } else {
            work.await;
        }
    }
}

#[tokio::test]
async fn test_run_command_success() {
    let (temp_dir, _xdg_dir) =
        setup_project_dir_with_approvals("[commands]\ntest = [\"echo\", \"test output\"]\n").await;
    let tool_result = run_test_cmd(&temp_dir).await.unwrap();
    assert_eq!(tool_result.is_error, Some(false));
    assert_eq!(tool_result.content.len(), 2);
}

#[tokio::test]
async fn test_run_command_not_configured() {
    // Verify that commands not in tools.toml are rejected with appropriate error
    let (temp_dir, _xdg_dir) =
        setup_project_dir_with_approvals("[commands]\nlint = [\"cargo\", \"clippy\"]\n").await;
    let Err(error) = run_test_cmd(&temp_dir).await else {
        panic!("Expected error for unconfigured command");
    };
    assert_eq!(error.code, ErrorCode::RESOURCE_NOT_FOUND);
    assert!(error.message.contains("not configured"));
}

#[tokio::test]
async fn test_run_command_not_approved() {
    // Unlike other tests, deliberately skip approval setup to test rejection path
    let _xdg_dir = setup_isolated_xdg_config();
    let temp_dir = setup_project_dir_with_config("[commands]\ntest = [\"echo\", \"hello\"]\n");
    let Err(error) = run_test_cmd(&temp_dir).await else {
        panic!("Expected error for unapproved project");
    };
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert!(error.message.contains("not approved"));
}

#[tokio::test]
async fn test_run_command_nonzero_exit() {
    let (temp_dir, _xdg_dir) =
        setup_project_dir_with_approvals("[commands]\ntest = [\"sh\", \"-c\", \"exit 1\"]\n").await;
    let tool_result = run_test_cmd(&temp_dir).await.unwrap();
    assert_eq!(tool_result.is_error, Some(true));
}

#[tokio::test]
async fn test_run_command_argv_change_blocked() {
    // An argument change after approval is rejected (ItemChanged) — the new argv is not among the
    // approved versions.
    let (temp_dir, _xdg_dir) = setup_project_dir_with_approvals(
        r#"
[commands]
test = ["echo", "original"]
"#,
    )
    .await;

    let config_path = temp_dir.path().join(".config/tools.toml");
    std::fs::write(
        config_path,
        r#"
[commands]
test = ["echo", "modified"]
"#,
    )
    .unwrap();

    let Err(error) = run_project_cmd(ProjectCommand::Test, temp_dir.path()).await else {
        panic!("Expected error for argv change");
    };

    assert_invalid_request(&error);
}

#[tokio::test]
async fn test_project_command_display() {
    assert_eq!(format!("{}", ProjectCommand::Lint), "lint");
    assert_eq!(format!("{}", ProjectCommand::Test), "test");
    assert_eq!(format!("{}", ProjectCommand::Build), "build");
    assert_eq!(format!("{}", ProjectCommand::Format), "format");
}

/// Runs each of the `run_*` handler shims through a minimal approved project
/// whose matching `[commands]` entry is `echo <label>`, asserting the tool
/// returns is_error=false with a 2-item content vec.
#[tokio::test]
async fn test_run_handler_matrix() {
    type Handler = for<'a> fn(
        &'a ToolRunner,
        Parameters<RunArgs>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = std::result::Result<rmcp::model::CallToolResult, rmcp::ErrorData>,
                > + Send
                + 'a,
        >,
    >;
    let cases: &[(&str, &str, Handler)] = &[
        ("lint", "Running lint", |s, a| Box::pin(s.run_lint(a))),
        ("build", "Building project", |s, a| Box::pin(s.run_build(a))),
        ("format", "Formatting code", |s, a| {
            Box::pin(s.run_formatter(a))
        }),
        ("test", "Running tests", |s, a| Box::pin(s.run_tests(a))),
    ];

    for (key, label, handler) in cases {
        let toml = format!("[commands]\n{key} = [\"echo\", \"{label}\"]\n");
        let (temp_dir, _xdg_dir) = setup_project_dir_with_approvals(&toml).await;
        let server = ToolRunner;
        let args = RunArgs {
            project_dir: temp_dir.path().to_path_buf(),
        };
        let tool_result = handler(&server, Parameters(args))
            .await
            .unwrap_or_else(|e| panic!("{key}: handler returned Err: {e:?}"));
        assert_eq!(tool_result.is_error, Some(false), "{key}");
        assert_eq!(tool_result.content.len(), 2, "{key}");
    }
}

#[tokio::test]
async fn test_rejects_path_traversal_in_run_command() {
    let temp_dir = setup_project_dir_with_config(
        r#"
[commands]
test = ["echo", "hello"]
"#,
    );

    // Try to escape the project directory using parent directory references
    let malicious_path = temp_dir.path().join("..").join("..").join("tmp");

    let args = RunArgs {
        project_dir: malicious_path,
    };

    let Err(error) = ToolRunner::run_command(ProjectCommand::Test, args).await else {
        panic!("Expected error for path traversal attempt");
    };

    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
}

#[tokio::test]
async fn retargeted_alias_cannot_authorize_an_unapproved_workspace() {
    let _xdg = setup_isolated_xdg_config();
    let config = "[commands]\ntest = [\"cat\", \"marker\"]\n[[checks]]\nname = \"root-only\"\ncommand = [\"cat\", \"marker\"]\n";
    let first = setup_project_dir_with_config(config);
    let second = setup_project_dir_with_config(config);
    for (project, marker) in [(&first, "approved-A-data"), (&second, "UNAPPROVED-B-DATA")] {
        setup_git_repo_with_commit(project.path());
        fs::write(project.path().join("marker"), marker).unwrap();
    }
    approvals::approve_project_config(first.path(), config)
        .await
        .unwrap();
    let unapproved = run_checks_tool(second.path()).await.unwrap();
    assert_eq!(unapproved.is_error, Some(true));
    let link_dir = TempDir::new().unwrap();
    let alias = link_dir.path().join("project_link");
    for adapter in ["command", "stop", "mcp"] {
        symlink(first.path(), &alias).unwrap();
        let changed_alias = alias.clone();
        let replacement = second.path().to_path_buf();
        let handler = move |spec: ProcessSpec, _| {
            let retarget = spec
                .args
                .last()
                .is_some_and(|arg| arg == "--git-common-dir");
            let changed_alias = changed_alias.clone();
            let replacement = replacement.clone();
            async move {
                let report = RunningProcess::spawn(spec, ProcessOptions::default())
                    .await?
                    .wait()
                    .await?;
                // Return the first probe only after the alias names B; the next discovery must stay at A.
                if retarget {
                    fs::remove_file(&changed_alias).unwrap();
                    symlink(&replacement, &changed_alias).unwrap();
                }
                Ok(report)
            }
        };
        let output = execution::test_seams::with_operations(handler, async {
            if adapter == "stop" {
                let CheckRunOutcome::Ran { outputs, failures } =
                    checks::run_configured_checks(&alias).await.unwrap()
                else {
                    panic!("must run A's configured check, never skip checks")
                };
                assert!(failures.is_empty(), "{failures:?}");
                outputs.join("\n")
            } else {
                let result = match adapter {
                    "mcp" => run_checks_tool(&alias).await,
                    "command" => run_project_cmd(ProjectCommand::Test, &alias).await,
                    _ => panic!("unknown adapter: {adapter}"),
                }
                .unwrap();
                assert_eq!(result.is_error, Some(false), "{adapter}");
                to_value(result).unwrap()["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            }
        })
        .await;
        assert_eq!(
            alias.canonicalize().unwrap(),
            second.path().canonicalize().unwrap()
        );
        assert!(output.contains("approved-A-data"), "{adapter}: {output}");
        assert!(!output.contains("UNAPPROVED-B-DATA"), "{adapter}: {output}");
        fs::remove_file(&alias).unwrap();
    }
}

#[tokio::test]
async fn test_detects_binary_swap_toctou_attack() {
    // TOCTOU attack: Approve legitimate binary, then swap with malicious one.
    let _xdg_dir = setup_isolated_xdg_config();
    let (temp_dir, script_path, config_content) =
        create_script_project("test", "legitimate.sh", "echo 'legitimate'");
    approvals::approve_project_config(temp_dir.path(), &config_content)
        .await
        .unwrap();

    // Swap the binary with malicious content after approval.
    overwrite_script(
        &script_path,
        "echo 'malicious'\nrm -rf /", // simulated malicious command
    );

    let args = RunArgs {
        project_dir: temp_dir.path().to_path_buf(),
    };
    let Err(error) = ToolRunner::run_command(ProjectCommand::Test, args).await else {
        panic!("TOCTOU attack should be detected - binary was swapped after approval");
    };
    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
}

#[tokio::test]
async fn test_detects_symlink_target_change_toctou() {
    // TOCTOU via symlink: Approve binary via symlink, then change symlink target
    // This tests that canonical path verification prevents symlink manipulation
    #[cfg(not(unix))]
    return; // Skip on non-Unix systems

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let _xdg_dir = setup_isolated_xdg_config();

        let temp_dir = TempDir::new().unwrap();
        let config_dir = temp_dir.path().join(".config");
        std::fs::create_dir(&config_dir).unwrap();

        // Create legitimate binary
        let legitimate_path = temp_dir.path().join("legitimate.sh");
        let mut legitimate = std::fs::File::create(&legitimate_path).unwrap();
        writeln!(legitimate, "#!/usr/bin/env bash").unwrap();
        writeln!(legitimate, "echo 'legitimate'").unwrap();
        drop(legitimate);
        let mut perms = std::fs::metadata(&legitimate_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&legitimate_path, perms).unwrap();

        // Create malicious binary
        let malicious_path = temp_dir.path().join("malicious.sh");
        let mut malicious = std::fs::File::create(&malicious_path).unwrap();
        writeln!(malicious, "#!/usr/bin/env bash").unwrap();
        writeln!(malicious, "echo 'malicious'").unwrap();
        drop(malicious);
        let mut perms = std::fs::metadata(&malicious_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&malicious_path, perms).unwrap();

        // Create symlink pointing to legitimate binary
        let symlink_path = temp_dir.path().join("script.sh");
        std::os::unix::fs::symlink(&legitimate_path, &symlink_path).unwrap();

        let config_content = format!(
            r#"
[commands]
test = ["{}"]
"#,
            symlink_path.display()
        );

        std::fs::write(config_dir.join("tools.toml"), &config_content).unwrap();

        // Approve via symlink (resolves to legitimate binary)
        approvals::approve_project_config(temp_dir.path(), &config_content)
            .await
            .unwrap();

        // TOCTOU attack: Change symlink to point to malicious binary
        std::fs::remove_file(&symlink_path).unwrap();
        std::os::unix::fs::symlink(&malicious_path, &symlink_path).unwrap();

        let Err(error) = run_project_cmd(ProjectCommand::Test, temp_dir.path()).await else {
            panic!("Symlink TOCTOU attack should be detected - target was changed");
        };

        assert_invalid_request(&error);
    }
}

#[tokio::test]
async fn test_full_approval_lifecycle() {
    // Integration test: approve → execute → modify config → reject → re-approve → execute.
    let xdg_dir = setup_isolated_xdg_config();
    let (temp_dir, script_path, config_content_v1) =
        create_script_project("test", "test.sh", "echo 'test'");
    let config_dir = temp_dir.path().join(".config");
    approvals::approve_project_config(temp_dir.path(), &config_content_v1)
        .await
        .unwrap();

    // Step 2: Execute command - should succeed
    let args = run_args(temp_dir.path());

    let result = ToolRunner::run_command(ProjectCommand::Test, args.clone()).await;
    result.expect("Initial execution should succeed");

    // Step 3: Modify tools.toml — change the test command's argv (an argument change triggers
    // re-approval under the argv+binary binding model).
    let config_content_v2 = format!(
        r#"
[commands]
test = ["{}", "extra-arg"]
"#,
        script_path.display()
    );

    std::fs::write(config_dir.join("tools.toml"), &config_content_v2).unwrap();

    // Step 4: Attempt execution - should fail because the new argv is not approved.
    let Err(error) = ToolRunner::run_command(ProjectCommand::Test, args.clone()).await else {
        panic!("Execution should fail after argv modification");
    };

    assert_invalid_request(&error);

    // Step 5: Re-approve with new config
    approvals::approve_project_config(temp_dir.path(), &config_content_v2)
        .await
        .unwrap();

    // Step 6: Execute command again - should succeed with new approval
    let result = ToolRunner::run_command(ProjectCommand::Test, args).await;
    result.expect("Execution should succeed after re-approval");

    // Keep xdg_dir alive
    drop(xdg_dir);
}

#[tokio::test]
async fn test_approval_lifecycle_with_binary_modification() {
    // Integration test: approve → modify binary → reject → re-approve → execute.
    let xdg_dir = setup_isolated_xdg_config();
    let (temp_dir, script_path, config_content) =
        create_script_project("build", "build.sh", "echo 'version 1'");
    approvals::approve_project_config(temp_dir.path(), &config_content)
        .await
        .unwrap();

    // Execute - should succeed
    let args = run_args(temp_dir.path());
    let result = ToolRunner::run_command(ProjectCommand::Build, args.clone()).await;
    result.expect("Initial execution should succeed");

    // Modify the binary
    overwrite_script(&script_path, "echo 'version 2 - modified'");

    // Attempt execution - should fail due to binary hash mismatch
    let Err(error) = ToolRunner::run_command(ProjectCommand::Build, args.clone()).await else {
        panic!("Execution should fail after binary modification");
    };

    assert_invalid_request(&error);

    // Re-approve with modified binary
    approvals::approve_project_config(temp_dir.path(), &config_content)
        .await
        .unwrap();

    // Execute again - should succeed with new approval
    let result = ToolRunner::run_command(ProjectCommand::Build, args).await;
    result.expect("Execution should succeed after re-approval");

    // Keep xdg_dir alive
    drop(xdg_dir);
}

#[test]
fn test_get_info_metadata() {
    let server = ToolRunner;
    let info = server.get_info();

    assert_eq!(info.protocol_version.as_str(), "2025-11-25");
    assert_eq!(
        server
            .supported_protocol_versions()
            .iter()
            .map(ProtocolVersion::as_str)
            .collect::<Vec<_>>(),
        EXPECTED_PROTOCOL_VERSIONS
    );
    assert!(
        info.capabilities.tools.is_some(),
        "ToolRunner must expose tools capability"
    );
    assert_eq!(info.server_info.name, "moriarty");
    assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
    assert!(
        info.capabilities.prompts.is_none(),
        "ToolRunner should not expose prompts capability"
    );
    assert!(
        info.instructions
            .as_deref()
            .unwrap_or("")
            .contains("configured tooling"),
        "instructions should mention configured tooling: {:?}",
        info.instructions
    );
}

async fn run_checks_tool(
    project_dir: &std::path::Path,
) -> std::result::Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    ToolRunner::run_checks_impl(run_args(project_dir)).await
}

fn content_contains(result: &rmcp::model::CallToolResult, needle: &str) -> bool {
    result
        .content
        .iter()
        .filter_map(|c| c.as_text())
        .any(|t| t.text.contains(needle))
}

#[tokio::test]
async fn test_run_checks_success() {
    let _xdg = setup_isolated_xdg_config();
    let config = "[commands]\n[[checks]]\nname = \"rel\"\ncommand = [\"./check.sh\"]\n";
    let (_base, _main, workspace) =
        setup_jj_main_and_secondary_workspace(config, "check.sh", false);
    approvals::approve_project_config(&workspace, config)
        .await
        .unwrap();
    let nested = workspace.join("nested");
    fs::create_dir(&nested).unwrap();
    create_executable_script(&nested.join("check.sh"), "echo shadow-copy\npwd -P");
    let cwd = workspace.canonicalize().unwrap();
    for caller in [&workspace, &nested] {
        let result = run_checks_tool(caller).await.unwrap();
        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.content.len(), 1);
        let text = &result.content[0].as_text().unwrap().text;
        assert_workspace_local_copy_ran(text, &workspace);
        assert!(!text.contains("shadow-copy"));
        assert!(text.lines().any(|line| Some(line) == cwd.to_str()));
    }
}

#[tokio::test]
async fn test_run_checks_failure() {
    let (temp_dir, _xdg_dir) = setup_project_dir_with_approvals(
        "[commands]\n[[checks]]\nname = \"boom\"\ncommand = [\"sh\", \"-c\", \"exit 3\"]\n",
    )
    .await;
    let tool_result = run_checks_tool(temp_dir.path()).await.unwrap();
    assert_eq!(tool_result.is_error, Some(true));
    // One per-check output block plus the appended "Checks failed" summary.
    assert_eq!(tool_result.content.len(), 2);
    assert!(content_contains(&tool_result, "Checks failed"));
    assert!(content_contains(&tool_result, "boom"));
    assert!(content_contains(&tool_result, "exit code: 3"));
}

#[tokio::test]
async fn test_run_checks_no_checks_configured() {
    // Commands but no [[checks]]: the tool reports success with an informational note.
    let (temp_dir, _xdg_dir) =
        setup_project_dir_with_approvals("[commands]\ntest = [\"echo\", \"hi\"]\n").await;
    let tool_result = run_checks_tool(temp_dir.path()).await.unwrap();
    assert_eq!(tool_result.is_error, Some(false));
    assert!(content_contains(&tool_result, "No checks"));
}

#[tokio::test]
async fn test_run_checks_not_approved() {
    // Unapproved checks fail closed, surfaced as a tool-result error with actionable guidance.
    let _xdg_dir = setup_isolated_xdg_config();
    let temp_dir = setup_project_dir_with_config(
        "[commands]\n[[checks]]\nname = \"ok\"\ncommand = [\"echo\", \"hi\"]\n",
    );
    let tool_result = run_checks_tool(temp_dir.path()).await.unwrap();
    assert_eq!(tool_result.is_error, Some(true));
    assert!(content_contains(&tool_result, "not approved"));
}
