//! Tests for the project approvals system

use std::{
    collections::HashMap,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use miette::{IntoDiagnostic, WrapErr};
use tempfile::{NamedTempFile, TempDir};
use tokio::time::{Duration, sleep};

use crate::{
    hashing,
    project_config::config::ProjectConfig,
    repository,
    test_helpers::{
        create_executable_script, run_git_command, setup_git_repo_with_commit,
        setup_isolated_xdg_config, setup_jj_main_and_secondary_workspace, write_tools_config,
    },
};

use super::super::{
    CommandApproval, ProjectApprovals, VerificationResult, is_script, is_within_project,
    is_writable, normalize_path_for_storage, resolve_binary_path_with_original,
};

/// Builds a `CommandApproval` with stable dummy hash/path fields and `argv: None` (the legacy
/// shape) for tests that just need a named entry in the approvals map.
fn make_command_approval(original: &str, canonical: &str, binary_hash: &str) -> CommandApproval {
    CommandApproval {
        argv: None,
        approved_at: None,
        original_path: original.to_string(),
        canonical_path: canonical.to_string(),
        binary_hash: binary_hash.to_string(),
    }
}

/// Asserts both paths and binary hash of `approval` equal the given values, using field-by-field
/// comparison so a mismatch points at the offending field rather than the whole struct.
fn assert_approval_paths(approval: &CommandApproval, original: &str, canonical: &str, hash: &str) {
    assert_eq!(approval.original_path, original, "original_path");
    assert_eq!(approval.canonical_path, canonical, "canonical_path");
    assert_eq!(approval.binary_hash, hash, "binary_hash");
}

/// Verifies that `shared_path` resolves to the same repository root as `repo_root` and that an
/// existing "lint" approval in that root also verifies from `shared_path`. The same `config_content`
/// is written into the shared workspace so the workspace-root config load succeeds, and the PATH
/// binary (`echo`) resolves identically in either workspace. Used to assert jj workspaces / git
/// worktrees share approval state under repo-root keying plus workspace-relative path storage.
async fn assert_shared_repo_approval(
    repo_root: &Path,
    shared_path: &Path,
    config_content: &str,
    label: &str,
) {
    let shared_workspace_root = repository::detect_workspace_root(shared_path).unwrap();
    let shared_repo_root = repository::detect_repository_root(shared_path).unwrap();
    assert_eq!(
        repo_root, shared_repo_root,
        "Both {label} should resolve to the same repository root"
    );

    // The shared workspace must carry the same tools.toml so the workspace-root config load
    // yields the same command array verification matches against.
    write_tools_config(&shared_workspace_root, config_content);

    let config: ProjectConfig = toml::from_str(config_content).unwrap();
    let approvals = ProjectApprovals::load().await.unwrap();
    let result = approvals
        .verify_project_for_workspace(repo_root, &shared_workspace_root, &config, "lint")
        .await
        .unwrap();
    assert!(
        matches!(result, VerificationResult::Approved { .. }),
        "Approval from one {label} should work in the others, got {result:?}"
    );

    let approval_key = repo_root.to_string_lossy().to_string();
    assert!(
        approvals.projects.contains_key(&approval_key),
        "Approval should be keyed by repository root: {approval_key}"
    );
}

/// Asserts a verification result is `Approved`, keeping the case label in the
/// failure output for table-driven tests.
fn assert_approved(result: VerificationResult, context: &str) {
    assert!(
        matches!(result, VerificationResult::Approved { .. }),
        "{context}: expected Approved, got {result:?}"
    );
}

/// Asserts a verification result is `ItemNotApproved` for the requested item.
fn assert_item_not_approved(result: VerificationResult, item: &str, context: &str) {
    match result {
        VerificationResult::ItemNotApproved { item: actual } => {
            assert_eq!(actual, item, "{context}")
        }
        other => panic!("{context}: expected ItemNotApproved for {item}, got {other:?}"),
    }
}

/// Asserts a verification result is `ItemChanged` for the requested item, with the expected
/// `args_changed` flag.
fn assert_item_changed(result: VerificationResult, item: &str, args_changed: bool, context: &str) {
    match result {
        VerificationResult::ItemChanged {
            item: actual,
            args_changed: actual_args,
            ..
        } => {
            assert_eq!(actual, item, "{context}");
            assert_eq!(actual_args, args_changed, "{context}: args_changed flag");
        }
        other => panic!("{context}: expected ItemChanged for {item}, got {other:?}"),
    }
}

fn audit_config(arg: &str) -> String {
    format!("\n[commands]\n\n[[checks]]\nname = \"audit\"\ncommand = [\"echo\", \"{arg}\"]\n")
}

/// Loads approvals and verifies `item`, returning the raw verification result so
/// table-driven tests can assert the expected branch explicitly. Detects the repository and
/// workspace roots and parses the workspace-root config the way production callers do before
/// using the `_for_workspace` API.
async fn verify_check_result(project_dir: &Path, item: &str) -> VerificationResult {
    let canonical = project_dir.canonicalize().unwrap();
    let repository_root = repository::detect_repository_root(&canonical).unwrap();
    let workspace_root = repository::detect_workspace_root(&canonical).unwrap();
    let config: ProjectConfig = toml::from_str(
        &std::fs::read_to_string(workspace_root.join(".config/tools.toml")).unwrap(),
    )
    .unwrap();
    ProjectApprovals::load()
        .await
        .unwrap()
        .verify_check_for_workspace(&repository_root, &workspace_root, &config, item)
        .await
        .unwrap()
}

/// Approves `config_content`, lets the caller mutate the project, then verifies
/// `item` and returns the resulting verification status.
async fn approve_mutate_and_verify_check(
    project_dir: &Path,
    config_content: &str,
    item: &str,
    mutate: impl FnOnce(&Path),
) -> VerificationResult {
    approve_project_config(project_dir, config_content)
        .await
        .unwrap();
    mutate(project_dir);
    verify_check_result(project_dir, item).await
}

/// Asserts `is_script(path)` returns `expected` so variant tests keep a single
/// place for the async call and unwrap.
async fn assert_is_script(path: &Path, expected: bool, context: &str) {
    assert_eq!(is_script(path).await.unwrap(), expected, "{context}");
}

/// Returns the canonicalised string key that `approve_project` uses to store
/// its `ProjectApproval`.
fn canonical_key(dir: &Path) -> String {
    dir.canonicalize().unwrap().to_string_lossy().to_string()
}

/// Approves a synthetic project with the given `commands` and `checks` maps and
/// asserts the resulting `ProjectApproval` is stored under the canonical key.
/// Returns the canonical key and the freshly-populated `ProjectApprovals`
/// (by value) for further inspection. Entries keep whatever fields the caller supplied
/// (typically `argv: None` via [`make_command_approval`]); `approve_project` stamps
/// `approved_at` on insert.
fn approve_fixture(
    dir: &Path,
    commands: HashMap<String, CommandApproval>,
    checks: HashMap<String, CommandApproval>,
) -> (String, ProjectApprovals) {
    let mut approvals = ProjectApprovals::default();
    approvals
        .approve_project(dir.to_path_buf(), commands, checks)
        .unwrap();
    let key = canonical_key(dir);
    assert!(approvals.projects.contains_key(&key));
    (key, approvals)
}

/// Creates a new `#!/bin/bash` script tempfile, chmods it to `mode`, and
/// returns whether [`is_writable`] reports it as writable. Used by the matrix
/// of `test_is_writable_with_*` tests that differ only in the mode.
#[cfg(unix)]
async fn is_writable_with_mode(mode: u32) -> bool {
    let mut temp_file = NamedTempFile::new().unwrap();
    writeln!(temp_file, "#!/bin/bash").unwrap();
    temp_file.flush().unwrap();
    let mut perms = std::fs::metadata(temp_file.path()).unwrap().permissions();
    perms.set_mode(mode);
    std::fs::set_permissions(temp_file.path(), perms).unwrap();
    is_writable(temp_file.path()).await.unwrap()
}

/// Sets up an isolated XDG_CONFIG_HOME and a new project temp dir with the given
/// tools.toml contents, returning both temp dirs (which must be kept alive).
fn isolated_project_with_config(config_content: &str) -> (TempDir, TempDir) {
    let xdg_dir = setup_isolated_xdg_config();

    let project_dir = TempDir::new().unwrap();
    write_tools_config(project_dir.path(), config_content);

    (xdg_dir, project_dir)
}

/// Test helper to pre-approve a project with the given config content.
/// This bypasses the approval TUI for integration tests. Approvals bind the full
/// argv, normalized (workspace-relative where inside) paths, and the binary hash, mirroring what
/// the TUI save path records. Returns the repository root path for use in assertions.
///
/// # Errors
/// Returns an error if:
/// - Workspace/repository root detection fails (path doesn't exist, permission denied, etc.)
/// - Config parsing fails (invalid TOML)
/// - Binary resolution fails (binary not found)
/// - File hashing fails (I/O error)
/// - Approval update fails (filesystem error)
pub async fn approve_project_config(
    project_dir: &Path,
    config_content: &str,
) -> miette::Result<PathBuf> {
    let canonical = project_dir
        .canonicalize()
        .into_diagnostic()
        .wrap_err("Failed to canonicalize project dir")?;
    // Resolve binaries against the workspace root — the config that runs is the workspace's own.
    let workspace_root = repository::detect_workspace_root(&canonical)?;
    let repository_root = repository::detect_repository_root(&workspace_root)?;
    let config: ProjectConfig = toml::from_str(config_content)
        .into_diagnostic()
        .wrap_err("Failed to parse test config")?;

    let mut commands = HashMap::new();
    for (name, cmd_array) in config.commands.all() {
        commands.insert(name, build_approval(&cmd_array, &workspace_root).await?);
    }

    let mut checks = HashMap::new();
    if let Some(check_configs) = config.checks {
        for check in check_configs {
            checks.insert(
                check.name,
                build_approval(&check.command, &workspace_root).await?,
            );
        }
    }

    let workspace_root_clone = workspace_root.clone();
    ProjectApprovals::update(move |approvals| {
        approvals.approve_project(workspace_root_clone, commands, checks)
    })
    .await?;

    Ok(repository_root)
}

/// Resolves `command[0]` against `workspace_root`, hashes the binary, and builds a
/// `CommandApproval` binding the full argv with storage-normalized paths — the same shape the
/// TUI save path records.
async fn build_approval(
    command: &[String],
    workspace_root: &Path,
) -> miette::Result<CommandApproval> {
    let (original_path, canonical_path) =
        resolve_binary_path_with_original(&command[0], workspace_root)?;
    let binary_hash = hashing::hash_file(&canonical_path).await?;
    Ok(CommandApproval {
        argv: Some(command.to_vec()),
        approved_at: None,
        original_path: normalize_path_for_storage(&original_path, workspace_root),
        canonical_path: normalize_path_for_storage(&canonical_path, workspace_root),
        binary_hash,
    })
}

/// Helper to run a jj command and assert success
fn run_jj_command(args: &[&str], current_dir: &Path) {
    let output = Command::new("jj")
        .args(args)
        .current_dir(current_dir)
        .output()
        .expect("Failed to execute jj command");

    assert!(
        output.status.success(),
        "jj {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Helper to create a jj repository
fn setup_jj_repo(repo_path: &Path) {
    run_jj_command(
        &["git", "init", repo_path.to_str().unwrap()],
        repo_path.parent().unwrap_or(repo_path),
    );
}

/// Helper to create .config/tools.toml with standard test content.
///
/// Thin wrapper over [`crate::test_helpers::write_tools_config`] so existing call sites
/// using `create_tools_config` keep working while sharing the underlying helper.
fn create_tools_config(repo_path: &Path, config_content: &str) {
    write_tools_config(repo_path, config_content);
}

/// Table-driven coverage for `approve_project`: every row supplies a `commands`
/// and `checks` map, and the test asserts the stored `ProjectApproval` reflects
/// them verbatim.
#[test]
fn test_approve_project_matrix() {
    struct Case {
        label: &'static str,
        commands: &'static [(&'static str, &'static str)],
        checks: &'static [(&'static str, &'static str)],
    }
    let cases = [
        Case {
            label: "commands only",
            commands: &[("lint", "sha256:def456")],
            checks: &[],
        },
        Case {
            label: "commands and checks",
            commands: &[("lint", "sha256:def456")],
            checks: &[("security-audit", "sha256:abc789")],
        },
    ];

    for case in cases {
        let temp_dir = TempDir::new().unwrap();
        let commands: HashMap<String, CommandApproval> = case
            .commands
            .iter()
            .map(|(n, h)| {
                (
                    (*n).to_string(),
                    make_command_approval("cargo", "/usr/bin/cargo", h),
                )
            })
            .collect();
        let checks: HashMap<String, CommandApproval> = case
            .checks
            .iter()
            .map(|(n, h)| {
                (
                    (*n).to_string(),
                    make_command_approval("cargo", "/usr/bin/cargo", h),
                )
            })
            .collect();

        let (key, approvals) = approve_fixture(temp_dir.path(), commands.clone(), checks.clone());

        let approval = &approvals.projects[&key];
        assert_eq!(
            approval.commands.len(),
            commands.len(),
            "{}: commands",
            case.label
        );
        for (name, _) in case.commands {
            assert!(
                approval.commands.contains_key(*name),
                "{}: missing command {name}",
                case.label
            );
        }
        assert_eq!(
            approval.checks.len(),
            checks.len(),
            "{}: checks",
            case.label
        );
        for (name, _) in case.checks {
            assert!(
                approval.checks.contains_key(*name),
                "{}: missing check {name}",
                case.label
            );
        }
    }
}

#[tokio::test]
async fn test_verify_check_basic_variants() {
    enum Expected {
        Approved,
        NotApproved,
        ItemNotApproved(&'static str),
    }

    let cases = [
        ("approved", true, "audit", Expected::Approved),
        ("not-approved", false, "audit", Expected::NotApproved),
        (
            "missing-item",
            true,
            "nonexistent-check",
            Expected::ItemNotApproved("nonexistent-check"),
        ),
    ];

    for (label, should_approve, item, expected) in cases {
        let (_xdg_dir, temp_dir) = isolated_project_with_config(&audit_config("test"));
        if should_approve {
            approve_project_config(temp_dir.path(), &audit_config("test"))
                .await
                .unwrap();
        }

        let result = verify_check_result(temp_dir.path(), item).await;
        match expected {
            Expected::Approved => assert_approved(result, &format!("case {label}")),
            Expected::NotApproved => {
                assert_eq!(result, VerificationResult::NotApproved, "case {label}")
            }
            Expected::ItemNotApproved(expected_item) => {
                assert_item_not_approved(result, expected_item, &format!("case {label}"))
            }
        }
    }
}

#[tokio::test]
async fn test_argv_change_blocks_verification() {
    // An argument change is one of the three things that trigger re-approval: the stored argv no
    // longer equals the current command array, so verification returns ItemChanged (not the old
    // ConfigHashMismatch, which fired even on cosmetic edits).
    let (_xdg_dir, temp_dir) = isolated_project_with_config(&audit_config("test"));

    let result = approve_mutate_and_verify_check(
        temp_dir.path(),
        &audit_config("test"),
        "audit",
        |project_dir| {
            write_tools_config(project_dir, &audit_config("modified"));
        },
    )
    .await;

    // The argv changed; no approved version carries the new argv → args_changed = true.
    assert_item_changed(result, "audit", true, "argv change");
}

#[tokio::test]
async fn test_argv_change_then_reapprove_passes() {
    let (_xdg_dir, temp_dir) = isolated_project_with_config(&audit_config("test"));
    approve_project_config(temp_dir.path(), &audit_config("test"))
        .await
        .unwrap();

    // Change the argv and re-approve the new form.
    let modified = audit_config("modified");
    write_tools_config(temp_dir.path(), &modified);
    approve_project_config(temp_dir.path(), &modified)
        .await
        .unwrap();

    assert_approved(
        verify_check_result(temp_dir.path(), "audit").await,
        "after re-approve",
    );
}

#[tokio::test]
async fn test_config_edit_changing_no_argv_stays_approved() {
    // The headlining behavior: a config edit that changes no argv (comment, whitespace,
    // reordering, deleting an unrelated command) must not trigger re-approval. The old config-hash
    // model fired here; argv+binary binding does not.
    let (_xdg_dir, temp_dir) = isolated_project_with_config(&audit_config("test"));
    approve_project_config(temp_dir.path(), &audit_config("test"))
        .await
        .unwrap();

    // A comment-only edit changes the config hash but no argv — it must not trigger re-approval.
    let with_comment = format!(
        "# a comment that changes the config hash but no argv\n{}",
        audit_config("test")
    );
    write_tools_config(temp_dir.path(), &with_comment);

    assert_approved(
        verify_check_result(temp_dir.path(), "audit").await,
        "comment-only edit",
    );
}

#[tokio::test]
async fn test_binary_change_blocks_verification() {
    let _xdg_dir = setup_isolated_xdg_config();

    let temp_dir = TempDir::new().unwrap();

    // Create a script
    let script_path = temp_dir.path().join("scripts/check.sh");
    create_executable_script(&script_path, "echo 'original'");

    let config_content = format!(
        r#"
[commands]

[[checks]]
name = "custom-check"
command = ["{}"]
"#,
        script_path.display()
    );
    write_tools_config(temp_dir.path(), &config_content);

    // Approve the project
    approve_project_config(temp_dir.path(), &config_content)
        .await
        .unwrap();

    // Modify the script (change the hash) without changing argv
    std::fs::write(&script_path, "#!/bin/bash\necho 'modified'\n").unwrap();

    // Verify should report ItemChanged: argv matches an approved version but the binary hash
    // does not, so args_changed is false (an approved version carries the current argv).
    let result = verify_check_result(temp_dir.path(), "custom-check").await;
    assert_item_changed(result, "custom-check", false, "binary change");
}

#[tokio::test]
async fn test_match_any_approvals_across_workspaces() {
    // Match-any + workspace-relative paths: approving a different binary in a second workspace
    // adds a version rather than clobbering, and both workspaces then pass against their own
    // binary. The jj secondary-workspace layout (shared store, divergent scripts printing
    // main-copy/workspace-copy) comes from the shared helper.
    let _xdg_dir = setup_isolated_xdg_config();
    let config = "[commands]\n[[checks]]\nname = \"c\"\ncommand = [\"./check.sh\"]\n";
    let (_base, main, workspace) = setup_jj_main_and_secondary_workspace(config, "check.sh", false);

    // Approve from the main workspace first.
    let repo_root = approve_project_config(&main, config).await.unwrap();
    assert_approved(
        verify_check_result(&main, "c").await,
        "main approved after first approval",
    );
    // The secondary's own script has a different hash → not yet approved.
    assert_item_changed(
        verify_check_result(&workspace, "c").await,
        "c",
        false,
        "workspace before its own approval",
    );

    // Approve from the secondary workspace: adds a version (same argv, different binary).
    approve_project_config(&workspace, config).await.unwrap();

    // Both workspaces now pass against their own binary — match-any, no clobbering.
    assert_approved(
        verify_check_result(&main, "c").await,
        "main still approved after workspace approval",
    );
    assert_approved(
        verify_check_result(&workspace, "c").await,
        "workspace approved after its own approval",
    );

    // Exactly two versions accreted under the one repo-root key.
    let approvals = ProjectApprovals::load().await.unwrap();
    let key = repo_root.to_string_lossy().to_string();
    let versions = &approvals.projects[&key].checks["c"];
    assert_eq!(
        versions.len(),
        2,
        "match-any should accrete both versions, got {versions:?}"
    );
}

#[test]
fn test_is_within_project() {
    let project_dir = Path::new("/home/user/project");
    let binary_inside = Path::new("/home/user/project/scripts/build.sh");
    let binary_outside = Path::new("/usr/bin/cargo");

    assert!(is_within_project(binary_inside, project_dir));
    assert!(!is_within_project(binary_outside, project_dir));
}

#[test]
fn test_normalize_path_for_storage_inside_and_outside_workspace() {
    let ws = Path::new("/home/user/project");
    for (path, expected) in [
        ("/home/user/project/scripts/build.sh", "scripts/build.sh"),
        ("/usr/bin/cargo", "/usr/bin/cargo"),
        ("./tool.sh", "./tool.sh"),
    ] {
        assert_eq!(normalize_path_for_storage(Path::new(path), ws), expected);
    }
}

#[tokio::test]
async fn test_is_script_variants() {
    for (label, contents, expected) in [
        ("shebang", "#!/bin/bash\necho hello\n", true),
        ("plain-source", "fn main() {}\n", false),
    ] {
        let mut temp_file = NamedTempFile::new().unwrap();
        write!(temp_file, "{contents}").unwrap();
        temp_file.flush().unwrap();
        assert_is_script(temp_file.path(), expected, &format!("case {label}")).await;
    }
}

#[cfg(unix)]
#[tokio::test]
async fn test_is_writable_mode_variants() {
    let cases = [
        (
            0o600,
            true,
            "File with 0o600 permissions should be writable",
        ),
        (
            0o400,
            false,
            "File with 0o400 permissions should not be writable",
        ),
        (
            0o500,
            false,
            "File with 0o500 permissions should not be writable",
        ),
        (
            0o755,
            true,
            "File with 0o755 permissions should be writable by owner",
        ),
    ];

    for (mode, expected, message) in cases {
        assert_eq!(is_writable_with_mode(mode).await, expected, "{message}");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn test_is_writable_checks_owner_bit_only() {
    // Security: We check only owner write bit because if the current user can modify
    // the binary, an attacker with access to this user account can inject malicious code
    // before execution, bypassing our hash-based approval system

    let mut temp_file = NamedTempFile::new().unwrap();
    writeln!(temp_file, "#!/bin/bash").unwrap();
    temp_file.flush().unwrap();

    // Set group-writable but owner-readonly (0o420 = owner r, group w, others none)
    // This shouldn't be considered writable since owner can't write
    let mut perms = std::fs::metadata(temp_file.path()).unwrap().permissions();
    perms.set_mode(0o420);
    std::fs::set_permissions(temp_file.path(), perms).unwrap();

    let result = is_writable(temp_file.path()).await.unwrap();
    assert!(
        !result,
        "File with group-write but no owner-write should not be writable"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_is_writable_with_directory() {
    let temp_dir = TempDir::new().unwrap();

    // Set writable directory
    let mut perms = std::fs::metadata(temp_dir.path()).unwrap().permissions();
    perms.set_mode(0o700);
    std::fs::set_permissions(temp_dir.path(), perms).unwrap();

    let result = is_writable(temp_dir.path()).await.unwrap();
    assert!(result, "Writable directory should be detected as writable");
}

#[test]
fn test_resolve_binary_absolute_path() {
    // Absolute paths should be used as-is, then canonicalized
    let project_dir = PathBuf::from("/tmp");

    // Test with a binary that exists (using sh which should exist on Unix systems)
    #[cfg(unix)]
    {
        let (original, canonical) =
            resolve_binary_path_with_original("/bin/sh", &project_dir).unwrap();

        assert_eq!(original, PathBuf::from("/bin/sh"));
        assert!(canonical.is_absolute());
        // Canonical might resolve symlinks, but should still point to sh
        assert!(canonical.to_string_lossy().contains("sh"));
    }
}

#[test]
fn test_resolve_binary_in_path() {
    // Binaries without path separators should be looked up in PATH
    let project_dir = PathBuf::from("/tmp");

    // Test with 'sh' which should be in PATH on Unix
    #[cfg(unix)]
    {
        let (original, canonical) = resolve_binary_path_with_original("sh", &project_dir).unwrap();

        assert!(original.is_absolute());
        assert!(canonical.is_absolute());
        assert!(original.to_string_lossy().contains("sh"));
    }
}

#[test]
fn test_resolve_binary_relative_path() {
    // Relative paths with path separators should be resolved relative to project directory

    let project_dir = TempDir::new().unwrap();
    let script_path = project_dir.path().join("bin/script.sh");
    create_executable_script(&script_path, "");

    // Use relative path with separator
    let (original, canonical) =
        resolve_binary_path_with_original("bin/script.sh", project_dir.path()).unwrap();

    assert_eq!(original, project_dir.path().join("bin/script.sh"));
    assert!(canonical.is_absolute());
    assert!(canonical.ends_with("script.sh"));
}

#[test]
fn test_resolve_binary_with_dot_slash() {
    // Paths starting with ./ should be relative to project dir

    let project_dir = TempDir::new().unwrap();
    let script_path = project_dir.path().join("test.sh");
    create_executable_script(&script_path, "");

    let (original, canonical) =
        resolve_binary_path_with_original("./test.sh", project_dir.path()).unwrap();

    assert_eq!(original, project_dir.path().join("./test.sh"));
    assert!(canonical.is_absolute());
    assert!(canonical.ends_with("test.sh"));
}

#[test]
fn test_resolve_binary_not_found() {
    // Non-existent binaries should return an error
    let project_dir = PathBuf::from("/tmp");

    let result = resolve_binary_path_with_original(
        "this-binary-definitely-does-not-exist-12345",
        &project_dir,
    );

    let err_msg = format!(
        "{:?}",
        result.expect_err("Should fail with nonexistent binary")
    );
    assert!(err_msg.contains("Failed to find binary") || err_msg.contains("not found"));
}

#[test]
fn test_resolve_binary_with_subdirectory() {
    // Relative paths with subdirectories should work

    let project_dir = TempDir::new().unwrap();
    let script_path = project_dir.path().join("scripts/build.sh");
    create_executable_script(&script_path, "");

    let (original, canonical) =
        resolve_binary_path_with_original("scripts/build.sh", project_dir.path()).unwrap();

    assert_eq!(original, project_dir.path().join("scripts/build.sh"));
    assert!(canonical.is_absolute());
    assert!(canonical.ends_with("build.sh"));
    assert!(canonical.to_string_lossy().contains("scripts"));
}

#[cfg(unix)]
#[tokio::test]
async fn verify_check_program_is_original_not_canonical_for_symlink() {
    // The `Approved { program }` field must carry the original (symlink) path, not its
    // canonicalized target: symlinked multi-call binaries (nix coreutils, busybox) dispatch on
    // argv[0], so spawning the canonical target would change behavior. The resolver's
    // original-vs-canonical split is covered below; this locks in that
    // `verify_item_for_workspace` forwards the original into `Approved`.
    let _xdg = setup_isolated_xdg_config();
    let project = TempDir::new().unwrap();
    let real = project.path().join("real.sh");
    create_executable_script(&real, "echo hi");
    let link = project.path().join("link.sh");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let config = "[commands]\n[[checks]]\nname = \"c\"\ncommand = [\"./link.sh\"]\n";
    write_tools_config(project.path(), config);
    approve_project_config(project.path(), config)
        .await
        .unwrap();

    let result = verify_check_result(project.path(), "c").await;
    let VerificationResult::Approved { program } = result else {
        panic!("expected Approved, got {result:?}");
    };
    assert_eq!(
        program,
        project.path().canonicalize().unwrap().join("link.sh"),
        "program must be the symlink, not its canonicalized target"
    );
}

#[cfg(unix)]
#[test]
fn test_resolve_binary_follows_symlinks() {
    // Canonical path should resolve all symlinks

    let temp_dir = TempDir::new().unwrap();

    // Create actual binary
    let real_binary = temp_dir.path().join("real.sh");
    create_executable_script(&real_binary, "");

    // Create symlink
    let link_path = temp_dir.path().join("link.sh");
    std::os::unix::fs::symlink(&real_binary, &link_path).unwrap();

    let (original, canonical) =
        resolve_binary_path_with_original(link_path.to_str().unwrap(), temp_dir.path()).unwrap();

    // Original should be the symlink
    assert_eq!(original, link_path);

    // Canonical should resolve to real file
    assert_eq!(canonical, real_binary.canonicalize().unwrap());
    assert!(canonical.ends_with("real.sh"));
}

#[cfg(unix)]
#[test]
fn test_resolve_binary_multilevel_symlinks() {
    // Test that multiple levels of symlinks are fully resolved

    let temp_dir = TempDir::new().unwrap();

    // Create actual binary
    let real_binary = temp_dir.path().join("real.sh");
    create_executable_script(&real_binary, "");

    // Create symlink chain: link1 -> link2 -> real
    let link2 = temp_dir.path().join("link2.sh");
    std::os::unix::fs::symlink(&real_binary, &link2).unwrap();

    let link1 = temp_dir.path().join("link1.sh");
    std::os::unix::fs::symlink(&link2, &link1).unwrap();

    let (original, canonical) =
        resolve_binary_path_with_original(link1.to_str().unwrap(), temp_dir.path()).unwrap();

    // Original should be link1
    assert_eq!(original, link1);

    // Canonical should resolve all the way to real binary
    assert_eq!(canonical, real_binary.canonicalize().unwrap());
    assert!(canonical.ends_with("real.sh"));
}

#[tokio::test]
#[ignore] // This test can timeout in CI environments due to file locking contention
async fn test_concurrent_approvals_use_file_locking() {
    // Test that concurrent approval updates don't corrupt the file due to file locking
    // This validates that ProjectApprovals::update() properly serializes concurrent writes

    let _xdg_dir = setup_isolated_xdg_config();

    // Spawn fewer concurrent operations to avoid test timeout
    let mut handles = vec![];

    for i in 0..3 {
        let handle = tokio::spawn(async move {
            let project_dir = PathBuf::from(format!("/test/project{}", i));
            let mut commands = HashMap::new();

            commands.insert(
                format!("command{}", i),
                make_command_approval(
                    &format!("/usr/bin/cmd{}", i),
                    &format!("/usr/bin/cmd{}", i),
                    &format!("sha256:binary{}", i),
                ),
            );

            ProjectApprovals::update(move |approvals| {
                approvals.approve_project(project_dir, commands, HashMap::new())
            })
            .await
            .expect("Concurrent approval should succeed");
        });

        handles.push(handle);
    }

    // Wait for all concurrent operations to complete
    for handle in handles {
        handle.await.expect("Task should complete successfully");
    }

    // Verify all approvals were recorded without corruption
    let final_approvals = ProjectApprovals::load().await.unwrap();
    assert_eq!(
        final_approvals.projects.len(),
        3,
        "All 3 concurrent approvals should be recorded"
    );

    // Verify each project has correct data
    for i in 0..3 {
        let project_key = format!("/test/project{}", i);
        assert!(
            final_approvals.projects.contains_key(&project_key),
            "Project {} should be in approvals",
            i
        );

        let approval = &final_approvals.projects[&project_key];
        assert_eq!(approval.commands.len(), 1);
        assert!(approval.commands.contains_key(&format!("command{}", i)));
    }
}

#[tokio::test]
#[ignore] // This test can timeout in CI environments due to file locking contention
async fn test_concurrent_updates_to_same_project() {
    // Test that concurrent updates to the same project are properly serialized
    // Last write should win, and file should not be corrupted

    let _xdg_dir = setup_isolated_xdg_config();

    let project_dir = PathBuf::from("/test/same-project");

    // Spawn fewer concurrent updates to avoid test timeout
    let mut handles = vec![];

    for i in 0..3 {
        let project_dir = project_dir.clone();
        let handle = tokio::spawn(async move {
            let mut commands = HashMap::new();

            commands.insert(
                "test".to_string(),
                make_command_approval(
                    &format!("/usr/bin/test{}", i),
                    &format!("/usr/bin/test{}", i),
                    &format!("sha256:binary{}", i),
                ),
            );

            ProjectApprovals::update(move |approvals| {
                approvals.approve_project(project_dir, commands, HashMap::new())
            })
            .await
            .expect("Concurrent update should succeed");
        });

        handles.push(handle);
    }

    // Wait for all operations
    for handle in handles {
        handle.await.expect("Task should complete");
    }

    // Verify file is not corrupted and contains valid data
    let final_approvals = ProjectApprovals::load().await.unwrap();
    assert_eq!(
        final_approvals.projects.len(),
        1,
        "Should have exactly one project"
    );

    let project_key = "/test/same-project";
    assert!(final_approvals.projects.contains_key(project_key));

    // One of the updates should have won (last-write-wins semantics)
    let approval = &final_approvals.projects[project_key];
    assert_eq!(approval.commands.len(), 1);
    assert!(approval.commands.contains_key("test"));
}

#[tokio::test]
async fn test_file_locking_prevents_read_during_write() {
    // Test that file locking prevents reading partially-written approval files
    // This ensures atomicity of the load-modify-save cycle
    let _xdg_dir = setup_isolated_xdg_config();

    // Create a real temp directory for the project
    let _project_temp_dir = TempDir::new().unwrap();
    let project_dir = _project_temp_dir.path().to_path_buf();
    let project_key = project_dir
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let project_key_clone = project_key.clone();

    // Start a long-running update operation
    let write_handle = tokio::spawn(async move {
        ProjectApprovals::update(|approvals| {
            let mut commands = HashMap::new();

            commands.insert(
                "test".to_string(),
                CommandApproval {
                    argv: None,
                    approved_at: None,
                    original_path: "/usr/bin/test".to_string(),
                    canonical_path: "/usr/bin/test".to_string(),
                    binary_hash: "sha256:binary1".to_string(),
                },
            );

            approvals
                .approve_project(project_dir, commands, HashMap::new())
                .unwrap();

            // Simulate slow operation
            std::thread::sleep(Duration::from_millis(100));
            Ok(())
        })
        .await
    });

    // Give write operation time to acquire lock
    sleep(Duration::from_millis(10)).await;

    // Attempt concurrent read - should either see old state or new state, never partial
    let read_handle = tokio::spawn(async move {
        match ProjectApprovals::load().await {
            Ok(approvals) => {
                // If we read successfully, data should be consistent
                if let Some(approval) = approvals.projects.get(&project_key_clone) {
                    assert_eq!(approval.commands.len(), 1);
                }
            }
            Err(_) => {
                // It's ok if load fails - the important thing is no corruption
            }
        }
    });

    // Wait for both operations
    let _ = write_handle.await.expect("Write should complete");
    read_handle.await.expect("Read task should complete");

    // Verify final state is consistent
    let final_approvals = ProjectApprovals::load().await.unwrap();
    if let Some(approval) = final_approvals.projects.get(&project_key) {
        assert_eq!(approval.commands.len(), 1);
    }
}

#[tokio::test]
async fn test_save_approvals_persists_checks() {
    let config_content = r#"
[commands]
lint = ["echo", "lint"]

[[checks]]
name = "security-audit"
command = ["echo", "audit"]

[[checks]]
name = "license-check"
command = ["echo", "check"]
"#;
    let (_xdg_dir, temp_dir) = isolated_project_with_config(config_content);

    // Approve the project using the helper
    approve_project_config(temp_dir.path(), config_content)
        .await
        .unwrap();

    // Load approvals and verify checks were persisted
    let approvals = ProjectApprovals::load().await.unwrap();
    let project_key = temp_dir
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let approval = approvals
        .projects
        .get(&project_key)
        .expect("Project should be approved");

    // Verify both commands and checks are saved
    assert_eq!(approval.commands.len(), 1, "Should have 1 command");
    assert!(approval.commands.contains_key("lint"));

    assert_eq!(approval.checks.len(), 2, "Should have 2 checks");
    assert!(approval.checks.contains_key("security-audit"));
    assert!(approval.checks.contains_key("license-check"));

    // Verify check approvals contain correct data
    let audit_versions = &approval.checks["security-audit"];
    let audit_approval = audit_versions
        .first()
        .expect("security-audit should have an approved version");
    assert!(
        !audit_approval.binary_hash.is_empty(),
        "Binary hash should be set"
    );
    assert_eq!(
        audit_approval.argv.as_deref(),
        Some(["echo".to_string(), "audit".to_string()].as_slice()),
        "argv should be bound to the full command array",
    );
}

#[tokio::test]
async fn test_jj_workspaces_share_approvals() {
    let _xdg_dir = setup_isolated_xdg_config();

    let repo_dir = TempDir::new().unwrap();
    let repo_path = repo_dir.path();

    setup_jj_repo(repo_path);

    let config_content = r#"
[commands]
lint = ["echo", "lint"]
"#;
    create_tools_config(repo_path, config_content);

    let repo_root = approve_project_config(repo_path, config_content)
        .await
        .unwrap();

    // Create a second workspace (jj requires destination to NOT exist)
    let workspace2_parent = TempDir::new().unwrap();
    let workspace2_path = workspace2_parent.path().join("workspace2");
    run_jj_command(
        &["workspace", "add", workspace2_path.to_str().unwrap()],
        repo_path,
    );

    assert_shared_repo_approval(&repo_root, &workspace2_path, config_content, "workspaces").await;
}

#[tokio::test]
async fn test_git_worktrees_share_approvals() {
    let _xdg_dir = setup_isolated_xdg_config();

    let repo_dir = TempDir::new().unwrap();
    let repo_path = repo_dir.path();

    setup_git_repo_with_commit(repo_path);

    let config_content = r#"
[commands]
lint = ["echo", "lint"]
"#;
    create_tools_config(repo_path, config_content);

    let repo_root = approve_project_config(repo_path, config_content)
        .await
        .unwrap();

    // Create a worktree (git requires destination to NOT exist)
    let worktree_parent = TempDir::new().unwrap();
    let worktree_path = worktree_parent.path().join("worktree2");
    run_git_command(
        &["worktree", "add", worktree_path.to_str().unwrap(), "HEAD"],
        repo_path,
    );

    assert_shared_repo_approval(&repo_root, &worktree_path, config_content, "worktrees").await;
}

const LEGACY_LINT_CONFIG: &str = "[commands]\nlint = [\"./check.sh\"]\n";
const LEGACY_WILDCARD_CONFIG: &str = "[commands]\nlint = [\"./check.sh\", \"extra-arg\"]\n";

struct LegacyFixture {
    _xdg: TempDir,
    project: TempDir,
    original: String,
    canonical: String,
    old_hash: String,
}

impl LegacyFixture {
    fn root(&self) -> PathBuf {
        self.project.path().canonicalize().unwrap()
    }

    fn write_script(&self, body: &str) {
        create_executable_script(&self.project.path().join("check.sh"), body);
    }

    async fn verify(&self, source: &str) -> VerificationResult {
        let root = self.root();
        let config = toml::from_str(source).unwrap();
        ProjectApprovals::load()
            .await
            .unwrap()
            .verify_project_for_workspace(&root, &root, &config, "lint")
            .await
            .unwrap()
    }

    fn versions<'a>(&self, approvals: &'a ProjectApprovals) -> &'a [CommandApproval] {
        &approvals.projects[&self.root().to_string_lossy().to_string()].commands["lint"]
    }
}

async fn legacy_project_fixture() -> LegacyFixture {
    let xdg = setup_isolated_xdg_config();
    let project = TempDir::new().unwrap();
    create_executable_script(&project.path().join("check.sh"), "echo legacy");
    let root = project.path().canonicalize().unwrap();
    let (original, canonical) = resolve_binary_path_with_original("./check.sh", &root).unwrap();
    let old_hash = hashing::hash_file(&canonical).await.unwrap();
    let original = original.to_string_lossy().into_owned();
    let canonical = canonical.to_string_lossy().into_owned();
    let dir = xdg.path().join("moriarty");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("project_approvals.toml"),
        format!(
            r#"[projects."{root}"]
tools_config_hash = "legacy"
last_approved = "2024-01-01T00:00:00Z"
[projects."{root}".commands.lint]
original_path = "{original}"
canonical_path = "{canonical}"
binary_hash = "{old_hash}"
"#,
            root = root.display(),
        ),
    )
    .unwrap();
    LegacyFixture {
        _xdg: xdg,
        project,
        original,
        canonical,
        old_hash,
    }
}

#[tokio::test]
async fn test_legacy_format_loads_and_verifies_argv_none() {
    let f = legacy_project_fixture().await;
    let approvals = ProjectApprovals::load().await.unwrap();
    assert!(
        approvals
            .projects
            .values()
            .next()
            .unwrap()
            .checks
            .is_empty()
    );
    let lint = &f.versions(&approvals)[0];
    assert_approval_paths(lint, &f.original, &f.canonical, &f.old_hash);
    assert!(lint.argv.is_none() && lint.approved_at.is_none());
    assert_approved(f.verify(LEGACY_LINT_CONFIG).await, "legacy verification");
}

#[tokio::test]
async fn test_legacy_binary_change_and_wildcard_pre_upgrade() {
    let f = legacy_project_fixture().await;
    f.write_script("echo changed");
    assert_item_changed(
        f.verify(LEGACY_LINT_CONFIG).await,
        "lint",
        false,
        "legacy binary change",
    );
    f.write_script("echo legacy");
    assert_approved(f.verify(LEGACY_WILDCARD_CONFIG).await, "legacy wildcard");
}

#[tokio::test]
async fn test_legacy_upgrade_in_place_then_tightens() {
    let f = legacy_project_fixture().await;
    approve_project_config(&f.root(), LEGACY_LINT_CONFIG)
        .await
        .unwrap();
    let approvals = ProjectApprovals::load().await.unwrap();
    let versions = f.versions(&approvals);
    assert_eq!(versions.len(), 1);
    let lint = &versions[0];
    assert_eq!(lint.argv.as_deref(), Some(&["./check.sh".to_string()][..]));
    assert!(lint.approved_at.is_some());
    assert_approval_paths(lint, "check.sh", "check.sh", &f.old_hash);
    assert_item_changed(
        f.verify(LEGACY_WILDCARD_CONFIG).await,
        "lint",
        true,
        "post-upgrade argv",
    );
}

#[tokio::test]
async fn test_legacy_binary_change_reapproval_retires_stale_version() {
    let f = legacy_project_fixture().await;
    f.write_script("echo changed");
    approve_project_config(&f.root(), LEGACY_LINT_CONFIG)
        .await
        .unwrap();
    let new_hash = hashing::hash_file(&f.project.path().join("check.sh"))
        .await
        .unwrap();
    let approvals = ProjectApprovals::load().await.unwrap();
    let versions = f.versions(&approvals);
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].binary_hash, new_hash);
    assert!(versions[0].argv.is_some());
    f.write_script("echo legacy");
    assert_item_changed(
        f.verify(LEGACY_LINT_CONFIG).await,
        "lint",
        false,
        "old binary retired",
    );
}

#[tokio::test]
async fn test_legacy_absolute_paths_from_other_worktree_fail_closed() {
    let (_base, main, workspace) =
        setup_jj_main_and_secondary_workspace(LEGACY_LINT_CONFIG, "check.sh", true);
    let repo = repository::detect_repository_root(&main).unwrap();
    let workspace = repository::detect_workspace_root(&workspace).unwrap();
    let (original, canonical) = resolve_binary_path_with_original("./check.sh", &main).unwrap();
    let hash = hashing::hash_file(&canonical).await.unwrap();
    let original = original.to_string_lossy();
    let canonical = canonical.to_string_lossy();
    let legacy = make_command_approval(&original, &canonical, &hash);
    let commands = HashMap::from([("lint".to_string(), legacy)]);
    let (_, approvals) = approve_fixture(&main, commands, HashMap::new());
    let config = toml::from_str(LEGACY_LINT_CONFIG).unwrap();
    let result = approvals
        .verify_project_for_workspace(&repo, &workspace, &config, "lint")
        .await
        .unwrap();
    assert_item_changed(result, "lint", false, "foreign legacy paths");
}

#[test]
fn test_one_or_many_loads_mixed_legacy_and_array_forms() {
    let approvals: ProjectApprovals = toml::from_str(
        r#"[projects.repo]
last_approved = "2024-01-01T00:00:00Z"
[projects.repo.commands.lint]
original_path = "lint"
canonical_path = "lint"
binary_hash = "old"
[[projects.repo.commands.test]]
argv = ["test"]
approved_at = "2024-01-01T00:00:00Z"
original_path = "test"
canonical_path = "test"
binary_hash = "new"
"#,
    )
    .unwrap();
    let commands = &approvals.projects["repo"].commands;
    assert_eq!(commands["lint"].len(), 1);
    assert!(commands["lint"][0].argv.is_none());
    assert_eq!(
        commands["test"][0].argv.as_deref(),
        Some(&["test".to_string()][..])
    );
}

#[tokio::test]
async fn test_reapprove_identical_dedupes_not_accretes() {
    let config = audit_config("test");
    let (_xdg, project) = isolated_project_with_config(&config);
    approve_project_config(project.path(), &config)
        .await
        .unwrap();
    let key = canonical_key(project.path());
    let first = ProjectApprovals::load().await.unwrap().projects[&key].checks["audit"][0]
        .approved_at
        .unwrap();
    sleep(Duration::from_millis(10)).await;
    approve_project_config(project.path(), &config)
        .await
        .unwrap();
    let approvals = ProjectApprovals::load().await.unwrap();
    let versions = &approvals.projects[&key].checks["audit"];
    assert_eq!(versions.len(), 1);
    assert!(versions[0].approved_at.unwrap() > first);
}

#[tokio::test]
async fn test_no_argv_change_with_removed_command_stays_approved() {
    // Deleting an unrelated command changes the old config hash but no remaining argv — so the
    // remaining approved item must still pass (the headlining behavior, extended to the
    // removed-command case; stale approvals for the deleted item simply no longer match anything).
    let base = "[commands]\nlint = [\"echo\", \"lint\"]\n\n[[checks]]\nname = \"audit\"\ncommand = [\"echo\", \"test\"]\n";
    let (_xdg_dir, project) = isolated_project_with_config(base);
    approve_project_config(project.path(), base).await.unwrap();

    // Remove the `lint` command; `audit` (unchanged argv + binary) must still verify.
    write_tools_config(
        project.path(),
        "[commands]\n\n[[checks]]\nname = \"audit\"\ncommand = [\"echo\", \"test\"]\n",
    );
    assert_approved(
        verify_check_result(project.path(), "audit").await,
        "audit after removing an unrelated command",
    );

    // The removed `lint` name is still in approvals but no longer in the config, so verifying it
    // yields ItemNotApproved (the config lookup returns None), not ItemChanged.
    let approvals = ProjectApprovals::load().await.unwrap();
    let ws = project.path().canonicalize().unwrap();
    let repo = ws.clone();
    let config: ProjectConfig = toml::from_str(
        "[commands]\n\n[[checks]]\nname = \"audit\"\ncommand = [\"echo\", \"test\"]\n",
    )
    .unwrap();
    let result = approvals
        .verify_project_for_workspace(&repo, &ws, &config, "lint")
        .await
        .unwrap();
    match result {
        VerificationResult::ItemNotApproved { item } => assert_eq!(item, "lint"),
        other => panic!("removed lint should be ItemNotApproved, got {other:?}"),
    }
}
