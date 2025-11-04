//! Project configuration types and loading.
//!
//! This module provides types and functions for loading project-specific tool
//! configurations from `.config/tools.toml`.
//!
//! # Configuration Format
//!
//! Projects must provide a `.config/tools.toml` file with command definitions:
//!
//! ```toml
//! [commands]
//! lint = ["cargo", "clippy", "--all-targets", "--", "--deny", "warnings"]
//! test = ["cargo", "nextest", "run"]
//! build = ["cargo", "build"]
//! format = ["cargo", "fmt"]
//! ```

use std::path::PathBuf;

use miette::IntoDiagnostic;
use serde::{Deserialize, Serialize};
use tokio::fs::read_to_string;

/// Project configuration loaded from `.config/tools.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// Available tool commands
    pub commands: Commands,
    /// User-defined validation checks.
    ///
    /// Optional to maintain compatibility with existing configurations that may not define checks.
    /// Projects can have commands only, checks only, or both.
    pub checks: Option<Vec<Check>>,
}

/// Tool command definitions.
///
/// Uses a fixed struct with 4 predefined fields because these commands correspond directly
/// to the MCP protocol's standardized tools (run_lint, run_test, run_build, run_formatter).
/// These are part of the public API contract that MCP clients expect. The fixed structure
/// provides compile-time type safety and prevents runtime errors when the MCP server
/// receives requests for these protocol-defined operations.
///
/// Each field is optional and contains the command and its arguments as a string array.
/// Projects may define some, all, or none of these commands.
///
/// This differs from [`Check`] which uses a dynamic Vec to support arbitrary user-defined
/// validation scripts with any names. See [`Check`] documentation for details on that design.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commands {
    /// Linter command (e.g., `["cargo", "clippy"]`)
    pub lint: Option<Vec<String>>,
    /// Test command (e.g., `["cargo", "nextest", "run"]`)
    pub test: Option<Vec<String>>,
    /// Build command (e.g., `["cargo", "build"]`)
    pub build: Option<Vec<String>>,
    /// Formatter command (e.g., `["cargo", "fmt"]`)
    pub format: Option<Vec<String>>,
}

impl Commands {
    /// Get all configured commands as a vector of (name, command_array) tuples.
    ///
    /// Returns a vector containing all non-None commands. The order is deterministic:
    /// lint, test, build, format.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use moriarty::project_config::Commands;
    /// let commands = Commands {
    ///     lint: Some(vec!["cargo".to_string(), "clippy".to_string()]),
    ///     test: Some(vec!["cargo".to_string(), "test".to_string()]),
    ///     build: None,
    ///     format: None,
    /// };
    ///
    /// let all = commands.all();
    /// assert_eq!(all.len(), 2);
    /// assert_eq!(all[0].0, "lint");
    /// ```
    pub fn all(&self) -> Vec<(String, Vec<String>)> {
        let mut result = Vec::new();

        if let Some(cmd) = &self.lint {
            result.push(("lint".to_string(), cmd.clone()));
        }
        if let Some(cmd) = &self.test {
            result.push(("test".to_string(), cmd.clone()));
        }
        if let Some(cmd) = &self.build {
            result.push(("build".to_string(), cmd.clone()));
        }
        if let Some(cmd) = &self.format {
            result.push(("format".to_string(), cmd.clone()));
        }

        result
    }
}

/// A user-defined check that can be run against the project.
///
/// Checks are similar to commands but are distinguished by their purpose:
/// - Commands (lint, test, build, format) are predefined project tools
/// - Checks are arbitrary validation scripts (e.g., security audits, license checks)
///
/// Both undergo the same approval process with binary verification and security checks.
///
/// # Fields
///
/// * `name` - Unique identifier for the check (e.g., "security-audit", "license-check")
/// * `command` - Command array where the first element is the binary path and subsequent
///   elements are arguments. The binary will be resolved, hashed, and verified during approval.
///
/// # Example
///
/// ```toml
/// [[checks]]
/// name = "security-audit"
/// command = ["cargo", "audit"]
///
/// [[checks]]
/// name = "license-check"
/// command = ["./scripts/check-licenses.sh"]
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize, Serialize)]
pub struct Check {
    pub name: String,
    pub command: Vec<String>,
}

