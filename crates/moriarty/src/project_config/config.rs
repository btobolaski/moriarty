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
}

/// Tool command definitions.
///
/// Each field is optional and contains the command and its arguments as a string array.
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

        assert!(result.is_err(), "Should fail when file doesn't exist");
        let err = result.unwrap_err();
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

        assert!(result.is_err(), "Should fail with invalid TOML");
        let err = result.unwrap_err();
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
}
