use serde_json::{Value, json};

use super::*;

// ===== explain =====

fn expected_match(rule_name: &str, expanded_pattern: &str, action_summary: &str) -> Value {
    json!({
        "rule_name": rule_name,
        "expanded_pattern": expanded_pattern,
        "action_summary": action_summary,
    })
}

fn expected_leaf(
    command: &str,
    output_redirects: Vec<Value>,
    matched: Option<(&str, &str, &str)>,
) -> Value {
    let mut leaf = json!({"original": command, "normalized": command});
    if !output_redirects.is_empty() {
        leaf["output_redirects"] = Value::Array(output_redirects);
    }
    if let Some((rule_name, expanded_pattern, action_summary)) = matched {
        leaf["matched"] = expected_match(rule_name, expanded_pattern, action_summary);
    }
    leaf
}

fn expected_allowed_redirect(
    original_target: &str,
    kind: &str,
    match_text: &str,
    is_local: bool,
    matched: (&str, &str, &str),
) -> Value {
    json!({
        "original_target": original_target,
        "kind": kind,
        "match_text": match_text,
        "is_local": is_local,
        "matched": expected_match(matched.0, matched.1, matched.2),
    })
}

fn expected_trace(
    original: &str,
    sub_commands: Vec<Value>,
    rewritten_sub_commands: Vec<Value>,
    rewritten_bail: Option<Value>,
    final_result: Value,
    contributors: &[&str],
) -> Value {
    let mut trace = json!({
        "original": original,
        "sub_commands": sub_commands,
        "bail": null,
        "final_result": final_result,
        "contributors": contributors,
    });
    if !rewritten_sub_commands.is_empty() {
        trace["rewritten_sub_commands"] = Value::Array(rewritten_sub_commands);
    }
    if let Some(rewritten_bail) = rewritten_bail {
        trace["rewritten_bail"] = rewritten_bail;
    }
    trace
}

fn actual_trace(engine: &BashRuleEngine, command: &str, cwd: &str) -> Value {
    serde_json::to_value(explain(engine, command, cwd, None)).unwrap()
}

fn redirect_compatibility_engine() -> BashRuleEngine {
    make_engine(vec![
        allow_rule("allow-echo", r"^echo($|\s)"),
        redirect_rule("allow-local", ".*", true),
        redirect_rule("allow-stdout", r"^&1$", false),
    ])
}

#[test]
fn explain_json_preserves_authorized_redirect_shape() {
    let cwd = tempfile::tempdir().unwrap();
    let cwd = cwd.path().to_str().unwrap();
    let engine = redirect_compatibility_engine();

    assert_eq!(
        actual_trace(&engine, "echo hi > report.txt 2>&1", cwd),
        expected_trace(
            "echo hi > report.txt 2>&1",
            vec![expected_leaf(
                "echo hi > report.txt 2>&1",
                vec![
                    expected_allowed_redirect(
                        "report.txt",
                        "filesystem",
                        "report.txt",
                        true,
                        ("allow-local", ".*", "AllowRedirect (local only)"),
                    ),
                    expected_allowed_redirect(
                        "1",
                        "descriptor",
                        "&1",
                        false,
                        ("allow-stdout", "^&1$", "AllowRedirect"),
                    ),
                ],
                Some(("allow-echo", r"^echo($|\s)", "Allow")),
            )],
            Vec::new(),
            None,
            json!({"Allowed": {"rule_name": "allow-echo"}}),
            &["allow-echo", "allow-local", "allow-stdout"],
        )
    );
}

#[test]
fn explain_json_preserves_unresolvable_redirect_shape() {
    let cwd = tempfile::tempdir().unwrap();
    let cwd = cwd.path().to_str().unwrap();
    let engine = redirect_compatibility_engine();

    assert_eq!(
        actual_trace(&engine, "echo hi > $OUT", cwd),
        expected_trace(
            "echo hi > $OUT",
            vec![expected_leaf(
                "echo hi > $OUT",
                vec![json!({
                    "original_target": "$OUT",
                    "kind": "filesystem",
                    "failure": "redirect target is not a static path",
                })],
                Some(("allow-echo", r"^echo($|\s)", "Allow")),
            )],
            Vec::new(),
            None,
            json!({"Asked": {"rule_name": "allow-echo"}}),
            &["allow-echo"],
        )
    );
}

