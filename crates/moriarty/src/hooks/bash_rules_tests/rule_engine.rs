use super::*;

#[test]
fn test_empty_rules() {
    let engine = make_engine(vec![]);
    let result = engine.apply_rules("ls -la", None);
    assert_eq!(result, RuleResult::NoMatch);
}

#[test]
fn test_deny_rule() {
    let engine = make_engine(vec![deny_rule(
        "deny-rm-rf",
        r"^rm\s+-rf\s+/",
        "Dangerous recursive delete",
    )]);
    let result = engine.apply_rules("rm -rf /", None);

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
    let result = engine.apply_rules("ls -la", None);
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
    let result = engine.apply_rules("docker build", None);
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
    let result = engine.apply_rules("docker system prune", None);

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
    let result = engine.apply_rules("echo hello world", None);

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

    let result = engine.apply_rules("ls -la", None);
    assert_eq!(
        result,
        RuleResult::Allowed {
            rule_name: "allow-ls".to_string()
        }
    );

    let result = engine.apply_rules("rm file.txt", None);
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

    let result = engine.apply_rules("docker system prune", None);
    assert_eq!(
        result,
        RuleResult::Asked {
            rule_name: "ask-specific-docker".to_string()
        }
    );

    let result = engine.apply_rules("docker build", None);
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
    let result = engine.apply_rules("docker system prune", None);
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
    let result = engine.apply_rules("docker system prune", None);
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
    let result = engine.apply_rules("docker system prune", None);
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
    let result = engine.apply_rules("docker system prune", None);
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
    let result = engine.apply_rules("ls -la", None);
    assert_eq!(result, RuleResult::NoMatch);
}

#[test]
fn test_invalid_regex() {
    let rules = vec![
        deny_rule("bad-regex", r"[invalid(", "test"),
        allow_rule("good-rule", r"^ls"),
    ];

    let engine = make_engine(rules);

    let result = engine.apply_rules("ls -la", None);
    assert_eq!(
        result,
        RuleResult::Allowed {
            rule_name: "good-rule".to_string()
        }
    );

    let result = engine.apply_rules("rm file.txt", None);
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
fn test_apply_rules_empty_command() {
    let engine = make_engine(vec![deny_rule("deny-all", r".*", "denied")]);
    let result = engine.apply_rules("", None);

    match result {
        RuleResult::Denied { .. } => {}
        _ => panic!("Expected empty command to match '.*' pattern"),
    }
}

#[test]
fn test_apply_rules_whitespace_only() {
    let engine = make_engine(vec![deny_rule(
        "deny-whitespace",
        r"^\s+$",
        "whitespace only",
    )]);
    let result = engine.apply_rules("   \t\n", None);

    match result {
        RuleResult::Denied { reason, .. } => {
            assert_eq!(reason, "whitespace only");
        }
        _ => panic!("Expected whitespace command to be denied"),
    }
}

#[test]
fn test_apply_rules_no_match_on_whitespace() {
    let engine = make_engine(vec![allow_rule("match-non-whitespace", r"^\S+$")]);
    let result = engine.apply_rules("   ", None);
    assert_eq!(result, RuleResult::NoMatch);
}

#[test]
fn test_regexset_individual_regex_invariant() {
    let engine = make_engine(vec![modify_rule(
        "capture-test",
        r"^(docker\s+\w+)",
        "$1 --flag",
    )]);
    let result = engine.apply_rules("docker build", None);

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

    let result = engine.apply_rules("rm -rf /", None);

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

    let result = engine.apply_rules("target-command", None);
    match result {
        RuleResult::Denied { rule_name, .. } => {
            assert_eq!(rule_name, "final-match");
        }
        _ => panic!("Expected to find the matching rule"),
    }
}

// Fragment expansion tests
