//! Shared project-check runner for the Stop hook and the `project-tools` MCP server.
//!
//! Both entry points must run a project's `[[checks]]` identically: the same approval
//! verification, the same resource limits, and the same fail-open/fail-closed policy described
//! in [`crate::hooks`]'s "Security Model: Fail-Open Design". This module is the single source of
//! that behavior so the two callers cannot drift apart; each maps [`CheckRunOutcome`] onto its own
//! result type ([`crate::hooks`] onto allow/deny, the MCP tool onto a tool result).
//!
//! This is intentionally distinct from `project_config::runner::run_all_checks`, which the
//! `moriarty test checks` CLI uses: that path has no batch deadline or aggregate output gate, but
//! still has bounded captures and also requires lint/test/build/format commands to be approved.
//! Checks here are verified on their own. The five-minute deadline includes queueing/startup;
//! cleanup has separate supervisor budgets and may extend past it. Incremental result retention
//! prevents collect-all history from bypassing the aggregate gate.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use futures::stream::{self, StreamExt};
use miette::{IntoDiagnostic, WrapErr};
use tokio::time::{Instant, sleep_until};
use tracing::{error, info};

use crate::{
    execution,
    project_config::{
        approvals::{ItemType, ProjectApprovals, VerificationResult},
        config::{Check, is_duplicate_check_name, load_project_settings},
    },
    repository::{detect_repository_root, detect_workspace_root},
};
use process_supervisor::{ProcessReport, ProcessSpec};

// Resource limits rationale:
//
// CHECK_TIMEOUT_SECS (5 minutes): Balances allowing slow checks (e.g., linting large
// codebases) while preventing indefinitely hanging processes that could DoS the system.
// Most CI checks complete in seconds; 5 minutes provides generous headroom.
//
// MAX_CONCURRENT_CHECKS (4): Limits resource consumption when many checks are configured.
// Prevents fork bombing or exhausting file descriptors if a malicious config defines hundreds
// of checks. Value chosen to match typical CPU core count while still providing parallelism.
//
// MAX_OUTPUT_SIZE (1 MiB per formatted stream, plus notice): Bounds individual converted results
// via stdout/stderr. Typical check output is <10 KiB; 1 MiB allows detailed error messages and
// verbose tooling while preventing abuse.
//
// MAX_TOTAL_OUTPUT (10 MiB total): Prevents aggregate memory exhaustion across all checks.
// This bounds retained formatted results, including headers and duplicated failure text, not active captures.
const CHECK_TIMEOUT_SECS: u64 = 300;
const MAX_CONCURRENT_CHECKS: usize = 4;
const MAX_OUTPUT_SIZE: usize = 1024 * 1024;
const MAX_TOTAL_OUTPUT: usize = 10 * 1024 * 1024;

/// Resource limits applied to a single check run. `defaults()` carries the production values
/// (the constants above); tests construct tighter limits to exercise the timeout and output-cap
/// paths without 5-minute waits or multi-megabyte output.
struct CheckLimits {
    timeout: Duration,
    max_concurrent: usize,
    max_output: usize,
    max_total: usize,
}

impl CheckLimits {
    fn defaults() -> Self {
        Self {
            timeout: Duration::from_secs(CHECK_TIMEOUT_SECS),
            max_concurrent: MAX_CONCURRENT_CHECKS,
            max_output: MAX_OUTPUT_SIZE,
            max_total: MAX_TOTAL_OUTPUT,
        }
    }
}

/// The result of attempting to run a project's configured checks.
///
/// Kept neutral so one routine serves both the hook (allow/deny) and the MCP tool (tool result)
/// without embedding either output format.
pub(crate) enum CheckRunOutcome {
    /// Fail-open: the repository or workspace root could not be detected, `.config/tools.toml`
    /// could not be loaded (except for duplicate check names), or no checks are configured. Carries
    /// a human-readable explanation that the MCP tool surfaces and the hook ignores (it allows).
    NoChecks(String),
    /// Fail-closed: a pre-execution gate failed (duplicate check name, empty command, unapproved,
    /// or an argv/binary change since approval) or a global limit was hit (timeout, total-output
    /// cap). Carries the reason the run was blocked.
    Blocked(String),
    /// Every check executed. `outputs` holds one formatted entry per check; `failures` is
    /// non-empty iff at least one check failed (non-zero exit or spawn error).
    Ran {
        outputs: Vec<String>,
        failures: Vec<String>,
    },
}

