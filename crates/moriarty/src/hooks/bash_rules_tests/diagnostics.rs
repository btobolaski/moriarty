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
        engine.apply_rules("ls -la", None),
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
