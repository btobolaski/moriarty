//! Shared project-check runner for the Stop hook and the `project-tools` MCP server.
//!
//! Both entry points must run a project's `[[checks]]` identically: the same approval
//! verification, the same resource limits, and the same fail-open/fail-closed policy described
//! in [`crate::hooks`]'s "Security Model: Fail-Open Design". This module is the single source of
//! that behavior so the two callers cannot drift apart; each maps [`CheckRunOutcome`] onto its own
//! result type ([`crate::hooks`] onto allow/deny, the MCP tool onto a tool result).
//!
//! This is intentionally distinct from `project_config::runner::run_all_checks`, which the
//! `moriarty test checks` CLI uses: that path has no timeout or output caps and also requires the
//! lint/test/build/format commands to be approved. Checks here are verified on their own and run
//! under the limits below.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use futures::stream::StreamExt;
use miette::Result;
use tracing::{error, info};

use crate::{
    project_config::{
        approvals::{ProjectApprovals, VerificationResult},
        config::{Check, load_project_settings},
    },
    repository::detect_repository_root,
};

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
// MAX_OUTPUT_SIZE (1MB per check): Prevents individual checks from consuming excessive memory
// via stdout/stderr. Typical check output is <10KB; 1MB allows detailed error messages and
// verbose tooling while preventing abuse.
//
// MAX_TOTAL_OUTPUT (10MB total): Prevents aggregate memory exhaustion across all checks.
// With 4 concurrent checks, this allows each to use its full 1MB quota with headroom.
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
    /// Fail-open: the repository root could not be detected, `.config/tools.toml` could not be
    /// loaded, or no checks are configured. Carries a human-readable explanation that the MCP tool
    /// surfaces and the hook ignores (it simply allows).
    NoChecks(String),
    /// Fail-closed: a pre-execution gate failed (empty command, unapproved, config- or
    /// binary-hash mismatch) or a global limit was hit (timeout, total-output cap). Carries the
    /// reason the run was blocked.
    Blocked(String),
    /// Every check executed. `outputs` holds one formatted entry per check; `failures` is
    /// non-empty iff at least one check failed (non-zero exit or spawn error).
    Ran {
        outputs: Vec<String>,
        failures: Vec<String>,
    },
}

/// The shared path behind both the Stop hook and the `run_checks` MCP tool: detect the repository
/// root, load `.config/tools.toml`, verify the checks (checks only, not commands), and run them
/// under the Stop hook's resource limits.
///
/// `Err` is reserved for unexpected failures the caller should surface as an error (e.g. the
/// approvals store failing to load). Every check-level decision is encoded in [`CheckRunOutcome`]
/// instead, so both callers share one control flow.
pub(crate) async fn run_configured_checks(project_dir: &Path) -> Result<CheckRunOutcome> {
    let repository_root = match detect_repository_root(project_dir) {
        Ok(root) => {
            info!(
                project_dir = %project_dir.display(),
                repository_root = %root.display(),
                "Detected repository root"
            );
            root
        }
        Err(e) => {
            error!(
                project_dir = %project_dir.display(),
                error = %e,
                "Failed to detect repository root"
            );
            return Ok(CheckRunOutcome::NoChecks(format!(
                "could not detect repository root for {}: {e}",
                project_dir.display()
            )));
        }
    };

    let config = match load_project_settings(repository_root.clone()).await {
        Ok(config) => config,
        Err(e) => {
            info!(error = %e, "No .config/tools.toml found, allowing without checks");
            return Ok(CheckRunOutcome::NoChecks(format!(
                "no usable {}/.config/tools.toml: {e}",
                repository_root.display()
            )));
        }
    };

    let checks = match config.checks {
        Some(checks) if !checks.is_empty() => checks,
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
                repository_root.display()
            )));
        }
    }

    let approvals = ProjectApprovals::load().await?;

    for check in &checks {
        let verification = approvals
            .verify_check(&repository_root, &check.name)
            .await?;

        match verification {
            VerificationResult::Approved => {
                info!(check_name = %check.name, "Check is approved");
            }
            VerificationResult::NotApproved => {
                error!(check_name = %check.name, "Check not approved");
                return Ok(CheckRunOutcome::Blocked(format!(
                    "Check '{}' is not approved. Run: moriarty approve-project {}",
                    check.name,
                    repository_root.display()
                )));
            }
            VerificationResult::ConfigHashMismatch { expected, actual } => {
                error!(
                    check_name = %check.name,
                    expected = %expected,
                    actual = %actual,
                    "Config hash mismatch"
                );
                return Ok(CheckRunOutcome::Blocked(format!(
                    "Project configuration changed. Run: moriarty approve-project {}",
                    repository_root.display()
                )));
            }
            VerificationResult::BinaryHashMismatch {
                item,
                expected,
                actual,
            } => {
                error!(
                    check_name = %check.name,
                    item = %item,
                    expected = %expected,
                    actual = %actual,
                    "Binary hash mismatch"
                );
                return Ok(CheckRunOutcome::Blocked(format!(
                    "Check '{}' binary changed. Run: moriarty approve-project {}",
                    check.name,
                    repository_root.display()
                )));
            }
            VerificationResult::ItemNotApproved { item } => {
                error!(check_name = %check.name, item = %item, "Item not approved");
                return Ok(CheckRunOutcome::Blocked(format!(
                    "Check '{}' not in approvals. Run: moriarty approve-project {}",
                    item,
                    repository_root.display()
                )));
            }
        }
    }

    Ok(run_checks_with_limits(repository_root, checks, CheckLimits::defaults()).await)
}

