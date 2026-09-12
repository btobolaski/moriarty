use std::{env, fs, hint::black_box, path::Path};

use criterion::{BatchSize, Criterion};
use miette::{IntoDiagnostic, WrapErr};
use regex::RegexSet;
use sha2::{Digest, Sha256};
use tokio::runtime::Builder;

use crate::{
    hooks::{
        bash_rules::{
            BashRuleEngine, EvaluationContext, EvaluationPurpose, RuleResult, default_fragments,
            expand_fragments, validate_rule_pattern,
        },
        command_split::split_command,
    },
    test_runner::command_trace,
    user_config::{BashRuleAction, RedirectRuleAction, UserConfig, load_user_config_from},
};

const COMMANDS: [(&str, &str); 3] = [
    ("simple", "ls"),
    ("compound", "ls && git status"),
    ("redirect", "ls >/dev/null 2>&1"),
];

fn validate_fixture(config: &UserConfig, cwd: &str) -> miette::Result<BashRuleEngine> {
    // Preserve the workload recorded in benches/fixtures/bash_rules.toml's header.
    assert_eq!(config.bash_rules.as_ref().unwrap().len(), 193);
    assert_eq!(config.tool_rules.as_ref().unwrap().len(), 165);
    assert_eq!(config.pattern_fragments.as_ref().unwrap().len(), 106);
    assert!(
        config
            .bash_rules
            .as_ref()
            .unwrap()
            .iter()
            .all(|rule| { !matches!(&rule.action, BashRuleAction::Modify { .. }) }),
        "Individual-regex benchmark assumes validation-only rules"
    );
    // This constructor omits private path aliases; warm evaluation must use from_config below.
    let (_, diagnostics) = BashRuleEngine::compile_with_diagnostics(
        config.bash_rules.clone().unwrap(),
        config.pattern_fragments.clone(),
    )?;
    assert!(
        diagnostics.is_empty(),
        "Invalid benchmark policy: {diagnostics:?}"
    );
    let engine = BashRuleEngine::from_config(config.clone())?;
    for (_, command) in COMMANDS {
        let context = EvaluationContext::new(cwd, None);
        let evaluation = engine.evaluate_sync(command, &context, EvaluationPurpose::Diagnostics);
        assert!(
            matches!(evaluation.rule_result(), RuleResult::Allowed { .. }),
            "Fixture no longer allows {command}"
        );
    }
    Ok(engine)
}

