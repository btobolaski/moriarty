use super::*;
use crate::test_helpers::{deny_redirect_rule, directional_redirect_rule, redirect_rule};

fn matched_redirect(
    trace: &RedirectEndpointTrace,
) -> (&RuleMatchExplanation, RedirectTraceDecision) {
    let RedirectTraceState::Matched { matched, decision } = &trace.state else {
        panic!("expected matched redirect trace");
    };
    (matched, *decision)
}

fn filter_command_for_test(command: &str) -> Option<FilterCommand> {
    let SplitOutcome::Commands(mut leaves) = split_command(command, "", &BTreeSet::new()) else {
        return None;
    };
    (leaves.len() == 1)
        .then(|| leaves.pop().and_then(|leaf| leaf.filter_command))
        .flatten()
}

fn filter_for_test(
    command: &str,
    remove: &Option<Vec<String>>,
    add: &Option<Vec<String>>,
    replace: &Option<HashMap<String, String>>,
) -> miette::Result<String> {
    let filter_command = filter_command_for_test(command)
        .ok_or_else(|| miette!("ArgumentFilter requires one brush-parsed simple command"))?;
    filter_arguments(&filter_command, remove, add, replace)
}

fn filter_remove(cmd: &str, remove: &[&str]) -> String {
    let remove = Some(remove.iter().map(|s| s.to_string()).collect());
    filter_for_test(cmd, &remove, &None, &None).unwrap()
}

fn filter_add(cmd: &str, add: &[&str]) -> String {
    let add = Some(add.iter().map(|s| s.to_string()).collect());
    filter_for_test(cmd, &None, &add, &None).unwrap()
}

fn filter_replace(cmd: &str, replacements: &[(&str, &str)]) -> String {
    let replace_map: HashMap<String, String> = replacements
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    filter_for_test(cmd, &None, &None, &Some(replace_map)).unwrap()
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

fn filter_rule(name: &str, pattern: &str, remove: &str, reason: Option<&str>) -> BashRule {
    BashRule {
        name: name.to_string(),
        pattern: pattern.to_string(),
        modes: None,
        action: BashRuleAction::ArgumentFilter {
            remove: Some(vec![remove.to_string()]),
            add: None,
            replace: None,
            reason: reason.map(str::to_string),
        },
    }
}

fn make_engine(rules: Vec<BashRule>) -> BashRuleEngine {
    make_engine_with_fragments(rules, None)
}

fn echo_redirect_engine(redirect_rules: Vec<BashRule>) -> BashRuleEngine {
    let mut rules = vec![allow_rule("allow-echo", r"^echo($|\s)")];
    rules.extend(redirect_rules);
    make_engine(rules)
}

fn local_echo_redirect_engine() -> BashRuleEngine {
    echo_redirect_engine(vec![redirect_rule("allow-local", ".*", true)])
}

fn review_redirect_engine() -> BashRuleEngine {
    make_engine(vec![
        modify_rule("redirecting-echo", r"^rewrite-echo$", "echo > report.txt"),
        modify_rule("safe-echo", r"^rewrite-safe$", "echo --safe"),
        allow_rule(
            "allow-context-change",
            r"^(cd|builtin|eval|let|trap|printf|export|read|HOME=)",
        ),
        allow_rule("allow-echo", r"^echo($|\s)"),
        redirect_rule("allow-local", ".*", true),
        redirect_rule("allow-dev-null", r"^/dev/null$", false),
        redirect_rule("allow-stdout", r"^&1$", false),
    ])
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

fn evaluate(
    engine: &BashRuleEngine,
    command: &str,
    cwd: &str,
    mode: Option<PermissionMode>,
) -> Evaluation {
    let context = EvaluationContext::new(cwd, mode);
    engine.evaluate_sync(command, &context, EvaluationPurpose::Decision)
}

fn evaluation_result(
    engine: &BashRuleEngine,
    command: &str,
    cwd: &str,
    mode: Option<PermissionMode>,
) -> RuleResult {
    evaluate(engine, command, cwd, mode).rule_result()
}

fn explain(
    engine: &BashRuleEngine,
    command: &str,
    cwd: &str,
    mode: Option<PermissionMode>,
) -> CommandTrace {
    let context = EvaluationContext::new(cwd, mode);
    let evaluation = engine.evaluate_sync(command, &context, EvaluationPurpose::Diagnostics);
    crate::test_runner::command_trace(command, &evaluation)
}

fn command_result(
    engine: &BashRuleEngine,
    command: &str,
    mode: Option<PermissionMode>,
) -> RuleResult {
    engine
        .match_command_decision(command, None, RedirectRewrite::NONE, mode)
        .outcome()
        .into()
}

fn leaf_command_result(
    engine: &BashRuleEngine,
    command: &str,
    cwd: &str,
    mode: Option<PermissionMode>,
) -> RuleResult {
    let SplitOutcome::Commands(leaves) = split_command(command, cwd, &engine.path_aliases) else {
        return command_result(engine, command, mode);
    };
    let [leaf] = leaves.as_slice() else {
        return RuleResult::NoMatch;
    };
    engine
        .match_command_decision(
            &leaf.match_text,
            leaf.filter_command.as_ref(),
            RedirectRewrite::from_leaf(leaf),
            mode,
        )
        .outcome()
        .into()
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
        redirect_rule("allow-dev-null", r"^/dev/null$", false),
    ])
}

fn assert_compound_cases<'a>(
    engine: &BashRuleEngine,
    cwd: &str,
    cases: impl IntoIterator<Item = (&'a str, RuleResult)>,
) {
    for (command, expected) in cases {
        assert_eq!(
            evaluation_result(engine, command, cwd, None),
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
mod rule_engine;
