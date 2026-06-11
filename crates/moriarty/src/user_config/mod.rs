//! User-level configuration for Moriarty hooks.
//!
//! This module manages user-specific settings that apply across all projects, such as
//! bash command validation rules. Configuration is stored in the XDG config directory
//! at `~/.config/moriarty/tool_rules.toml`.
//!
//! Unlike project-level configuration, user-level configuration does not go through
//! the approval/hashing system since it represents the user's personal preferences
//! rather than untrusted project settings.

use std::{collections::HashMap, path::Path};

use miette::{Context, IntoDiagnostic, Result};
use serde::{Deserialize, Serialize};

use crate::persistence::FileType;

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
/// [pattern_fragments]
/// safe_chars = "[^|&;$`]"
///
/// [[bash_rules]]
/// name = "deny-rm-rf"
/// pattern = "^rm\\s+-rf\\s+/"
/// action = { type = "Deny", value = "Dangerous recursive delete detected" }
///
/// [[bash_rules]]
/// name = "allow-ls"
/// pattern = "^ls{{safe_chars}}*$"
/// action = { type = "Allow" }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub struct UserConfig {
    /// Reusable regex fragments that can be referenced in patterns using {{fragment_name}} syntax.
    /// Fragments are expanded at configuration load time, providing zero runtime overhead.
    #[serde(default)]
    pub pattern_fragments: Option<HashMap<String, String>>,

    #[serde(default)]
    pub bash_rules: Option<Vec<BashRule>>,

    #[serde(default)]
    pub tool_rules: Option<Vec<ToolRule>>,
}

impl UserConfig {
    /// Stable hash of the effective rule set, used to stamp each hook decision (`rules_hash`) with
    /// the rules that produced it so `rules replay`/`rules suggest` can scope recorded history to the
    /// rules currently in force.
    ///
    /// The parsed config is hashed — not the file bytes — so comment, whitespace, and key-order edits
    /// do not fragment history, while any pattern/action/rule-order/fragment change yields a new hash.
    /// Hashing goes through `serde_json::to_value`, whose objects are `BTreeMap`-backed (the
    /// `preserve_order` feature is off), so the map keys (`pattern_fragments` and an ArgumentFilter
    /// `replace` table) serialize in sorted order and the hash is reproducible; rule `Vec` order, which
    /// is significant for first-match-wins, is preserved. The whole config is hashed because a
    /// `tool_rule` matching Bash short-circuits the bash engine and so co-determines the decision.
    pub fn effective_hash(&self) -> String {
        let canonical = serde_json::to_value(self)
            .and_then(|value| serde_json::to_string(&value))
            .expect("UserConfig is always JSON-serializable");
        crate::hashing::hash_string(&canonical)
    }
}