/// Execute the already-verified `checks` in parallel under `limits`.
///
/// Non-zero exits and spawn errors become `failures` entries rather than propagating, so a failing
/// check is reported (fail-closed) rather than aborting the run. A timeout or a breach of the
/// aggregate output cap short-circuits to [`CheckRunOutcome::Blocked`].
async fn run_checks_with_limits(
    repository_root: PathBuf,
    checks: Vec<Check>,
    limits: CheckLimits,
) -> CheckRunOutcome {
    let timeout_duration = limits.timeout;
    let repository_root_clone = repository_root.clone();

    let check_futures = futures::stream::iter(checks.into_iter().map(move |check| {
        let repository_root = repository_root_clone.clone();
        async move {
            // Defensive: empty commands are rejected before this point, but config can change
            // between validation and spawn, and degrading gracefully beats panicking.
            let Some((cmd, args)) = check.command.split_first() else {
                return (
                    check.name,
                    check.command,
                    Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "Check command array is empty",
                    )),
                );
            };

            let output = tokio::process::Command::new(cmd)
                .args(args)
                .current_dir(&repository_root)
                .output()
                .await;

            (check.name, check.command, output)
        }
    }))
    .buffer_unordered(limits.max_concurrent)
    .collect::<Vec<_>>();

    let results = match tokio::time::timeout(timeout_duration, check_futures).await {
        Ok(results) => results,
        Err(_) => {
            let timeout_secs = timeout_duration.as_secs();
            error!(timeout_secs, "Checks timed out");
            return CheckRunOutcome::Blocked(format!(
                "Checks timed out after {timeout_secs} seconds"
            ));
        }
    };

    let mut failures = Vec::new();
    let mut outputs = Vec::new();
    let mut total_output_size = 0;

    for (check_name, command, output_result) in results {
        match output_result {
            Ok(output) => {
                let exit_code = output.status.code().unwrap_or(-1);

                let stdout = truncate_to_char_boundary(
                    &String::from_utf8_lossy(&output.stdout),
                    limits.max_output,
                );
                let stderr = truncate_to_char_boundary(
                    &String::from_utf8_lossy(&output.stderr),
                    limits.max_output,
                );

                let combined_output = if stdout.is_empty() && stderr.is_empty() {
                    "<no output>".to_string()
                } else if stderr.is_empty() {
                    stdout.clone()
                } else if stdout.is_empty() {
                    stderr.clone()
                } else {
                    format!("stdout:\n{}\nstderr:\n{}", stdout, stderr)
                };

                total_output_size += combined_output.len();

                if total_output_size > limits.max_total {
                    error!(
                        total_size = total_output_size,
                        max_total = limits.max_total,
                        "Total check output exceeded limit"
                    );
                    return CheckRunOutcome::Blocked(format!(
                        "Total check output exceeded {} MB limit. Checks produced too much output.",
                        limits.max_total / (1024 * 1024)
                    ));
                }

                info!(
                    check_name = %check_name,
                    exit_code = exit_code,
                    output_size = combined_output.len(),
                    "Check completed"
                );

                outputs.push(format!(
                    "Check '{}' [exit code: {}]:\n{}",
                    check_name, exit_code, combined_output
                ));

                if exit_code != 0 {
                    failures.push(format!(
                        "Check '{}' failed with exit code {}\nCommand: {:?}\n{}",
                        check_name, exit_code, command, combined_output
                    ));
                }
            }
            Err(e) => {
                error!(check_name = %check_name, error = %e, "Failed to execute check");
                failures.push(format!(
                    "Check '{}' failed to execute: {}\nCommand: {:?}",
                    check_name, e, command
                ));
            }
        }
    }

    info!(
        total_output_size = total_output_size,
        "Finished processing all check results"
    );

    if failures.is_empty() {
        info!("All checks passed");
    } else {
        error!(failure_count = failures.len(), "Some checks failed");
    }

    CheckRunOutcome::Ran { outputs, failures }
}

