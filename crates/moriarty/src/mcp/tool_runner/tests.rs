use super::*;
use crate::project_config::{approvals, load_project_settings};
use tempfile::TempDir;

// IMPORTANT: Tests use `_xdg_dir` variables to keep TempDir instances alive.
// TempDir deletes its directory when dropped, so binding it to a variable (even
// with underscore prefix) prevents premature cleanup. Without this binding, the
// temporary XDG_CONFIG_HOME directory would be deleted before the test completes.

fn setup_project_dir_with_config(config_content: &str) -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join(".config");
    std::fs::create_dir(&config_dir).unwrap();
    std::fs::write(config_dir.join("tools.toml"), config_content).unwrap();
    temp_dir
}

/// Safe to use std::env::set_var because cargo nextest isolates each test in a separate process.
fn setup_isolated_xdg_config() -> tempfile::TempDir {
    let temp_dir = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", temp_dir.path());
    temp_dir
}

async fn setup_project_dir_with_approvals(config_content: &str) -> (TempDir, TempDir) {
    let xdg_dir = setup_isolated_xdg_config();
    let temp_dir = setup_project_dir_with_config(config_content);
    approvals::approve_project_config(temp_dir.path(), config_content).await;
    (temp_dir, xdg_dir)
}

#[tokio::test]
async fn test_load_project_settings_success() {
    let temp_dir = setup_project_dir_with_config(
        r#"
[commands]
lint = ["cargo", "clippy"]
test = ["cargo", "test"]
build = ["cargo", "build"]
format = ["cargo", "fmt"]
"#,
    );

    let config = load_project_settings(temp_dir.path().to_path_buf())
        .await
        .unwrap();

    assert_eq!(
        config.commands.lint,
        Some(vec!["cargo".to_string(), "clippy".to_string()])
    );
    assert_eq!(
        config.commands.test,
        Some(vec!["cargo".to_string(), "test".to_string()])
    );
    assert_eq!(
        config.commands.build,
        Some(vec!["cargo".to_string(), "build".to_string()])
    );
    assert_eq!(
        config.commands.format,
        Some(vec!["cargo".to_string(), "fmt".to_string()])
    );
}

#[tokio::test]
async fn test_load_project_settings_partial_config() {
    let temp_dir = setup_project_dir_with_config(
        r#"
[commands]
lint = ["cargo", "clippy"]
"#,
    );

    let config = load_project_settings(temp_dir.path().to_path_buf())
        .await
        .unwrap();

    assert_eq!(
        config.commands.lint,
        Some(vec!["cargo".to_string(), "clippy".to_string()])
    );
    assert_eq!(config.commands.test, None);
    assert_eq!(config.commands.build, None);
    assert_eq!(config.commands.format, None);
}

#[tokio::test]
async fn test_load_project_settings_missing_file() {
    let temp_dir = TempDir::new().unwrap();

    let result = load_project_settings(temp_dir.path().to_path_buf()).await;

    let error_msg = format!("{:?}", result.expect_err("Should fail with missing file"));
    assert!(error_msg.contains("failed to read project settings"));
}

#[tokio::test]
async fn test_load_project_settings_malformed_toml() {
    let temp_dir = setup_project_dir_with_config("this is not valid toml [[[");

    let result = load_project_settings(temp_dir.path().to_path_buf()).await;

    let error_msg = format!("{:?}", result.expect_err("Should fail with malformed TOML"));
    assert!(error_msg.contains("failed to parse project settings"));
}

#[tokio::test]
async fn test_run_command_success() {
    let (temp_dir, _xdg_dir) = setup_project_dir_with_approvals(
        r#"
[commands]
test = ["echo", "test output"]
"#,
    )
    .await;

    let args = RunArgs {
        project_dir: temp_dir.path().to_path_buf(),
    };

    let tool_result = ToolRunner::run_command(ProjectCommand::Test, args)
        .await
        .unwrap();
    assert_eq!(tool_result.is_error, Some(false));
    assert_eq!(tool_result.content.len(), 2);
}

#[tokio::test]
async fn test_run_command_not_configured() {
    // Verify that commands not in tools.toml are rejected with appropriate error
    let (temp_dir, _xdg_dir) = setup_project_dir_with_approvals(
        r#"
[commands]
lint = ["cargo", "clippy"]
"#,
    )
    .await;

    let args = RunArgs {
        project_dir: temp_dir.path().to_path_buf(),
    };

    let Err(error) = ToolRunner::run_command(ProjectCommand::Test, args).await else {
        panic!("Expected error for unconfigured command");
    };

    assert_eq!(error.code, ErrorCode::RESOURCE_NOT_FOUND);
    assert!(error.message.contains("not configured"));
}

#[tokio::test]
async fn test_run_command_not_approved() {
    // Unlike other tests, deliberately skip approval setup to test rejection path
    let _xdg_dir = setup_isolated_xdg_config();
    let temp_dir = setup_project_dir_with_config(
        r#"
[commands]
test = ["echo", "hello"]
"#,
    );

    let args = RunArgs {
        project_dir: temp_dir.path().to_path_buf(),
    };

    let Err(error) = ToolRunner::run_command(ProjectCommand::Test, args).await else {
        panic!("Expected error for unapproved project");
    };

    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert!(error.message.contains("not approved"));
}

