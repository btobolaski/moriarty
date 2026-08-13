use super::*;

fn filter_remove(cmd: &str, remove: &[&str]) -> String {
    let remove = Some(remove.iter().map(|s| s.to_string()).collect());
    filter_arguments(cmd, &remove, &None, &None).unwrap()
}

fn filter_add(cmd: &str, add: &[&str]) -> String {
    let add = Some(add.iter().map(|s| s.to_string()).collect());
    filter_arguments(cmd, &None, &add, &None).unwrap()
}

fn filter_replace(cmd: &str, replacements: &[(&str, &str)]) -> String {
    let replace_map: HashMap<String, String> = replacements
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    filter_arguments(cmd, &None, &None, &Some(replace_map)).unwrap()
}

fn allow_rule(name: &str, pattern: &str) -> BashRule {
    BashRule {
        name: name.to_string(),
        pattern: pattern.to_string(),
        modes: None,
        action: BashRuleAction::Allow,
    }
}

fn deny_rule(name: &str, pattern: &str, reason: &str) -> BashRule {
    BashRule {
        name: name.to_string(),
        pattern: pattern.to_string(),
        modes: None,
        action: BashRuleAction::Deny {
            value: reason.to_string(),
        },
    }
}

fn ask_rule(name: &str, pattern: &str) -> BashRule {
    BashRule {
        name: name.to_string(),
        pattern: pattern.to_string(),
        modes: None,
        action: BashRuleAction::Ask,
    }
}

fn modify_rule(name: &str, pattern: &str, replacement: &str) -> BashRule {
    BashRule {
        name: name.to_string(),
        pattern: pattern.to_string(),
        modes: None,
        action: BashRuleAction::Modify {
            value: replacement.to_string(),
        },
    }
}

fn make_engine(rules: Vec<BashRule>) -> BashRuleEngine {
    make_engine_with_fragments(rules, None)
}

fn make_engine_with_fragments(
    rules: Vec<BashRule>,
    pattern_fragments: Option<HashMap<String, String>>,
) -> BashRuleEngine {
    make_engine_with_aliases_and_fragments(rules, &[], pattern_fragments)
}

fn make_engine_with_aliases(rules: Vec<BashRule>, aliases: &[&str]) -> BashRuleEngine {
    make_engine_with_aliases_and_fragments(rules, aliases, None)
}

fn make_engine_with_aliases_and_fragments(
    rules: Vec<BashRule>,
    aliases: &[&str],
    pattern_fragments: Option<HashMap<String, String>>,
) -> BashRuleEngine {
    BashRuleEngine::from_config(UserConfig {
        pattern_fragments,
        bash_path_aliases: aliases
            .iter()
            .map(|alias| BashPathAlias::validate((*alias).to_string()).unwrap())
            .collect(),
        bash_rules: Some(rules),
        tool_rules: None,
    })
    .unwrap()
}

fn allowed(name: &str) -> RuleResult {
    RuleResult::Allowed {
        rule_name: name.to_string(),
    }
}
fn denied(name: &str, reason: &str) -> RuleResult {
    RuleResult::Denied {
        rule_name: name.to_string(),
        reason: reason.to_string(),
    }
}
fn asked(name: &str) -> RuleResult {
    RuleResult::Asked {
        rule_name: name.to_string(),
    }
}
fn modified(name: &str, new_command: &str) -> RuleResult {
    RuleResult::Modified {
        rule_name: name.to_string(),
        new_command: new_command.to_string(),
    }
}

fn read_only_starter_engine() -> BashRuleEngine {
    make_engine(vec![
        allow_rule("allow-echo", r"^echo($|\s)"),
        allow_rule("allow-ls", r"^ls($|\s)"),
        allow_rule("allow-cat", r"^cat($|\s)"),
        allow_rule("allow-head", r"^head($|\s)"),
    ])
}

fn assert_compound_cases<'a>(
    engine: &BashRuleEngine,
    cwd: &str,
    cases: impl IntoIterator<Item = (&'a str, RuleResult)>,
) {
    for (command, expected) in cases {
        assert_eq!(
            engine.apply_rules_compound(command, cwd, None),
            expected,
            "case {command:?}"
        );
    }
}

mod arguments;
mod compound;
mod diagnostics;
mod explain;
mod fragments;
mod merge;
mod rule_engine;
