use super::*;

#[test]
fn test_empty_rules() {
    let engine = make_engine(vec![]);
    let result = command_result(&engine, "ls -la", None);
    assert_eq!(result, RuleResult::NoMatch);
}

#[test]
fn command_and_redirect_first_match_domains_are_independent() {
    let cwd = tempfile::tempdir().unwrap();
    let mut disabled = redirect_rule("disabled", r"^out$", false);
    disabled.modes = Some(BTreeSet::new());
    let mut plan = redirect_rule("plan-target", r"^out$", false);
    plan.modes = Some(BTreeSet::from([PermissionMode::Plan]));
    let engine = make_engine(vec![
        disabled,
        plan,
        redirect_rule("fallback-target", r"^out$", false),
        allow_rule("allow-echo", r"^echo($|\s)"),
    ]);

    assert_eq!(
        command_result(&engine, "echo hi", None),
        allowed("allow-echo")
    );
    assert_eq!(
        command_result(&engine, "unknown", None),
        RuleResult::NoMatch
    );
    for (mode, target) in [
        (PermissionMode::Plan, "plan-target"),
        (PermissionMode::Default, "fallback-target"),
    ] {
        let evaluation = evaluate(
            &engine,
            "echo hi > out",
            cwd.path().to_str().unwrap(),
            Some(mode),
        );
        assert_eq!(evaluation.rule_result(), allowed("allow-echo"));
        assert_eq!(evaluation.contributors(), ["allow-echo", target]);
    }
}

#[test]
fn matched_rule_metadata_is_shared_across_evaluations() {
    let engine = make_engine(vec![
        allow_rule("allow-ls", r"^ls($|\s)"),
        redirect_rule("allow-out", r"^out$", false),
    ]);
    let command_matches = [
        engine.match_command_decision("ls", None, None),
        engine.match_command_decision("ls -la", None, None),
    ];
    let [Some(first), Some(second)] = command_matches.map(|decision| {
        decision
            .matched_rule()
            .map(|rule| Arc::clone(&rule.metadata))
    }) else {
        panic!("expected command matches");
    };
    assert!(Arc::ptr_eq(&first, &second));

    let redirect_matches = [
        engine.match_redirect_rule("out", None, RedirectEndpointKind::Filesystem, false),
        engine.match_redirect_rule("out", None, RedirectEndpointKind::Filesystem, false),
    ];
    let [Some(first), Some(second)] =
        redirect_matches.map(|matched| matched.map(|rule| rule.metadata))
    else {
        panic!("expected redirect matches");
    };
    assert!(Arc::ptr_eq(&first, &second));
}

#[test]
fn test_deny_rule() {
    let engine = make_engine(vec![deny_rule(
        "deny-rm-rf",
        r"^rm\s+-rf\s+/",
        "Dangerous recursive delete",
    )]);
    let result = command_result(&engine, "rm -rf /", None);

    match result {
        RuleResult::Denied { rule_name, reason } => {
            assert_eq!(rule_name, "deny-rm-rf");
            assert_eq!(reason, "Dangerous recursive delete");
        }
        _ => panic!("Expected Denied result"),
    }
}

#[test]
fn test_allow_rule() {
    let engine = make_engine(vec![allow_rule("allow-ls", r"^ls($|\s)")]);
    let result = command_result(&engine, "ls -la", None);
    assert_eq!(
        result,
        RuleResult::Allowed {
            rule_name: "allow-ls".to_string()
        }
    );
}

#[test]
fn test_ask_rule() {
    let engine = make_engine(vec![ask_rule("ask-docker", r"^docker")]);
    let result = command_result(&engine, "docker build", None);
    assert_eq!(
        result,
        RuleResult::Asked {
            rule_name: "ask-docker".to_string()
        }
    );
}

