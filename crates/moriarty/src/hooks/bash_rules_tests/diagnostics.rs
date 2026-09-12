use super::*;

// ===== compile_with_diagnostics =====

#[test]
fn test_compile_with_diagnostics_reports_dropped_rules_and_keeps_good_ones() {
    let rules = vec![
        deny_rule("undefined-frag", "^{{nope}}$", "x"),
        deny_rule("bad-regex", "[invalid(", "y"),
        allow_rule("good", r"^ls($|\s)"),
    ];
    let (engine, diagnostics) = BashRuleEngine::compile_with_diagnostics(rules, None).unwrap();

    // The valid rule still compiled and is enforced.
    assert!(matches!(
        command_result(&engine, "ls -la", None),
        RuleResult::Allowed { .. }
    ));

    assert_eq!(diagnostics.len(), 2, "diagnostics: {diagnostics:?}");
    let kind_of = |name: &str| {
        diagnostics
            .iter()
            .find(|diagnostic| diagnostic.rule_name == name)
            .unwrap_or_else(|| panic!("no diagnostic for {name}"))
            .kind
    };
    // Each diagnostic is attributed to the right rule, not merely present somewhere.
    assert_eq!(
        kind_of("undefined-frag"),
        RuleDiagnosticKind::UndefinedFragment
    );
    assert_eq!(kind_of("bad-regex"), RuleDiagnosticKind::InvalidRegex);
}

#[test]
fn test_rule_pattern_validation_matches_regex_defaults() {
    let excessive_nesting = format!("{}a{}", "(".repeat(251), ")".repeat(251));
    for pattern in [
        "",
        "a|ab",
        r"(?im)^café$",
        r"\b\w+\b",
        r"[\p{Greek}&&\p{L}]+",
        r"(?-u:[a-z]+)",
        r"(?-u:\xFF)",
        r"(?-u:.)",
        r"(?P<name>a)(?P<name>b)",
        r"(?=a)",
        "[invalid(",
        r"a{100000}",
        r"a{1000000}",
        r"\w{32}",
        r"\w{128}",
        &excessive_nesting,
    ] {
        let expected = Regex::new(pattern)
            .map(drop)
            .map_err(|error| error.to_string());
        let actual = validate_rule_pattern(pattern).map_err(|error| error.to_string());
        assert_eq!(actual, expected, "validation differs for {pattern:?}");
    }
}

#[test]
fn test_oversized_patterns_are_dropped_in_both_rule_domains() {
    let pattern = r"a{1000000}";
    let error = Regex::new(pattern).expect_err("pattern must exceed the default size limit");
    assert!(matches!(
        &error,
        RegexError::CompiledTooBig(limit) if *limit == REGEX_NFA_SIZE_LIMIT
    ));
    let rules = vec![
        allow_rule("oversized-command", pattern),
        redirect_rule("oversized-redirect", pattern, false),
        modify_rule("oversized-modify", pattern, "$0"),
        allow_rule("good", "^ls$"),
    ];
    let (engine, diagnostics) = BashRuleEngine::compile_with_diagnostics(rules, None).unwrap();
    assert_eq!(diagnostics.len(), 3);
    for (diagnostic, name) in diagnostics.iter().zip([
        "oversized-command",
        "oversized-redirect",
        "oversized-modify",
    ]) {
        assert_eq!(diagnostic.rule_name, name);
        assert_eq!(diagnostic.kind, RuleDiagnosticKind::InvalidRegex);
        assert_eq!(diagnostic.pattern, pattern);
        assert_eq!(diagnostic.message, error.to_string());
    }
    assert!(matches!(
        command_result(&engine, "ls", None),
        RuleResult::Allowed { rule_name } if rule_name == "good"
    ));
}

#[test]
fn test_classify_fragment_error_distinguishes_kinds() {
    let undefined = expand_fragments("{{nope}}", &HashMap::new())
        .expect_err("undefined fragment")
        .to_string();
    assert_eq!(
        classify_fragment_error(&undefined),
        RuleDiagnosticKind::UndefinedFragment
    );

    let mut circular = HashMap::new();
    circular.insert("a".to_string(), "{{b}}".to_string());
    circular.insert("b".to_string(), "{{a}}".to_string());
    let circular_msg = expand_fragments("{{a}}", &circular)
        .expect_err("circular fragments")
        .to_string();
    assert_eq!(
        classify_fragment_error(&circular_msg),
        RuleDiagnosticKind::CircularFragment
    );

    // The classifier falls through to UndefinedFragment, so an unmatched limit message would be
    // misreported rather than failing loudly.
    let mut leaf = HashMap::new();
    leaf.insert("a".to_string(), "x".to_string());
    let over_count = expand_fragments(&"{{a}}".repeat(FragmentExpander::MAX_EXPANSIONS + 1), &leaf)
        .expect_err("expansion count over the cap")
        .to_string();
    assert_eq!(
        classify_fragment_error(&over_count),
        RuleDiagnosticKind::FragmentExpansionLimitExceeded
    );
}
