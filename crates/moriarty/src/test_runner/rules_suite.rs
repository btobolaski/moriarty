use std::{
    collections::HashSet,
    fmt::Write as _,
    num::NonZeroUsize,
    path::{Path, PathBuf},
};

use miette::{IntoDiagnostic, WrapErr};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    hooks::{
        bash_rules::{
            BashRuleEngine, EvaluationContext, EvaluationPurpose, RuleDiagnostic, RuleResult,
        },
        tool_rules::{ToolRuleEngine, ToolRuleResult},
    },
    permission_mode::PermissionMode,
    user_config::{UserConfig, load_user_config_from},
};

const FORMAT_VERSION: u64 = 1;
const CWD_MARKER: &str = "{{cwd}}";

#[derive(Debug, Deserialize)]
#[serde(try_from = "u64")]
enum SuiteFormatVersion {
    V1,
}

impl TryFrom<u64> for SuiteFormatVersion {
    type Error = String;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            FORMAT_VERSION => Ok(Self::V1),
            _ => Err(format!(
                "Unsupported rule suite format_version {value}; expected {FORMAT_VERSION}"
            )),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Suite {
    format_version: SuiteFormatVersion,
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    name: String,
    request: Request,
    expect: Expectation,
    modes: Option<Vec<PermissionMode>>,
    cwd: Option<PathBuf>,
    reason: Option<String>,
    #[serde(default)]
    expand_cwd: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Bash { command: String },
    Tool { tool: String, input_json: String },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Expectation {
    Allowed,
    Ask,
    Denied,
    NoMatch,
    NotAutoAllowed,
}

#[derive(Debug)]
struct PreparedSuite {
    source_case_count: NonZeroUsize,
    evaluation_count: NonZeroUsize,
    rows: Vec<PreparedRow>,
}

struct ValidatedCwd {
    path: PathBuf,
    value: String,
}

#[derive(Debug)]
struct PreparedRow {
    id: String,
    name: String,
    mode: Option<PermissionMode>,
    cwd: String,
    request: PreparedRequest,
    expect: Expectation,
    reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PreparedRequest {
    Bash { command: String },
    Tool { tool: String, input: Value },
}

// The source distinction is only needed in memory; report JSON exposes the engine result directly.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(untagged)]
enum ActualResult {
    Bash(RuleResult),
    Tool(ToolRuleResult),
}

#[derive(Debug, Serialize)]
pub(super) struct RulesSuiteRow {
    id: String,
    name: String,
    mode: Option<PermissionMode>,
    effective_cwd: String,
    request: PreparedRequest,
    expected: Expectation,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_reason: Option<String>,
    actual: ActualResult,
    passed: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct RulesSuiteReport {
    format_version: u64,
    source_case_count: NonZeroUsize,
    evaluation_count: NonZeroUsize,
    passed_count: usize,
    failed_count: usize,
    rows: Vec<RulesSuiteRow>,
}

impl RulesSuiteReport {
    pub(super) fn has_failures(&self) -> bool {
        self.failed_count != 0
    }

    pub(super) fn render(&self, json: bool) -> miette::Result<String> {
        if json {
            return serde_json::to_string(self)
                .into_diagnostic()
                .wrap_err("Failed to serialize rule suite report")
                .map(|mut output| {
                    output.push('\n');
                    output
                });
        }

        let mut output = String::new();
        macro_rules! line {
            ($($args:tt)*) => {
                writeln!(output, $($args)*).expect("writing to a String cannot fail")
            };
        }
        for row in self.rows.iter().filter(|row| !row.passed) {
            let id = render_row_value(&row.id)?;
            let name = render_row_value(&row.name)?;
            let cwd = render_row_value(&row.effective_cwd)?;
            let request = render_row_value(&row.request)?;
            let expected = render_row_value(&row.expected)?;
            let actual = render_row_value(&row.actual)?;
            line!("FAIL {id} ({name})");
            line!(
                "  mode: {}",
                row.mode
                    .map_or_else(|| "mode-less".to_string(), |mode| mode.to_string())
            );
            line!("  cwd: {cwd}");
            line!("  request: {request}");
            line!("  expected: {expected}");
            if let Some(reason) = &row.expected_reason {
                let reason = render_row_value(reason)?;
                line!("  expected reason: {reason}");
            }
            line!("  actual: {actual}");
            line!();
        }
        line!(
            "{} cases, {} evaluations: {} passed, {} failed",
            self.source_case_count,
            self.evaluation_count,
            self.passed_count,
            self.failed_count
        );
        Ok(output)
    }
}

fn render_row_value(value: &impl Serialize) -> miette::Result<String> {
    serde_json::to_string(value)
        .into_diagnostic()
        .wrap_err("Failed to serialize rule suite row")
}

struct Engines {
    bash: BashRuleEngine,
    tool: ToolRuleEngine,
}

pub(super) async fn run_rules_suite(
    suite_path: &Path,
    config_path: &Path,
    invocation_cwd: &Path,
    cwd: Option<&Path>,
) -> miette::Result<RulesSuiteReport> {
    let default_cwd = validate_cwd(resolve_cwd(invocation_cwd, cwd), "default evaluation cwd")?;
    let source = tokio::fs::read_to_string(suite_path)
        .await
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to read rule suite: {}", suite_path.display()))?;
    let suite = parse_suite(&source)
        .wrap_err_with(|| format!("Failed to parse rule suite: {}", suite_path.display()))?;
    let prepared = prepare_suite(suite, &default_cwd)?;

    let config = load_user_config_from(Some(config_path)).await?;
    let engines = Engines::compile(config)?;
    evaluate_suite(prepared, &engines).await
}

fn parse_suite(source: &str) -> miette::Result<Suite> {
    toml::from_str(source).into_diagnostic()
}

fn resolve_cwd(base: &Path, cwd: Option<&Path>) -> PathBuf {
    match cwd {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => base.join(path),
        None => base.to_path_buf(),
    }
}

fn validate_cwd(path: PathBuf, context: &str) -> miette::Result<ValidatedCwd> {
    let value = path
        .to_str()
        .ok_or_else(|| miette::miette!("{context} is not valid UTF-8: {}", path.display()))?
        .to_string();
    if !path.is_dir() {
        return Err(miette::miette!(
            "{context} is not an existing directory: {}",
            path.display()
        ));
    }
    Ok(ValidatedCwd { path, value })
}

fn prepare_suite(suite: Suite, default_cwd: &ValidatedCwd) -> miette::Result<PreparedSuite> {
    let Suite {
        format_version: SuiteFormatVersion::V1,
        cases,
    } = suite;
    let source_case_count = NonZeroUsize::new(cases.len())
        .ok_or_else(|| miette::miette!("Rule suite must contain at least one case"))?;
    let mut ids = HashSet::new();
    let mut rows = Vec::new();
    for case in cases {
        if case.id.is_empty() {
            return Err(miette::miette!("Rule suite case id must not be empty"));
        }
        if !ids.insert(case.id.clone()) {
            return Err(miette::miette!(
                "Rule suite case id '{}' is duplicated",
                case.id
            ));
        }
        if case.name.is_empty() {
            return Err(miette::miette!(
                "Rule suite case '{}' name must not be empty",
                case.id
            ));
        }
        let modes = validate_modes(&case)?;
        if case.reason.is_some() && case.expect != Expectation::Denied {
            return Err(miette::miette!(
                "Rule suite case '{}' may use reason only with expect = 'denied'",
                case.id
            ));
        }
        let mut request = prepare_request(case.request, &case.id)?;
        let cwd = match case.cwd.as_deref() {
            Some(override_cwd) => {
                validate_cwd(
                    resolve_cwd(&default_cwd.path, Some(override_cwd)),
                    &format!("Rule suite case '{}' cwd", case.id),
                )?
                .value
            }
            None => default_cwd.value.clone(),
        };
        expand_request_cwd(&mut request, case.expand_cwd, &cwd, &case.id)?;

        for mode in modes {
            rows.push(PreparedRow {
                id: case.id.clone(),
                name: case.name.clone(),
                mode,
                cwd: cwd.clone(),
                request: request.clone(),
                expect: case.expect,
                reason: case.reason.clone(),
            });
        }
    }

    let evaluation_count = NonZeroUsize::new(rows.len())
        .expect("validated nonempty cases and modes always produce evaluations");
    Ok(PreparedSuite {
        source_case_count,
        evaluation_count,
        rows,
    })
}

fn validate_modes(case: &Case) -> miette::Result<Vec<Option<PermissionMode>>> {
    let Some(modes) = &case.modes else {
        return Ok(vec![None]);
    };
    if modes.is_empty() {
        return Err(miette::miette!(
            "Rule suite case '{}' modes must not be empty",
            case.id
        ));
    }
    let mut seen = HashSet::new();
    for mode in modes {
        if !seen.insert(*mode) {
            return Err(miette::miette!(
                "Rule suite case '{}' repeats mode '{mode}'",
                case.id
            ));
        }
    }
    Ok(modes.iter().copied().map(Some).collect())
}

fn prepare_request(request: Request, case_id: &str) -> miette::Result<PreparedRequest> {
    match request {
        Request::Bash { command } => {
            if command.is_empty() {
                return Err(miette::miette!(
                    "Rule suite case '{case_id}' Bash command must not be empty"
                ));
            }
            Ok(PreparedRequest::Bash { command })
        }
        Request::Tool { tool, input_json } => {
            if tool.is_empty() {
                return Err(miette::miette!(
                    "Rule suite case '{case_id}' tool name must not be empty"
                ));
            }
            if tool == "Bash" {
                return Err(miette::miette!(
                    "Rule suite case '{case_id}' must use a bash request for tool 'Bash'"
                ));
            }
            let input = serde_json::from_str(&input_json)
                .into_diagnostic()
                .wrap_err_with(|| format!("Rule suite case '{case_id}' has invalid input_json"))?;
            Ok(PreparedRequest::Tool { tool, input })
        }
    }
}

fn expand_request_cwd(
    request: &mut PreparedRequest,
    expand_cwd: bool,
    cwd: &str,
    case_id: &str,
) -> miette::Result<()> {
    if !expand_cwd {
        return Ok(());
    }
    let replaced = match request {
        PreparedRequest::Bash { command } if command.contains(CWD_MARKER) => {
            *command = command.replace(CWD_MARKER, cwd);
            true
        }
        PreparedRequest::Tool { input, .. } => replace_cwd_values(input, cwd),
        PreparedRequest::Bash { .. } => false,
    };
    if !replaced {
        return Err(miette::miette!(
            "Rule suite case '{case_id}' enables expand_cwd but has no {CWD_MARKER} marker in the request"
        ));
    }
    Ok(())
}

fn replace_cwd_values(value: &mut Value, cwd: &str) -> bool {
    match value {
        Value::String(text) => {
            if text.contains(CWD_MARKER) {
                *text = text.replace(CWD_MARKER, cwd);
                true
            } else {
                false
            }
        }
        Value::Array(values) => {
            let mut found = false;
            for value in values {
                found |= replace_cwd_values(value, cwd);
            }
            found
        }
        Value::Object(values) => {
            let mut found = false;
            for value in values.values_mut() {
                found |= replace_cwd_values(value, cwd);
            }
            found
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

impl Engines {
    fn compile(mut config: UserConfig) -> miette::Result<Self> {
        let tool_rules = config.tool_rules.take().unwrap_or_default();
        let fragments = config.pattern_fragments.clone();
        let (bash, bash_diagnostics) = BashRuleEngine::from_config_with_diagnostics(config)?;
        let (tool, tool_diagnostics) =
            ToolRuleEngine::compile_with_diagnostics(tool_rules, fragments);

        if !bash_diagnostics.is_empty() || !tool_diagnostics.is_empty() {
            let diagnostics = bash_diagnostics
                .iter()
                .map(|diagnostic| format_diagnostic("bash", diagnostic))
                .chain(
                    tool_diagnostics
                        .iter()
                        .map(|diagnostic| format_diagnostic("tool", diagnostic)),
                )
                .collect::<Vec<_>>()
                .join("\n");
            return Err(miette::miette!(
                "Candidate policy contains rules that could not be compiled:\n{diagnostics}"
            ));
        }

        Ok(Self { bash, tool })
    }
}

fn format_diagnostic(rule_kind: &str, diagnostic: &RuleDiagnostic) -> String {
    let pattern = if diagnostic.pattern.is_empty() {
        String::new()
    } else {
        format!(" (pattern {:?})", diagnostic.pattern)
    };
    format!(
        "{rule_kind} rule {:?}{pattern}: {}",
        diagnostic.rule_name, diagnostic.message
    )
}

async fn evaluate_suite(
    prepared: PreparedSuite,
    engines: &Engines,
) -> miette::Result<RulesSuiteReport> {
    let source_case_count = prepared.source_case_count;
    let evaluation_count = prepared.evaluation_count;
    let mut rows = Vec::with_capacity(evaluation_count.get());
    for row in prepared.rows {
        let actual = match &row.request {
            PreparedRequest::Bash { command } => {
                let context = EvaluationContext::new(&row.cwd, row.mode);
                engines
                    .bash
                    .evaluate_sync(command, &context, EvaluationPurpose::Decision)
                    .rule_result()
                    .into()
            }
            PreparedRequest::Tool { tool, input } => engines
                .tool
                .apply_rules_checked(tool, input, &row.cwd, row.mode)
                .await?
                .into(),
        };
        let passed = assertion_passes(row.expect, row.reason.as_deref(), &actual);
        rows.push(RulesSuiteRow {
            id: row.id,
            name: row.name,
            mode: row.mode,
            effective_cwd: row.cwd,
            request: row.request,
            expected: row.expect,
            expected_reason: row.reason,
            actual,
            passed,
        });
    }

    let passed_count = rows.iter().filter(|row| row.passed).count();
    Ok(RulesSuiteReport {
        format_version: FORMAT_VERSION,
        source_case_count,
        evaluation_count,
        passed_count,
        failed_count: rows.len() - passed_count,
        rows,
    })
}

fn assertion_passes(
    expected: Expectation,
    expected_reason: Option<&str>,
    actual: &ActualResult,
) -> bool {
    let result_matches = match (actual, expected) {
        (
            ActualResult::Bash(RuleResult::Allowed { .. } | RuleResult::ArgumentFiltered { .. })
            | ActualResult::Tool(ToolRuleResult::Allowed { .. }),
            Expectation::Allowed,
        )
        | (
            ActualResult::Bash(RuleResult::Asked { .. })
            | ActualResult::Tool(ToolRuleResult::Asked { .. }),
            Expectation::Ask,
        )
        | (
            ActualResult::Bash(RuleResult::Denied { .. })
            | ActualResult::Tool(ToolRuleResult::Denied { .. }),
            Expectation::Denied,
        )
        | (
            ActualResult::Bash(RuleResult::NoMatch) | ActualResult::Tool(ToolRuleResult::NoMatch),
            Expectation::NoMatch,
        )
        | (
            ActualResult::Bash(
                RuleResult::Asked { .. } | RuleResult::Denied { .. } | RuleResult::NoMatch,
            )
            | ActualResult::Tool(
                ToolRuleResult::Asked { .. }
                | ToolRuleResult::Denied { .. }
                | ToolRuleResult::NoMatch,
            ),
            Expectation::NotAutoAllowed,
        ) => true,
        (
            ActualResult::Bash(
                RuleResult::Allowed { .. }
                | RuleResult::Denied { .. }
                | RuleResult::Modified { .. }
                | RuleResult::Asked { .. }
                | RuleResult::ArgumentFiltered { .. }
                | RuleResult::NoMatch,
            )
            | ActualResult::Tool(
                ToolRuleResult::Allowed { .. }
                | ToolRuleResult::Denied { .. }
                | ToolRuleResult::Asked { .. }
                | ToolRuleResult::NoMatch,
            ),
            _,
        ) => false,
    };
    result_matches
        && expected_reason.is_none_or(|expected_reason| match actual {
            ActualResult::Bash(RuleResult::Denied { reason, .. })
            | ActualResult::Tool(ToolRuleResult::Denied { reason, .. }) => {
                reason == expected_reason
            }
            ActualResult::Bash(
                RuleResult::Allowed { .. }
                | RuleResult::Modified { .. }
                | RuleResult::Asked { .. }
                | RuleResult::ArgumentFiltered { .. }
                | RuleResult::NoMatch,
            )
            | ActualResult::Tool(
                ToolRuleResult::Allowed { .. }
                | ToolRuleResult::Asked { .. }
                | ToolRuleResult::NoMatch,
            ) => false,
        })
}

impl From<RuleResult> for ActualResult {
    fn from(result: RuleResult) -> Self {
        Self::Bash(result)
    }
}

impl From<ToolRuleResult> for ActualResult {
    fn from(result: ToolRuleResult) -> Self {
        Self::Tool(result)
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;

    #[test]
    fn report_rendering_preserves_private_row_invariants() {
        let count = NonZeroUsize::new(2).unwrap();
        let mut report = RulesSuiteReport {
            format_version: FORMAT_VERSION,
            source_case_count: count,
            evaluation_count: count,
            passed_count: 1,
            failed_count: 1,
            rows: vec![
                RulesSuiteRow {
                    id: "pass".to_string(),
                    name: "passing row".to_string(),
                    mode: Some(PermissionMode::Default),
                    effective_cwd: "/work".to_string(),
                    request: PreparedRequest::Tool {
                        tool: "Read".to_string(),
                        input: Value::Null,
                    },
                    expected: Expectation::NoMatch,
                    expected_reason: None,
                    actual: ActualResult::Tool(ToolRuleResult::NoMatch),
                    passed: true,
                },
                RulesSuiteRow {
                    id: "fail\nrow".to_string(),
                    name: "failing row".to_string(),
                    mode: None,
                    effective_cwd: "/work".to_string(),
                    request: PreparedRequest::Tool {
                        tool: "Write".to_string(),
                        input: Value::Null,
                    },
                    expected: Expectation::Denied,
                    expected_reason: Some(String::new()),
                    actual: ActualResult::Tool(ToolRuleResult::Denied {
                        rule_name: "deny".to_string(),
                        reason: "actual".to_string(),
                    }),
                    passed: false,
                },
            ],
        };

        assert!(report.has_failures());
        let human = report.render(false).unwrap();
        assert!(human.contains("FAIL \"fail\\nrow\""));
        assert!(human.contains("mode: mode-less"));
        assert!(human.contains("expected reason: \"\""));
        assert!(!human.contains("passing row"));
        report.rows[1].mode = Some(PermissionMode::Default);
        assert!(report.render(false).unwrap().contains("mode: default"));
        let json: Value = serde_json::from_str(&report.render(true).unwrap()).unwrap();
        assert!(json["rows"][0].get("expected_reason").is_none());
        assert_eq!(json["rows"][1]["expected_reason"], "");
    }
}

#[cfg(test)]
#[path = "rules_suite_tests/mod.rs"]
mod tests;