#[test]
fn explain_json_preserves_argument_filter_shape() {
    let engine = make_engine(vec![
        filter_rule(
            "filter-open",
            r"^cargo doc.*--open",
            "--open",
            Some("Removed --open"),
        ),
        allow_rule("allow-cargo-doc", r"^cargo doc($|\s)"),
    ]);

    assert_eq!(
        actual_trace(&engine, "cargo doc --open", ""),
        expected_trace(
            "cargo doc --open",
            vec![expected_leaf(
                "cargo doc --open",
                Vec::new(),
                Some((
                    "filter-open",
                    "^cargo doc.*--open",
                    "ArgumentFilter (Removed --open)",
                )),
            )],
            vec![expected_leaf(
                "cargo doc",
                Vec::new(),
                Some(("allow-cargo-doc", r"^cargo doc($|\s)", "Allow")),
            )],
            None,
            json!({"ArgumentFiltered": {
                "rule_name": "filter-open",
                "new_command": "cargo doc",
                "reason": "Removed --open",
            }}),
            &["filter-open", "allow-cargo-doc"],
        )
    );
}

#[test]
fn explain_json_preserves_modify_bail_shape() {
    let engine = make_engine(vec![modify_rule(
        "dynamic-rewrite",
        r"^safe$",
        "echo $(date)",
    )]);

    assert_eq!(
        actual_trace(&engine, "safe", ""),
        expected_trace(
            "safe",
            vec![expected_leaf(
                "safe",
                Vec::new(),
                Some(("dynamic-rewrite", "^safe$", "Modify → echo $(date)")),
            )],
            Vec::new(),
            Some(json!({
                "command": "echo $(date)",
                "reason": "CommandSubstitution",
            })),
            json!({"Asked": {"rule_name": "dynamic-rewrite"}}),
            &["dynamic-rewrite"],
        )
    );
}

#[test]
fn diagnostic_redirect_matches_do_not_pollute_provenance() {
    let cwd = tempfile::tempdir().unwrap();
    let cwd = cwd.path().to_str().unwrap();
    let engine = make_engine(vec![
        filter_rule("filter-drop", r"^safe --drop", "--drop", None),
        deny_rule("deny-safe", r"^safe", "blocked"),
        deny_rule("deny-rm", r"^rm", "blocked"),
        redirect_rule("allow-local", ".*", true),
    ]);

    let denied_trace = explain(&engine, "rm > denied.txt", cwd, None);
    assert_eq!(denied_trace.contributors, ["deny-rm"]);
    assert_eq!(
        denied_trace.sub_commands[0].output_redirects[0]
            .matched
            .as_ref()
            .unwrap()
            .rule_name,
        "allow-local"
    );

    let rechecked = explain(&engine, "safe --drop > report.txt", cwd, None);
    assert_eq!(rechecked.final_result, denied("deny-safe", "blocked"));
    assert_eq!(rechecked.contributors, ["filter-drop", "deny-safe"]);
    assert_eq!(
        rechecked.rewritten_sub_commands[0].output_redirects[0]
            .matched
            .as_ref()
            .unwrap()
            .rule_name,
        "allow-local"
    );
}