#[test]
fn test_modify_rule_simple() {
    let engine = make_engine(vec![modify_rule(
        "add-dry-run",
        r"^(docker\s+system\s+prune)$",
        "$1 --dry-run",
    )]);
    let result = command_result(&engine, "docker system prune", None);

    match result {
        RuleResult::Modified {
            rule_name,
            new_command,
        } => {
            assert_eq!(rule_name, "add-dry-run");
            assert_eq!(new_command, "docker system prune --dry-run");
        }
        _ => panic!("Expected Modified result"),
    }
}

#[test]
fn test_modify_rule_multiple_groups() {
    let engine = make_engine(vec![modify_rule(
        "swap-args",
        r"^echo\s+(\w+)\s+(\w+)$",
        "echo $2 $1",
    )]);
    let result = command_result(&engine, "echo hello world", None);

    match result {
        RuleResult::Modified {
            rule_name,
            new_command,
        } => {
            assert_eq!(rule_name, "swap-args");
            assert_eq!(new_command, "echo world hello");
        }
        _ => panic!("Expected Modified result"),
    }
}

#[test]
fn test_first_match_wins() {
    let engine = make_engine(vec![
        allow_rule("allow-ls", r"^ls"),
        deny_rule("deny-all", r".*", "All commands denied"),
    ]);

    let result = command_result(&engine, "ls -la", None);
    assert_eq!(
        result,
        RuleResult::Allowed {
            rule_name: "allow-ls".to_string()
        }
    );

    let result = command_result(&engine, "rm file.txt", None);
    match result {
        RuleResult::Denied { rule_name, .. } => {
            assert_eq!(rule_name, "deny-all");
        }
        _ => panic!("Expected Denied result"),
    }
}

#[test]
fn test_ask_overrides_allow_with_ordering() {
    let engine = make_engine(vec![
        ask_rule("ask-specific-docker", r"^docker\s+system\s+prune"),
        allow_rule("allow-all-docker", r"^docker"),
    ]);

    let result = command_result(&engine, "docker system prune", None);
    assert_eq!(
        result,
        RuleResult::Asked {
            rule_name: "ask-specific-docker".to_string()
        }
    );

    let result = command_result(&engine, "docker build", None);
    assert_eq!(
        result,
        RuleResult::Allowed {
            rule_name: "allow-all-docker".to_string()
        }
    );
}

#[test]
fn test_ask_vs_deny_ordering() {
    // Test 1: Ask before Deny - Ask wins
    let engine = make_engine(vec![
        ask_rule("ask-specific", r"^docker\s+system\s+prune"),
        deny_rule("deny-all-docker", r"^docker", "Docker denied"),
    ]);
    let result = command_result(&engine, "docker system prune", None);
    assert_eq!(
        result,
        RuleResult::Asked {
            rule_name: "ask-specific".to_string()
        }
    );

    // Test 2: Deny before Ask - Deny wins
    let engine = make_engine(vec![
        deny_rule("deny-all-docker", r"^docker", "Docker denied"),
        ask_rule("ask-specific", r"^docker\s+system\s+prune"),
    ]);
    let result = command_result(&engine, "docker system prune", None);
    match result {
        RuleResult::Denied { rule_name, reason } => {
            assert_eq!(rule_name, "deny-all-docker");
            assert_eq!(reason, "Docker denied");
        }
        _ => panic!("Expected Denied result"),
    }
}

#[test]
fn test_ask_vs_modify_ordering() {
    // Test 1: Ask before Modify - Ask wins
    let engine = make_engine(vec![
        ask_rule("ask-specific", r"^docker\s+system\s+prune"),
        modify_rule("modify-all-docker", r"^(docker\s+.*)", "$1 --dry-run"),
    ]);
    let result = command_result(&engine, "docker system prune", None);
    assert_eq!(
        result,
        RuleResult::Asked {
            rule_name: "ask-specific".to_string()
        }
    );

    // Test 2: Modify before Ask - Modify wins
    let engine = make_engine(vec![
        modify_rule("modify-all-docker", r"^(docker\s+.*)", "$1 --dry-run"),
        ask_rule("ask-specific", r"^docker\s+system\s+prune"),
    ]);
    let result = command_result(&engine, "docker system prune", None);
    match result {
        RuleResult::Modified {
            rule_name,
            new_command,
        } => {
            assert_eq!(rule_name, "modify-all-docker");
            assert_eq!(new_command, "docker system prune --dry-run");
        }
        _ => panic!("Expected Modified result"),
    }
}

