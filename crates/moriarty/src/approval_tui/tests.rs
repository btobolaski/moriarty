use super::ApprovalApp;
use crate::{
    approval_tui::approval_state::{Screen, Section},
    test_helpers::{create_executable_script, setup_project_dir_with_config, write_tools_config},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tempfile::TempDir;

/// Alias for the shared fixture, chosen for readability in this module's tests.
use setup_project_dir_with_config as project_with_config;

/// Shared config with one `lint` command and one `security` check used by the
/// command→check section transition tests and the "requires all checks approved"
/// regression test.
const LINT_PLUS_SECURITY_TOML: &str = r#"
[commands]
lint = ["echo", "lint"]

[[checks]]
name = "security"
command = ["echo", "security"]
"#;

/// Creates a temp project with a `test.sh` in-project script plus an `echo`-based
/// `lint` command, writing a `.config/tools.toml` wired to both.
fn setup_test_project() -> TempDir {
    let temp_dir = TempDir::new().unwrap();

    let script_path = temp_dir.path().join("test.sh");
    create_executable_script(&script_path, "echo 'test'");

    let config_content = format!(
        r#"
[commands]
test = ["{}"]
lint = ["echo", "lint"]
"#,
        script_path.display()
    );
    write_tools_config(temp_dir.path(), &config_content);

    temp_dir
}

/// Builds an `ApprovalApp` for `temp_dir`, keeping the fixture alive in the caller.
async fn new_test_app(temp_dir: &TempDir) -> ApprovalApp {
    ApprovalApp::new(temp_dir.path().to_path_buf())
        .await
        .expect("ApprovalApp initialization should succeed")
}

#[tokio::test]
async fn test_approval_app_initialization() {
    // Test that ApprovalApp correctly loads project configuration
    let temp_dir = setup_test_project();

    let app = new_test_app(&temp_dir).await;

    // Verify initial state
    assert_eq!(app.state.current_item_index, 0);
    assert_eq!(app.state.screen, Screen::ProjectOverview);
    assert!(!app.should_quit);
    assert!(app.error_message.is_none());

    // Verify commands were loaded
    assert_eq!(app.state.commands.len(), 2, "Should load 2 commands");
    assert!(
        app.state.commands.iter().any(|c| c.name == "test"),
        "Should have test command"
    );
    assert!(
        app.state.commands.iter().any(|c| c.name == "lint"),
        "Should have lint command"
    );

    // Verify all commands start unapproved
    for cmd in &app.state.commands {
        assert!(!cmd.approved, "Commands should start unapproved");
    }
}

#[tokio::test]
async fn test_approval_app_initialization_with_empty_config() {
    // Test that empty tools.toml returns an error
    let temp_dir = project_with_config("[commands]\n");

    let err_msg = format!(
        "{:?}",
        ApprovalApp::new(temp_dir.path().to_path_buf())
            .await
            .expect_err("Empty config should return error")
    );
    assert!(
        err_msg.contains("No commands or checks configured"),
        "Error should mention no commands or checks configured"
    );
}

#[tokio::test]
async fn test_approval_app_initialization_with_missing_config() {
    // Test that missing tools.toml returns an error
    let temp_dir = TempDir::new().unwrap();

    let err_msg = format!(
        "{:?}",
        ApprovalApp::new(temp_dir.path().to_path_buf())
            .await
            .expect_err("Missing config should return error")
    );
    assert!(
        err_msg.contains("failed to read project settings") || err_msg.contains("No such file"),
        "Error should mention missing file, got: {}",
        err_msg
    );
}

#[tokio::test]
async fn test_approve_current_and_advance() {
    // Test state transitions when approving commands
    let temp_dir = setup_test_project();

    let mut app = new_test_app(&temp_dir).await;

    // Start at command review screen
    app.state.screen = Screen::CommandReview;
    app.state.current_item_index = 0;

    // Approve first command
    assert!(!app.state.commands[0].approved);
    app.approve_current_and_advance();
    assert!(
        app.state.commands[0].approved,
        "First command should be approved"
    );
    assert_eq!(
        app.state.current_item_index, 1,
        "Should advance to next command"
    );
    assert_eq!(
        app.state.screen,
        Screen::CommandReview,
        "Should stay in review screen"
    );

    // Approve second command
    assert!(!app.state.commands[1].approved);
    app.approve_current_and_advance();
    assert!(
        app.state.commands[1].approved,
        "Second command should be approved"
    );
    assert_eq!(
        app.state.screen,
        Screen::Summary,
        "Should move to summary screen"
    );
}

#[tokio::test]
async fn test_save_approvals_validation() {
    // Test that save_approvals validates all commands are approved
    let temp_dir = setup_test_project();
    let _xdg_dir = TempDir::new().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", _xdg_dir.path());

    let mut app = new_test_app(&temp_dir).await;

    // Try to save with unapproved commands
    let result = app.save_approvals().await;

    let err_msg = format!(
        "{:?}",
        result.expect_err("Should fail with unapproved commands")
    );
    assert!(
        err_msg.contains("not approved"),
        "Error should mention unapproved commands"
    );

    // Approve all commands
    for cmd in &mut app.state.commands {
        cmd.approved = true;
    }

    // Now save should succeed
    let result = app.save_approvals().await;
    result.expect("Should succeed with all commands approved");

    // Verify approvals were saved
    let approvals = crate::project_config::ProjectApprovals::load()
        .await
        .unwrap();
    let project_key = app.state.project_dir.to_string_lossy().to_string();
    assert!(
        approvals.projects.contains_key(&project_key),
        "Project should be in approvals"
    );
}

#[tokio::test]
async fn test_command_info_metadata_loading() {
    // Test that CommandInfo captures all necessary metadata
    let temp_dir = setup_test_project();

    let app = new_test_app(&temp_dir).await;

    // Find the test command
    let test_cmd = app
        .state
        .commands
        .iter()
        .find(|c| c.name == "test")
        .expect("Should have test command");

    // Verify metadata
    assert!(test_cmd.is_script, "test.sh should be detected as script");
    assert!(test_cmd.is_in_project, "test.sh should be in project");
    assert!(!test_cmd.binary_hash.is_empty(), "Should have binary hash");
    let script_contents = test_cmd
        .script_contents
        .as_ref()
        .expect("Should have script contents for writable script");
    assert!(!script_contents.is_empty(), "Script contents should be non-empty");

    // Find the lint command (uses echo binary)
    let lint_cmd = app
        .state
        .commands
        .iter()
        .find(|c| c.name == "lint")
        .expect("Should have lint command");

    assert!(
        !lint_cmd.is_script || lint_cmd.script_contents.is_none(),
        "echo binary should not have script contents"
    );
    assert!(!lint_cmd.is_in_project, "echo should not be in project");
}

#[tokio::test]
async fn test_in_project_warning_flow() {
    // Test that in-project writeable scripts trigger warning screen
    let temp_dir = setup_test_project();

    let app = new_test_app(&temp_dir).await;

    // Find the test command (in-project script)
    let test_cmd_idx = app
        .state
        .commands
        .iter()
        .position(|c| c.name == "test")
        .expect("Should have test command");

    let test_cmd = &app.state.commands[test_cmd_idx];

    // Verify it would trigger warning
    assert!(
        test_cmd.is_in_project && test_cmd.is_writable,
        "test.sh should trigger in-project warning"
    );
}

#[tokio::test]
async fn test_tools_config_hash_captured() {
    // Test that tools.toml hash is correctly computed and stored
    let temp_dir = setup_test_project();

    let app = new_test_app(&temp_dir).await;

    assert!(
        app.state.tools_config_hash.starts_with("sha256:"),
        "Config hash should have sha256 prefix"
    );
    assert_eq!(
        app.state.tools_config_hash.len(),
        71,
        "SHA-256 hash should be 7 (prefix) + 64 (hex) = 71 chars"
    );
}

#[tokio::test]
async fn test_canonical_path_resolution() {
    // Test that project directory is properly canonicalized
    let temp_dir = setup_test_project();

    // Create a symlink to the project
    #[cfg(unix)]
    {
        let link_dir = TempDir::new().unwrap();
        let link_path = link_dir.path().join("project_link");
        std::os::unix::fs::symlink(temp_dir.path(), &link_path).unwrap();

        let app = ApprovalApp::new(link_path).await.unwrap();

        // Project dir should be canonicalized (symlink resolved)
        assert!(
            app.state.project_dir.is_absolute(),
            "Project dir should be absolute"
        );
        assert_eq!(
            app.state.project_dir,
            temp_dir.path().canonicalize().unwrap(),
            "Symlink should be resolved to real path"
        );
    }
}

#[tokio::test]
async fn test_approval_app_with_checks() {
    // Test that ApprovalApp correctly loads projects with checks
    let temp_dir = project_with_config(
        r#"
[commands]
lint = ["echo", "lint"]

[[checks]]
name = "security-audit"
command = ["echo", "audit"]

[[checks]]
name = "license-check"
command = ["echo", "license"]
"#,
    );

    let app = new_test_app(&temp_dir).await;

    // Verify commands were loaded
    assert_eq!(app.state.commands.len(), 1, "Should load 1 command");
    assert!(
        app.state.commands.iter().any(|c| c.name == "lint"),
        "Should have lint command"
    );

    // Verify checks were loaded
    assert_eq!(app.state.checks.len(), 2, "Should load 2 checks");
    assert!(
        app.state.checks.iter().any(|c| c.name == "security-audit"),
        "Should have security-audit check"
    );
    assert!(
        app.state.checks.iter().any(|c| c.name == "license-check"),
        "Should have license-check check"
    );

    // Verify all checks start unapproved
    for check in &app.state.checks {
        assert!(!check.approved, "Checks should start unapproved");
    }

    // Verify initial section is Commands
    assert_eq!(
        app.state.current_section,
        Section::Commands,
        "Should start with Commands section"
    );
}

#[tokio::test]
async fn test_approve_commands_then_checks_section_transition() {
    // Test the section transition from Commands to Checks during approval flow
    let temp_dir = project_with_config(LINT_PLUS_SECURITY_TOML);

    let mut app = new_test_app(&temp_dir).await;

    // Start at command review
    app.state.screen = Screen::CommandReview;
    app.state.current_section = Section::Commands;
    app.state.current_item_index = 0;

    // Approve the command
    app.approve_current_and_advance();

    // Should transition to Checks section
    assert_eq!(
        app.state.current_section,
        Section::Checks,
        "Should transition to Checks section after last command"
    );
    assert_eq!(app.state.current_item_index, 0, "Should reset index to 0");
    assert_eq!(
        app.state.screen,
        Screen::CommandReview,
        "Should stay in CommandReview for checks"
    );

    // Approve the check
    app.approve_current_and_advance();

    // Should transition to Summary
    assert_eq!(
        app.state.screen,
        Screen::Summary,
        "Should transition to Summary after last check"
    );
}

#[tokio::test]
async fn test_save_approvals_requires_all_checks_approved() {
    // Test that save_approvals fails when checks are not approved
    let _xdg_dir = TempDir::new().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", _xdg_dir.path());

    let temp_dir = project_with_config(LINT_PLUS_SECURITY_TOML);

    let mut app = new_test_app(&temp_dir).await;

    // Approve only commands, not checks
    for cmd in &mut app.state.commands {
        cmd.approved = true;
    }

    // Should fail because checks not approved
    let result = app.save_approvals().await;
    let err_msg = format!(
        "{:?}",
        result.expect_err("Should fail when checks not approved")
    );
    assert!(
        err_msg.contains("security"),
        "Error should mention unapproved check"
    );

    // Now approve checks too
    for check in &mut app.state.checks {
        check.approved = true;
    }

    // Should succeed
    app.save_approvals()
        .await
        .expect("Should succeed when all items approved");
}

#[tokio::test]
async fn test_approval_app_with_only_checks() {
    // Test that a project with only checks (no commands) loads correctly
    let temp_dir = project_with_config(
        r#"
[commands]

[[checks]]
name = "security"
command = ["echo", "security"]

[[checks]]
name = "license"
command = ["echo", "license"]
"#,
    );

    let app = new_test_app(&temp_dir).await;

    assert_eq!(app.state.commands.len(), 0, "Should have no commands");
    assert_eq!(app.state.checks.len(), 2, "Should have 2 checks");
}

#[tokio::test]
async fn test_approve_checks_when_no_commands() {
    let _xdg_dir = TempDir::new().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", _xdg_dir.path());

    let temp_dir = project_with_config(
        r#"
[commands]

[[checks]]
name = "security-audit"
command = ["echo", "security"]

[[checks]]
name = "license-check"
command = ["echo", "license"]
"#,
    );

    let mut app = new_test_app(&temp_dir).await;

    // Verify no commands, only checks
    assert_eq!(app.state.commands.len(), 0);
    assert_eq!(app.state.checks.len(), 2);

    // Verify overview screen
    assert_eq!(app.state.screen, Screen::ProjectOverview);

    // Simulate pressing Enter to start approval (should go directly to Checks section)
    app.handle_overview_keys(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    // Should start at Checks section (not Commands since there are no commands)
    assert_eq!(app.state.current_section, Section::Checks);
    assert_eq!(app.state.screen, Screen::CommandReview);
    assert_eq!(app.state.current_item_index, 0);

    // Approve first check
    app.handle_command_review_keys(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.state.checks[0].approved);
    assert_eq!(app.state.current_item_index, 1);

    // Approve second check
    app.handle_command_review_keys(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.state.checks[1].approved);

    // Should transition to Summary (no Commands section to go to)
    assert_eq!(app.state.screen, Screen::Summary);

    // All checks should be approved
    assert!(app.state.checks.iter().all(|c| c.approved));
}

#[tokio::test]
async fn test_check_with_relative_script_path() {
    let _xdg_dir = TempDir::new().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", _xdg_dir.path());

    let temp_dir = TempDir::new().unwrap();

    // Create a script in a subdirectory with relative path
    let script_path = temp_dir.path().join("scripts/custom-check.sh");
    create_executable_script(&script_path, "echo 'checking'");

    write_tools_config(
        temp_dir.path(),
        r#"
[commands]

[[checks]]
name = "custom-check"
command = ["./scripts/custom-check.sh"]
"#,
    );

    let app = new_test_app(&temp_dir).await;

    assert_eq!(app.state.checks.len(), 1);
    let check = &app.state.checks[0];

    // Verify the check was loaded
    assert_eq!(check.name, "custom-check");

    // Verify paths were resolved relative to project directory
    // Canonicalize temp_dir to handle symlinks (e.g., /var -> /private/var on macOS)
    let canonical_temp_dir = temp_dir.path().canonicalize().unwrap();
    assert!(
        check.canonical_path.starts_with(&canonical_temp_dir),
        "Canonical path should be within project directory. Expected prefix: {:?}, Got: {:?}",
        canonical_temp_dir,
        check.canonical_path
    );
    assert!(
        check.canonical_path.ends_with("scripts/custom-check.sh"),
        "Should resolve to the script path"
    );

    // Verify hash was computed for the resolved file
    assert!(
        !check.binary_hash.is_empty(),
        "Binary hash should be computed"
    );

    // Verify script was detected
    assert!(check.is_script, "Should detect script from shebang");
}

#[tokio::test]
async fn test_check_metadata_captured() {
    let _xdg_dir = TempDir::new().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", _xdg_dir.path());

    let temp_dir = TempDir::new().unwrap();

    // Create a writable script in the project
    let writable_script = temp_dir.path().join("scripts/writable-check.sh");
    create_executable_script(&writable_script, "echo 'writable check'");

    let config_content = format!(
        r#"
[commands]

[[checks]]
name = "writable-check"
command = ["{}"]
"#,
        writable_script.display()
    );
    write_tools_config(temp_dir.path(), &config_content);

    let app = new_test_app(&temp_dir).await;

    assert_eq!(app.state.checks.len(), 1);
    let check = &app.state.checks[0];

    // Verify metadata is captured correctly for checks
    assert_eq!(check.name, "writable-check");
    assert!(check.is_script, "Should detect as script from shebang");

    #[cfg(unix)]
    {
        assert!(check.is_writable, "Should detect as writable on Unix");
        assert!(
            check.is_in_project,
            "Should detect as in-project since it's in scripts/"
        );

        // For writable scripts, content should be captured
        let contents = check
            .script_contents
            .as_ref()
            .expect("Writable script contents should be captured");
        assert!(
            contents.contains("writable check"),
            "Script contents should include the echo message"
        );
    }
}