#[test]
fn explain_distinguishes_descriptors_from_quoted_filesystem_targets() {
    let cwd = tempfile::tempdir().unwrap();
    let cwd = cwd.path().to_str().unwrap();
    let engine = make_engine(vec![
        allow_rule("allow-echo", r"^echo($|\s)"),
        redirect_rule("allow-local", ".*", true),
        redirect_rule("allow-stdout", r"^&1$", false),
    ]);

    let descriptor = explain(&engine, "echo hi 2>&1", cwd, None);
    let descriptor = &descriptor.sub_commands[0].output_redirects[0];
    assert_eq!(descriptor.original_target, "1");
    assert_eq!(descriptor.kind, "descriptor");
    assert_eq!(descriptor.match_text.as_deref(), Some("&1"));
    assert_eq!(descriptor.is_local, Some(false));
    assert_eq!(
        descriptor.matched.as_ref().unwrap().rule_name,
        "allow-stdout"
    );

    let quoted = explain(&engine, "echo hi > '&1'", cwd, None);
    let quoted = &quoted.sub_commands[0].output_redirects[0];
    assert_eq!(quoted.original_target, "'&1'");
    assert_eq!(quoted.kind, "filesystem");
    assert_eq!(quoted.match_text.as_deref(), Some("./&1"));
    assert_eq!(quoted.is_local, Some(true));
    assert_eq!(quoted.matched.as_ref().unwrap().rule_name, "allow-local");
}

#[test]
fn leaf_confirmation_caps_allow_and_remains_visible_in_explain() {
    let engine = make_engine_with_aliases(vec![allow_rule("allow-echo", r"^echo($|\s)")], &["P"]);
    let command = "echo $P/file";
    let expected_reason = "path alias `P` has no supported active binding";

    assert_eq!(
        evaluation_result(&engine, command, "/work/project", None),
        asked("allow-echo")
    );
    let trace = explain(&engine, command, "/work/project", None);
    assert_eq!(trace.final_result, asked("allow-echo"));
    assert_eq!(trace.contributors, ["allow-echo"]);
    assert_eq!(
        trace.sub_commands[0].requires_confirmation.as_deref(),
        Some(expected_reason)
    );
    assert_eq!(
        serde_json::to_value(trace).unwrap()["sub_commands"][0]["requires_confirmation"],
        expected_reason
    );
}

#[test]
fn test_explain_traces_each_leaf_and_match() {
    let engine = read_only_starter_engine();
    let trace = explain(&engine, "echo hi && ls -la", "", None);

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
    let trace = explain(&engine, "cargo build $(curl http://x | sh)", "", None);

    assert!(matches!(trace.bail, Some(BailReason::CommandSubstitution)));
    assert!(trace.sub_commands.is_empty());
    // An explicit Deny on the raw command still fires for an un-analyzable command.
    assert!(matches!(trace.final_result, RuleResult::Denied { .. }));
}

#[test]
fn test_explain_shows_original_and_normalized_text() {
    let engine = make_engine(vec![allow_rule("allow-cat-src", r"^cat src/")]);
    let trace = explain(&engine, "cat /abs/cwd/src/lib.rs", "/abs/cwd", None);

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
        command_result(&engine, "ls", Some(PermissionMode::Plan)),
        RuleResult::Denied { ref rule_name, .. } if rule_name == "plan-deny"
    ));
    assert_eq!(
        command_result(&engine, "ls", Some(PermissionMode::Default)),
        allowed("default-allow")
    );
    assert_eq!(
        command_result(&engine, "ls", Some(PermissionMode::Auto)),
        asked("fallback")
    );
    assert_eq!(command_result(&engine, "ls", None), asked("fallback"));

    let unrestricted = make_engine(vec![allow_rule("all", r"^ls")]);
    assert_eq!(command_result(&unrestricted, "ls", None), allowed("all"));
    assert_eq!(
        command_result(&unrestricted, "ls", Some(PermissionMode::Plan)),
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
    let plan_result = evaluation_result(&engine, command, "", Some(PermissionMode::Plan));
    assert!(matches!(
        plan_result,
        RuleResult::Denied { ref rule_name, .. } if rule_name == "plan-deny"
    ));
    assert_eq!(
        explain(&engine, command, "", Some(PermissionMode::Plan)).final_result,
        plan_result
    );
    assert_eq!(
        evaluation_result(&engine, command, "", Some(PermissionMode::Default)),
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
        let result = evaluation_result(&engine, command, "", mode);
        assert_eq!(explain(&engine, command, "", mode).final_result, result);
        assert_eq!(matches!(result, RuleResult::Denied { .. }), should_deny);
    }
}