#[tokio::test]
async fn test_run_command_nonzero_exit() {
    let (temp_dir, _xdg_dir) = setup_project_dir_with_approvals(
        r#"
[commands]
test = ["sh", "-c", "exit 1"]
"#,
    )
    .await;

    let args = RunArgs {
        project_dir: temp_dir.path().to_path_buf(),
    };

    let tool_result = ToolRunner::run_command(ProjectCommand::Test, args)
        .await
        .unwrap();
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

    let args = RunArgs {
        project_dir: temp_dir.path().to_path_buf(),
    };

    let Err(error) = ToolRunner::run_command(ProjectCommand::Test, args).await else {
        panic!("Expected error for modified config");
    };

    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    assert!(error.message.contains("tools.toml has been modified"));
}

#[tokio::test]
async fn test_project_command_display() {
    assert_eq!(format!("{}", ProjectCommand::Lint), "lint");
    assert_eq!(format!("{}", ProjectCommand::Test), "test");
    assert_eq!(format!("{}", ProjectCommand::Build), "build");
    assert_eq!(format!("{}", ProjectCommand::Format), "format");
}

#[tokio::test]
async fn test_run_lint_handler() {
    let (temp_dir, _xdg_dir) = setup_project_dir_with_approvals(
        r#"
[commands]
lint = ["echo", "Running lint"]
"#,
    )
    .await;

    let server = ToolRunner::default();
    let args = RunArgs {
        project_dir: temp_dir.path().to_path_buf(),
    };

    let tool_result = server.run_lint(Parameters(args)).await.unwrap();
    assert_eq!(tool_result.is_error, Some(false));
    assert_eq!(tool_result.content.len(), 2);
}

#[tokio::test]
async fn test_run_build_handler() {
    let (temp_dir, _xdg_dir) = setup_project_dir_with_approvals(
        r#"
[commands]
build = ["echo", "Building project"]
"#,
    )
    .await;

    let server = ToolRunner::default();
    let args = RunArgs {
        project_dir: temp_dir.path().to_path_buf(),
    };

    let tool_result = server.run_build(Parameters(args)).await.unwrap();
    assert_eq!(tool_result.is_error, Some(false));
    assert_eq!(tool_result.content.len(), 2);
}

#[tokio::test]
async fn test_run_formatter_handler() {
    let (temp_dir, _xdg_dir) = setup_project_dir_with_approvals(
        r#"
[commands]
format = ["echo", "Formatting code"]
"#,
    )
    .await;

    let server = ToolRunner::default();
    let args = RunArgs {
        project_dir: temp_dir.path().to_path_buf(),
    };

    let tool_result = server.run_formatter(Parameters(args)).await.unwrap();
    assert_eq!(tool_result.is_error, Some(false));
    assert_eq!(tool_result.content.len(), 2);
}

#[tokio::test]
async fn test_run_tests_handler() {
    let (temp_dir, _xdg_dir) = setup_project_dir_with_approvals(
        r#"
[commands]
test = ["echo", "Running tests"]
"#,
    )
    .await;

    let server = ToolRunner::default();
    let args = RunArgs {
        project_dir: temp_dir.path().to_path_buf(),
    };

    let tool_result = server.run_tests(Parameters(args)).await.unwrap();
    assert_eq!(tool_result.is_error, Some(false));
    assert_eq!(tool_result.content.len(), 2);
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
    // TOCTOU attack: Approve legitimate binary, then swap with malicious one
    // This simulates an attacker replacing a binary after approval but before execution
    use std::io::Write;

    let _xdg_dir = setup_isolated_xdg_config();

    // Create a custom script that will be approved
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join(".config");
    std::fs::create_dir(&config_dir).unwrap();

    let script_path = temp_dir.path().join("legitimate.sh");
    let mut script = std::fs::File::create(&script_path).unwrap();
    writeln!(script, "#!/usr/bin/env bash").unwrap();
    writeln!(script, "echo 'legitimate'").unwrap();
    drop(script);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms).unwrap();
    }

    let config_content = format!(
        r#"
[commands]
test = ["{}"]
"#,
        script_path.display()
    );

    std::fs::write(config_dir.join("tools.toml"), &config_content).unwrap();

    // Approve the legitimate binary
    approvals::approve_project_config(temp_dir.path(), &config_content).await;

    // TOCTOU attack: Swap the binary with malicious content after approval
    let mut malicious_script = std::fs::File::create(&script_path).unwrap();
    writeln!(malicious_script, "#!/usr/bin/env bash").unwrap();
    writeln!(malicious_script, "echo 'malicious'").unwrap();
    writeln!(malicious_script, "rm -rf /").unwrap(); // Simulated malicious command
    drop(malicious_script);

    // Attempt to execute - should be rejected due to hash mismatch
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
        approvals::approve_project_config(temp_dir.path(), &config_content).await;

        // TOCTOU attack: Change symlink to point to malicious binary
        std::fs::remove_file(&symlink_path).unwrap();
        std::os::unix::fs::symlink(&malicious_path, &symlink_path).unwrap();

        // Attempt execution - should be rejected (canonical path changed)
        let args = RunArgs {
            project_dir: temp_dir.path().to_path_buf(),
        };

        let Err(error) = ToolRunner::run_command(ProjectCommand::Test, args).await else {
            panic!("Symlink TOCTOU attack should be detected - target was changed");
        };

        assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
        let msg_lower = error.message.to_lowercase();
        assert!(
            (msg_lower.contains("canonical path")
                || msg_lower.contains("binary")
                || msg_lower.contains("modified"))
                && (msg_lower.contains("hash") || msg_lower.contains("sha256")),
            "Error should indicate path or hash mismatch. Got: {}",
            error.message
        );
    }
}

