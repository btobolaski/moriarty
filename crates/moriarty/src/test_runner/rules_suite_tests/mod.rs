use std::collections::BTreeSet;

use tempfile::TempDir;

use super::*;
use crate::{
    test_helpers::with_saturated_blocking_pool,
    user_config::{
        BashPathAlias, BashRule, BashRuleAction, RedirectDirection, ToolRule, ToolRuleAction,
        ToolRuleCondition,
    },
};

fn suite(cases: Vec<Case>) -> Suite {
    Suite {
        format_version: SuiteFormatVersion::V1,
        cases,
    }
}

fn bash_case(id: &str, command: &str, expect: Expectation) -> Case {
    Case {
        id: id.to_string(),
        name: format!("case {id}"),
        request: Request::Bash {
            command: command.to_string(),
        },
        expect,
        modes: None,
        cwd: None,
        reason: None,
        expand_cwd: false,
    }
}

fn tool_case(id: &str, tool: &str, input_json: &str, expect: Expectation) -> Case {
    Case {
        id: id.to_string(),
        name: format!("case {id}"),
        request: Request::Tool {
            tool: tool.to_string(),
            input_json: input_json.to_string(),
        },
        expect,
        modes: None,
        cwd: None,
        reason: None,
        expand_cwd: false,
    }
}

fn bash_rule(name: &str, pattern: &str, action: BashRuleAction) -> BashRule {
    BashRule {
        name: name.to_string(),
        pattern: pattern.to_string(),
        modes: None,
        action,
    }
}

fn tool_rule(name: &str, tool: &str, action: ToolRuleAction) -> ToolRule {
    ToolRule {
        name: name.to_string(),
        tool: tool.to_string(),
        modes: None,
        allow_local: false,
        field: None,
        pattern: None,
        conditions: Vec::new(),
        action,
    }
}

fn prepare(cases: Vec<Case>, cwd: &Path) -> miette::Result<PreparedSuite> {
    let cwd = validate_cwd(cwd.to_path_buf(), "test cwd")?;
    prepare_suite(suite(cases), &cwd)
}

async fn evaluate(cases: Vec<Case>, config: UserConfig, cwd: &Path) -> RulesSuiteReport {
    let engines = Engines::compile(config).unwrap();
    evaluate_suite(prepare(cases, cwd).unwrap(), &engines)
        .await
        .unwrap()
}

fn local_allow_engines() -> Engines {
    let mut local = tool_rule("local", "Read", ToolRuleAction::Allow);
    local.allow_local = true;
    Engines::compile(UserConfig {
        tool_rules: Some(vec![local]),
        ..UserConfig::default()
    })
    .unwrap()
}

#[test]
fn strict_models_reject_unknown_incompatible_and_missing_fields() {
    let invalid = [
        "format_version = 1\nunknown = true\ncases = []",
        r#"format_version = 1
[[cases]]
id = "a"
name = "a"
unknown = true
request = { kind = "bash", command = "echo" }
expect = "allowed""#,
        r#"format_version = 1
[[cases]]
id = "a"
name = "a"
request = { kind = "bash", command = "echo", tool = "Read" }
expect = "allowed""#,
        r#"format_version = 1
[[cases]]
id = "a"
name = "a"
request = { kind = "bash", command = "echo" }"#,
        r#"format_version = 1
[[cases]]
id = "a"
name = "a"
modes = ["future"]
request = { kind = "bash", command = "echo" }
expect = "allowed""#,
        r#"format_version = 1
[[cases]]
id = "a"
name = "a"
request = { kind = "future", command = "echo" }
expect = "allowed""#,
    ];
    for source in invalid {
        assert!(parse_suite(source).is_err(), "accepted {source:?}");
    }
}

#[test]
fn format_version_is_closed_at_the_parse_boundary() {
    let parsed = parse_suite("format_version = 1\ncases = []").unwrap();
    assert!(matches!(parsed.format_version, SuiteFormatVersion::V1));

    for value in [0, 2] {
        let error = parse_suite(&format!("format_version = {value}\ncases = []"))
            .expect_err("unsupported numeric version must fail during parsing")
            .to_string();
        assert!(error.contains(&format!(
            "Unsupported rule suite format_version {value}; expected {FORMAT_VERSION}"
        )));
    }

    for source in [
        "format_version = -1\ncases = []",
        "format_version = 1.0\ncases = []",
        "format_version = \"1\"\ncases = []",
        "format_version = true\ncases = []",
        "cases = []",
    ] {
        assert!(parse_suite(source).is_err(), "accepted {source:?}");
    }
}

