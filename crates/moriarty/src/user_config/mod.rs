//! User-level configuration for Moriarty hooks.
//!
//! This module manages user-specific settings that apply across all projects, such as
//! bash command validation rules. Configuration is stored in the XDG config directory
//! at `~/.config/moriarty/tool_rules.toml`.
//!
//! Unlike project-level configuration, user-level configuration does not go through
//! the approval/hashing system since it represents the user's personal preferences
//! rather than untrusted project settings.

use std::{
    borrow::Borrow,
    collections::{BTreeSet, HashMap},
    path::Path,
};

use miette::{Context, IntoDiagnostic};
use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::Value;

use crate::{permission_mode::PermissionMode, persistence::FileType};

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

/// Serde validation prevents malformed or shell-control names from entering the analysis policy,
/// even when a tool rule would short-circuit Bash evaluation before engine construction.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct BashPathAlias(String);

impl Borrow<str> for BashPathAlias {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl BashPathAlias {
    pub(crate) fn validate(value: String) -> Result<Self, String> {
        const SHELL_CONTROL_NAMES: &[&str] = &[
            "PATH",
            "HOME",
            "IFS",
            "CDPATH",
            "GLOBIGNORE",
            "BASH_ENV",
            "ENV",
            "SHELLOPTS",
            "BASHOPTS",
            "FPATH",
            "PS4",
            "PROMPT_COMMAND",
        ];

        let mut chars = value.chars();
        let valid_identifier = chars
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
            && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_');
        if !valid_identifier {
            return Err(format!(
                "invalid bash path alias `{value}`: expected a shell identifier matching [A-Za-z_][A-Za-z0-9_]*"
            ));
        }
        if SHELL_CONTROL_NAMES.contains(&value.as_str()) {
            return Err(format!(
                "unsafe bash path alias `{value}`: shell-control variables cannot be configured as path aliases"
            ));
        }

        Ok(Self(value))
    }
}

impl<'de> Deserialize<'de> for BashPathAlias {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::validate(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// # Example
///
/// ```toml
/// bash_path_aliases = ["P"]
///
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

    /// Only these trusted shell variables may participate in path-alias analysis.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub bash_path_aliases: BTreeSet<BashPathAlias>,

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
    /// do not fragment history, while any rule, fragment, mode, or alias-policy change yields a new hash.
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

/// Command actions and `AllowRedirect` actions use separate declaration-ordered domains, so a
/// match in one cannot shadow or authorize the other.
///
/// # Compound commands
///
/// The hook splits each Bash command into leaf simple-commands and matches `pattern` against each
/// leaf independently (see `hooks::command_split`), so a pattern only needs to describe one command
/// — not a whole `a && b | c` pipeline. Operators are split off, command substitution/subshells bail
/// to a prompt, and output endpoints require separate redirect rules, so allow-rules can be simple
/// prefixes (`^ls`) without spelling out pipes or shell-metacharacter exclusions. A pattern still
/// guards a program's *own* dangerous flags (e.g. `find -exec`, `sed -i`), which are invisible to
/// the splitter.
///
/// # Security: Shell Injection Risk
///
/// Modify actions use unescaped capture group replacement. Avoid patterns like `^docker (.*)`
/// that capture arbitrary input - use specific patterns like `^(docker\s+system\s+prune)$` instead.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct BashRule {
    pub name: String,
    pub pattern: String,
    /// Omission is the permanently supported unrestricted form and applies in every permission
    /// mode; an empty set intentionally disables the rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modes: Option<BTreeSet<PermissionMode>>,
    #[serde(deserialize_with = "deserialize_bash_rule_action")]
    pub action: BashRuleAction,
}

/// Action to take when a Bash rule matches its command or redirect-endpoint domain.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum BashRuleAction {
    /// Deny execution of the command with the specified reason.
    Deny { value: String },
    /// Modify the command using the template string. Supports regex capture groups ($0, $1, $2, etc.).
    Modify { value: String },
    /// Explicitly allow the command to execute.
    Allow,
    /// Authorize a matched output-redirect endpoint without granting command execution.
    ///
    /// When `allow_local` is true, the resolved ordinary path must remain within the canonical hook
    /// cwd; devices, special files, and descriptors require an unrestricted target rule.
    /// Redirect rules form a separate rule domain and never participate in command first-match
    /// selection.
    AllowRedirect {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        allow_local: bool,
    },
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AllowRedirectFields {
    #[serde(default)]
    allow_local: bool,
}

/// Action tables historically tolerated extra metadata, so existing variants retain that config
/// compatibility. AllowRedirect stays strict because misspelling `allow_local` widens path scope.
fn deserialize_bash_rule_action<'de, D>(deserializer: D) -> Result<BashRuleAction, D::Error>
where
    D: Deserializer<'de>,
{
    let mut value = Value::deserialize(deserializer)?;
    if let Value::Object(fields) = &mut value
        && fields.get("type").and_then(Value::as_str) == Some("AllowRedirect")
    {
        fields.remove("type");
        let fields =
            serde_json::from_value::<AllowRedirectFields>(value).map_err(de::Error::custom)?;
        return Ok(BashRuleAction::AllowRedirect {
            allow_local: fields.allow_local,
        });
    }
    serde_json::from_value(value).map_err(de::Error::custom)
}

/// Conditions target literal top-level input keys and are conjoined; ordered rules remain the
/// mechanism for alternatives and fallback behavior.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum ToolRuleCondition {
    /// Key membership is sufficient, including when the input value is null.
    Present { field: String },
    /// This is the exact inverse of `Present` for object inputs.
    Absent { field: String },
    /// Raw JSON equality preserves booleans, strings, numbers, arrays, and objects.
    Equals { field: String, value: Value },
    /// Only strings, numbers, and booleans participate in regex matching.
    Matches { field: String, pattern: String },
}

/// A rule for permissioning any Claude Code tool call (Read, Write, Edit, Bash, etc.).
///
/// Rules are evaluated in order with first-match-wins semantics. The `tool` field is an exact
/// string match against the tool name (or `"*"` for catch-all). With `allow_local = true`,
/// condition-free rules retain their legacy path selection, while condition-bearing rules require
/// every `path` or `file_path` selected by a presence-requiring predicate or legacy path field to
/// resolve within the hook cwd. Optional `field` + `pattern` provide legacy regex matching against
/// one field, while `conditions` adds typed, presence-aware predicates over literal top-level input
/// keys. Every configured check is conjoined.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ToolRule {
    pub name: String,
    /// Exact tool name to match (e.g., "Read", "Write", "Bash"), or `"*"` for any tool.
    pub tool: String,
    /// Omission is the permanently supported unrestricted form and applies in every permission
    /// mode; an empty set intentionally disables the rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modes: Option<BTreeSet<PermissionMode>>,
    /// When `true`, every path selected by the rule must resolve within the canonicalized hook
    /// `cwd`; rules without conditions retain the legacy single-path or either-path fallback.
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
    /// Optional top-level input predicates that must all match. Separate ordered rules provide
    /// alternatives and fallback behavior.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<ToolRuleCondition>,
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
pub async fn load_user_config() -> miette::Result<UserConfig> {
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
pub async fn load_user_config_from(path: Option<&Path>) -> miette::Result<UserConfig> {
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
    use serde_json::{json, to_value};

    use super::*;
    use crate::test_helpers::setup_isolated_xdg_config;

    /// Builds a minimal `UserConfig` fixture for round-trip tests that only vary
    /// the bash/tool rule lists.
    fn sample_config(
        bash_rules: Option<Vec<BashRule>>,
        tool_rules: Option<Vec<ToolRule>>,
    ) -> UserConfig {
        UserConfig {
            pattern_fragments: None,
            bash_path_aliases: BTreeSet::new(),
            bash_rules,
            tool_rules,
        }
    }

    #[test]
    fn empty_bash_path_aliases_are_omitted() {
        let absent = UserConfig::default();
        let explicit: UserConfig = toml::from_str("bash_path_aliases = []").unwrap();
        let serialized = serde_json::to_value(&explicit).unwrap();

        assert_eq!(explicit, absent);
        assert_eq!(explicit.effective_hash(), absent.effective_hash());
        assert!(serialized.get("bash_path_aliases").is_none());
        assert!(
            !toml::to_string(&explicit)
                .unwrap()
                .contains("bash_path_aliases")
        );
    }

    #[test]
    fn existing_bash_actions_tolerate_unknown_fields() {
        let config = r#"
            [[bash_rules]]
            name = "compatible"
            pattern = ".*"
            action = { type = "Allow", note = "read only" }
        "#;

        let config = toml::from_str::<UserConfig>(config).unwrap();
        assert_eq!(config.bash_rules.unwrap()[0].action, BashRuleAction::Allow);
    }

    #[test]
    fn allow_redirect_rejects_invalid_shapes() {
        let config = r#"
            [[bash_rules]]
            name = "typo"
            pattern = ".*"
            action = { type = "AllowRedirect", allow_locl = true }
        "#;

        let error = toml::from_str::<UserConfig>(config)
            .unwrap_err()
            .to_string();
        assert!(error.contains("allow_locl"), "error: {error}");

        let scalar = config.replace(
            "{ type = \"AllowRedirect\", allow_locl = true }",
            "\"AllowRedirect\"",
        );
        assert!(toml::from_str::<UserConfig>(&scalar).is_err());
    }

    #[test]
    fn unsafe_bash_path_alias_names_are_rejected() {
        let malformed = ["", "1P", "P-X", "P X", "Å"]
            .into_iter()
            .map(|alias| (alias, "shell identifier"));
        let shell_control =
            "PATH HOME IFS CDPATH GLOBIGNORE BASH_ENV ENV SHELLOPTS BASHOPTS FPATH PS4 PROMPT_COMMAND"
                .split_whitespace()
                .map(|alias| (alias, "shell-control"));

        for (alias, reason) in malformed.chain(shell_control) {
            let error = BashPathAlias::validate(alias.to_string()).unwrap_err();
            assert!(
                error.contains(alias),
                "error should name {alias:?}: {error}"
            );
            assert!(error.contains(reason), "error: {error}");
        }
    }

    fn allow(name: &str, pattern: &str) -> BashRule {
        BashRule {
            name: name.to_string(),
            pattern: pattern.to_string(),
            modes: None,
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
            bash_path_aliases: BTreeSet::new(),
            bash_rules: Some(vec![allow("ls", "^ls")]),
            tool_rules: None,
        };
        let config_b = UserConfig {
            pattern_fragments: Some(b),
            bash_path_aliases: BTreeSet::new(),
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
    fn mode_restrictions_round_trip_canonically() {
        let config: UserConfig = toml::from_str(
            r#"
[[bash_rules]]
name = "allow-ls"
pattern = "^ls"
modes = ["plan", "default", "plan"]
action = { type = "Allow" }
"#,
        )
        .unwrap();

        assert_eq!(
            config.bash_rules.as_ref().unwrap()[0].modes,
            Some(BTreeSet::from([
                PermissionMode::Default,
                PermissionMode::Plan,
            ]))
        );
        let serialized = toml::to_string(&config).unwrap();
        assert!(serialized.contains("modes = [\"default\", \"plan\"]"));
        assert_eq!(toml::from_str::<UserConfig>(&serialized).unwrap(), config);

        let mut disabled = allow("disabled", "^never");
        disabled.modes = Some(BTreeSet::new());
        let round_tripped: BashRule = toml::from_str(&toml::to_string(&disabled).unwrap()).unwrap();
        assert_eq!(round_tripped.modes, Some(BTreeSet::new()));
    }

    #[test]
    fn rules_without_modes_keep_the_pre_feature_hash() {
        let unrestricted = sample_config(Some(vec![allow("ls", "^ls")]), None);
        let json = serde_json::to_value(&unrestricted).unwrap();
        assert!(json["bash_rules"][0].get("modes").is_none());
        let expected = r#"{"bash_rules":[{"action":{"type":"Allow"},"name":"ls","pattern":"^ls"}],"pattern_fragments":null,"tool_rules":null}"#;
        assert_eq!(
            unrestricted.effective_hash(),
            crate::hashing::hash_string(expected)
        );
    }

    #[test]
    fn redirect_actions_change_the_legacy_allow_hash() {
        let legacy = sample_config(Some(vec![allow("ls", "^ls")]), None);
        let redirect = |allow_local| {
            let mut config = legacy.clone();
            config.bash_rules.as_mut().unwrap()[0].action =
                BashRuleAction::AllowRedirect { allow_local };
            config
        };
        let unrestricted = redirect(false);
        let local = redirect(true);

        assert_ne!(unrestricted.effective_hash(), legacy.effective_hash());
        assert_ne!(local.effective_hash(), legacy.effective_hash());
        assert_ne!(unrestricted.effective_hash(), local.effective_hash());
    }

    #[test]
    fn adding_a_mode_restriction_changes_the_hash() {
        let unrestricted = sample_config(Some(vec![allow("ls", "^ls")]), None);
        let mut restricted = unrestricted.clone();
        restricted.bash_rules.as_mut().unwrap()[0].modes =
            Some(BTreeSet::from([PermissionMode::Plan]));

        assert_ne!(restricted.effective_hash(), unrestricted.effective_hash());
    }

    #[test]
    fn adding_or_removing_a_path_alias_changes_the_hash_deterministically() {
        let without_alias = sample_config(Some(vec![allow("ls", "^ls")]), None);
        let mut with_alias = without_alias.clone();
        let mut reordered_aliases = without_alias.clone();
        for alias in ["Z", "P"] {
            with_alias
                .bash_path_aliases
                .insert(BashPathAlias::validate(alias.to_string()).unwrap());
        }
        for alias in ["P", "Z", "P"] {
            reordered_aliases
                .bash_path_aliases
                .insert(BashPathAlias::validate(alias.to_string()).unwrap());
        }

        assert_ne!(with_alias.effective_hash(), without_alias.effective_hash());
        assert_eq!(
            with_alias.effective_hash(),
            reordered_aliases.effective_hash()
        );
    }

    #[test]
    fn test_bash_rule_serialization() {
        let rule = BashRule {
            name: "test-rule".to_string(),
            pattern: "^test".to_string(),
            modes: None,
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
            BashRuleAction::AllowRedirect { allow_local: false },
            BashRuleAction::AllowRedirect { allow_local: true },
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

        let redirect = BashRuleAction::AllowRedirect { allow_local: false };
        assert_eq!(
            toml::to_string(&redirect).unwrap(),
            "type = \"AllowRedirect\"\n"
        );

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
        let _xdg_dir = setup_isolated_xdg_config();

        let config = load_user_config().await.unwrap();
        assert_eq!(config, UserConfig::default());
    }

    /// Persist `test_config` to a temp XDG_CONFIG_HOME and assert load_user_config
    /// round-trips the same value.
    async fn assert_config_roundtrips(test_config: UserConfig) {
        let _temp_dir = setup_isolated_xdg_config();
        FileType::Config
            .persist("tool_rules.toml", &test_config)
            .await
            .unwrap();
        let loaded_config = load_user_config().await.unwrap();
        assert_eq!(loaded_config, test_config);
    }

    #[tokio::test]
    async fn test_load_user_config_with_rules() {
        assert_config_roundtrips(sample_config(
            Some(vec![
                BashRule {
                    name: "test-deny".to_string(),
                    pattern: "^rm".to_string(),
                    modes: None,
                    action: BashRuleAction::Deny {
                        value: "rm not allowed".to_string(),
                    },
                },
                BashRule {
                    name: "test-allow".to_string(),
                    pattern: "^ls".to_string(),
                    modes: None,
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
        let temp_dir = setup_isolated_xdg_config();

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
    }

    #[test]
    fn test_tool_rule_serialization() {
        let rule = ToolRule {
            name: "test-rule".to_string(),
            tool: "Read".to_string(),
            modes: None,
            allow_local: false,
            field: Some("file_path".to_string()),
            pattern: Some("\\.env$".to_string()),
            conditions: Vec::new(),
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
            modes: None,
            allow_local: false,
            field: None,
            pattern: None,
            conditions: Vec::new(),
            action: ToolRuleAction::Allow,
        };

        let toml = toml::to_string(&rule).unwrap();
        assert!(!toml.contains("field"));
        assert!(!toml.contains("pattern"));
        assert!(!toml.contains("allow_local"));
        assert!(!toml.contains("conditions"));

        let deserialized: ToolRule = toml::from_str(&toml).unwrap();
        assert_eq!(rule, deserialized);
    }

    #[test]
    fn tool_rule_conditions_round_trip_all_variants_and_nested_value() {
        let config: UserConfig = toml::from_str(
            r#"
[[tool_rules]]
name = "typed-conditions"
tool = "subagent"
conditions = [
  { type = "Present", field = "action" },
  { type = "Absent", field = "turnBudget" },
  { type = "Equals", field = "settings", value = { mode = "safe" } },
  { type = "Matches", field = "name", pattern = "^worker-{{number}}$" },
]
action = { type = "Allow" }
"#,
        )
        .unwrap();

        let conditions = &config.tool_rules.as_ref().unwrap()[0].conditions;
        assert_eq!(
            conditions,
            &[
                ToolRuleCondition::Present {
                    field: "action".to_string(),
                },
                ToolRuleCondition::Absent {
                    field: "turnBudget".to_string(),
                },
                ToolRuleCondition::Equals {
                    field: "settings".to_string(),
                    value: json!({"mode": "safe"}),
                },
                ToolRuleCondition::Matches {
                    field: "name".to_string(),
                    pattern: "^worker-{{number}}$".to_string(),
                },
            ]
        );

        let serialized = toml::to_string(&config).unwrap();
        let round_tripped: UserConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(round_tripped, config);
    }

    #[test]
    fn tool_rule_conditions_reject_missing_and_extra_variant_fields() {
        let malformed_conditions = [
            r#"{ type = "Equals", field = "async" }"#,
            r#"{ type = "Present", field = "action", typo = true }"#,
        ];

        for condition in malformed_conditions {
            let config = format!(
                r#"
[[tool_rules]]
name = "malformed"
tool = "subagent"
conditions = [{condition}]
action = {{ type = "Allow" }}
"#,
            );
            assert!(
                toml::from_str::<UserConfig>(&config).is_err(),
                "condition should be rejected: {condition}"
            );
        }
    }

    #[test]
    fn condition_free_tool_rule_keeps_legacy_hash_and_nonempty_conditions_change_it() {
        let legacy: UserConfig = toml::from_str(
            r#"
[[tool_rules]]
name = "allow-read"
tool = "Read"
action = { type = "Allow" }
"#,
        )
        .unwrap();

        let serialized = to_value(&legacy).unwrap();
        let rule = &serialized["tool_rules"][0];
        assert!(rule.get("conditions").is_none());

        let legacy_hash = legacy.effective_hash();
        let round_tripped: UserConfig = toml::from_str(&toml::to_string(&legacy).unwrap()).unwrap();
        assert_eq!(round_tripped.effective_hash(), legacy_hash);

        let mut conditioned = legacy.clone();
        conditioned.tool_rules.as_mut().unwrap()[0]
            .conditions
            .push(ToolRuleCondition::Absent {
                field: "action".to_string(),
            });
        assert_ne!(conditioned.effective_hash(), legacy.effective_hash());
    }

    #[test]
    fn test_tool_rule_serialization_with_allow_local() {
        let rule = ToolRule {
            name: "allow-local-read".to_string(),
            tool: "Read".to_string(),
            modes: None,
            allow_local: true,
            field: Some("file_path".to_string()),
            pattern: Some(r"^src/.*\.rs$".to_string()),
            conditions: Vec::new(),
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
            modes: None,
            allow_local: false,
            field: None,
            pattern: None,
            conditions: Vec::new(),
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
                modes: None,
                action: BashRuleAction::Allow,
            }]),
            Some(vec![
                ToolRule {
                    name: "allow-read".to_string(),
                    tool: "Read".to_string(),
                    modes: None,
                    allow_local: false,
                    field: None,
                    pattern: None,
                    conditions: Vec::new(),
                    action: ToolRuleAction::Allow,
                },
                ToolRule {
                    name: "deny-env-write".to_string(),
                    tool: "Write".to_string(),
                    modes: None,
                    allow_local: false,
                    field: Some("file_path".to_string()),
                    pattern: Some(r"\.env$".to_string()),
                    conditions: Vec::new(),
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
