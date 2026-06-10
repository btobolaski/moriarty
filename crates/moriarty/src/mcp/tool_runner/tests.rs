use tempfile::TempDir;

use super::*;
use crate::project_config::approvals;
use crate::test_helpers::{setup_isolated_xdg_config, setup_project_dir_with_config};

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
/// and verification failures are surfaced to callers.
fn assert_invalid_request(err: &rmcp::ErrorData) {
    assert_eq!(err.code, ErrorCode::INVALID_REQUEST);
}

/// Asserts an error message mentions generic config-modification or hash
/// invalidation terms, as expected when tools.toml changes after approval.
fn assert_hash_or_modified_message(err: &rmcp::ErrorData) {
    let msg_lower = err.message.to_lowercase();
    assert!(
        msg_lower.contains("modified")
            || msg_lower.contains("hash")
            || msg_lower.contains("sha256"),
        "Error should indicate a hash/modification problem. Got: {}",
        err.message
    );
}

/// Asserts an error message specifically points at binary-hash verification,
/// not just a generic config mutation, for swapped-binary scenarios.
fn assert_binary_hash_message(err: &rmcp::ErrorData) {
    let msg_lower = err.message.to_lowercase();
    assert!(
        (msg_lower.contains("binary") || msg_lower.contains("modified"))
            && (msg_lower.contains("hash") || msg_lower.contains("sha256")),
        "Error should indicate binary hash mismatch. Got: {}",
        err.message
    );
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
async fn test_run_command_config_hash_mismatch() {
    // Simulate an attacker modifying tools.toml after legitimate approval
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
        panic!("Expected error for modified config");
    };

    assert_invalid_request(&error);
    assert!(error.message.contains("tools.toml has been modified"));
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
        let server = ToolRunner::default();
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
async fn test_resolves_symlinks_in_tool_runner() {
    let (temp_dir, _xdg_dir) = setup_project_dir_with_approvals(
        r#"
[commands]
test = ["echo", "hello"]
"#,
    )
    .await;

    // Create a symlink to the project directory
    let link_dir = TempDir::new().unwrap();
    let link_path = link_dir.path().join("project_link");

    #[cfg(unix)]
    std::os::unix::fs::symlink(temp_dir.path(), &link_path).unwrap();

    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(temp_dir.path(), &link_path).unwrap();

    let args = RunArgs {
        project_dir: link_path,
    };

    // Should succeed - canonicalize resolves the symlink
    let tool_result = ToolRunner::run_command(ProjectCommand::Test, args)
        .await
        .unwrap();
    assert_eq!(tool_result.is_error, Some(false));
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
    let msg_lower = error.message.to_lowercase();
    assert!(
        (msg_lower.contains("binary") || msg_lower.contains("modified"))
            && (msg_lower.contains("hash") || msg_lower.contains("sha256")),
        "Error should indicate binary hash mismatch. Got: {}",
        error.message
    );
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
        let msg_lower = error.message.to_lowercase();
        assert!(
            msg_lower.contains("canonical path")
                || msg_lower.contains("binary")
                || msg_lower.contains("modified"),
            "Error should indicate path or binary mismatch. Got: {}",
            error.message
        );
        assert!(
            msg_lower.contains("hash") || msg_lower.contains("sha256"),
            "Error should indicate a hash mismatch. Got: {}",
            error.message
        );
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

    // Step 3: Modify tools.toml
    let config_content_v2 = format!(
        r#"
[commands]
test = ["{}"]
build = ["echo", "build"]
"#,
        script_path.display()
    );

    std::fs::write(config_dir.join("tools.toml"), &config_content_v2).unwrap();

    // Step 4: Attempt execution - should fail due to config hash mismatch
    let Err(error) = ToolRunner::run_command(ProjectCommand::Test, args.clone()).await else {
        panic!("Execution should fail after config modification");
    };

    assert_invalid_request(&error);
    assert_hash_or_modified_message(&error);

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
    assert_binary_hash_message(&error);

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
    let server = ToolRunner::default();
    let info = server.get_info();

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