#[test]
fn deterministic_preflight_rejects_invalid_suite_and_cases() {
    let cwd = tempfile::tempdir().unwrap();
    let cases = vec![
        ("empty suite", suite(Vec::new())),
        (
            "empty id",
            suite(vec![bash_case("", "echo", Expectation::Allowed)]),
        ),
        (
            "duplicate id",
            suite(vec![
                bash_case("a", "echo", Expectation::Allowed),
                bash_case("a", "echo", Expectation::Allowed),
            ]),
        ),
        (
            "empty name",
            suite(vec![Case {
                name: String::new(),
                ..bash_case("a", "echo", Expectation::Allowed)
            }]),
        ),
        (
            "empty command",
            suite(vec![bash_case("a", "", Expectation::Allowed)]),
        ),
        (
            "empty tool",
            suite(vec![tool_case("a", "", "{}", Expectation::Allowed)]),
        ),
        (
            "Bash tool",
            suite(vec![tool_case("a", "Bash", "{}", Expectation::Allowed)]),
        ),
        (
            "invalid JSON",
            suite(vec![tool_case("a", "Read", "{", Expectation::Allowed)]),
        ),
        (
            "reason on non-denied",
            suite(vec![Case {
                reason: Some("why".to_string()),
                ..bash_case("a", "echo", Expectation::Allowed)
            }]),
        ),
        (
            "missing cwd marker",
            suite(vec![Case {
                expand_cwd: true,
                ..bash_case("a", "echo", Expectation::Allowed)
            }]),
        ),
        (
            "missing directory",
            suite(vec![Case {
                cwd: Some(PathBuf::from("missing")),
                ..bash_case("a", "echo", Expectation::Allowed)
            }]),
        ),
    ];
    for (label, suite) in cases {
        let cwd = validate_cwd(cwd.path().to_path_buf(), "test cwd").unwrap();
        assert!(prepare_suite(suite, &cwd).is_err(), "accepted {label}");
    }
}

#[cfg(unix)]
#[test]
fn non_utf8_cwd_is_rejected() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let path = PathBuf::from(OsString::from_vec(vec![0xff]));
    assert!(validate_cwd(path, "test cwd").is_err());
}

#[test]
fn mode_omission_and_expansion_are_distinct_and_ordered() {
    let cwd = tempfile::tempdir().unwrap();
    let mut expanded = bash_case("expanded", "echo", Expectation::Allowed);
    expanded.modes = Some(vec![PermissionMode::Plan, PermissionMode::Default]);
    let prepared = prepare(
        vec![bash_case("none", "echo", Expectation::Allowed), expanded],
        cwd.path(),
    )
    .unwrap();
    assert_eq!(prepared.source_case_count.get(), 2);
    assert_eq!(prepared.evaluation_count.get(), 3);
    assert_eq!(prepared.rows.len(), 3);
    assert_eq!(prepared.rows[0].mode, None);
    assert_eq!(
        prepared.rows[1..]
            .iter()
            .map(|row| row.mode.unwrap())
            .collect::<Vec<_>>(),
        [PermissionMode::Plan, PermissionMode::Default]
    );

    for modes in [Vec::new(), vec![PermissionMode::Plan, PermissionMode::Plan]] {
        let mut invalid = bash_case("bad", "echo", Expectation::Allowed);
        invalid.modes = Some(modes);
        assert!(prepare(vec![invalid], cwd.path()).is_err());
    }
}