/// Rules evaluated in order with first-match-wins semantics.
///
/// # Compound commands
///
/// The hook splits each Bash command into leaf simple-commands and matches `pattern` against each
/// leaf independently (see `hooks::command_split`), so a pattern only needs to describe one command
/// — not a whole `a && b | c` pipeline. Operators are split off, command substitution/subshells bail
/// to a prompt, and writes to real files are capped at Ask, so allow-rules can be simple prefixes
/// (`^ls`) without spelling out pipes or shell-metacharacter exclusions. A pattern still guards a
/// program's *own* dangerous flags (e.g. `find -exec`, `sed -i`), which are invisible to the splitter.
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
#[serde(tag = "type")]
pub enum BashRuleAction {
    /// Deny execution of the command with the specified reason.
    Deny { value: String },
    /// Modify the command using the template string. Supports regex capture groups ($0, $1, $2, etc.).
    Modify { value: String },
    /// Explicitly allow the command to execute.
    Allow,
    /// Defer to the user when a command requires explicit authorization but shouldn't be auto-approved.
    /// Use this for potentially dangerous operations that need case-by-case evaluation.
    Ask,
    /// Filter command arguments by removing, adding, or replacing them.
    ///
    /// After filtering, the command is re-validated against all rules to ensure it's still safe.
    /// If the filtered command doesn't match an Allow rule or matches a Deny rule, it will be rejected.
    ///
    /// # Example
    /// ```toml
    /// [[bash_rules]]
    /// name = "cargo doc - strip browser flag"
    /// pattern = "^cargo doc\\b"
    /// action = {
    ///   type = "ArgumentFilter",
    ///   remove = ["--open", "-o"],
    ///   reason = "Browser flags removed"
    /// }
    /// ```
    ArgumentFilter {
        /// Arguments to remove from the command.
        /// Matches exact argument or argument prefix for --flag=value syntax.
        #[serde(skip_serializing_if = "Option::is_none")]
        remove: Option<Vec<String>>,
        /// Arguments to add to the end of the command.
        #[serde(skip_serializing_if = "Option::is_none")]
        add: Option<Vec<String>>,
        /// Map of arguments to replace (old -> new).
        #[serde(skip_serializing_if = "Option::is_none")]
        replace: Option<HashMap<String, String>>,
        /// Explanation of why arguments were filtered.
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

/// A rule for permissioning any Claude Code tool call (Read, Write, Edit, Bash, etc.).
///
/// Rules are evaluated in order with first-match-wins semantics. The `tool` field is an exact
/// string match against the tool name (or `"*"` for catch-all). Optional `allow_local = true`
/// requires that the `path` or `file_path` input resolves to a canonical path within the hook
/// cwd. Optional `field` + `pattern` provide regex matching against a specific field in the tool
/// input.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ToolRule {
    pub name: String,
    /// Exact tool name to match (e.g., "Read", "Write", "Bash"), or `"*"` for any tool.
    pub tool: String,
    /// When `true`, the rule only fires if the relevant path resolves within the
    /// canonicalized hook `cwd`. Prevents rules from matching absolute paths that point
    /// outside the current project, regardless of whether the regex would match.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub allow_local: bool,
    /// Optional field name in tool_input to match against.
    /// Must be paired with `pattern`; if only one is present, the rule is skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// Optional regex pattern to match against the field value.
    /// Must be paired with `field`; if only one is present, the rule is skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    pub action: ToolRuleAction,
}

/// Action to take when a tool rule matches.
///
/// Only Allow, Deny, and Ask are supported. Modify and ArgumentFilter are Bash-specific
/// and excluded because they operate on command strings, not arbitrary tool inputs.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum ToolRuleAction {
    /// Explicitly allow the tool call to execute.
    Allow,
    /// Deny execution of the tool call with the specified reason.
    Deny { value: String },
    /// Defer to the user for case-by-case authorization.
    Ask,
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

/// Loads user config from an explicit path, or the default XDG location when `path` is `None`.
///
/// Unlike [`load_user_config`], an explicit path that is missing or malformed is a hard error: the
/// user named the file, so silently falling back to defaults would mask a typo.
pub async fn load_user_config_from(path: Option<&Path>) -> Result<UserConfig> {
    let Some(path) = path else {
        return load_user_config().await;
    };

    let contents = tokio::fs::read(path)
        .await
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to read config file: {}", path.display()))?;
    toml::from_slice::<UserConfig>(&contents)
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to parse config file: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal `UserConfig` fixture for round-trip tests that only vary
    /// the bash/tool rule lists.
    fn sample_config(
        bash_rules: Option<Vec<BashRule>>,
        tool_rules: Option<Vec<ToolRule>>,
    ) -> UserConfig {
        UserConfig {
            pattern_fragments: None,
            bash_rules,
            tool_rules,
        }
    }

    #[test]
    fn test_user_config_default() {
        let config = UserConfig::default();
        assert_eq!(config.bash_rules, None);
        assert_eq!(config.pattern_fragments, None);
        assert_eq!(config.tool_rules, None);
    }

    fn allow(name: &str, pattern: &str) -> BashRule {
        BashRule {
            name: name.to_string(),
            pattern: pattern.to_string(),
            action: BashRuleAction::Allow,
        }
    }

    #[test]
    fn effective_hash_is_stable_across_fragment_insertion_order() {
        // pattern_fragments is a HashMap, so two configs with the same fragments inserted in a
        // different order must still hash identically — otherwise the rules_hash would flap between
        // runs of an unchanged config and the replay/suggest filter would exclude its own history.
        let mut a = HashMap::new();
        a.insert("alpha".to_string(), "a".to_string());
        a.insert("beta".to_string(), "b".to_string());
        let mut b = HashMap::new();
        b.insert("beta".to_string(), "b".to_string());
        b.insert("alpha".to_string(), "a".to_string());

        let config_a = UserConfig {
            pattern_fragments: Some(a),
            bash_rules: Some(vec![allow("ls", "^ls")]),
            tool_rules: None,
        };
        let config_b = UserConfig {
            pattern_fragments: Some(b),
            bash_rules: Some(vec![allow("ls", "^ls")]),
            tool_rules: None,
        };

        assert_eq!(config_a.effective_hash(), config_b.effective_hash());
    }

    #[test]
    fn effective_hash_changes_on_semantic_edits() {
        let base = sample_config(Some(vec![allow("ls", "^ls")]), None);

        // A changed pattern is a different rule set.
        let changed_pattern = sample_config(Some(vec![allow("ls", "^ls -la")]), None);
        assert_ne!(base.effective_hash(), changed_pattern.effective_hash());

        // Rule order is significant for first-match-wins, so reordering changes the hash.
        let reordered = sample_config(Some(vec![allow("cat", "^cat"), allow("ls", "^ls")]), None);
        let original_order =
            sample_config(Some(vec![allow("ls", "^ls"), allow("cat", "^cat")]), None);
        assert_ne!(reordered.effective_hash(), original_order.effective_hash());
    }

    #[test]
    fn test_bash_rule_serialization() {
        let rule = BashRule {
            name: "test-rule".to_string(),
            pattern: "^test".to_string(),
            action: BashRuleAction::Deny {
                value: "test reason".to_string(),
            },
        };

        let toml = toml::to_string(&rule).unwrap();
        let deserialized: BashRule = toml::from_str(&toml).unwrap();
        assert_eq!(rule, deserialized);
    }

    #[test]
    fn test_bash_rule_action_serialization() {
        let actions = vec![
            BashRuleAction::Deny {
                value: "reason".to_string(),
            },
            BashRuleAction::Modify {
                value: "$1 --flag".to_string(),
            },
            BashRuleAction::Allow,
            BashRuleAction::Ask,
        ];

        for action in actions {
            let toml = toml::to_string(&action).unwrap();
            let deserialized: BashRuleAction = toml::from_str(&toml).unwrap();
            assert_eq!(action, deserialized);
        }
    }

    #[test]
    fn test_bash_rule_action_argument_filter_serialization() {
        let mut replace_map = HashMap::new();
        replace_map.insert("-f".to_string(), "-i".to_string());

        let action = BashRuleAction::ArgumentFilter {
            remove: Some(vec!["--open".to_string(), "-o".to_string()]),
            add: Some(vec!["--offline".to_string()]),
            replace: Some(replace_map),
            reason: Some("Security".to_string()),
        };

        let toml = toml::to_string(&action).unwrap();
        assert!(toml.contains("ArgumentFilter"));
        assert!(toml.contains("--open"));
        assert!(toml.contains("--offline"));
        assert!(toml.contains("Security"));

        let deserialized: BashRuleAction = toml::from_str(&toml).unwrap();
        assert_eq!(deserialized, action);
    }

    #[test]
    fn test_bash_rule_action_argument_filter_partial_fields() {
        // Test with only remove field
        let action = BashRuleAction::ArgumentFilter {
            remove: Some(vec!["--open".to_string()]),
            add: None,
            replace: None,
            reason: None,
        };

        let toml = toml::to_string(&action).unwrap();
        let deserialized: BashRuleAction = toml::from_str(&toml).unwrap();
        assert_eq!(deserialized, action);

        // Test with only add field
        let action = BashRuleAction::ArgumentFilter {
            remove: None,
            add: Some(vec!["--offline".to_string()]),
            replace: None,
            reason: Some("Added offline flag".to_string()),
        };

        let toml = toml::to_string(&action).unwrap();
        let deserialized: BashRuleAction = toml::from_str(&toml).unwrap();
        assert_eq!(deserialized, action);
    }

    #[test]
    fn test_bash_rule_action_toml_format_compatibility() {
        // Verify that the TOML format matches what users would write in their config files.
        // This ensures the change from tuple variants to struct variants didn't break
        // the user-facing configuration format.

        // Test Deny action
        let toml_deny = r#"type = "Deny"
value = "reason for denial""#;
        let action: BashRuleAction = toml::from_str(toml_deny).unwrap();
        assert_eq!(
            action,
            BashRuleAction::Deny {
                value: "reason for denial".to_string()
            }
        );

        // Test Modify action
        let toml_modify = r#"type = "Modify"
value = "$1 --flag""#;
        let action: BashRuleAction = toml::from_str(toml_modify).unwrap();
        assert_eq!(
            action,
            BashRuleAction::Modify {
                value: "$1 --flag".to_string()
            }
        );

        // Test Allow action
        let toml_allow = r#"type = "Allow""#;
        let action: BashRuleAction = toml::from_str(toml_allow).unwrap();
        assert_eq!(action, BashRuleAction::Allow);

        // Test Ask action
        let toml_ask = r#"type = "Ask""#;
        let action: BashRuleAction = toml::from_str(toml_ask).unwrap();
        assert_eq!(action, BashRuleAction::Ask);

        // Test ArgumentFilter action
        let toml_filter = r#"type = "ArgumentFilter"
remove = ["--open", "-o"]
reason = "Browser not needed""#;
        let action: BashRuleAction = toml::from_str(toml_filter).unwrap();
        assert_eq!(
            action,
            BashRuleAction::ArgumentFilter {
                remove: Some(vec!["--open".to_string(), "-o".to_string()]),
                add: None,
                replace: None,
                reason: Some("Browser not needed".to_string()),
            }
        );
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

    /// Persist `test_config` to a temp XDG_CONFIG_HOME and assert load_user_config
    /// round-trips the same value.
    async fn assert_config_roundtrips(test_config: UserConfig) {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_CONFIG_HOME", temp_dir.path());
        FileType::Config
            .persist("tool_rules.toml", &test_config)
            .await
            .unwrap();
        let loaded_config = load_user_config().await.unwrap();
        assert_eq!(loaded_config, test_config);
        std::env::remove_var("XDG_CONFIG_HOME");
    }

    #[tokio::test]
    async fn test_load_user_config_with_rules() {
        assert_config_roundtrips(sample_config(
            Some(vec![
                BashRule {
                    name: "test-deny".to_string(),
                    pattern: "^rm".to_string(),
                    action: BashRuleAction::Deny {
                        value: "rm not allowed".to_string(),
                    },
                },
                BashRule {
                    name: "test-allow".to_string(),
                    pattern: "^ls".to_string(),
                    action: BashRuleAction::Allow,
                },
            ]),
            None,
        ))
        .await;
    }

    #[tokio::test]
    async fn test_load_user_config_empty_rules() {
        assert_config_roundtrips(sample_config(None, None)).await;
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

        let err_msg = load_user_config()
            .await
            .expect_err("Invalid TOML should return an error, not fail-open")
            .to_string();
        assert!(
            err_msg.contains("Failed to load user configuration") || err_msg.contains("TOML"),
            "Error message should mention configuration failure or TOML, got: {}",
            err_msg
        );

        std::env::remove_var("XDG_CONFIG_HOME");
    }

    #[test]
    fn test_tool_rule_serialization() {
        let rule = ToolRule {
            name: "test-rule".to_string(),
            tool: "Read".to_string(),
            allow_local: false,
            field: Some("file_path".to_string()),
            pattern: Some("\\.env$".to_string()),
            action: ToolRuleAction::Deny {
                value: "Cannot read .env files".to_string(),
            },
        };

        let toml = toml::to_string(&rule).unwrap();
        let deserialized: ToolRule = toml::from_str(&toml).unwrap();
        assert_eq!(rule, deserialized);
    }

    #[test]
    fn test_tool_rule_serialization_without_field_pattern() {
        let rule = ToolRule {
            name: "allow-read".to_string(),
            tool: "Read".to_string(),
            allow_local: false,
            field: None,
            pattern: None,
            action: ToolRuleAction::Allow,
        };

        let toml = toml::to_string(&rule).unwrap();
        assert!(!toml.contains("field"));
        assert!(!toml.contains("pattern"));
        assert!(!toml.contains("allow_local"));

        let deserialized: ToolRule = toml::from_str(&toml).unwrap();
        assert_eq!(rule, deserialized);
    }

    #[test]
    fn test_tool_rule_serialization_with_allow_local() {
        let rule = ToolRule {
            name: "allow-local-read".to_string(),
            tool: "Read".to_string(),
            allow_local: true,
            field: Some("file_path".to_string()),
            pattern: Some(r"^src/.*\.rs$".to_string()),
            action: ToolRuleAction::Allow,
        };

        let toml = toml::to_string(&rule).unwrap();
        assert!(toml.contains("allow_local = true"));

        let deserialized: ToolRule = toml::from_str(&toml).unwrap();
        assert_eq!(rule, deserialized);
    }

    #[test]
    fn test_tool_rule_action_serialization() {
        let actions = vec![
            ToolRuleAction::Allow,
            ToolRuleAction::Deny {
                value: "reason".to_string(),
            },
            ToolRuleAction::Ask,
        ];

        for action in actions {
            let toml = toml::to_string(&action).unwrap();
            let deserialized: ToolRuleAction = toml::from_str(&toml).unwrap();
            assert_eq!(action, deserialized);
        }
    }

    #[test]
    fn test_tool_rule_action_toml_format_compatibility() {
        let toml_allow = r#"type = "Allow""#;
        let action: ToolRuleAction = toml::from_str(toml_allow).unwrap();
        assert_eq!(action, ToolRuleAction::Allow);

        let toml_deny = r#"type = "Deny"
value = "not allowed""#;
        let action: ToolRuleAction = toml::from_str(toml_deny).unwrap();
        assert_eq!(
            action,
            ToolRuleAction::Deny {
                value: "not allowed".to_string()
            }
        );

        let toml_ask = r#"type = "Ask""#;
        let action: ToolRuleAction = toml::from_str(toml_ask).unwrap();
        assert_eq!(action, ToolRuleAction::Ask);
    }

    #[test]
    fn test_tool_rule_wildcard() {
        let rule = ToolRule {
            name: "catch-all".to_string(),
            tool: "*".to_string(),
            allow_local: false,
            field: None,
            pattern: None,
            action: ToolRuleAction::Ask,
        };

        let toml = toml::to_string(&rule).unwrap();
        let deserialized: ToolRule = toml::from_str(&toml).unwrap();
        assert_eq!(rule, deserialized);
    }

    #[test]
    fn test_user_config_round_trip_with_tool_rules() {
        let config = sample_config(
            Some(vec![BashRule {
                name: "allow-ls".to_string(),
                pattern: "^ls".to_string(),
                action: BashRuleAction::Allow,
            }]),
            Some(vec![
                ToolRule {
                    name: "allow-read".to_string(),
                    tool: "Read".to_string(),
                    allow_local: false,
                    field: None,
                    pattern: None,
                    action: ToolRuleAction::Allow,
                },
                ToolRule {
                    name: "deny-env-write".to_string(),
                    tool: "Write".to_string(),
                    allow_local: false,
                    field: Some("file_path".to_string()),
                    pattern: Some(r"\.env$".to_string()),
                    action: ToolRuleAction::Deny {
                        value: "Cannot write .env".to_string(),
                    },
                },
            ]),
        );

        let toml = toml::to_string(&config).unwrap();
        let deserialized: UserConfig = toml::from_str(&toml).unwrap();
        assert_eq!(config, deserialized);
    }
}