pub(crate) fn run() -> miette::Result<()> {
    let config_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("benches/fixtures/bash_rules.toml");
    let bytes = fs::read(&config_path)
        .into_diagnostic()
        .wrap_err("Failed to read benchmark policy")?;
    let config: UserConfig = toml::from_slice(&bytes).into_diagnostic()?;
    let hash = hex::encode(Sha256::digest(&bytes));
    let rules = config.bash_rules.as_deref().unwrap_or_default();
    let mut fragments = default_fragments();
    fragments.extend(config.pattern_fragments.clone().unwrap_or_default());
    let mut command_patterns = Vec::new();
    let mut redirect_patterns = Vec::new();
    for rule in rules {
        let pattern = expand_fragments(&rule.pattern, &fragments)?;
        if RedirectRuleAction::from_config(&rule.action).is_some() {
            redirect_patterns.push(pattern);
        } else {
            command_patterns.push(pattern);
        }
    }
    let cwd = env::current_dir().into_diagnostic()?;
    let cwd = cwd
        .to_str()
        .ok_or_else(|| miette::miette!("Non-UTF-8 benchmark cwd"))?;
    let engine = validate_fixture(&config, cwd)?;
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .into_diagnostic()?;
    let allocator = if cfg!(feature = "mimalloc") {
        "mimalloc"
    } else {
        "system"
    };
    eprintln!(
        "Policy SHA-256: {hash}; allocator: {allocator}; bytes: {}; command rules: {}; redirect rules: {}; cwd: {cwd}",
        bytes.len(),
        command_patterns.len(),
        redirect_patterns.len(),
    );
    let mut criterion = Criterion::default().configure_from_args();
    // Different policy bytes cannot accidentally compare against each other's saved baseline.
    let mut group = criterion.benchmark_group(format!("bash_rules/{}", &hash[..12]));

    group.bench_function("config/parse_toml", |b| {
        b.iter(|| {
            drop(black_box(
                toml::from_slice::<UserConfig>(black_box(&bytes)).unwrap(),
            ))
        });
    });
    group.bench_function("config/read_and_parse", |b| {
        b.iter(|| {
            drop(black_box(
                runtime
                    .block_on(load_user_config_from(Some(black_box(&config_path))))
                    .unwrap(),
            ));
        });
    });
    group.bench_function("compile/expand_fragments", |b| {
        b.iter(|| {
            for rule in rules {
                drop(black_box(
                    expand_fragments(black_box(&rule.pattern), &fragments).unwrap(),
                ));
            }
        });
    });
    group.bench_function("compile/individual_regexes", |b| {
        b.iter(|| {
            for pattern in command_patterns.iter().chain(&redirect_patterns) {
                black_box(validate_rule_pattern(black_box(pattern))).unwrap();
            }
        });
    });
    for (name, patterns) in [
        ("command_set", &command_patterns),
        ("redirect_set", &redirect_patterns),
    ] {
        group.bench_function(format!("compile/{name}"), |b| {
            b.iter(|| drop(black_box(RegexSet::new(black_box(patterns)).unwrap())));
        });
    }
    group.bench_function("compile/engine_and_drop", |b| {
        b.iter_batched(
            || config.clone(),
            |config| drop(black_box(BashRuleEngine::from_config(config).unwrap())),
            BatchSize::PerIteration,
        );
    });
    for (name, command) in COMMANDS {
        group.bench_function(format!("split/{name}"), |b| {
            b.iter(|| {
                drop(black_box(split_command(
                    black_box(command),
                    cwd,
                    &config.bash_path_aliases,
                )))
            });
        });
        group.bench_function(format!("evaluate_warm/{name}"), |b| {
            b.iter(|| {
                // A fresh context keeps filesystem resolution in the measured operation.
                let context = EvaluationContext::new(cwd, None);
                drop(black_box(engine.evaluate_sync(
                    black_box(command),
                    &context,
                    EvaluationPurpose::Diagnostics,
                )));
            });
        });
        let context = EvaluationContext::new(cwd, None);
        let evaluation = engine.evaluate_sync(command, &context, EvaluationPurpose::Diagnostics);
        group.bench_function(format!("trace/{name}"), |b| {
            b.iter(|| {
                drop(black_box(command_trace(
                    black_box(command),
                    black_box(&evaluation),
                )))
            });
        });
        let trace = command_trace(command, &evaluation);
        group.bench_function(format!("json/{name}"), |b| {
            b.iter(|| {
                drop(black_box(
                    serde_json::to_string_pretty(black_box(&trace)).unwrap(),
                ))
            });
        });
    }
    for (name, threads) in [("default", None), ("one_worker", Some(1))] {
        group.bench_function(format!("startup/runtime_{name}"), |b| {
            b.iter(|| {
                let mut builder = Builder::new_multi_thread();
                builder.enable_all();
                if let Some(threads) = threads {
                    builder.worker_threads(threads);
                }
                drop(black_box(builder.build().unwrap()));
            });
        });
    }
    group.finish();
    criterion.final_summary();
    Ok(())
}

#[test]
fn benchmark_fixture_is_valid() -> miette::Result<()> {
    let config = toml::from_slice(include_bytes!("../benches/fixtures/bash_rules.toml"))
        .into_diagnostic()?;
    validate_fixture(&config, env!("CARGO_MANIFEST_DIR"))?;
    Ok(())
}
