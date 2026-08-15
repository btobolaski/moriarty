use super::*;

#[test]
fn test_expand_fragments_simple() {
    let mut fragments = HashMap::new();
    fragments.insert("safe".to_string(), "[^|&;$]".to_string());

    let pattern = "^ls{{safe}}*$";
    let expanded = expand_fragments(pattern, &fragments).unwrap();
    assert_eq!(expanded, "^ls[^|&;$]*$");
}

#[test]
fn test_expand_fragments_multiple() {
    let mut fragments = HashMap::new();
    fragments.insert("safe".to_string(), "[^|&;$]".to_string());
    fragments.insert("num".to_string(), "[0-9]+".to_string());

    let pattern = "^cmd{{safe}}*{{num}}$";
    let expanded = expand_fragments(pattern, &fragments).unwrap();
    assert_eq!(expanded, "^cmd[^|&;$]*[0-9]+$");
}

#[test]
fn test_expand_fragments_nested() {
    let mut fragments = HashMap::new();
    fragments.insert("safe".to_string(), "[^|&;$]".to_string());
    fragments.insert("arg".to_string(), "( {{safe}}+)".to_string());

    let pattern = "^ls{{arg}}*$";
    let expanded = expand_fragments(pattern, &fragments).unwrap();
    assert_eq!(expanded, "^ls( [^|&;$]+)*$");
}

#[test]
fn test_expand_fragments_deeply_nested() {
    let mut fragments = HashMap::new();
    fragments.insert("a".to_string(), "x".to_string());
    fragments.insert("b".to_string(), "{{a}}y".to_string());
    fragments.insert("c".to_string(), "{{b}}z".to_string());

    let pattern = "{{c}}";
    let expanded = expand_fragments(pattern, &fragments).unwrap();
    assert_eq!(expanded, "xyz");
}

/// A fragment reachable by more than one path is a DAG, not a cycle: `safe_arg` references
/// `safe_chars`, and the pattern references both. Cycle detection must not flag the second
/// occurrence just because the name was already expanded elsewhere.
#[test]
fn test_expand_fragments_shared_fragment_across_nesting_levels() {
    let mut fragments = HashMap::new();
    fragments.insert("safe_chars".to_string(), "[^|&;$]".to_string());
    fragments.insert("safe_arg".to_string(), "( {{safe_chars}}+)".to_string());

    let pattern = "^ls{{safe_arg}}*{{safe_chars}}$";
    let expanded = expand_fragments(pattern, &fragments).unwrap();
    assert_eq!(expanded, "^ls( [^|&;$]+)*[^|&;$]$");
}

#[test]
fn test_expand_fragments_undefined() {
    let fragments = HashMap::new();

    let pattern = "^ls{{undefined}}*$";
    let result = expand_fragments(pattern, &fragments);

    let error_msg = result
        .expect_err("Should fail on undefined fragment")
        .to_string();
    assert!(error_msg.contains("Undefined pattern fragment"));
    assert!(error_msg.contains("undefined"));
}

#[test]
fn test_expand_fragments_circular_dependency() {
    let mut fragments = HashMap::new();
    fragments.insert("a".to_string(), "{{b}}".to_string());
    fragments.insert("b".to_string(), "{{a}}".to_string());

    let pattern = "{{a}}";
    let result = expand_fragments(pattern, &fragments);

    let error_msg = result
        .expect_err("Should detect circular dependency")
        .to_string();

    // Should specifically detect circular dependency, not hit depth limit
    assert!(
        error_msg.contains("Circular dependency"),
        "Expected circular dependency error, got: {}",
        error_msg
    );
    assert!(
        !error_msg.contains("exceeded maximum depth"),
        "Should detect circular dependency before hitting depth limit"
    );
}

/// A direct self-reference trips the active-chain check one frame earlier than the a → b → a
/// case, so an off-by-one in when membership is tested relative to the push would pass one test
/// and fail the other.
#[test]
fn test_expand_fragments_self_reference() {
    let mut fragments = HashMap::new();
    fragments.insert("a".to_string(), "{{a}}".to_string());

    let result = expand_fragments("{{a}}", &fragments);

    let error_msg = result
        .expect_err("Should detect self-referencing cycle")
        .to_string();
    assert!(
        error_msg.contains("Circular dependency"),
        "Expected circular dependency error, got: {}",
        error_msg
    );
}