/// Truncate `s` to at most `max_bytes`, backing up to a UTF-8 char boundary so the slice never
/// splits a multibyte codepoint (which would panic). `String::from_utf8_lossy` yields valid UTF-8,
/// but a fixed byte cap can still land mid-character.
fn truncate_to_char_boundary(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }

    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1; // is_char_boundary(0) is always true, so this terminates
    }

    format!("{}... [truncated {} bytes]", &s[..end], s.len() - end)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tempfile::TempDir;

    use super::*;
    use crate::{
        hashing::hash_string,
        project_config::approvals::approve_project_config,
        test_helpers::{setup_isolated_xdg_config, setup_project_dir_with_config},
    };

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
        assert!(failures.is_empty());
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
        assert!(failure.contains("sh"), "got: {failure}");
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
    async fn changed_config_is_blocked() {
        let (project, _xdg) = approved_project(
            "[commands]\n[[checks]]\nname = \"ok\"\ncommand = [\"echo\", \"v1\"]\n",
        )
        .await;
        // Modify tools.toml after approval: its hash no longer matches the recorded one.
        std::fs::write(
            project.path().join(".config/tools.toml"),
            "[commands]\n[[checks]]\nname = \"ok\"\ncommand = [\"echo\", \"v2\"]\n",
        )
        .unwrap();
        let outcome = run_configured_checks(project.path()).await.unwrap();
        let CheckRunOutcome::Blocked(reason) = outcome else {
            panic!("expected Blocked");
        };
        assert!(reason.contains("configuration changed"), "got: {reason}");
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
        std::fs::write(&binary, "tampered").unwrap();

        let outcome = run_configured_checks(project.path()).await.unwrap();
        let CheckRunOutcome::Blocked(reason) = outcome else {
            panic!("expected Blocked");
        };
        assert!(reason.contains("binary changed"), "got: {reason}");
    }

    #[tokio::test]
    async fn check_missing_from_approvals_is_blocked() {
        let _xdg = setup_isolated_xdg_config();
        let config = "[commands]\n[[checks]]\nname = \"present\"\ncommand = [\"echo\", \"hi\"]\n";
        let project = setup_project_dir_with_config(config);
        let project_path = project.path().to_path_buf();
        // Record an approval whose config hash matches but whose checks map omits "present",
        // exercising ItemNotApproved (distinct from a wholly-unapproved project -> NotApproved).
        let approve_path = project_path.clone();
        let hash = hash_string(config);
        ProjectApprovals::update(move |a| {
            a.approve_project(approve_path, hash, HashMap::new(), HashMap::new())
        })
        .await
        .unwrap();

        let outcome = run_configured_checks(&project_path).await.unwrap();
        let CheckRunOutcome::Blocked(reason) = outcome else {
            panic!("expected Blocked");
        };
        assert!(reason.contains("not in approvals"), "got: {reason}");
    }

    #[tokio::test]
    async fn timeout_blocks_the_run() {
        // run_checks_with_limits runs already-verified checks, so the limit paths can be driven
        // directly without setting up approvals.
        let cwd = TempDir::new().unwrap();
        let checks = vec![Check {
            name: "slow".to_string(),
            command: vec!["sleep".to_string(), "1".to_string()],
        }];
        let limits = CheckLimits {
            timeout: Duration::from_millis(50),
            ..CheckLimits::defaults()
        };
        let outcome = run_checks_with_limits(cwd.path().to_path_buf(), checks, limits).await;
        let CheckRunOutcome::Blocked(reason) = outcome else {
            panic!("expected Blocked");
        };
        assert!(reason.contains("timed out"), "got: {reason}");
    }

    #[tokio::test]
    async fn total_output_cap_blocks_the_run() {
        let cwd = TempDir::new().unwrap();
        let checks = vec![Check {
            name: "loud".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
        }];
        let limits = CheckLimits {
            max_total: 4, // far below the echoed output
            ..CheckLimits::defaults()
        };
        let outcome = run_checks_with_limits(cwd.path().to_path_buf(), checks, limits).await;
        let CheckRunOutcome::Blocked(reason) = outcome else {
            panic!("expected Blocked");
        };
        assert!(
            reason.contains("Total check output exceeded"),
            "got: {reason}"
        );
    }

    #[tokio::test]
    async fn spawn_error_becomes_a_failure() {
        let cwd = TempDir::new().unwrap();
        let checks = vec![Check {
            name: "missing".to_string(),
            command: vec!["/nonexistent/moriarty-test-binary".to_string()],
        }];
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
        assert!(out.starts_with("a..."), "got: {out}");
        assert!(out.contains("truncated 3 bytes"), "got: {out}");
        // A 1-byte cap on a leading 2-byte char backs all the way up to byte 0.
        assert_eq!(truncate_to_char_boundary("é", 1), "... [truncated 2 bytes]");
    }

    #[test]
    fn truncate_with_zero_cap_keeps_no_prefix() {
        // Degenerate cap: backs up to byte 0 (always a boundary), dropping the whole string.
        assert_eq!(
            truncate_to_char_boundary("abc", 0),
            "... [truncated 3 bytes]"
        );
    }
}
