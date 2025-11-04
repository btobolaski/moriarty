//! User-level configuration for Moriarty hooks.
//!
//! This module manages user-specific settings that apply across all projects, such as
//! bash command validation rules. Configuration is stored in the XDG config directory
//! at `~/.config/moriarty/tool_rules.toml`.
//!
//! Unlike project-level configuration, user-level configuration does not go through
//! the approval/hashing system since it represents the user's personal preferences
//! rather than untrusted project settings.

use miette::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::persistence::FileType;

/// Check if a miette error represents a "file not found" condition.
///
/// Miette's `into_diagnostic()` converts io::Error to miette::Report, losing type information.
/// We must check the error message for ENOENT (os error 2) which indicates file not found.
/// This is fragile but unavoidable given miette's design - the original io::Error::NotFound
/// is consumed during conversion and cannot be recovered.
fn is_not_found_error(error: &miette::Report) -> bool {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error.as_ref());

    while let Some(err) = current {
        let err_str = err.to_string();
        // Check both errno and text because different platforms/error sources format ENOENT differently
        if err_str.contains("os error 2") || err_str.contains("No such file or directory") {
            return true;
        }
        current = err.source();
    }

    false
}

/// # Example
///
/// ```toml
/// [[bash_rules]]
/// name = "deny-rm-rf"
/// pattern = "^rm\\s+-rf\\s+/"
/// action = { type = "Deny", value = "Dangerous recursive delete detected" }
///
/// [[bash_rules]]
/// name = "add-dry-run-to-docker-prune"
/// pattern = "^(docker\\s+system\\s+prune)"
/// action = { type = "Modify", value = "$1 --dry-run" }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub struct UserConfig {
    #[serde(default)]
    pub bash_rules: Option<Vec<BashRule>>,
}

/// Rules evaluated in order with first-match-wins semantics.
///
/// # Security: Shell Injection Risk
///
/// Modify actions use unescaped capture group replacement. Avoid patterns like `^docker (.*)`
/// that capture arbitrary input - use specific patterns like `^(docker\s+system\s+prune)$` instead.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct BashRule {
    pub name: String,
    pub pattern: String,
    pub action: BashRuleAction,
}

/// Action to take when a Bash rule matches a command.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", content = "value")]
pub enum BashRuleAction {
    /// Deny execution of the command with the specified reason.
    Deny(String),
    /// Modify the command using the template string. Supports regex capture groups ($0, $1, $2, etc.).
    Modify(String),
    /// Explicitly allow the command to execute.
    Allow,
}

/// Load user-level configuration from `~/.config/moriarty/tool_rules.toml`.
///
/// Fails-open only when the config file is missing, returning a default (empty) configuration.
/// If the file exists but contains invalid TOML or schema errors, this function returns an error
/// to alert the user of configuration problems that need fixing.
///
/// # Errors
///
/// Returns an error if:
/// - The configuration file exists but cannot be read
/// - The file contains invalid TOML syntax
/// - The TOML structure doesn't match the expected schema
///
/// # Example
///
/// ```no_run
/// # use moriarty::user_config::load_user_config;
/// # async fn example() -> miette::Result<()> {
/// let config = load_user_config().await?;
/// if let Some(rules) = config.bash_rules {
///     println!("Found {} bash rules", rules.len());
/// }
/// # Ok(())
/// # }
/// ```
pub async fn load_user_config() -> Result<UserConfig> {
    let result = FileType::Config.load::<UserConfig>("tool_rules.toml").await;

    match result {
        Ok(config) => Ok(config),
        Err(e) => {
            if is_not_found_error(&e) {
                Ok(UserConfig::default())
            } else {
                Err(e).context("Failed to load user configuration from tool_rules.toml")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_config_default() {
        let config = UserConfig::default();
        assert_eq!(config.bash_rules, None);
    }

    #[test]
    fn test_bash_rule_serialization() {
        let rule = BashRule {
            name: "test-rule".to_string(),
            pattern: "^test".to_string(),
            action: BashRuleAction::Deny("test reason".to_string()),
        };

        let toml = toml::to_string(&rule).unwrap();
        let deserialized: BashRule = toml::from_str(&toml).unwrap();
        assert_eq!(rule, deserialized);
    }

    #[test]
    fn test_bash_rule_action_serialization() {
        let actions = vec![
            BashRuleAction::Deny("reason".to_string()),
            BashRuleAction::Modify("$1 --flag".to_string()),
            BashRuleAction::Allow,
        ];

        for action in actions {
            let toml = toml::to_string(&action).unwrap();
            let deserialized: BashRuleAction = toml::from_str(&toml).unwrap();
            assert_eq!(action, deserialized);
        }
    }

    #[tokio::test]
    async fn test_load_user_config_missing_file() {
        // Set XDG_CONFIG_HOME to a temp directory that doesn't have the config file
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path());

        let config = load_user_config().await.unwrap();
        assert_eq!(config, UserConfig::default());

        std::env::remove_var("XDG_CONFIG_HOME");
    }

    #[tokio::test]
    async fn test_load_user_config_with_rules() {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path());

        let test_config = UserConfig {
            bash_rules: Some(vec![
                BashRule {
                    name: "test-deny".to_string(),
                    pattern: "^rm".to_string(),
                    action: BashRuleAction::Deny("rm not allowed".to_string()),
                },
                BashRule {
                    name: "test-allow".to_string(),
                    pattern: "^ls".to_string(),
                    action: BashRuleAction::Allow,
                },
            ]),
        };

        FileType::Config
            .persist("tool_rules.toml", &test_config)
            .await
            .unwrap();

        let loaded_config = load_user_config().await.unwrap();
        assert_eq!(loaded_config, test_config);

        std::env::remove_var("XDG_CONFIG_HOME");
    }

    #[tokio::test]
    async fn test_load_user_config_empty_rules() {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path());

        let test_config = UserConfig { bash_rules: None };

        FileType::Config
            .persist("tool_rules.toml", &test_config)
            .await
            .unwrap();

        let loaded_config = load_user_config().await.unwrap();
        assert_eq!(loaded_config, test_config);

        std::env::remove_var("XDG_CONFIG_HOME");
    }

    #[tokio::test]
    async fn test_load_user_config_invalid_toml() {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path());

        let moriarty_dir = temp_dir.path().join("moriarty");
        tokio::fs::create_dir_all(&moriarty_dir).await.unwrap();
        tokio::fs::write(
            moriarty_dir.join("tool_rules.toml"),
            "this is not valid [[[[ toml",
        )
        .await
        .unwrap();

        let result = load_user_config().await;
        assert!(
            result.is_err(),
            "Invalid TOML should return an error, not fail-open"
        );

        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Failed to load user configuration") || err_msg.contains("TOML"),
            "Error message should mention configuration failure or TOML, got: {}",
            err_msg
        );

        std::env::remove_var("XDG_CONFIG_HOME");
    }
}