/// The shared path behind both the Stop hook and the `run_checks` MCP tool: detect the repository
/// root, load `.config/tools.toml` from the **workspace root**, verify the checks (checks only,
/// not commands) against approvals keyed by the repository root, and run them in the workspace
/// root under the Stop hook's resource limits.
///
/// Two distinct roots are in play. The repository root (shared across jj workspaces and git
/// worktrees) keys the approvals, so identical content across worktrees needs no re-approval.
/// The workspace root is where the config is loaded from, where a relative check program resolves
/// against, and where the checks execute — so a secondary workspace runs its own tooling and
/// validates its own working copy. Per-command approvals bind the argv, resolved binary paths,
/// and binary hash as match-any versions, so only a new name, an argument change, or a binary
/// change triggers re-approval; cosmetic config edits do not.
///
/// Capture one canonical input before either awaited discovery so retargeting a caller's alias
/// cannot pair one repository's approvals with another workspace's configuration and execution.
/// This does not eliminate filesystem TOCTOU after that capture.
///
/// `Err` is reserved for unexpected failures the caller should surface as an error (e.g. the
/// approvals store failing to load). Every check-level decision is encoded in [`CheckRunOutcome`]
/// instead, so both callers share one control flow.
pub(crate) async fn run_configured_checks(project_dir: &Path) -> miette::Result<CheckRunOutcome> {
    let canonical_dir = match project_dir
        .canonicalize()
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to canonicalize path: {}", project_dir.display()))
    {
        Ok(path) => path,
        Err(error) => return Ok(discovery_failure(project_dir, "workspace", error, true)),
    };
    let repository_root = match detect_repository_root(&canonical_dir).await {
        Ok(root) => {
            info!(
                project_dir = %project_dir.display(),
                repository_root = %root.as_path().display(),
                "Detected repository root"
            );
            root
        }
        Err(error) => return Ok(discovery_failure(project_dir, "repository", error, false)),
    };

    let workspace_root = match detect_workspace_root(&canonical_dir).await {
        Ok(root) => {
            info!(
                project_dir = %project_dir.display(),
                workspace_root = %root.as_path().display(),
                "Detected workspace root for check execution"
            );
            root
        }
        Err(error) => return Ok(discovery_failure(project_dir, "workspace", error, false)),
    };

    let config = match load_project_settings(workspace_root.as_path().to_path_buf()).await {
        Ok(config) => config,
        Err(e) if is_duplicate_check_name(&e) => {
            return Ok(CheckRunOutcome::Blocked(e.to_string()));
        }
        Err(e) => {
            info!(error = %e, "No .config/tools.toml found, allowing without checks");
            return Ok(CheckRunOutcome::NoChecks(format!(
                "no usable {}/.config/tools.toml: {e}",
                workspace_root.as_path().display()
            )));
        }
    };
    // Keep `config` whole because verification borrows each check's name-keyed command array.
    let checks: Vec<Check> = match &config.checks {
        Some(checks) if !checks.is_empty() => checks.clone(),
        _ => {
            info!("No checks defined in config, allowing");
            return Ok(CheckRunOutcome::NoChecks(
                "no checks defined in .config/tools.toml".to_string(),
            ));
        }
    };

    info!(check_count = checks.len(), "Found checks to run");

    for check in &checks {
        if check.command.is_empty() {
            error!(check_name = %check.name, "Check has empty command");
            return Ok(CheckRunOutcome::Blocked(format!(
                "Check '{}' has empty command array in {}/.config/tools.toml\n\
                 Expected format: command = [\"binary\", \"arg1\", \"arg2\"]",
                check.name,
                workspace_root.as_path().display()
            )));
        }
    }

    let approvals = ProjectApprovals::load().await?;

    // Preserve the verified PathBuf, including dispatch-sensitive symlinks.
    // Re-resolving a relative program in the execution cwd could select an unapproved workspace copy.
    let mut resolved_checks = Vec::with_capacity(checks.len());
    for check in checks {
        let verification = approvals
            .verify_check_for_workspace(&repository_root, &workspace_root, &config, &check.name)
            .await?;

        match verification {
            VerificationResult::Approved { program } => {
                info!(check_name = %check.name, "Check is approved");
                resolved_checks.push((check, program));
            }
            VerificationResult::NotApproved => {
                error!(check_name = %check.name, "Check not approved");
                return Ok(CheckRunOutcome::Blocked(format!(
                    "Check '{}' is not approved. Run: moriarty approve-project {}",
                    check.name,
                    workspace_root.as_path().display()
                )));
            }
            VerificationResult::ItemNotApproved { item } => {
                error!(check_name = %check.name, item = %item, "Item not approved");
                return Ok(CheckRunOutcome::Blocked(format!(
                    "Check '{}' not in approvals. Run: moriarty approve-project {}",
                    item,
                    workspace_root.as_path().display()
                )));
            }
            VerificationResult::ItemChanged {
                item,
                args_changed,
                approved_versions,
            } => {
                // Defer to the shared, kind-typed sentence so the checks runner and command runner
                // wording cannot drift and the kind is always "Check".
                let sentence = VerificationResult::item_changed_sentence(
                    ItemType::Check,
                    &item,
                    args_changed,
                    approved_versions,
                );
                error!(
                    check_name = %check.name,
                    "Check argv or binary changed since approval"
                );
                return Ok(CheckRunOutcome::Blocked(format!(
                    "{}. Run: moriarty approve-project {}",
                    sentence,
                    workspace_root.as_path().display()
                )));
            }
        }
    }

    Ok(run_checks_with_limits(
        workspace_root.as_path().to_path_buf(),
        resolved_checks,
        CheckLimits::defaults(),
    )
    .await)
}