#[test]
fn cwd_resolution_and_interpolation_are_lexical_opt_in_and_value_only() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("nested")).unwrap();
    let default = validate_cwd(
        resolve_cwd(root.path(), Some(Path::new("nested"))),
        "test cwd",
    )
    .unwrap();
    assert_eq!(default.path, root.path().join("nested"));

    let mut bash = bash_case(
        "bash",
        "printf '{{cwd}}' '$ENV' \"$(date)\" {x}\n",
        Expectation::NoMatch,
    );
    bash.expand_cwd = true;
    let mut tool = tool_case(
        "tool",
        "Read",
        r#"{"{{cwd}}":"key","path":"{{cwd}}/a","nested":["x{{cwd}}",null,false,true,42,{"x":1}],"a.b":"literal-key","literal":"$ENV\n{}"}"#,
        Expectation::NoMatch,
    );
    tool.expand_cwd = true;
    let literal = bash_case("literal", "echo {{cwd}} $ENV\n", Expectation::NoMatch);
    let prepared = prepare_suite(suite(vec![bash, tool, literal]), &default).unwrap();
    let cwd = default.value;
    assert_eq!(
        prepared.rows[0].request,
        PreparedRequest::Bash {
            command: format!("printf '{cwd}' '$ENV' \"$(date)\" {{x}}\n")
        }
    );
    let PreparedRequest::Tool { input, .. } = &prepared.rows[1].request else {
        panic!("expected tool request");
    };
    assert_eq!(
        input,
        &serde_json::json!({
            "{{cwd}}": "key",
            "path": format!("{cwd}/a"),
            "nested": [format!("x{cwd}"), null, false, true, 42, {"x": 1}],
            "a.b": "literal-key",
            "literal": "$ENV\n{}",
        })
    );
    assert!(input.get("absent").is_none());
    assert_eq!(
        prepared.rows[2].request,
        PreparedRequest::Bash {
            command: "echo {{cwd}} $ENV\n".to_string()
        }
    );

    let mut non_recursive = serde_json::json!({"path": "x{{cwd}}"});
    assert!(replace_cwd_values(&mut non_recursive, "/{{cwd}}"));
    assert_eq!(non_recursive["path"], "x/{{cwd}}");
}

#[test]
fn exhaustive_bash_expectation_table_keeps_filtered_and_modified_distinct() {
    let actuals = [
        ActualResult::Bash(RuleResult::Allowed {
            rule_name: "rule".to_string(),
        }),
        ActualResult::Bash(RuleResult::ArgumentFiltered {
            rule_name: "rule".to_string(),
            new_command: "filtered".to_string(),
            reason: None,
        }),
        ActualResult::Bash(RuleResult::Asked {
            rule_name: "rule".to_string(),
        }),
        ActualResult::Bash(RuleResult::Denied {
            rule_name: "rule".to_string(),
            reason: "reason".to_string(),
        }),
        ActualResult::Bash(RuleResult::NoMatch),
        ActualResult::Bash(RuleResult::Modified {
            rule_name: "rule".to_string(),
            new_command: "rewrite".to_string(),
        }),
    ];
    let expected = [
        Expectation::Allowed,
        Expectation::Ask,
        Expectation::Denied,
        Expectation::NoMatch,
        Expectation::NotAutoAllowed,
    ];
    let pass = [
        [true, false, false, false, false],
        [true, false, false, false, false],
        [false, true, false, false, true],
        [false, false, true, false, true],
        [false, false, false, true, true],
        [false, false, false, false, false],
    ];
    for (actual_index, actual) in actuals.iter().enumerate() {
        for (expected_index, expectation) in expected.iter().enumerate() {
            assert_eq!(
                assertion_passes(*expectation, None, actual),
                pass[actual_index][expected_index],
                "actual={actual:?} expected={expectation:?}"
            );
        }
    }
}

#[test]
fn denial_reasons_distinguish_omission_empty_exact_and_punctuation() {
    let denied = ActualResult::Tool(ToolRuleResult::Denied {
        rule_name: "deny".to_string(),
        reason: String::new(),
    });
    assert!(assertion_passes(Expectation::Denied, None, &denied));
    assert!(assertion_passes(Expectation::Denied, Some(""), &denied));
    assert!(!assertion_passes(Expectation::Denied, Some("."), &denied));

    let punctuated = ActualResult::Tool(ToolRuleResult::Denied {
        rule_name: "deny".to_string(),
        reason: "blocked.".to_string(),
    });
    assert!(assertion_passes(
        Expectation::Denied,
        Some("blocked."),
        &punctuated
    ));
    assert!(!assertion_passes(
        Expectation::Denied,
        Some("blocked"),
        &punctuated
    ));
}