/// Loads project settings from `.config/tools.toml` in the specified directory.
///
/// # Arguments
///
/// * `canonical_dir` - The canonical (absolute, symlink-resolved) path to the project directory.
///   Path should already be canonicalized by the caller to prevent traversal attacks.
///
/// # Returns
///
/// Returns the parsed `ProjectConfig` on success, or an error if the file cannot be read or parsed.
///
/// # Errors
///
/// Returns an error if:
/// - The `.config/tools.toml` file does not exist
/// - The file cannot be read
/// - The file contains invalid TOML syntax
/// - The TOML structure doesn't match the expected schema
pub async fn load_project_settings(canonical_dir: PathBuf) -> miette::Result<ProjectConfig> {
    // Path is already canonicalized by the caller to prevent traversal attacks
    let mut config_path = canonical_dir.clone();
    config_path.push(".config");
    config_path.push("tools.toml");

    let project_settings_contents = read_to_string(&config_path)
        .await
        .into_diagnostic()
        .map_err(|error| {
            error.context(format!(
                "failed to read project settings: {}",
                config_path.to_string_lossy()
            ))
        })?;

    let settings: ProjectConfig = toml::from_str(project_settings_contents.as_str())
        .into_diagnostic()
        .map_err(|error| error.context("failed to parse project settings"))?;

    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_project_with_config(config_content: &str) -> TempDir {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config_dir = temp_dir.path().join(".config");
        std::fs::create_dir(&config_dir).expect("Failed to create .config dir");
        std::fs::write(config_dir.join("tools.toml"), config_content)
            .expect("Failed to write tools.toml");
        temp_dir
    }

    #[tokio::test]
    async fn test_load_project_settings_valid_config() {
        let temp_dir = setup_project_with_config(
            r#"
[commands]
lint = ["cargo", "clippy"]
test = ["cargo", "nextest", "run"]
build = ["cargo", "build"]
format = ["cargo", "fmt"]
"#,
        );

        let config = load_project_settings(temp_dir.path().to_path_buf())
            .await
            .expect("Should load valid config");

        assert_eq!(
            config.commands.lint,
            Some(vec!["cargo".to_string(), "clippy".to_string()])
        );
        assert_eq!(
            config.commands.test,
            Some(vec![
                "cargo".to_string(),
                "nextest".to_string(),
                "run".to_string()
            ])
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
        let temp_dir = setup_project_with_config(
            r#"
[commands]
lint = ["cargo", "clippy"]
"#,
        );

        let config = load_project_settings(temp_dir.path().to_path_buf())
            .await
            .expect("Should load partial config");

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
        let temp_dir = TempDir::new().expect("Failed to create temp dir");

        let result = load_project_settings(temp_dir.path().to_path_buf()).await;

        let err = result.expect_err("Should fail when file doesn't exist");
        let err_msg = format!("{:?}", err);
        assert!(
            err_msg.contains("failed to read project settings") || err_msg.contains("No such file"),
            "Error should mention missing file: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_load_project_settings_invalid_toml() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config_dir = temp_dir.path().join(".config");
        std::fs::create_dir(&config_dir).expect("Failed to create .config dir");
        std::fs::write(config_dir.join("tools.toml"), "this is not valid toml [[[[")
            .expect("Failed to write invalid toml");

        let result = load_project_settings(temp_dir.path().to_path_buf()).await;

        let err = result.expect_err("Should fail with invalid TOML");
        let err_msg = format!("{:?}", err);
        assert!(
            err_msg.contains("failed to parse project settings") || err_msg.contains("TOML"),
            "Error should mention parsing failure: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_load_project_settings_empty_commands() {
        let temp_dir = setup_project_with_config(
            r#"
[commands]
"#,
        );

        let config = load_project_settings(temp_dir.path().to_path_buf())
            .await
            .expect("Should load config with empty commands");

        assert_eq!(config.commands.lint, None);
        assert_eq!(config.commands.test, None);
        assert_eq!(config.commands.build, None);
        assert_eq!(config.commands.format, None);
    }

    #[test]
    fn test_commands_all_empty() {
        let commands = Commands {
            lint: None,
            test: None,
            build: None,
            format: None,
        };

        let result = commands.all();
        assert_eq!(result.len(), 0, "Empty commands should return empty vector");
    }

    #[test]
    fn test_commands_all_single_command() {
        let commands = Commands {
            lint: Some(vec!["cargo".to_string(), "clippy".to_string()]),
            test: None,
            build: None,
            format: None,
        };

        let result = commands.all();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "lint");
        assert_eq!(result[0].1, vec!["cargo".to_string(), "clippy".to_string()]);
    }

    #[test]
    fn test_commands_all_present() {
        let commands = Commands {
            lint: Some(vec!["cargo".to_string(), "clippy".to_string()]),
            test: Some(vec!["cargo".to_string(), "test".to_string()]),
            build: Some(vec!["cargo".to_string(), "build".to_string()]),
            format: Some(vec!["cargo".to_string(), "fmt".to_string()]),
        };

        let result = commands.all();
        assert_eq!(result.len(), 4, "Should return all 4 commands");

        let names: Vec<String> = result.iter().map(|(name, _)| name.clone()).collect();
        assert!(names.contains(&"lint".to_string()));
        assert!(names.contains(&"test".to_string()));
        assert!(names.contains(&"build".to_string()));
        assert!(names.contains(&"format".to_string()));
    }

    #[test]
    fn test_commands_all_deterministic_order() {
        let commands = Commands {
            format: Some(vec!["fmt".to_string()]),
            lint: Some(vec!["lint".to_string()]),
            build: Some(vec!["build".to_string()]),
            test: Some(vec!["test".to_string()]),
        };

        let result = commands.all();

        // Order should always be: lint, test, build, format
        assert_eq!(result.len(), 4);
        assert_eq!(result[0].0, "lint");
        assert_eq!(result[1].0, "test");
        assert_eq!(result[2].0, "build");
        assert_eq!(result[3].0, "format");
    }

    #[test]
    fn test_commands_all_partial() {
        let commands = Commands {
            lint: Some(vec!["cargo".to_string(), "clippy".to_string()]),
            test: None,
            build: Some(vec!["cargo".to_string(), "build".to_string()]),
            format: None,
        };

        let result = commands.all();
        assert_eq!(result.len(), 2);

        let names: Vec<String> = result.iter().map(|(name, _)| name.clone()).collect();
        assert!(names.contains(&"lint".to_string()));
        assert!(names.contains(&"build".to_string()));
        assert!(!names.contains(&"test".to_string()));
        assert!(!names.contains(&"format".to_string()));
    }

    #[tokio::test]
    async fn test_load_project_settings_with_checks() {
        let temp_dir = setup_project_with_config(
            r#"
[commands]
lint = ["cargo", "clippy"]

[[checks]]
name = "security-audit"
command = ["cargo", "audit"]

[[checks]]
name = "license-check"
command = ["./scripts/check-licenses.sh"]
"#,
        );

        let config = load_project_settings(temp_dir.path().to_path_buf())
            .await
            .expect("Should load config with checks");

        assert_eq!(
            config.commands.lint,
            Some(vec!["cargo".to_string(), "clippy".to_string()])
        );
        assert!(config.checks.is_some());
        let checks = config.checks.unwrap();
        assert_eq!(checks.len(), 2);
        assert_eq!(checks[0].name, "security-audit");
        assert_eq!(
            checks[0].command,
            vec!["cargo".to_string(), "audit".to_string()]
        );
        assert_eq!(checks[1].name, "license-check");
        assert_eq!(
            checks[1].command,
            vec!["./scripts/check-licenses.sh".to_string()]
        );
    }

    #[tokio::test]
    async fn test_load_project_settings_with_checks_only() {
        let temp_dir = setup_project_with_config(
            r#"
[commands]

[[checks]]
name = "security-audit"
command = ["cargo", "audit"]
"#,
        );

        let config = load_project_settings(temp_dir.path().to_path_buf())
            .await
            .expect("Should load config with checks only");

        assert_eq!(config.commands.lint, None);
        assert_eq!(config.commands.test, None);
        assert_eq!(config.commands.build, None);
        assert_eq!(config.commands.format, None);
        assert!(config.checks.is_some());
        let checks = config.checks.unwrap();
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].name, "security-audit");
    }

    #[tokio::test]
    async fn test_load_project_settings_with_commands_and_checks() {
        let temp_dir = setup_project_with_config(
            r#"
[commands]
lint = ["cargo", "clippy"]
test = ["cargo", "nextest", "run"]
build = ["cargo", "build"]
format = ["cargo", "fmt"]

[[checks]]
name = "audit"
command = ["cargo", "audit"]

[[checks]]
name = "licenses"
command = ["./check-licenses.sh"]
"#,
        );

        let config = load_project_settings(temp_dir.path().to_path_buf())
            .await
            .expect("Should load config with both commands and checks");

        assert_eq!(
            config.commands.lint,
            Some(vec!["cargo".to_string(), "clippy".to_string()])
        );
        assert_eq!(
            config.commands.test,
            Some(vec![
                "cargo".to_string(),
                "nextest".to_string(),
                "run".to_string()
            ])
        );
        assert!(config.checks.is_some());
        let checks = config.checks.unwrap();
        assert_eq!(checks.len(), 2);
    }

    #[tokio::test]
    async fn test_load_project_settings_checks_missing_name() {
        let temp_dir = setup_project_with_config(
            r#"
[commands]

[[checks]]
command = ["cargo", "audit"]
"#,
        );

        let result = load_project_settings(temp_dir.path().to_path_buf()).await;
        let err = result.expect_err("Should fail when check is missing name");
        let err_msg = format!("{:?}", err);
        assert!(
            err_msg.contains("failed to parse project settings")
                || err_msg.contains("missing field"),
            "Error should mention parsing failure: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_load_project_settings_checks_missing_command() {
        let temp_dir = setup_project_with_config(
            r#"
[commands]

[[checks]]
name = "security-audit"
"#,
        );

        let result = load_project_settings(temp_dir.path().to_path_buf()).await;
        let err = result.expect_err("Should fail when check is missing command");
        let err_msg = format!("{:?}", err);
        assert!(
            err_msg.contains("failed to parse project settings")
                || err_msg.contains("missing field"),
            "Error should mention parsing failure: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_load_project_settings_invalid_checks_toml() {
        let temp_dir = setup_project_with_config(
            r#"
[commands]

[[checks]]
name = "audit"
command = "not an array"
"#,
        );

        let result = load_project_settings(temp_dir.path().to_path_buf()).await;
        let err = result.expect_err("Should fail with invalid checks TOML");
        let err_msg = format!("{:?}", err);
        assert!(
            err_msg.contains("failed to parse project settings")
                || err_msg.contains("invalid type"),
            "Error should mention parsing failure: {}",
            err_msg
        );
    }

    #[test]
    fn test_check_ordering() {
        let check_a = Check {
            name: "aaa-first".to_string(),
            command: vec!["echo".to_string()],
        };
        let check_z = Check {
            name: "zzz-last".to_string(),
            command: vec!["echo".to_string()],
        };
        let check_m = Check {
            name: "mmm-middle".to_string(),
            command: vec!["echo".to_string()],
        };

        // Checks should sort alphabetically by name
        assert!(check_a < check_z, "aaa should be less than zzz");
        assert!(check_a < check_m, "aaa should be less than mmm");
        assert!(check_m < check_z, "mmm should be less than zzz");

        // Test with a vector to ensure sorting works correctly
        let mut checks = [check_z.clone(), check_a.clone(), check_m.clone()];
        checks.sort();

        assert_eq!(checks[0].name, "aaa-first");
        assert_eq!(checks[1].name, "mmm-middle");
        assert_eq!(checks[2].name, "zzz-last");
    }
}