#[test]
fn test_no_match() {
    let engine = make_engine(vec![deny_rule("deny-rm", r"^rm\s", "rm denied")]);
    let result = command_result(&engine, "ls -la", None);
    assert_eq!(result, RuleResult::NoMatch);
}

#[test]
fn test_invalid_regex() {
    let rules = vec![
        deny_rule("bad-regex", r"[invalid(", "test"),
        allow_rule("good-rule", r"^ls"),
    ];

    let engine = make_engine(rules);

    let result = command_result(&engine, "ls -la", None);
    assert_eq!(
        result,
        RuleResult::Allowed {
            rule_name: "good-rule".to_string()
        }
    );

    let result = command_result(&engine, "rm file.txt", None);
    assert_eq!(result, RuleResult::NoMatch);
}

#[test]
fn test_expand_captures_full_match() {
    let re = Regex::new(r"^(echo)\s+(\w+)$").unwrap();
    let caps = re.captures("echo hello").unwrap();
    let result = expand_captures(&caps, "$0");
    assert_eq!(result, "echo hello");
}

#[test]
fn test_expand_captures_groups() {
    let re = Regex::new(r"^(\w+)\s+(\w+)$").unwrap();
    let caps = re.captures("hello world").unwrap();
    let result = expand_captures(&caps, "$2 $1");
    assert_eq!(result, "world hello");
}

#[test]
fn test_expand_captures_no_placeholder() {
    let re = Regex::new(r"^test$").unwrap();
    let caps = re.captures("test").unwrap();
    let result = expand_captures(&caps, "replacement");
    assert_eq!(result, "replacement");
}

#[test]
fn test_expand_captures_double_digit_groups() {
    let re =
        Regex::new(r"^(\w+) (\w+) (\w+) (\w+) (\w+) (\w+) (\w+) (\w+) (\w+) (\w+) (\w+)$").unwrap();
    let caps = re.captures("a1 a2 a3 a4 a5 a6 a7 a8 a9 a10 a11").unwrap();
    let result = expand_captures(&caps, "$10 then $1");
    assert_eq!(result, "a10 then a1");
}

#[test]
fn test_expand_captures_adjacent_groups() {
    let re = Regex::new(r"^(\w+) (\w+)$").unwrap();
    let caps = re.captures("hello world").unwrap();
    let result = expand_captures(&caps, "$1$2");
    assert_eq!(result, "helloworld");
}

#[test]
fn test_expand_captures_nonexistent_group() {
    let re = Regex::new(r"^(\w+) (\w+)$").unwrap();
    let caps = re.captures("hello world").unwrap();
    let result = expand_captures(&caps, "$1 $999");
    assert_eq!(result, "hello $999");
}

#[test]
fn test_match_command_decision_empty_command() {
    let engine = make_engine(vec![deny_rule("deny-all", r".*", "denied")]);
    let result = command_result(&engine, "", None);

    match result {
        RuleResult::Denied { .. } => {}
        _ => panic!("Expected empty command to match '.*' pattern"),
    }
}

#[test]
fn test_match_command_decision_whitespace_only() {
    let engine = make_engine(vec![deny_rule(
        "deny-whitespace",
        r"^\s+$",
        "whitespace only",
    )]);
    let result = command_result(&engine, "   \t\n", None);

    match result {
        RuleResult::Denied { reason, .. } => {
            assert_eq!(reason, "whitespace only");
        }
        _ => panic!("Expected whitespace command to be denied"),
    }
}