#[test]
fn engine_result_wire_shapes_are_stable() {
    let cases = [
        (
            serde_json::to_value(RuleResult::Allowed {
                rule_name: "r".to_string(),
            })
            .unwrap(),
            serde_json::json!({"result": "allowed", "rule": "r"}),
        ),
        (
            serde_json::to_value(RuleResult::Denied {
                rule_name: "r".to_string(),
                reason: "why".to_string(),
            })
            .unwrap(),
            serde_json::json!({"result": "denied", "rule": "r", "reason": "why"}),
        ),
        (
            serde_json::to_value(RuleResult::Modified {
                rule_name: "r".to_string(),
                new_command: "new".to_string(),
            })
            .unwrap(),
            serde_json::json!({"result": "modified", "rule": "r", "rewrite": "new"}),
        ),
        (
            serde_json::to_value(RuleResult::Asked {
                rule_name: "r".to_string(),
            })
            .unwrap(),
            serde_json::json!({"result": "asked", "rule": "r"}),
        ),
        (
            serde_json::to_value(RuleResult::ArgumentFiltered {
                rule_name: "r".to_string(),
                new_command: "new".to_string(),
                reason: None,
            })
            .unwrap(),
            serde_json::json!({"result": "argument_filtered", "rule": "r", "rewrite": "new"}),
        ),
        (
            serde_json::to_value(RuleResult::ArgumentFiltered {
                rule_name: "r".to_string(),
                new_command: "new".to_string(),
                reason: Some("why".to_string()),
            })
            .unwrap(),
            serde_json::json!({
                "result": "argument_filtered",
                "rule": "r",
                "rewrite": "new",
                "reason": "why",
            }),
        ),
        (
            serde_json::to_value(RuleResult::NoMatch).unwrap(),
            serde_json::json!({"result": "no_match"}),
        ),
        (
            serde_json::to_value(ToolRuleResult::Allowed {
                rule_name: "r".to_string(),
            })
            .unwrap(),
            serde_json::json!({"result": "allowed", "rule": "r"}),
        ),
        (
            serde_json::to_value(ToolRuleResult::Denied {
                rule_name: "r".to_string(),
                reason: "why".to_string(),
            })
            .unwrap(),
            serde_json::json!({"result": "denied", "rule": "r", "reason": "why"}),
        ),
        (
            serde_json::to_value(ToolRuleResult::Asked {
                rule_name: "r".to_string(),
            })
            .unwrap(),
            serde_json::json!({"result": "asked", "rule": "r"}),
        ),
        (
            serde_json::to_value(ToolRuleResult::NoMatch).unwrap(),
            serde_json::json!({"result": "no_match"}),
        ),
    ];

    for (actual, expected) in cases {
        assert_eq!(actual, expected);
    }
}

#[tokio::test]
async fn empty_policy_yields_normal_no_match_results() {
    let cwd = tempfile::tempdir().unwrap();
    let report = evaluate(
        vec![
            bash_case("bash", "unknown", Expectation::NoMatch),
            tool_case("tool", "Read", "{}", Expectation::NoMatch),
        ],
        UserConfig::default(),
        cwd.path(),
    )
    .await;
    assert!(report.rows.iter().all(|row| row.passed));
    assert!(report.rows.iter().all(|row| matches!(
        &row.actual,
        ActualResult::Bash(RuleResult::NoMatch) | ActualResult::Tool(ToolRuleResult::NoMatch)
    )));
}

#[tokio::test]
async fn tool_truth_table_preserves_no_match_reasons_and_modes() {
    let cwd = tempfile::tempdir().unwrap();
    let mut plan_rule = tool_rule("plan", "Read", ToolRuleAction::Allow);
    plan_rule.modes = Some(BTreeSet::from([PermissionMode::Plan]));
    let mut explicit = tool_case("explicit", "Read", "{}", Expectation::Allowed);
    explicit.modes = Some(vec![PermissionMode::Plan]);
    let mut denied = tool_case("denied", "Delete", "{}", Expectation::Denied);
    denied.reason = Some("blocked exactly.".to_string());
    let report = evaluate(
        vec![
            tool_case("mode-less", "Read", "{}", Expectation::NoMatch),
            explicit,
            tool_case("ask", "Write", "{}", Expectation::Ask),
            denied,
            tool_case("not-auto", "Other", "{}", Expectation::NotAutoAllowed),
        ],
        UserConfig {
            tool_rules: Some(vec![
                plan_rule,
                tool_rule("ask-write", "Write", ToolRuleAction::Ask),
                tool_rule(
                    "deny-delete",
                    "Delete",
                    ToolRuleAction::Deny {
                        value: "blocked exactly.".to_string(),
                    },
                ),
            ]),
            ..UserConfig::default()
        },
        cwd.path(),
    )
    .await;
    assert!(report.rows.iter().all(|row| row.passed));
    assert!(matches!(
        &report.rows[0].actual,
        ActualResult::Tool(ToolRuleResult::NoMatch)
    ));
    assert!(matches!(
        &report.rows[1].actual,
        ActualResult::Tool(ToolRuleResult::Allowed { .. })
    ));
    assert!(matches!(
        &report.rows[2].actual,
        ActualResult::Tool(ToolRuleResult::Asked { .. })
    ));
    assert!(matches!(
        &report.rows[3].actual,
        ActualResult::Tool(ToolRuleResult::Denied { .. })
    ));
    assert!(matches!(
        &report.rows[4].actual,
        ActualResult::Tool(ToolRuleResult::NoMatch)
    ));
}