#[tokio::test]
async fn test_full_approval_lifecycle() {
    // Integration test: approve → execute → modify config → reject → re-approve → execute
    // This validates the complete approval workflow end-to-end
    use std::io::Write;

    // Setup isolated XDG config to avoid cross-test contamination
    let xdg_dir = setup_isolated_xdg_config();

    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join(".config");
    std::fs::create_dir(&config_dir).unwrap();

    let script_path = temp_dir.path().join("test.sh");
    let mut script = std::fs::File::create(&script_path).unwrap();
    writeln!(script, "#!/usr/bin/env bash").unwrap();
    writeln!(script, "echo 'test'").unwrap();
    drop(script);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms).unwrap();
    }

    let config_content_v1 = format!(
        r#"
[commands]
test = ["{}"]
"#,
        script_path.display()
    );

    std::fs::write(config_dir.join("tools.toml"), &config_content_v1).unwrap();

    // Step 1: Initial approval
    approvals::approve_project_config(temp_dir.path(), &config_content_v1).await;

    // Step 2: Execute command - should succeed
    let args = RunArgs {
        project_dir: temp_dir.path().to_path_buf(),
    };

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

    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    let msg_lower = error.message.to_lowercase();
    assert!(
        msg_lower.contains("modified")
            || msg_lower.contains("hash")
            || msg_lower.contains("sha256"),
        "Error should indicate config modification. Got: {}",
        error.message
    );

    // Step 5: Re-approve with new config
    approvals::approve_project_config(temp_dir.path(), &config_content_v2).await;

    // Step 6: Execute command again - should succeed with new approval
    let result = ToolRunner::run_command(ProjectCommand::Test, args).await;
    result.expect("Execution should succeed after re-approval");

    // Keep xdg_dir alive
    drop(xdg_dir);
}

#[tokio::test]
async fn test_approval_lifecycle_with_binary_modification() {
    // Integration test: approve → modify binary → reject → re-approve → execute
    // Validates that binary hash verification works throughout the lifecycle
    use std::io::Write;

    // Setup isolated XDG config to avoid cross-test contamination
    let xdg_dir = setup_isolated_xdg_config();

    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join(".config");
    std::fs::create_dir(&config_dir).unwrap();

    let script_path = temp_dir.path().join("build.sh");
    let mut script = std::fs::File::create(&script_path).unwrap();
    writeln!(script, "#!/usr/bin/env bash").unwrap();
    writeln!(script, "echo 'version 1'").unwrap();
    drop(script);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms).unwrap();
    }

    let config_content = format!(
        r#"
[commands]
build = ["{}"]
"#,
        script_path.display()
    );

    std::fs::write(config_dir.join("tools.toml"), &config_content).unwrap();

    // Approve version 1
    approvals::approve_project_config(temp_dir.path(), &config_content).await;

    // Execute - should succeed
    let args = RunArgs {
        project_dir: temp_dir.path().to_path_buf(),
    };
    let result = ToolRunner::run_command(ProjectCommand::Build, args.clone()).await;
    result.expect("Initial execution should succeed");

    // Modify the binary
    let mut script = std::fs::File::create(&script_path).unwrap();
    writeln!(script, "#!/usr/bin/env bash").unwrap();
    writeln!(script, "echo 'version 2 - modified'").unwrap();
    drop(script);

    // Attempt execution - should fail due to binary hash mismatch
    let Err(error) = ToolRunner::run_command(ProjectCommand::Build, args.clone()).await else {
        panic!("Execution should fail after binary modification");
    };

    assert_eq!(error.code, ErrorCode::INVALID_REQUEST);
    let msg_lower = error.message.to_lowercase();
    assert!(
        (msg_lower.contains("binary") || msg_lower.contains("modified"))
            && (msg_lower.contains("hash") || msg_lower.contains("sha256")),
        "Error should indicate binary modification. Got: {}",
        error.message
    );

    // Re-approve with modified binary
    approvals::approve_project_config(temp_dir.path(), &config_content).await;

    // Execute again - should succeed with new approval
    let result = ToolRunner::run_command(ProjectCommand::Build, args).await;
    result.expect("Execution should succeed after re-approval");

    // Keep xdg_dir alive
    drop(xdg_dir);
}