#[test]
fn test_expand_fragments_depth_limit() {
    let mut fragments = HashMap::new();
    // Create a chain: a -> b -> c -> d -> ... (11 levels deep)
    fragments.insert("a".to_string(), "{{b}}".to_string());
    fragments.insert("b".to_string(), "{{c}}".to_string());
    fragments.insert("c".to_string(), "{{d}}".to_string());
    fragments.insert("d".to_string(), "{{e}}".to_string());
    fragments.insert("e".to_string(), "{{f}}".to_string());
    fragments.insert("f".to_string(), "{{g}}".to_string());
    fragments.insert("g".to_string(), "{{h}}".to_string());
    fragments.insert("h".to_string(), "{{i}}".to_string());
    fragments.insert("i".to_string(), "{{j}}".to_string());
    fragments.insert("j".to_string(), "{{k}}".to_string());
    fragments.insert("k".to_string(), "x".to_string());

    let pattern = "{{a}}";
    let result = expand_fragments(pattern, &fragments);

    // Should fail due to depth limit (MAX_DEPTH = 10)
    let error_msg = result
        .expect_err("Should fail due to depth limit")
        .to_string();
    assert!(error_msg.contains("exceeded maximum depth"));
}

/// Breadth escapes the depth limit: each level doubles, so a graph deliberately built one level
/// inside `MAX_DEPTH` demands 2^10 - 1 = 1023 substitutions while never tripping the depth or
/// cycle check. Only the total-expansion cap stops it.
#[test]
fn test_expand_fragments_expansion_count_limit() {
    let deepest = FragmentExpander::MAX_DEPTH - 1;
    let mut fragments = HashMap::new();
    fragments.insert("f0".to_string(), "x".to_string());
    for level in 1..=deepest {
        fragments.insert(
            format!("f{}", level),
            format!("{{{{f{prev}}}}}{{{{f{prev}}}}}", prev = level - 1),
        );
    }

    let result = expand_fragments(&format!("{{{{f{deepest}}}}}"), &fragments);

    let error_msg = result
        .expect_err("Should fail once total substitutions exceed the cap")
        .to_string();
    assert!(
        error_msg.contains("exceeded maximum expansion count"),
        "Expected expansion-count error, got: {}",
        error_msg
    );
    assert!(
        !error_msg.contains("exceeded maximum depth"),
        "The graph is within the depth limit; breadth is what must be rejected"
    );
}

/// Pins the exact edge rather than a comfortable margin, so an off-by-one in the `>` comparison
/// cannot pass. Repeated references are ordinary usage, so the accepted side matters as much as
/// the rejected one.
#[test]
fn test_expand_fragments_expansion_count_boundary() {
    let mut fragments = HashMap::new();
    fragments.insert("safe".to_string(), "[^|&;$]".to_string());

    let at_limit = "{{safe}}".repeat(FragmentExpander::MAX_EXPANSIONS);
    let expanded = expand_fragments(&at_limit, &fragments).unwrap();
    assert_eq!(expanded, "[^|&;$]".repeat(FragmentExpander::MAX_EXPANSIONS));

    let over_limit = "{{safe}}".repeat(FragmentExpander::MAX_EXPANSIONS + 1);
    let error_msg = expand_fragments(&over_limit, &fragments)
        .expect_err("One substitution past the cap must fail")
        .to_string();
    assert!(
        error_msg.contains("exceeded maximum expansion count"),
        "Expected expansion-count error, got: {}",
        error_msg
    );
}

#[test]
fn test_expand_fragments_no_fragments() {
    let fragments = HashMap::new();

    let pattern = "^ls [^|&;$]*$";
    let expanded = expand_fragments(pattern, &fragments).unwrap();
    assert_eq!(expanded, "^ls [^|&;$]*$");
}

#[test]
fn test_expand_fragments_empty_pattern() {
    let fragments = HashMap::new();

    let pattern = "";
    let expanded = expand_fragments(pattern, &fragments).unwrap();
    assert_eq!(expanded, "");
}

#[test]
fn test_expand_fragments_with_regex_special_chars() {
    let mut fragments = HashMap::new();
    fragments.insert("paren".to_string(), "()".to_string());
    fragments.insert("bracket".to_string(), "[]".to_string());

    let pattern = "{{paren}}{{bracket}}";
    let expanded = expand_fragments(pattern, &fragments).unwrap();
    assert_eq!(expanded, "()[]");
}

#[test]
fn test_expand_fragments_no_collision_with_capture_groups() {
    let mut fragments = HashMap::new();
    fragments.insert("safe".to_string(), "[^|&;$]".to_string());

    // Pattern contains both fragments and regex capture groups
    let pattern = "^(cargo {{safe}}+) (build|check)$";
    let expanded = expand_fragments(pattern, &fragments).unwrap();
    assert_eq!(expanded, "^(cargo [^|&;$]+) (build|check)$");
}

#[test]
fn test_engine_with_fragments() {
    let mut fragments = HashMap::new();
    fragments.insert("safe".to_string(), "[^|&;$`]".to_string());

    let rules = vec![allow_rule("allow-ls", "^ls{{safe}}*$")];
    let engine = make_engine_with_fragments(rules, Some(fragments));

    let result = engine.apply_rules("ls -la", None);
    assert!(matches!(result, RuleResult::Allowed { .. }));

    let result = engine.apply_rules("ls | grep foo", None);
    assert!(matches!(result, RuleResult::NoMatch));
}