#[test]
fn either_engines_compile_diagnostics_reject_the_policy() {
    let bash_error = Engines::compile(UserConfig {
        bash_rules: Some(vec![bash_rule("bad-bash", "[", BashRuleAction::Allow)]),
        ..UserConfig::default()
    })
    .err()
    .unwrap()
    .to_string();
    assert!(bash_error.contains("bash rule \"bad-bash\""));
    assert!(bash_error.contains("pattern \"[\""));

    let mut bad_tool = tool_rule("bad-tool", "Read", ToolRuleAction::Allow);
    bad_tool.field = Some("path".to_string());
    let tool_error = Engines::compile(UserConfig {
        tool_rules: Some(vec![bad_tool]),
        ..UserConfig::default()
    })
    .err()
    .unwrap()
    .to_string();
    assert!(tool_error.contains("tool rule \"bad-tool\""));
    assert!(tool_error.contains("field"));
}

#[tokio::test]
async fn completed_bash_evaluator_handles_rewrites_filters_redirects_and_alias_isolation() {
    let cwd = tempfile::tempdir().unwrap();
    let rules = vec![
        bash_rule("read-alias", r"^cat file$", BashRuleAction::Allow),
        bash_rule(
            "filter-doc",
            r"^cargo doc --open$",
            BashRuleAction::ArgumentFilter {
                remove: Some(vec!["--open".to_string()]),
                add: None,
                replace: None,
                reason: Some("removed".to_string()),
            },
        ),
        bash_rule("allow-doc", r"^cargo doc$", BashRuleAction::Allow),
        bash_rule(
            "filter-drop",
            r"^rewrite --drop$",
            BashRuleAction::ArgumentFilter {
                remove: Some(vec!["--drop".to_string()]),
                add: None,
                replace: None,
                reason: None,
            },
        ),
        bash_rule(
            "rewrite",
            r"^rewrite$",
            BashRuleAction::Modify {
                value: "echo > protected".to_string(),
            },
        ),
        bash_rule(
            "rewrite-safe",
            r"^rewrite-safe$",
            BashRuleAction::Modify {
                value: "echo hi".to_string(),
            },
        ),
        bash_rule("allow-echo", r"^echo($|\s)", BashRuleAction::Allow),
        bash_rule(
            "deny-protected",
            r"^protected$",
            BashRuleAction::DenyRedirect {
                value: "protected output".to_string(),
                direction: RedirectDirection::Output,
            },
        ),
    ];
    let config = UserConfig {
        bash_path_aliases: BTreeSet::from([BashPathAlias::validate("P".to_string()).unwrap()]),
        bash_rules: Some(rules),
        ..UserConfig::default()
    };
    let mut alias = bash_case("alias", "P={{cwd}}; cat $P/file", Expectation::Allowed);
    alias.expand_cwd = true;
    let mut redirect = bash_case("redirect", "rewrite --drop", Expectation::Denied);
    redirect.reason = Some("protected output".to_string());
    let report = evaluate(
        vec![
            alias,
            bash_case("no-leak", "cat $P/file", Expectation::NotAutoAllowed),
            bash_case("filter", "cargo doc --open", Expectation::Allowed),
            bash_case("compound", "echo hi && cargo doc", Expectation::Allowed),
            redirect,
            bash_case("modified", "rewrite-safe", Expectation::Allowed),
        ],
        config,
        cwd.path(),
    )
    .await;
    assert!(report.rows[0].passed);
    assert!(report.rows[1].passed);
    assert!(matches!(
        &report.rows[2].actual,
        ActualResult::Bash(RuleResult::ArgumentFiltered { .. })
    ));
    assert!(matches!(
        &report.rows[3].actual,
        ActualResult::Bash(RuleResult::Allowed { .. })
    ));
    assert!(matches!(
        &report.rows[4].actual,
        ActualResult::Bash(RuleResult::Denied { reason, .. })
            if reason == "protected output"
    ));
    assert!(matches!(
        &report.rows[5].actual,
        ActualResult::Bash(RuleResult::Modified { new_command, .. })
            if new_command == "echo hi"
    ));
    assert_eq!((report.passed_count, report.failed_count), (5, 1));
}