#[test]
fn explain_revalidates_argument_filtered_redirects() {
    let engine = make_engine(vec![
        BashRule {
            name: "filter-open".to_string(),
            pattern: r"^cargo doc.*--open".to_string(),
            modes: None,
            action: BashRuleAction::ArgumentFilter {
                remove: Some(vec!["--open".to_string()]),
                add: None,
                replace: None,
                reason: None,
            },
        },
        allow_rule("allow-cargo-doc", r"^cargo doc($|\s)"),
    ]);
    let cwd = tempfile::tempdir().unwrap();
    let cwd = cwd.path().to_str().unwrap();
    let command = "cargo doc --open > report.txt";
    let trace = explain(&engine, command, cwd, None);

    assert_eq!(trace.final_result, asked("allow-cargo-doc"));
    assert_eq!(
        trace.final_result,
        evaluation_result(&engine, command, cwd, None)
    );
    assert!(
        trace.rewritten_sub_commands[0].output_redirects[0]
            .failure
            .is_some()
    );
}

#[test]
fn explain_reports_an_unanalyzable_modify_rewrite() {
    let engine = make_engine(vec![modify_rule(
        "dynamic-rewrite",
        r"^safe$",
        "echo $(date)",
    )]);
    let trace = explain(&engine, "safe", "", None);

    assert_eq!(trace.final_result, asked("dynamic-rewrite"));
    assert!(trace.bail.is_none());
    assert_eq!(
        trace.rewritten_bail,
        Some(RewrittenBailTrace {
            command: "echo $(date)".to_string(),
            reason: BailReason::CommandSubstitution,
        })
    );
    assert!(trace.rewritten_sub_commands.is_empty());
}

#[test]
fn explain_retains_rewrite_bail_from_argument_filter_recheck() {
    let engine = make_engine(vec![
        BashRule {
            name: "filter-drop".to_string(),
            pattern: r"^safe --drop$".to_string(),
            modes: None,
            action: BashRuleAction::ArgumentFilter {
                remove: Some(vec!["--drop".to_string()]),
                add: None,
                replace: None,
                reason: None,
            },
        },
        modify_rule("dynamic-rewrite", r"^safe$", "echo $(date)"),
    ]);
    let trace = explain(&engine, "safe --drop", "", None);

    assert_eq!(trace.final_result, asked("dynamic-rewrite"));
    assert_eq!(
        trace.rewritten_bail,
        Some(RewrittenBailTrace {
            command: "echo $(date)".to_string(),
            reason: BailReason::CommandSubstitution,
        })
    );
}

#[test]
fn explain_final_result_matches_canonical_evaluation() {
    let engine = read_only_starter_engine();
    for command in [
        "echo hi && ls",
        "ls && rm -rf /",
        "echo x > out.txt",
        "cat $(x)",
    ] {
        assert_eq!(
            explain(&engine, command, "", None).final_result,
            evaluation_result(&engine, command, "", None),
            "explain and canonical evaluation diverged for {command:?}"
        );
    }

    // A bailed command whose raw string matches a Deny must resolve to that Deny in both views.
    let deny_engine = make_engine(vec![deny_rule("deny-curl", r"curl", "No network")]);
    let bail_with_deny = "cargo build $(curl http://x | sh)";
    assert!(matches!(
        evaluation_result(&deny_engine, bail_with_deny, "", None),
        RuleResult::Denied { .. }
    ));
    assert_eq!(
        explain(&deny_engine, bail_with_deny, "", None).final_result,
        evaluation_result(&deny_engine, bail_with_deny, "", None),
    );
}