#[test]
fn test_default_fragments() {
    let defaults = default_fragments();

    // Verify key fragments exist
    assert!(defaults.contains_key("safe_chars"));
    assert!(defaults.contains_key("identifier"));
    assert!(defaults.contains_key("number"));
    assert!(defaults.contains_key("safe_arg"));
    assert!(defaults.contains_key("safe_pipe"));

    // Verify safe_chars blocks injection characters
    let safe_chars = &defaults["safe_chars"];
    assert!(safe_chars.contains('|'));
    assert!(safe_chars.contains('&'));
    assert!(safe_chars.contains('$'));
    assert!(safe_chars.contains('`'));
}

#[test]
fn test_default_fragments_no_circular_deps() {
    let defaults = default_fragments();

    // Try expanding each default fragment
    for name in defaults.keys() {
        let pattern = format!("{{{{{}}}}}", name);
        let result = expand_fragments(&pattern, &defaults);
        assert!(
            result.is_ok(),
            "Default fragment '{}' has circular dependency",
            name
        );
    }
}

#[test]
fn test_user_fragments_override_defaults() {
    let mut user_fragments = HashMap::new();
    user_fragments.insert("safe_chars".to_string(), "[a-z]".to_string());

    let rules = vec![allow_rule("test", "^{{safe_chars}}+$")];
    let engine = make_engine_with_fragments(rules, Some(user_fragments));

    let result = engine.apply_rules("abc", None);
    assert!(matches!(result, RuleResult::Allowed { .. }));

    let result = engine.apply_rules("ABC", None);
    assert!(matches!(result, RuleResult::NoMatch));
}

#[test]
fn test_fragment_expansion_error_logged_and_skipped() {
    let mut fragments = HashMap::new();
    fragments.insert("valid".to_string(), "[a-z]".to_string());

    let rules = vec![
        deny_rule("bad-fragment", "^{{undefined}}$", "test"),
        allow_rule("good-rule", "^{{valid}}+$"),
    ];

    let engine = make_engine_with_fragments(rules, Some(fragments));

    let result = engine.apply_rules("abc", None);
    assert!(matches!(result, RuleResult::Allowed { .. }));
}

#[test]
fn test_fragment_in_modify_action() {
    let mut fragments = HashMap::new();
    fragments.insert("safe".to_string(), "[^|&;$`]".to_string());

    let rules = vec![modify_rule(
        "modify-docker",
        "^(docker{{safe}}+)$",
        "$1 --dry-run",
    )];
    let engine = make_engine_with_fragments(rules, Some(fragments));
    let result = engine.apply_rules("docker build", None);

    match result {
        RuleResult::Modified { new_command, .. } => {
            assert_eq!(new_command, "docker build --dry-run");
        }
        _ => panic!("Expected Modified result"),
    }
}

#[test]
fn test_expand_fragments_same_fragment_multiple_times() {
    let mut fragments = HashMap::new();
    fragments.insert("x".to_string(), "abc".to_string());

    let pattern = "{{x}}-{{x}}";
    let expanded = expand_fragments(pattern, &fragments).unwrap();
    assert_eq!(expanded, "abc-abc");
}

#[test]
fn test_expand_fragments_adjacent_no_separator() {
    let mut fragments = HashMap::new();
    fragments.insert("a".to_string(), "foo".to_string());
    fragments.insert("b".to_string(), "bar".to_string());

    let pattern = "{{a}}{{b}}";
    let expanded = expand_fragments(pattern, &fragments).unwrap();
    assert_eq!(expanded, "foobar");
}

#[test]
fn test_expand_fragments_invalid_name_starting_with_digit() {
    let mut fragments = HashMap::new();
    fragments.insert("123".to_string(), "value".to_string());

    // Fragment names starting with digits don't match the pattern,
    // so they remain unexpanded
    let pattern = "{{123}}";
    let expanded = expand_fragments(pattern, &fragments).unwrap();
    assert_eq!(expanded, "{{123}}");
}

#[test]
fn test_expand_fragments_with_spaces_not_allowed() {
    let mut fragments = HashMap::new();
    fragments.insert("safe".to_string(), "[^|&;$]".to_string());

    // Spaces inside braces don't match the fragment pattern
    let pattern = "{{ safe }}";
    let expanded = expand_fragments(pattern, &fragments).unwrap();
    assert_eq!(expanded, "{{ safe }}");
}

#[test]
fn test_default_fragments_compile_to_valid_regex() {
    let defaults = default_fragments();

    for name in defaults.keys() {
        // Each default fragment should expand without error
        let test_pattern = format!("{{{{{}}}}}", name);
        let expanded = expand_fragments(&test_pattern, &defaults)
            .unwrap_or_else(|_| panic!("Fragment '{}' should expand without error", name));

        // And should compile to valid regex
        Regex::new(&expanded).unwrap_or_else(|_| {
            panic!(
                "Fragment '{}' should produce valid regex: {}",
                name, expanded
            )
        });
    }
}

// Tests for command parsing and argument filtering functions