#[tokio::test]
async fn shared_tool_engine_keeps_effective_cwd_locality_independent() {
    let root = tempfile::tempdir().unwrap();
    let left = root.path().join("left");
    let right = root.path().join("right");
    std::fs::create_dir_all(&left).unwrap();
    std::fs::create_dir_all(&right).unwrap();
    let mut allow_local = tool_rule("local", "Read", ToolRuleAction::Allow);
    allow_local.allow_local = true;
    allow_local.conditions = vec![ToolRuleCondition::Present {
        field: "path".to_string(),
    }];

    let local_case = |id: &str, cwd: PathBuf| {
        let mut case = tool_case(
            id,
            "Read",
            r#"{"path":"{{cwd}}/new.txt"}"#,
            Expectation::Allowed,
        );
        case.cwd = Some(cwd);
        case.expand_cwd = true;
        case
    };
    let report = evaluate(
        vec![local_case("left", left), local_case("right", right)],
        UserConfig {
            tool_rules: Some(vec![allow_local]),
            ..UserConfig::default()
        },
        root.path(),
    )
    .await;
    assert_eq!((report.passed_count, report.failed_count), (2, 0));
    assert_ne!(report.rows[0].effective_cwd, report.rows[1].effective_cwd);
}

#[test]
fn locality_timeout_cannot_satisfy_not_auto_allowed() {
    let cwd = tempfile::tempdir().unwrap();
    let mut engines = local_allow_engines();
    engines.tool.force_locality_timeout();
    let prepared = prepare(
        vec![tool_case(
            "local",
            "Read",
            r#"{"path":"file"}"#,
            Expectation::NotAutoAllowed,
        )],
        cwd.path(),
    )
    .unwrap();

    let error = with_saturated_blocking_pool(evaluate_suite(prepared, &engines))
        .expect_err("infrastructure failure must abort the report");
    assert!(error.to_string().contains("timed out"));
}

#[tokio::test]
async fn disappearing_cwd_aborts_report_instead_of_satisfying_not_auto_allowed() {
    let cwd = tempfile::tempdir().unwrap();
    let prepared = prepare(
        vec![tool_case(
            "local",
            "Read",
            r#"{"path":"file"}"#,
            Expectation::NotAutoAllowed,
        )],
        cwd.path(),
    )
    .unwrap();
    let engines = local_allow_engines();
    let cwd_path = cwd.path().to_string_lossy().into_owned();
    drop(cwd);

    let error = evaluate_suite(prepared, &engines)
        .await
        .expect_err("missing cwd must abort the report");
    assert!(error.to_string().contains(&cwd_path));
}

#[tokio::test]
async fn mixed_rows_retain_source_and_mode_order_and_counts() {
    let cwd = tempfile::tempdir().unwrap();
    let mut bash = bash_case("bash", "echo", Expectation::Allowed);
    bash.modes = Some(vec![PermissionMode::Plan, PermissionMode::Default]);
    let mut tool = tool_case("tool", "Read", "{}", Expectation::Ask);
    tool.modes = Some(vec![PermissionMode::Auto]);
    let report = evaluate(
        vec![bash, tool, bash_case("last", "other", Expectation::NoMatch)],
        UserConfig {
            bash_rules: Some(vec![bash_rule("echo", r"^echo$", BashRuleAction::Allow)]),
            tool_rules: Some(vec![tool_rule("ask-read", "Read", ToolRuleAction::Ask)]),
            ..UserConfig::default()
        },
        cwd.path(),
    )
    .await;
    assert_eq!(report.source_case_count.get(), 3);
    assert_eq!(report.evaluation_count.get(), 4);
    assert_eq!((report.passed_count, report.failed_count), (4, 0));
    assert_eq!(
        report
            .rows
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        ["bash", "bash", "tool", "last"]
    );
    assert_eq!(
        report.rows.iter().map(|row| row.mode).collect::<Vec<_>>(),
        [
            Some(PermissionMode::Plan),
            Some(PermissionMode::Default),
            Some(PermissionMode::Auto),
            None,
        ]
    );
}