fn discovery_failure(
    project_dir: &Path,
    kind: &str,
    error: miette::Report,
    initial_input: bool,
) -> CheckRunOutcome {
    // Once the input snapshot exists, losing it is not evidence that checks are absent.
    if !initial_input || execution::is_execution_error(&error) {
        return CheckRunOutcome::Blocked(format!("{error:#}"));
    }
    error!(project_dir = %project_dir.display(), error = %error, "Failed to detect {kind} root");
    CheckRunOutcome::NoChecks(format!(
        "could not detect {kind} root for {}: {error}",
        project_dir.display()
    ))
}

/// Verification precedes the batch clock; queued checks never receive a fresh deadline.
/// A blocking cause survives cancel-then-await settlement, including cleanup uncertainty.
async fn run_checks_with_limits(
    workspace_root: PathBuf,
    checks: Vec<(Check, PathBuf)>,
    limits: CheckLimits,
) -> CheckRunOutcome {
    let deadline = Instant::now() + limits.timeout;
    let outcome = execution::in_scope(async move {
        let pending = stream::iter(checks).map(move |(check, program)| {
            let cwd = workspace_root.clone();
            async move {
                // Direct callers can bypass config validation; reject empty argv without panicking.
                let Some((_, args)) = check.command.split_first() else {
                    return Err(format!("Check '{}' has empty command array", check.name));
                };
                let operation = execution::submit(ProcessSpec {
                    program: program.into_os_string(), args: args.iter().map(Into::into).collect(), cwd: Some(cwd),
                }).map_err(|error| error.to_string())?;
                Ok((check, operation.await))
            }
        }).buffer_unordered(limits.max_concurrent);
        tokio::pin!(pending);
        let mut outputs = Vec::new();
        let mut failures = Vec::new();
        let mut retained = 0usize;
        loop {
            let next = tokio::select! {
                biased;
                _ = sleep_until(deadline) => return CheckRunOutcome::Blocked(format!("Checks timed out after {} seconds", limits.timeout.as_secs())),
                next = pending.next() => next,
            };
            let Some(next) = next else { return CheckRunOutcome::Ran { outputs, failures }; };
            let (check, result) = match next { Ok(value) => value, Err(reason) => return CheckRunOutcome::Blocked(reason) };
            let (output, failure) = format_check_result(&check, result, limits.max_output);
            let incoming = output.as_ref().map_or(0, String::len) + failure.as_ref().map_or(0, String::len);
            if retained.saturating_add(incoming) > limits.max_total {
                return CheckRunOutcome::Blocked(format!("Total check output exceeded {} MB limit. Checks produced too much output.", limits.max_total / (1024 * 1024)));
            }
            retained += incoming;
            outputs.extend(output);
            failures.extend(failure);
        }
        // Returning drops every active operation before the owning scope concurrently drains them.
    }).await;
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(error) => return CheckRunOutcome::Blocked(error.to_string()),
    };
    match outcome.cleanup {
        Ok(()) => outcome.value,
        Err(error) => {
            let reason = match outcome.value {
                CheckRunOutcome::Blocked(reason) | CheckRunOutcome::NoChecks(reason) => reason,
                CheckRunOutcome::Ran { outputs, failures } => outputs
                    .into_iter()
                    .chain(failures)
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            };
            CheckRunOutcome::Blocked(format!("{reason}\nSettlement: {error}"))
        }
    }
}