#[test]
fn test_match_command_decision_no_match_on_whitespace() {
    let engine = make_engine(vec![allow_rule("match-non-whitespace", r"^\S+$")]);
    let result = command_result(&engine, "   ", None);
    assert_eq!(result, RuleResult::NoMatch);
}

#[test]
fn test_regexset_individual_regex_invariant() {
    let engine = make_engine(vec![modify_rule(
        "capture-test",
        r"^(docker\s+\w+)",
        "$1 --flag",
    )]);
    let result = command_result(&engine, "docker build", None);

    match result {
        RuleResult::Modified { new_command, .. } => {
            assert_eq!(new_command, "docker build --flag");
        }
        _ => panic!("Expected Modified result"),
    }
}

#[test]
fn test_multiple_patterns_match_first_wins() {
    let engine = make_engine(vec![
        deny_rule("specific-deny", r"^rm\s+-rf", "Dangerous rm -rf"),
        allow_rule("generic-allow-rm", r"^rm"),
    ]);

    let result = command_result(&engine, "rm -rf /", None);

    match result {
        RuleResult::Denied { rule_name, reason } => {
            assert_eq!(rule_name, "specific-deny");
            assert_eq!(reason, "Dangerous rm -rf");
        }
        _ => panic!("Expected first rule (deny) to win, got: {:?}", result),
    }
}

#[test]
fn test_large_rule_set_still_matches_correctly() {
    let mut rules: Vec<BashRule> = (0..100)
        .map(|i| allow_rule(&format!("rule-{}", i), &format!(r"^command-{}($|\s)", i)))
        .collect();
    rules.push(deny_rule("final-match", r"^target-command", "Found it"));

    let engine = make_engine(rules);

    let result = command_result(&engine, "target-command", None);
    match result {
        RuleResult::Denied { rule_name, .. } => {
            assert_eq!(rule_name, "final-match");
        }
        _ => panic!("Expected to find the matching rule"),
    }
}

#[test]
fn policy_merge_precedence_and_ties_are_derived_directly() {
    let matched = |name: &str| MatchedCommandRule {
        metadata: Arc::new(MatchedRuleMetadata {
            rule_name: name.to_string(),
            expanded_pattern: String::new(),
            action_summary: String::new(),
        }),
    };
    let leaf = |command: CommandDecision| {
        let endpoints = if matches!(command, CommandDecision::Allow { .. }) {
            EndpointCoverage::Analyzed(Vec::new())
        } else {
            EndpointCoverage::Skipped
        };
        PolicyLeafAnalysis {
            identity: LeafIdentity {
                original: String::new(),
                alias_expanded: None,
                normalized: String::new(),
                bindings: Vec::new(),
                requires_confirmation: None,
                command_shape: None,
            },
            command,
            endpoints,
        }
    };
    let ask = |name| {
        leaf(CommandDecision::Ask {
            rule: matched(name),
        })
    };
    let deny = |name| {
        leaf(CommandDecision::Deny {
            rule: matched(name),
            reason: "blocked".to_string(),
        })
    };
    let allow = |name| {
        leaf(CommandDecision::Allow {
            rule: matched(name),
        })
    };
    let modify = |name| {
        leaf(CommandDecision::Modify {
            rule: matched(name),
            new_command: "rewritten".to_string(),
        })
    };
    let cases = [
        (
            "ask then deny",
            vec![ask("ask"), deny("deny")],
            denied("deny", "blocked"),
        ),
        (
            "deny then ask",
            vec![deny("deny"), ask("ask")],
            denied("deny", "blocked"),
        ),
        (
            "modify outranks allow but cannot be stitched",
            vec![modify("modify"), allow("allow")],
            RuleResult::NoMatch,
        ),
        (
            "first ask wins a tie",
            vec![ask("ask-first"), ask("ask-second")],
            asked("ask-first"),
        ),
    ];

    for (label, leaves, expected) in cases {
        let actual: RuleResult = PolicyAnalysis::Leaves(leaves).outcome().into();
        assert_eq!(actual, expected, "case {label}");
    }
}

// Fragment expansion tests
