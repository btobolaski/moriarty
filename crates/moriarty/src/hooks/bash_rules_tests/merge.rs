use super::*;

// ===== Compound splitting: merge / cap / downgrade helpers =====

#[test]
fn test_merge_results_empty_is_nomatch() {
    assert_eq!(merge_results(vec![]), RuleResult::NoMatch);
}

#[test]
fn test_merge_results_single_element_is_verbatim() {
    // Every variant passes through unchanged so single-command behavior is preserved exactly.
    for result in [
        allowed("a"),
        denied("d", "r"),
        asked("k"),
        modified("m", "x"),
        RuleResult::ArgumentFiltered {
            rule_name: "f".to_string(),
            new_command: "y".to_string(),
            reason: None,
        },
        RuleResult::NoMatch,
    ] {
        assert_eq!(merge_results(vec![result.clone()]), result);
    }
}

#[test]
fn test_merge_results_all_allow_attributes_first_leaf() {
    assert_eq!(
        merge_results(vec![allowed("first"), allowed("second")]),
        allowed("first")
    );
}

#[test]
fn test_merge_results_deny_beats_everything_regardless_of_order() {
    // Deny must win over Allow/Ask/Modify/NoMatch no matter where the dangerous leaf sits.
    assert_eq!(
        merge_results(vec![
            allowed("a"),
            asked("k"),
            denied("d", "boom"),
            RuleResult::NoMatch,
        ]),
        denied("d", "boom")
    );
    assert_eq!(
        merge_results(vec![denied("d", "boom"), allowed("a")]),
        denied("d", "boom")
    );
}

#[test]
fn test_merge_results_two_denies_keeps_first_rule_name() {
    // The fold's `>=` is the tie-break: at equal rank the earlier leaf wins, so a compound with
    // two denying leaves is attributed to the first one's rule.
    assert_eq!(
        merge_results(vec![
            denied("first-deny", "boom"),
            denied("second-deny", "bang")
        ]),
        denied("first-deny", "boom")
    );
}

#[test]
fn test_merge_results_two_asks_keeps_first_rule_name() {
    // Same first-wins tie-break at Ask rank: the first asking leaf's rule name survives.
    assert_eq!(
        merge_results(vec![asked("first-ask"), asked("second-ask")]),
        asked("first-ask")
    );
}

#[test]
fn test_merge_results_ask_beats_allow_and_nomatch() {
    assert_eq!(
        merge_results(vec![allowed("a"), asked("k"), RuleResult::NoMatch]),
        asked("k")
    );
}

#[test]
fn test_merge_results_nomatch_forces_prompt() {
    assert_eq!(
        merge_results(vec![allowed("a"), RuleResult::NoMatch]),
        RuleResult::NoMatch
    );
}

#[test]
fn test_merge_results_mixed_allow_and_modify_is_nomatch() {
    // We never reconstruct a rewritten compound, so a Modify among Allows falls back to prompt.
    assert_eq!(
        merge_results(vec![allowed("a"), modified("m", "x")]),
        RuleResult::NoMatch
    );
}

#[test]
fn test_cap_allow_at_ask() {
    assert_eq!(cap_allow_at_ask(allowed("a")), asked("a"));
    // Non-allow decisions (including Deny) are untouched.
    assert_eq!(cap_allow_at_ask(denied("d", "r")), denied("d", "r"));
    assert_eq!(cap_allow_at_ask(RuleResult::NoMatch), RuleResult::NoMatch);
}

#[test]
fn test_downgrade_non_deny_to_ask() {
    // Only Deny survives a bail; every other variant collapses to NoMatch (which mod.rs prompts).
    assert_eq!(
        downgrade_non_deny_to_ask(denied("d", "r")),
        denied("d", "r")
    );
    assert_eq!(downgrade_non_deny_to_ask(allowed("a")), RuleResult::NoMatch);
    assert_eq!(downgrade_non_deny_to_ask(asked("k")), RuleResult::NoMatch);
    assert_eq!(
        downgrade_non_deny_to_ask(modified("m", "x")),
        RuleResult::NoMatch
    );
    assert_eq!(
        downgrade_non_deny_to_ask(RuleResult::ArgumentFiltered {
            rule_name: "f".to_string(),
            new_command: "y".to_string(),
            reason: None,
        }),
        RuleResult::NoMatch
    );
    assert_eq!(
        downgrade_non_deny_to_ask(RuleResult::NoMatch),
        RuleResult::NoMatch
    );
}