#[tokio::test]
async fn late_invalid_case_prevents_reporting_and_never_executes_commands() {
    let dir = tempfile::tempdir().unwrap();
    let sentinel = dir.path().join("sentinel");
    let suite_path = dir.path().join("suite.toml");
    let config_path = dir.path().join("rules.toml");
    std::fs::write(
        &suite_path,
        format!(
            r#"format_version = 1
[[cases]]
id = "first"
name = "would create sentinel"
request = {{ kind = "bash", command = "touch {}" }}
expect = "allowed"
[[cases]]
id = "late"
name = "invalid"
request = {{ kind = "tool", tool = "Read", input_json = "{{" }}
expect = "not_auto_allowed"
"#,
            sentinel.display()
        ),
    )
    .unwrap();
    std::fs::write(
        &config_path,
        r#"[[bash_rules]]
name = "touch"
pattern = "^touch "
action = { type = "Allow" }
"#,
    )
    .unwrap();
    let before = std::env::current_dir().unwrap();
    let error = run_rules_suite(&suite_path, &config_path, dir.path(), None)
        .await
        .expect_err("late invalid input must fail preflight");
    assert!(
        error
            .to_string()
            .contains("case 'late' has invalid input_json")
    );
    assert!(!sentinel.exists());
    assert_eq!(std::env::current_dir().unwrap(), before);
}

#[tokio::test]
async fn missing_suite_reports_read_context() {
    let dir = tempfile::tempdir().unwrap();
    let suite_path = dir.path().join("missing.toml");
    let error = run_rules_suite(&suite_path, Path::new("unused"), dir.path(), None)
        .await
        .expect_err("missing suite must fail");
    assert!(error.to_string().contains("Failed to read rule suite"));
    assert!(error.to_string().contains("missing.toml"));
}

#[tokio::test]
async fn public_core_returns_serializable_report() {
    let dir = TempDir::new().unwrap();
    let suite_path = dir.path().join("suite.toml");
    let config_path = dir.path().join("rules.toml");
    let effective_cwd = dir.path().join("relative");
    std::fs::create_dir(&effective_cwd).unwrap();
    std::fs::write(
        &suite_path,
        r#"format_version = 1
[[cases]]
id = "echo"
name = "echo denial mismatch"
request = { kind = "bash", command = "echo {{cwd}}" }
expect = "denied"
reason = "expected reason"
expand_cwd = true
"#,
    )
    .unwrap();
    std::fs::write(
        &config_path,
        r#"[[bash_rules]]
name = "echo"
pattern = "^echo "
action = { type = "Deny", value = "actual reason" }
"#,
    )
    .unwrap();
    let report = run_rules_suite(
        &suite_path,
        &config_path,
        dir.path(),
        Some(Path::new("relative")),
    )
    .await
    .unwrap();
    assert_eq!(report.source_case_count.get(), 1);
    assert_eq!(report.evaluation_count.get(), 1);
    assert_eq!((report.passed_count, report.failed_count), (0, 1));
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["format_version"], 1);
    assert_eq!(json["source_case_count"], 1);
    assert_eq!(json["evaluation_count"], 1);
    assert_eq!(json["rows"][0]["mode"], Value::Null);
    assert_eq!(
        json["rows"][0]["effective_cwd"],
        effective_cwd.to_str().unwrap()
    );
    assert_eq!(
        json["rows"][0]["request"]["command"],
        format!("echo {}", effective_cwd.display())
    );
    assert_eq!(json["rows"][0]["expected_reason"], "expected reason");
    assert_eq!(json["rows"][0]["actual"]["result"], "denied");
    assert_eq!(json["rows"][0]["actual"]["reason"], "actual reason");
}