fn format_check_result(
    check: &Check,
    result: miette::Result<ProcessReport>,
    max_output: usize,
) -> (Option<String>, Option<String>) {
    let result = result.and_then(|report| {
        execution::exited(&report).map(|status| (report, status.code().unwrap_or(-1)))
    });
    match result {
        Ok((report, exit_code)) => {
            let stdout = execution::present(&report.stdout, Some(max_output));
            let stderr = execution::present(&report.stderr, Some(max_output));
            let text = match (stdout.is_empty(), stderr.is_empty()) {
                (true, true) => "<no output>".to_owned(),
                (false, true) => stdout,
                (true, false) => stderr,
                (false, false) => format!("stdout:\n{stdout}\nstderr:\n{stderr}"),
            };
            let output = format!("Check '{}' [exit code: {exit_code}]:\n{text}", check.name);
            let failure = (exit_code != 0).then(|| {
                format!(
                    "Check '{}' failed with exit code {exit_code}\nCommand: {:?}\n{text}",
                    check.name, check.command
                )
            });
            (Some(output), failure)
        }
        Err(error) => (
            None,
            Some(format!(
                "Check '{}' failed to execute: {error}\nCommand: {:?}",
                check.name, check.command
            )),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        fs,
        future::ready,
        os::unix::process::ExitStatusExt,
        process::ExitStatus,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use tempfile::TempDir;
    use tokio::{sync::Barrier, time::timeout};

    use super::*;
    use crate::{
        project_config::approvals::approve_project_config,
        test_helpers::{
            assert_workspace_local_copy_ran, create_executable_script, setup_isolated_xdg_config,
            setup_jj_main_and_secondary_workspace, setup_project_dir_with_config, test_report,
        },
    };
    use process_supervisor::{CancellationToken, Completion, PartialState, ProcessError};

    /// Isolates XDG config, writes `config` to a temp project's `.config/tools.toml`, and approves
    /// it (commands and `[[checks]]`). Returns both temp dirs so callers keep them alive.
    async fn approved_project(config: &str) -> (TempDir, TempDir) {
        let xdg = setup_isolated_xdg_config();
        let project = setup_project_dir_with_config(config);
        approve_project_config(project.path(), config)
            .await
            .unwrap();
        (project, xdg)
    }

    fn prepared_check(name: &str, program: &str) -> (Check, PathBuf) {
        (
            Check {
                name: name.into(),
                command: vec![program.into()],
            },
            PathBuf::from(program),
        )
    }

    fn blocked_reason(outcome: CheckRunOutcome) -> String {
        let CheckRunOutcome::Blocked(reason) = outcome else {
            panic!("must block")
        };
        reason
    }

    #[derive(Default)]
    struct BatchCounts {
        started: AtomicUsize,
        cancelled: AtomicUsize,
        settled: AtomicUsize,
    }

    #[tokio::test]
    async fn uncertain_discovery_is_blocked_not_no_checks() {
        let project = TempDir::new().unwrap();
        let outcome = execution::test_seams::with_result(
            Err("Startup timed out; creation and cleanup unconfirmed"),
            run_configured_checks(project.path()),
        )
        .await
        .unwrap();
        assert!(
            matches!(outcome, CheckRunOutcome::Blocked(reason) if reason.contains("unconfirmed"))
        );
    }

    #[tokio::test]
    async fn no_checks_when_only_commands_configured() {
        let (project, _xdg) = approved_project("[commands]\ntest = [\"echo\", \"hi\"]\n").await;
        let outcome = run_configured_checks(project.path()).await.unwrap();
        assert!(matches!(outcome, CheckRunOutcome::NoChecks(_)));
    }

    #[tokio::test]
    async fn no_checks_when_config_missing() {
        let _xdg = setup_isolated_xdg_config();
        let project = TempDir::new().unwrap();
        let outcome = run_configured_checks(project.path()).await.unwrap();
        assert!(matches!(outcome, CheckRunOutcome::NoChecks(_)));
    }

    #[tokio::test]
    async fn passing_check_runs_without_failures() {
        let (project, _xdg) = approved_project(
            "[commands]\n[[checks]]\nname = \"ok\"\ncommand = [\"echo\", \"hello\"]\n",
        )
        .await;
        let outcome = run_configured_checks(project.path()).await.unwrap();
        let CheckRunOutcome::Ran { outputs, failures } = outcome else {
            panic!("expected Ran");
        };
        assert_eq!(outputs.len(), 1);
        assert!(failures.is_empty(), "unexpected failures: {failures:?}");
    }

    #[tokio::test]
    async fn failing_check_is_reported() {
        let (project, _xdg) = approved_project(
            "[commands]\n[[checks]]\nname = \"fail\"\ncommand = [\"sh\", \"-c\", \"exit 1\"]\n",
        )
        .await;
        let outcome = run_configured_checks(project.path()).await.unwrap();
        let CheckRunOutcome::Ran { failures, .. } = outcome else {
            panic!("expected Ran");
        };
        assert_eq!(failures.len(), 1);
        let failure = &failures[0];
        assert!(failure.contains("fail"), "got: {failure}");
        assert!(failure.contains("exit code 1"), "got: {failure}");
        // The reported command shows the verification-resolved program, an absolute
        // machine-dependent path (e.g. /bin/sh vs a nix store path), so only the stable
        // basename is asserted.
        assert!(failure.contains("sh"), "got: {failure}");
    }

    #[tokio::test]
    async fn nonexistent_project_dir_inside_repository_fails_open() {
        // Canonicalization precedes both discoveries, so a jj ancestor cannot rescue a missing
        // descendant; keep that setup failure's historical workspace-root classification.
        let _xdg = setup_isolated_xdg_config();
        let base = TempDir::new().unwrap();
        let main = base.path().join("main");
        std::fs::create_dir_all(main.join(".jj/repo")).unwrap();

        let outcome = run_configured_checks(&main.join("does-not-exist"))
            .await
            .unwrap();
        let CheckRunOutcome::NoChecks(reason) = outcome else {
            panic!("expected NoChecks");
        };
        assert!(reason.contains("workspace root"), "got: {reason}");
    }

    #[tokio::test]
    async fn relative_check_program_runs_without_workspace_divergence() {
        // Baseline for the ordinary case: no jj/git metadata, so repository root and workspace
        // root coincide and a relative program must keep working exactly as before the root
        // split.
        let _xdg = setup_isolated_xdg_config();
        let config = "[commands]\n[[checks]]\nname = \"rel\"\ncommand = [\"./check.sh\"]\n";
        let project = setup_project_dir_with_config(config);
        create_executable_script(&project.path().join("check.sh"), "echo baseline-copy");
        approve_project_config(project.path(), config)
            .await
            .unwrap();

        let outcome = run_configured_checks(project.path()).await.unwrap();
        let CheckRunOutcome::Ran { outputs, failures } = outcome else {
            panic!("expected Ran");
        };
        assert!(failures.is_empty(), "unexpected failures: {failures:?}");
        assert!(outputs[0].contains("baseline-copy"), "got: {}", outputs[0]);
    }

    #[tokio::test]
    async fn secondary_workspace_runs_approved_check_in_workspace_cwd() {
        // Stop checks use workspace cwd even when the nested caller contains a shadow executable.
        let _xdg = setup_isolated_xdg_config();
        let config = "[commands]\n[[checks]]\nname = \"rel\"\ncommand = [\"./check.sh\"]\n";
        let (_base, _main, workspace) =
            setup_jj_main_and_secondary_workspace(config, "check.sh", false);
        approve_project_config(&workspace, config).await.unwrap();

        let nested = workspace.join("nested");
        fs::create_dir(&nested).unwrap();
        create_executable_script(&nested.join("check.sh"), "echo shadow-copy\npwd -P");
        let cwd = workspace.canonicalize().unwrap();
        for caller in [&workspace, &nested] {
            let CheckRunOutcome::Ran { outputs, failures } =
                run_configured_checks(caller).await.unwrap()
            else {
                panic!("expected Ran")
            };
            assert!(failures.is_empty(), "unexpected failures: {failures:?}");
            assert_workspace_local_copy_ran(&outputs[0], &workspace);
            assert!(!outputs[0].contains("shadow-copy"));
            assert!(outputs[0].lines().any(|line| Some(line) == cwd.to_str()));
        }
    }

    #[tokio::test]
    async fn unapproved_check_is_blocked() {
        let _xdg = setup_isolated_xdg_config();
        let project = setup_project_dir_with_config(
            "[commands]\n[[checks]]\nname = \"ok\"\ncommand = [\"echo\", \"hi\"]\n",
        );
        // Deliberately skip approval to exercise the fail-closed gate.
        let outcome = run_configured_checks(project.path()).await.unwrap();
        let CheckRunOutcome::Blocked(reason) = outcome else {
            panic!("expected Blocked");
        };
        assert!(reason.contains("not approved"));
    }

    #[tokio::test]
    async fn empty_command_is_blocked() {
        let _xdg = setup_isolated_xdg_config();
        let project = setup_project_dir_with_config(
            "[commands]\n[[checks]]\nname = \"empty\"\ncommand = []\n",
        );
        // No approval needed: the empty-command gate fires before ProjectApprovals::load().
        let outcome = run_configured_checks(project.path()).await.unwrap();
        let CheckRunOutcome::Blocked(reason) = outcome else {
            panic!("expected Blocked");
        };
        assert!(reason.contains("empty command array"));
    }

    #[tokio::test]
    async fn config_edit_changing_no_argv_is_allowed() {
        // The headlining behavior inverted from the old config-hash model: a config edit that
        // changes no argv (here, a comment) must NOT trigger re-approval.
        let (project, _xdg) = approved_project(
            "[commands]\n[[checks]]\nname = \"ok\"\ncommand = [\"echo\", \"v1\"]\n",
        )
        .await;
        std::fs::write(
            project.path().join(".config/tools.toml"),
            "# comment that changes the config hash but no argv\n[commands]\n[[checks]]\nname = \"ok\"\ncommand = [\"echo\", \"v1\"]\n",
        )
        .unwrap();
        let outcome = run_configured_checks(project.path()).await.unwrap();
        let CheckRunOutcome::Ran { outputs, failures } = outcome else {
            panic!("expected Ran after a no-argv config edit");
        };
        assert!(failures.is_empty(), "unexpected failures: {failures:?}");
        assert_eq!(outputs.len(), 1);
    }

    #[tokio::test]
    async fn argv_change_blocks_check() {
        // An argument change is one of the three things that trigger re-approval.
        let (project, _xdg) = approved_project(
            "[commands]\n[[checks]]\nname = \"ok\"\ncommand = [\"echo\", \"v1\"]\n",
        )
        .await;
        std::fs::write(
            project.path().join(".config/tools.toml"),
            "[commands]\n[[checks]]\nname = \"ok\"\ncommand = [\"echo\", \"v2\"]\n",
        )
        .unwrap();
        let outcome = run_configured_checks(project.path()).await.unwrap();
        assert!(
            matches!(outcome, CheckRunOutcome::Blocked(_)),
            "an argv change must block the run"
        );
    }

    #[tokio::test]
    async fn changed_check_binary_is_blocked() {
        let _xdg = setup_isolated_xdg_config();
        let project = TempDir::new().unwrap();
        std::fs::create_dir(project.path().join(".config")).unwrap();
        let binary = project.path().join("check-bin");
        std::fs::write(&binary, "original").unwrap();
        let config = format!(
            "[commands]\n[[checks]]\nname = \"s\"\ncommand = [\"{}\"]\n",
            binary.display()
        );
        std::fs::write(project.path().join(".config/tools.toml"), &config).unwrap();
        approve_project_config(project.path(), &config)
            .await
            .unwrap();

        // Swap the approved binary's contents; verification catches the hash change before running.
        // argv is unchanged, so the wording reports a binary change.
        std::fs::write(&binary, "tampered").unwrap();

        let outcome = run_configured_checks(project.path()).await.unwrap();
        assert!(
            matches!(outcome, CheckRunOutcome::Blocked(_)),
            "a binary change must block the run"
        );
    }

    #[tokio::test]
    async fn check_missing_from_approvals_is_blocked() {
        let _xdg = setup_isolated_xdg_config();
        let config = "[commands]\n[[checks]]\nname = \"present\"\ncommand = [\"echo\", \"hi\"]\n";
        let project = setup_project_dir_with_config(config);
        let project_path = project.path().to_path_buf();
        // Record an approval for the project (so it is keyed and `last_approved` is set) but with
        // an empty checks map, exercising ItemNotApproved (distinct from a wholly-unapproved
        // project -> NotApproved): "present" is configured but has no approved version.
        let approve_path = detect_workspace_root(&project_path).await.unwrap();
        let root = detect_repository_root(&project_path).await.unwrap();
        ProjectApprovals::update(move |a| {
            a.approve_project(approve_path, root, HashMap::new(), HashMap::new())
        })
        .await
        .unwrap();

        let outcome = run_configured_checks(&project_path).await.unwrap();
        let CheckRunOutcome::Blocked(reason) = outcome else {
            panic!("expected Blocked");
        };
        assert!(reason.contains("not in approvals"), "got: {reason}");
    }

    async fn stopped_batch(leading: &str, max_total: usize) -> (String, Arc<BatchCounts>) {
        let aggregate = leading == "loud" || leading == "failed";
        let expected_cancelled = if aggregate { 3 } else { 4 };
        let directory = TempDir::new().unwrap();
        let counts = Arc::new(BatchCounts::default());
        let barrier = Arc::new(Barrier::new(expected_cancelled));
        let all_started = Arc::new(Barrier::new(5));
        let handler_ready = all_started.clone();
        let checks = [leading, "slow", "slow", "slow", "queued"]
            .into_iter()
            .enumerate()
            .map(|(index, program)| prepared_check(&format!("check-{index}"), program))
            .collect();
        let handler_counts = counts.clone();
        let handler = move |spec: ProcessSpec, token: CancellationToken| {
            handler_counts.started.fetch_add(1, Ordering::SeqCst);
            let counts = handler_counts.clone();
            let barrier = barrier.clone();
            let ready = handler_ready.clone();
            async move {
                ready.wait().await;
                let slow = spec.program == "slow" || spec.program == "uncertain";
                if slow {
                    token.cancelled().await;
                    counts.cancelled.fetch_add(1, Ordering::SeqCst);
                    barrier.wait().await;
                }
                counts.settled.fetch_add(1, Ordering::SeqCst);
                if spec.program == "uncertain" {
                    return Err(miette::miette!("Execution cancelled; cleanup unconfirmed"));
                }
                let status = ExitStatus::from_raw(if spec.program == "failed" { 256 } else { 0 });
                let mut report = test_report(if slow {
                    Completion::Cancelled(status)
                } else {
                    Completion::Exited(status)
                });
                if spec.program == "loud" || spec.program == "failed" {
                    report.stdout.bytes = vec![b'x'; 1024];
                }
                Ok(report)
            }
        };
        let limits = CheckLimits {
            timeout: Duration::from_secs(1),
            max_total,
            ..CheckLimits::defaults()
        };
        let result = execution::test_seams::with_operations(handler, async {
            let delay = limits.timeout;
            let advance = async {
                all_started.wait().await;
                if !aggregate {
                    tokio::time::advance(delay).await;
                }
            };
            let run = timeout(
                Duration::from_secs(2),
                run_checks_with_limits(directory.path().to_path_buf(), checks, limits),
            );
            let (result, ()) = tokio::join!(run, advance);
            result
        })
        .await
        .unwrap();
        (blocked_reason(result), counts)
    }

    #[tokio::test(start_paused = true)]
    async fn batch_stops_queued_work_and_cancels_all_before_settlement() {
        for (label, leading, expected_cancelled, max_total, cause) in [
            ("deadline", "slow", 4, MAX_TOTAL_OUTPUT, "timed out"),
            (
                "deadline cleanup",
                "uncertain",
                4,
                MAX_TOTAL_OUTPUT,
                "timed out",
            ),
            ("aggregate", "loud", 3, 128, "Total check output"),
            (
                "failed text aggregate",
                "failed",
                3,
                1500,
                "Total check output",
            ),
        ] {
            let (reason, counts) = stopped_batch(leading, max_total).await;
            assert!(reason.contains(cause), "{label}: {reason}");
            if leading == "uncertain" {
                assert!(reason.contains("cleanup unconfirmed"), "{reason}");
            }
            assert_eq!(
                (
                    counts.started.load(Ordering::SeqCst),
                    counts.cancelled.load(Ordering::SeqCst),
                    counts.settled.load(Ordering::SeqCst)
                ),
                (4, expected_cancelled, 4),
                "{label}: admitted/cancelled/settled"
            );
        }
    }

    #[tokio::test]
    async fn completed_check_evidence_survives_uncertain_settlement() {
        let directory = TempDir::new().unwrap();
        let checks = ["completed", "uncertain"]
            .into_iter()
            .map(|name| prepared_check(name, name))
            .collect();
        let handler = |spec: ProcessSpec, _| {
            ready(if spec.program == "completed" {
                let mut report = test_report(Completion::Exited(ExitStatus::from_raw(0)));
                report.stdout.bytes = b"completed check evidence".to_vec();
                Ok(report)
            } else {
                Err(ProcessError::CleanupUnconfirmed {
                    state: Box::<PartialState>::default(),
                }
                .into())
            })
        };
        let result = execution::test_seams::with_operations(
            handler,
            run_checks_with_limits(
                directory.path().to_path_buf(),
                checks,
                CheckLimits::defaults(),
            ),
        )
        .await;
        let reason = blocked_reason(result);
        assert!(
            reason.contains("Check 'completed' [exit code: 0]:\ncompleted check evidence"),
            "{reason}"
        );
        assert!(
            reason.contains("Check 'uncertain' failed to execute"),
            "{reason}"
        );
        assert!(reason.contains("\nSettlement:"), "{reason}");
    }

    #[test]
    fn check_presentation_bounds_lossy_text_and_discloses_both_streams() {
        let (check, _) = prepared_check("bounded", "echo");
        let mut report = test_report(Completion::Exited(ExitStatus::from_raw(0)));
        report.stdout.bytes = vec![0xff; 16];
        report.stdout.truncated = true;
        report.stderr.bytes = b"retained".to_vec();
        report.stderr.truncated = true;
        let (output, failure) = format_check_result(&check, Ok(report), 8);
        let output = output.unwrap();
        assert!(failure.is_none());
        assert_eq!(output.matches("output truncated").count(), 2);
        assert!(output.contains('�'));
        assert!(output.len() < 120, "converted result must stay bounded");
    }

    #[tokio::test]
    async fn empty_command_blocks_the_batch() {
        let (mut check, program) = prepared_check("empty", "must-not-start");
        check.command.clear();
        let checks = vec![(check, program)];
        let outcome =
            run_checks_with_limits(PathBuf::from("."), checks, CheckLimits::defaults()).await;
        let CheckRunOutcome::Blocked(reason) = outcome else {
            panic!("expected Blocked");
        };
        assert_eq!(reason, "Check 'empty' has empty command array");
    }

    #[tokio::test]
    async fn spawn_error_becomes_a_failure() {
        let cwd = TempDir::new().unwrap();
        let checks = vec![prepared_check(
            "missing",
            "/nonexistent/moriarty-test-binary",
        )];
        let outcome =
            run_checks_with_limits(cwd.path().to_path_buf(), checks, CheckLimits::defaults()).await;
        let CheckRunOutcome::Ran { failures, .. } = outcome else {
            panic!("expected Ran");
        };
        assert_eq!(failures.len(), 1);
        assert!(
            failures[0].contains("failed to execute"),
            "got: {}",
            failures[0]
        );
    }

    fn truncate_to_char_boundary(text: &str, cap: usize) -> String {
        let mut report = test_report(Completion::Exited(ExitStatus::from_raw(0)));
        report.stdout.bytes = text.as_bytes().to_vec();
        execution::present(&report.stdout, Some(cap))
    }

    #[test]
    fn truncate_passes_short_strings_through() {
        assert_eq!(truncate_to_char_boundary("hello", 1024), "hello");
        // Exactly at the cap: the `<=` branch returns the string unchanged.
        assert_eq!(truncate_to_char_boundary("hello", 5), "hello");
    }

    #[test]
    fn truncate_backs_up_to_char_boundary() {
        // "aéz" is 4 bytes — 'a' (1 byte, index 0), 'é' (2 bytes, indices 1–2), 'z' (1 byte,
        // index 3). A cap of 2 lands inside 'é', so the safe slice backs up to byte 1 and drops the
        // 3-byte suffix "éz"; the old `&s[..2]` would have panicked on the non-boundary index.
        let out = truncate_to_char_boundary("aéz", 2);
        assert_eq!(out, "a\n[output truncated]");
        // A 1-byte cap on a leading 2-byte char backs all the way up to byte 0.
        assert_eq!(truncate_to_char_boundary("é", 1), "\n[output truncated]");
    }

    #[test]
    fn truncate_with_zero_cap_keeps_no_prefix() {
        // Degenerate cap: backs up to byte 0 (always a boundary), dropping the whole string.
        assert_eq!(truncate_to_char_boundary("abc", 0), "\n[output truncated]");
    }
}
