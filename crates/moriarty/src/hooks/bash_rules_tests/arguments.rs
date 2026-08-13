use super::*;

#[test]
fn parses_commands() {
    for (command, program, args) in [
        ("cargo build", "cargo", vec!["build"]),
        (
            "cargo build --release --features foo",
            "cargo",
            vec!["build", "--release", "--features", "foo"],
        ),
        ("", "", vec![]),
    ] {
        let actual = parse_command(command).unwrap();
        assert_eq!(actual.0, program, "case {command:?}");
        assert_eq!(actual.1, args, "case {command:?}");
    }
}

#[test]
fn builds_commands() {
    for (program, args, expected) in [
        (
            "cargo",
            &["build".to_string(), "--release".to_string()][..],
            "cargo build --release",
        ),
        ("ls", &[][..], "ls"),
    ] {
        assert_eq!(build_command(program, args), expected);
    }
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
    let filtered = filter_arguments("npm start --open --verbose", &remove, &add, &None).unwrap();
    assert_eq!(filtered, "npm start --verbose --no-browser");
}

#[test]
fn no_op_filters_preserve_commands() {
    for (command, filtered) in [
        (
            "cargo build",
            filter_arguments("cargo build", &None, &None, &None).unwrap(),
        ),
        ("cargo build", filter_remove("cargo build", &["--open"])),
        ("", filter_arguments("", &None, &None, &None).unwrap()),
        ("", filter_remove("", &["--flag"])),
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
        "cargo doc --no-deps"
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
fn test_parse_command_quoted_arguments() {
    let (prog, args) = parse_command(r#"echo "hello world""#).unwrap();
    assert_eq!(prog, "echo");
    assert_eq!(args, vec!["hello world"]);

    let (prog, args) = parse_command(r#"rm 'file with spaces.txt'"#).unwrap();
    assert_eq!(prog, "rm");
    assert_eq!(args, vec!["file with spaces.txt"]);
}

#[test]
fn test_parse_command_escaped_characters() {
    let (prog, args) = parse_command(r"rm file\ name.txt").unwrap();
    assert_eq!(prog, "rm");
    assert_eq!(args, vec!["file name.txt"]);
}

#[test]
fn test_parse_command_invalid_syntax() {
    let result = parse_command(r#"echo "unmatched quote"#);
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Failed to parse command")
    );
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

    let result = engine.apply_rules("cargo doc --open --no-deps", None);
    match result {
        RuleResult::ArgumentFiltered { new_command, .. } => {
            assert_eq!(new_command, "cargo doc --no-deps");
        }
        _ => panic!("Expected ArgumentFiltered result"),
    }
}

#[test]
fn test_argument_filter_with_revalidation() {
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

    // First check: matches filter rule
    let result = engine.apply_rules("cargo doc --open", None);
    let filtered_cmd = match result {
        RuleResult::ArgumentFiltered { new_command, .. } => new_command,
        _ => panic!("Expected ArgumentFiltered result"),
    };

    // Revalidation: filtered command should match allow rule
    let recheck = engine.apply_rules(&filtered_cmd, None);
    assert!(matches!(recheck, RuleResult::Allowed { .. }));
}
