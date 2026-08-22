use super::*;
use crate::hooks::command_split::FilterArgument;

#[test]
fn brush_exposes_filterable_arguments_without_redirect_targets() {
    let filter =
        filter_command_for_test(r#">log cargo doc --open --output "my report.html" 2>err"#)
            .unwrap();
    assert_eq!(filter.program.value, "cargo");
    assert_eq!(
        &r#">log cargo doc --open --output "my report.html" 2>err"#[filter.program.range],
        "cargo"
    );
    assert_eq!(
        filter
            .arguments
            .iter()
            .map(|argument| argument.value.as_str())
            .collect::<Vec<_>>(),
        ["doc", "--open", "--output", "my report.html"]
    );
}

#[test]
fn test_filter_arguments_remove_position_independent() {
    // --open at the beginning
    assert_eq!(
        filter_remove("cargo doc --open --no-deps", &["--open"]),
        "cargo doc --no-deps"
    );
    // --open in the middle
    assert_eq!(
        filter_remove("cargo doc --no-deps --open foo", &["--open"]),
        "cargo doc --no-deps foo"
    );
    // --open at the end
    assert_eq!(
        filter_remove("cargo doc --no-deps --open", &["--open"]),
        "cargo doc --no-deps"
    );
}

#[test]
fn filter_arguments_preserves_quoted_argument_boundaries() {
    assert_eq!(
        filter_remove(r#"cargo doc --open --output "my report.html""#, &["--open"]),
        r#"cargo doc --output "my report.html""#
    );
    assert_eq!(
        filter_remove(
            "cargo doc --open --output 'my report.html' > log.txt",
            &["--open"],
        ),
        "cargo doc --output 'my report.html' > log.txt"
    );
    assert_eq!(
        filter_replace(
            r#"cargo doc --output "my report.html""#,
            &[("my report.html", "new report.html")],
        ),
        "cargo doc --output 'new report.html'"
    );
}

#[test]
fn filter_arguments_preserves_quoted_normalized_paths() {
    for (command, expected) in [
        (
            "cat --number '/tmp/p/report file'",
            "cat '/tmp/p/report file'",
        ),
        ("cat --number '/tmp/p/*.pem'", "cat '/tmp/p/*.pem'"),
        (
            r"cat --number /tmp/p/report\ file",
            r"cat /tmp/p/report\ file",
        ),
    ] {
        let filter_command = match split_command(command, "/tmp/p", &BTreeSet::new()) {
            SplitOutcome::Commands(mut leaves) => leaves.pop().unwrap().filter_command.unwrap(),
            SplitOutcome::Bail { reason, .. } => panic!("unexpected bail: {reason:?}"),
        };
        assert_eq!(
            filter_arguments(
                &filter_command,
                &Some(vec!["--number".to_string()]),
                &None,
                &None,
            )
            .unwrap(),
            expected,
            "case {command:?}"
        );
    }
}

#[test]
fn filter_arguments_preserves_unchanged_shell_syntax() {
    for (command, expected) in [
        (
            "cargo doc --open --output $OUT/report.html",
            "cargo doc --output $OUT/report.html",
        ),
        (
            r#"cargo doc --open >"report.txt""#,
            r#"cargo doc >"report.txt""#,
        ),
        (">report.txt cargo doc --open", ">report.txt cargo doc"),
        (">report cargo doc --open 2>err", ">report cargo doc 2>err"),
        ("cargo >report doc --open 2>err", "cargo >report doc 2>err"),
    ] {
        assert_eq!(filter_remove(command, &["--open"]), expected);
    }
}

#[test]
fn replacing_an_expansion_bearing_argument_is_safe() {
    assert_eq!(
        filter_replace("echo $OUT", &[("$OUT", "safe")]),
        "echo safe"
    );
}

#[test]
fn filter_arguments_distinguishes_literal_and_operator_metacharacters() {
    assert_eq!(
        filter_remove("echo --drop '>' > out", &["--drop"]),
        "echo '>' > out"
    );
}

#[test]
fn test_filter_arguments_remove_with_equals() {
    assert_eq!(
        filter_remove("cargo build --color=always", &["--color"]),
        "cargo build"
    );
}

#[test]
fn test_filter_arguments_remove_multiple() {
    assert_eq!(
        filter_remove(
            "cargo doc --open --color=always --no-deps",
            &["--open", "--color"]
        ),
        "cargo doc --no-deps"
    );
}

#[test]
fn adds_arguments() {
    for (add, expected) in [
        (&["--read-only"][..], "docker run ubuntu --read-only"),
        (
            &["--read-only", "--network=none"][..],
            "docker run ubuntu --read-only --network=none",
        ),
    ] {
        assert_eq!(filter_add("docker run ubuntu", add), expected);
    }
}

#[test]
fn replaces_arguments() {
    for (command, replacements, expected) in [
        ("rm -f file.txt", &[("-f", "-i")][..], "rm -i file.txt"),
        (
            "rm -f file1.txt -rf file2.txt",
            &[("-f", "-i"), ("-rf", "-ri")][..],
            "rm -i file1.txt -ri file2.txt",
        ),
        ("cargo build", &[("--open", "--offline")][..], "cargo build"),
    ] {
        assert_eq!(filter_replace(command, replacements), expected);
    }
}

#[test]
fn test_filter_arguments_combined() {
    let remove = Some(vec!["--open".to_string()]);
    let add = Some(vec!["--no-browser".to_string()]);
    let filtered = filter_for_test("npm start --open --verbose", &remove, &add, &None).unwrap();
    assert_eq!(filtered, "npm start --verbose --no-browser");
}

#[test]
fn filter_arguments_handles_trailing_whitespace_and_invalid_spans() {
    let mut filter_command = filter_command_for_test("cargo doc --open").unwrap();
    filter_command.source.push(' ');
    assert_eq!(
        filter_arguments(
            &filter_command,
            &Some(vec!["--open".to_string()]),
            &Some(vec!["--no-browser".to_string()]),
            &None,
        )
        .unwrap(),
        "cargo doc --no-browser"
    );

    let invalid = FilterCommand {
        source: "cargo".to_string(),
        program: filter_command.program,
        arguments: vec![FilterArgument {
            value: "bad".to_string(),
            range: 99..102,
        }],
        rewrite_prefix: String::new(),
        rewrite_suffix: String::new(),
    };
    assert!(
        filter_arguments(
            &invalid,
            &None,
            &None,
            &Some(HashMap::from([("bad".to_string(), "safe".to_string())])),
        )
        .is_err()
    );
}

#[test]
fn argument_filter_matches_without_redirects_but_rewrites_the_full_source() {
    let cwd = tempfile::tempdir().unwrap();
    let engine = make_engine(vec![
        filter_rule("filter-open", r"^cargo doc --open$", "--open", None),
        allow_rule("allow-cargo-doc", r"^cargo doc$"),
        redirect_rule("allow-local", ".*", true),
    ]);

    assert!(matches!(
        evaluation_result(
            &engine,
            ">report cargo doc --open 2>err",
            cwd.path().to_str().unwrap(),
            None,
        ),
        RuleResult::ArgumentFiltered { new_command, .. }
            if new_command == ">report cargo doc 2>err"
    ));
}

#[test]
fn filters_preserve_pipeline_prefixes_and_background_execution() {
    let engine = make_engine(vec![
        filter_rule("filter-open", r"^cargo doc.*--open", "--open", None),
        allow_rule("allow-cargo-doc", r"^cargo doc($|\s)"),
    ]);
    for (command, expected) in [
        ("cargo doc --open &", "cargo doc &"),
        ("! cargo doc --open", "! cargo doc"),
        ("time cargo doc --open", "time cargo doc"),
        ("time -p ! cargo doc --open", "time -p ! cargo doc"),
    ] {
        assert!(
            matches!(
                evaluation_result(&engine, command, "", None),
                RuleResult::ArgumentFiltered { new_command, .. } if new_command == expected
            ),
            "case {command:?}"
        );
    }
}

#[test]
fn no_op_filters_preserve_commands() {
    for (command, filtered) in [
        (
            "cargo build",
            filter_for_test("cargo build", &None, &None, &None).unwrap(),
        ),
        ("cargo build", filter_remove("cargo build", &["--open"])),
        ("cargo build", filter_remove("cargo build", &[])),
        ("cargo build", filter_add("cargo build", &[])),
    ] {
        assert_eq!(filtered, command);
    }
}

#[test]
fn test_filter_arguments_whitespace_handling() {
    assert_eq!(
        filter_remove("cargo  doc    --open   --no-deps", &["--open"]),
        "cargo  doc   --no-deps"
    );
}

#[test]
fn test_filter_arguments_prefix_match_boundaries() {
    assert_eq!(
        filter_remove("cargo build --color=always", &["--color"]),
        "cargo build"
    );
    // --col should NOT match --color
    assert_eq!(
        filter_remove("cargo build --color=always", &["--col"]),
        "cargo build --color=always"
    );
    // --color should NOT match --colours
    assert_eq!(
        filter_remove("cargo build --colours=always", &["--color"]),
        "cargo build --colours=always"
    );
}

#[test]
fn test_filter_arguments_replace_exact_match_only() {
    assert_eq!(filter_replace("rm -f file", &[("-f", "-i")]), "rm -i file");
    // -rf should NOT be affected by -f replacement
    assert_eq!(
        filter_replace("rm -rf file", &[("-f", "-i")]),
        "rm -rf file"
    );
    // file-f.txt should NOT be affected
    assert_eq!(
        filter_replace("rm file-f.txt", &[("-f", "-i")]),
        "rm file-f.txt"
    );
}

#[test]
fn argument_filter_fails_closed_without_a_filterable_simple_command() {
    let engine = make_engine(vec![BashRule {
        name: "filter".to_string(),
        pattern: ".*".to_string(),
        modes: None,
        action: BashRuleAction::ArgumentFilter {
            remove: Some(vec!["--open".to_string()]),
            add: None,
            replace: None,
            reason: None,
        },
    }]);

    for command in [
        "cargo doc --open $(date)",
        "> out",
        "cargo doc --open | cat",
        "cargo doc --open # comment",
    ] {
        assert_eq!(command_result(&engine, command, None), asked("filter"));
    }
}

#[test]
fn argument_filter_matches_cwd_normalized_argument_values() {
    let engine = make_engine(vec![BashRule {
        name: "filter-path".to_string(),
        pattern: r"^cat file$".to_string(),
        modes: None,
        action: BashRuleAction::ArgumentFilter {
            remove: None,
            add: None,
            replace: Some(HashMap::from([("file".to_string(), "safe".to_string())])),
            reason: None,
        },
    }]);

    assert!(matches!(
        leaf_command_result(&engine, "cat /work/project/file", "/work/project", None),
        RuleResult::ArgumentFiltered { new_command, .. } if new_command == "cat safe"
    ));
}

#[test]
fn test_argument_filter_action() {
    let engine = make_engine(vec![BashRule {
        name: "filter-cargo-doc".to_string(),
        pattern: r"^cargo doc\b".to_string(),
        modes: None,
        action: BashRuleAction::ArgumentFilter {
            remove: Some(vec!["--open".to_string()]),
            add: None,
            replace: None,
            reason: Some("Browser flags removed".to_string()),
        },
    }]);

    let result = leaf_command_result(&engine, "cargo doc --open --no-deps", "", None);
    match result {
        RuleResult::ArgumentFiltered { new_command, .. } => {
            assert_eq!(new_command, "cargo doc --no-deps");
        }
        _ => panic!("Expected ArgumentFiltered result"),
    }
}

#[test]
fn argument_filter_revalidation_keeps_the_original_rewrite_when_allowed() {
    let engine = make_engine(vec![
        BashRule {
            name: "filter-cargo-doc".to_string(),
            pattern: r"^cargo doc.*--open".to_string(),
            modes: None,
            action: BashRuleAction::ArgumentFilter {
                remove: Some(vec!["--open".to_string()]),
                add: None,
                replace: None,
                reason: Some("Removed --open".to_string()),
            },
        },
        allow_rule("allow-cargo-doc", r"^cargo doc($|\s)"),
    ]);

    assert!(matches!(
        evaluation_result(&engine, "cargo doc --open", "", None),
        RuleResult::ArgumentFiltered {
            rule_name,
            new_command,
            reason: Some(reason),
        } if rule_name == "filter-cargo-doc"
            && new_command == "cargo doc"
            && reason == "Removed --open"
    ));
}

#[test]
fn argument_filter_revalidation_preserves_quoted_path_semantics() {
    let engine = make_engine(vec![
        BashRule {
            name: "filter-number".to_string(),
            pattern: r"^cat --number report file$".to_string(),
            modes: None,
            action: BashRuleAction::ArgumentFilter {
                remove: Some(vec!["--number".to_string()]),
                add: None,
                replace: None,
                reason: None,
            },
        },
        allow_rule("allow-cat-path", r"^cat report file$"),
    ]);

    assert!(matches!(
        evaluation_result(
            &engine,
            "cat --number '/tmp/p/report file'",
            "/tmp/p",
            None,
        ),
        RuleResult::ArgumentFiltered { new_command, .. }
            if new_command == "cat '/tmp/p/report file'"
    ));
}

#[test]
fn filtered_modify_recheck_propagates_redirect_denial() {
    let engine = make_engine(vec![
        filter_rule("filter-drop", r"^rewrite --drop$", "--drop", None),
        modify_rule("rewrite", r"^rewrite$", "echo > protected"),
        deny_redirect_rule(
            "deny-protected",
            r"^protected$",
            "protected output",
            RedirectDirection::Output,
        ),
    ]);
    let cwd = tempfile::tempdir().unwrap();
    let evaluation = evaluate(
        &engine,
        "rewrite --drop",
        cwd.path().to_str().unwrap(),
        None,
    );

    assert_eq!(
        evaluation.rule_result(),
        denied("deny-protected", "protected output")
    );
    assert_eq!(
        evaluation.contributors(),
        ["filter-drop", "rewrite", "deny-protected"]
    );
}

#[test]
fn chained_argument_filter_revalidation_asks() {
    let filter = |name: &str, pattern: &str, remove: &str| BashRule {
        name: name.to_string(),
        pattern: pattern.to_string(),
        modes: None,
        action: BashRuleAction::ArgumentFilter {
            remove: Some(vec![remove.to_string()]),
            add: None,
            replace: None,
            reason: None,
        },
    };
    let engine = make_engine(vec![
        filter("first", r"^cargo doc.*--open", "--open"),
        filter("second", r"^cargo doc --second$", "--second"),
    ]);

    assert_eq!(
        evaluation_result(&engine, "cargo doc --open --second", "", None),
        RuleResult::Asked {
            rule_name: "second".to_string()
        }
    );
}
