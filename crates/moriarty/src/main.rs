use std::{
    env,
    path::{Path, PathBuf},
};

use clap::{Args, Parser, Subcommand};
use hooks::result::PreToolResult;
use mcp::McpServers;
use miette::{IntoDiagnostic, WrapErr};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

mod api_pricing;
mod approval_tui;
mod checks;
mod cost_report;
mod hashing;
mod hooks;
mod mcp;
mod persistence;
mod pi_cost;
mod project_config;
mod repository;
mod rules;
#[cfg(test)]
mod test_helpers;
mod test_runner;
mod tui;
mod user_config;

#[tokio::main]
async fn main() -> miette::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::ApiPricing { dir, cost_args } => {
            init_cost_report_tracing();
            let dir = resolve_claude_logs_dir(dir)?;
            let timezone = parse_date_timezone(&cost_args.timezone)?;
            let filter = cost_args.time_filter(timezone)?;
            let report_mode = cost_args.report_mode();
            print_time_range_filter(&filter, timezone);
            api_pricing::run(
                &dir,
                timezone,
                cost_args.conversations,
                &filter,
                report_mode,
            )
            .await?;
        }
        Command::Graphs { subcommand } => match subcommand {
            GraphsCommand::Claude { dir, cost_args } => {
                init_cost_report_tracing();
                let dir = resolve_claude_logs_dir(dir)?;
                let timezone = parse_date_timezone(&cost_args.timezone)?;
                let filter = cost_args.time_filter(timezone)?;
                let report_mode = cost_args.report_mode();
                print_time_range_filter(&filter, timezone);
                api_pricing::run_graphs(
                    &dir,
                    timezone,
                    cost_args.conversations,
                    &filter,
                    report_mode,
                )
                .await?;
            }
            GraphsCommand::Pi { dir, cost_args } => {
                init_cost_report_tracing();
                let dir = resolve_pi_sessions_dir(dir)?;
                let timezone = parse_date_timezone(&cost_args.timezone)?;
                let filter = cost_args.time_filter(timezone)?;
                let report_mode = cost_args.report_mode();
                print_time_range_filter(&filter, timezone);
                pi_cost::run_graphs(
                    &dir,
                    timezone,
                    cost_args.conversations,
                    &filter,
                    report_mode,
                )
                .await?;
            }
        },
        Command::Pi { subcommand } => match subcommand {
            PiCommand::Cost { dir, cost_args } => {
                init_cost_report_tracing();
                let dir = resolve_pi_sessions_dir(dir)?;
                let timezone = parse_date_timezone(&cost_args.timezone)?;
                let filter = cost_args.time_filter(timezone)?;
                let report_mode = cost_args.report_mode();
                print_time_range_filter(&filter, timezone);
                pi_cost::run(
                    &dir,
                    timezone,
                    cost_args.conversations,
                    &filter,
                    report_mode,
                )
                .await?;
            }
        },
        Command::Mcp { server } => {
            server.run().await?;
        }
        Command::ApproveProject { project_dir } => {
            let terminal = ratatui::init();
            let app = approval_tui::ApprovalApp::new(project_dir).await?;
            let approved = app.run(terminal).await;

            ratatui::restore();

            match approved {
                Ok(true) => {
                    println!("✓ Project tools approved successfully!");
                    Ok(())
                }
                Ok(false) => {
                    eprintln!("✗ Approval cancelled");
                    std::process::exit(1);
                }
                Err(e) => Err(e),
            }?;
        }
        Command::Hooks { subcommand } => {
            hooks::exec_hooks(subcommand).await?;
        }
        Command::Rules { subcommand } => {
            rules::exec_rules(subcommand).await?;
        }
        Command::Test { subcommand } => {
            test_runner::exec_test(subcommand).await?;
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Args)]
struct CostCommandArgs {
    /// Timezone for interpreting date-only and naive datetime inputs (local or utc)
    #[arg(long, default_value = "local")]
    timezone: String,
    /// Aggregate by conversation/session instead of by date
    #[arg(long)]
    conversations: bool,
    /// Show token counts instead of dollar costs
    #[arg(long)]
    tokens: bool,
    /// Start time for filtering messages (ISO 8601 format, e.g., "2025-01-01T00:00:00Z" or "2025-01-01")
    /// Date-only and naive datetime inputs use the command --timezone (default: local)
    #[arg(long, value_name = "DATETIME")]
    start_time: Option<String>,
    /// End time for filtering messages (ISO 8601 format, e.g., "2025-01-01T23:59:59Z" or "2025-01-01")
    /// Date-only and naive datetime inputs use the command --timezone (default: local)
    #[arg(long, value_name = "DATETIME")]
    end_time: Option<String>,
}

impl CostCommandArgs {
    fn time_filter(
        &self,
        timezone: cost_report::DateTimezone,
    ) -> miette::Result<cost_report::TimeRangeFilter> {
        cost_report::TimeRangeFilter::new(self.start_time.clone(), self.end_time.clone(), timezone)
    }

    fn report_mode(&self) -> cost_report::ReportMode {
        if self.tokens {
            cost_report::ReportMode::Tokens
        } else {
            cost_report::ReportMode::Cost
        }
    }
}

fn init_cost_report_tracing() {
    // Default to info-level so cost_analyzer's parse-failure and
    // unrecognized-model events reach the operator without setting RUST_LOG.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .try_init();
}

fn parse_date_timezone(timezone: &str) -> miette::Result<cost_report::DateTimezone> {
    cost_report::parse_timezone(timezone)
}

fn print_time_range_filter(
    filter: &cost_report::TimeRangeFilter,
    timezone: cost_report::DateTimezone,
) {
    if filter.is_unrestricted() {
        return;
    }

    println!("Applying time range filter:");
    if let Some(start) = filter.start {
        println!("  Start: {}", timezone.display_timestamp(&start));
    }
    if let Some(end) = filter.end {
        println!("  End:   {}", timezone.display_timestamp(&end));
    }
    println!();
}

fn resolve_pi_sessions_dir(override_dir: Option<PathBuf>) -> miette::Result<PathBuf> {
    resolve_logs_dir(override_dir, ".pi/agent/sessions", "pi sessions")
}

fn resolve_claude_logs_dir(override_dir: Option<PathBuf>) -> miette::Result<PathBuf> {
    resolve_logs_dir(override_dir, ".claude/projects", "Claude logs")
}

fn resolve_logs_dir(
    override_dir: Option<PathBuf>,
    home_relative_default: &str,
    kind: &str,
) -> miette::Result<PathBuf> {
    let dir = if let Some(dir) = override_dir {
        dir
    } else {
        let Some(home) = env::var_os("HOME") else {
            return Err(miette::miette!(
                "HOME is not set; pass --dir to specify the {kind} directory"
            ));
        };

        PathBuf::from(home).join(home_relative_default)
    };

    validate_logs_dir(&dir, kind)?;
    Ok(dir)
}

fn validate_logs_dir(dir: &Path, kind: &str) -> miette::Result<()> {
    if !dir
        .try_exists()
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to check {kind} directory '{}'", dir.display()))?
    {
        return Err(miette::miette!(
            "{kind} directory '{}' does not exist",
            dir.display()
        ));
    }

    if !dir.is_dir() {
        return Err(miette::miette!(
            "{kind} path '{}' is not a directory",
            dir.display()
        ));
    }

    Ok(())
}

#[derive(Debug, Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    ApiPricing {
        /// The directory to analyze for API usage (defaults to ~/.claude/projects)
        #[arg(short, long)]
        dir: Option<PathBuf>,
        #[command(flatten)]
        cost_args: CostCommandArgs,
    },
    /// Render chart-focused cost/token graphs
    Graphs {
        #[command(subcommand)]
        subcommand: GraphsCommand,
    },
    /// Analyze pi session logs
    Pi {
        #[command(subcommand)]
        subcommand: PiCommand,
    },
    /// Runs one of the mcp servers
    Mcp {
        #[command(subcommand)]
        server: McpServers,
    },
    /// Approve project-configured tools and checks for the project-tools MCP server
    ApproveProject {
        /// The project directory containing .config/tools.toml
        project_dir: PathBuf,
    },
    /// Execute and test hooks
    Hooks {
        #[command(subcommand)]
        subcommand: HooksCommand,
    },
    /// Inspect, validate, and author bash/tool permission rules
    Rules {
        #[command(subcommand)]
        subcommand: RulesCommand,
    },
    /// Run project tests and tools
    Test {
        #[command(subcommand)]
        subcommand: TestCommand,
    },
}

#[derive(Debug, Subcommand)]
enum RulesCommand {
    /// Report rules the hook silently ignores; with --strict, also likely-shadowed/over-broad rules
    Lint {
        /// Custom config file path (defaults to ~/.config/moriarty/tool_rules.toml)
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Output as JSON instead of human-readable text
        #[arg(long)]
        json: bool,
        /// Also warn about likely-shadowed rules and over-broad Allow rules
        #[arg(long)]
        strict: bool,
    },
    /// List the merged pattern fragments (built-in defaults plus user-defined) usable in patterns
    ListFragments {
        /// Custom config file path (defaults to ~/.config/moriarty/tool_rules.toml)
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Output as JSON instead of a table
        #[arg(long)]
        json: bool,
    },
    /// Print a canonical example tool_rules.toml covering every rule and action variant
    Schema {
        /// Output the parsed config as JSON instead of TOML
        #[arg(long)]
        json: bool,
    },
    /// Print a paste-ready starter pack of allow-rules for common read-only commands
    Starter {
        /// Output as JSON instead of TOML
        #[arg(long)]
        json: bool,
    },
    /// Suggest anchored rules for commands the hook frequently prompted on (from the hook logs)
    Suggest {
        /// Directory containing hook logs (defaults to ~/.local/state/moriarty/hooks)
        #[arg(short, long)]
        dir: Option<PathBuf>,
        /// Only include results at or after this time (ISO 8601; date-only and naive inputs use --timezone)
        #[arg(long, value_name = "DATETIME")]
        start_time: Option<String>,
        /// Only include results before this time (ISO 8601; date-only and naive inputs use --timezone)
        #[arg(long, value_name = "DATETIME")]
        end_time: Option<String>,
        /// Timezone for interpreting date-only and naive datetime inputs (local or utc)
        #[arg(long, default_value = "local")]
        timezone: String,
        /// Which recorded outcome to mine for suggestions (ask or deny)
        #[arg(long, value_enum, default_value = "ask")]
        result: PreToolResult,
        /// Maximum number of suggestions to emit
        #[arg(long, default_value_t = 10)]
        limit: usize,
        /// Only suggest commands seen at least this many times
        #[arg(long, default_value_t = 2)]
        min_count: u64,
        /// Generated pattern shape: a fully-literal exact match per leaf, a program-name prefix,
        /// or a fuzzy per-program generalization over observed subcommands
        #[arg(long = "match", value_enum, default_value = "exact")]
        match_mode: rules::MatchMode,
        /// Action for generated rules (defaults to ask, or deny when --result deny)
        #[arg(long, value_enum)]
        action: Option<rules::SuggestAction>,
        /// Mine every recorded line, not just those produced by the rules currently in force
        #[arg(long)]
        all_rules: bool,
        /// Mine only lines recorded under this exact rule-set hash (default: the active config's hash)
        #[arg(long, value_name = "HASH")]
        rules_hash: Option<String>,
        /// Output as JSON ([{rule, count, observed_commands}]) instead of TOML
        #[arg(long)]
        json: bool,
    },
    /// Re-evaluate recorded Bash decisions against a candidate config and report divergences
    Replay {
        /// Directory containing hook logs (defaults to ~/.local/state/moriarty/hooks)
        #[arg(short, long)]
        dir: Option<PathBuf>,
        /// Candidate config to evaluate against (defaults to ~/.config/moriarty/tool_rules.toml)
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Only replay commands recorded at or after this time (ISO 8601; date-only and naive inputs use --timezone)
        #[arg(long, value_name = "DATETIME")]
        start_time: Option<String>,
        /// Only replay commands recorded before this time (ISO 8601; date-only and naive inputs use --timezone)
        #[arg(long, value_name = "DATETIME")]
        end_time: Option<String>,
        /// Timezone for interpreting date-only and naive datetime inputs (local or utc)
        #[arg(long, default_value = "local")]
        timezone: String,
        /// Only replay commands whose recorded outcome was this (e.g. allow to guard auto-approvals)
        #[arg(long, value_enum)]
        result: Option<PreToolResult>,
        /// Replay every recorded line, not just those produced by the rules currently in force
        #[arg(long)]
        all_rules: bool,
        /// Replay only lines recorded under this exact rule-set hash (default: the active config's hash)
        #[arg(long, value_name = "HASH")]
        rules_hash: Option<String>,
        /// Output as JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum GraphsCommand {
    /// Render Claude/API usage graphs
    Claude {
        /// The directory to analyze for API usage (defaults to ~/.claude/projects)
        #[arg(short, long)]
        dir: Option<PathBuf>,
        #[command(flatten)]
        cost_args: CostCommandArgs,
    },
    /// Render pi session usage graphs
    Pi {
        /// The directory to analyze for pi session usage
        #[arg(short, long)]
        dir: Option<PathBuf>,
        #[command(flatten)]
        cost_args: CostCommandArgs,
    },
}

#[derive(Debug, Subcommand)]
enum PiCommand {
    /// Analyze pi session cost reports
    Cost {
        /// The directory to analyze for pi session usage
        #[arg(short, long)]
        dir: Option<PathBuf>,
        #[command(flatten)]
        cost_args: CostCommandArgs,
    },
}

#[derive(Debug, Subcommand)]
enum HooksCommand {
    /// Execute hook with input and log the results
    Exec,
    /// Generate a JSON report of recorded PreToolUse hook results
    Report(HooksReportArgs),
}

#[derive(Debug, Args)]
struct HooksReportArgs {
    /// Directory containing hook logs (defaults to ~/.local/state/moriarty/hooks)
    #[arg(short, long)]
    dir: Option<PathBuf>,
    /// Only include results at or after this time (ISO 8601, e.g. "2026-01-01"; date-only and naive inputs use --timezone)
    #[arg(long, value_name = "DATETIME")]
    start_time: Option<String>,
    /// Only include results before this time (ISO 8601, e.g. "2026-01-01"; date-only and naive inputs use --timezone)
    #[arg(long, value_name = "DATETIME")]
    end_time: Option<String>,
    /// Only include calls to this tool (exact match)
    #[arg(long)]
    tool: Option<String>,
    /// Only include results with this outcome
    #[arg(long, value_enum)]
    result: Option<PreToolResult>,
    /// Timezone for interpreting date-only and naive datetime inputs (local or utc)
    #[arg(long, default_value = "local")]
    timezone: String,
}

#[derive(Debug, Subcommand)]
enum TestCommand {
    /// Run all configured project tools in parallel
    ProjectTools {
        /// The project directory containing .config/tools.toml
        #[arg(default_value = ".")]
        project_dir: PathBuf,
    },
    /// Run all configured project checks in parallel
    Checks {
        /// The project directory containing .config/tools.toml
        #[arg(default_value = ".")]
        project_dir: PathBuf,
    },
    /// Test bash command against configured rules
    BashRules {
        /// Command to test (if not provided, reads from stdin)
        command: Option<String>,

        /// Custom config file path (defaults to ~/.config/moriarty/tool_rules.toml)
        #[arg(short, long)]
        config: Option<PathBuf>,

        /// Output as JSON instead of pretty-printed
        #[arg(long)]
        json: bool,

        /// Show how the command splits into leaves and which rule matched each, then the decision
        #[arg(long)]
        explain: bool,

        /// Simulate the hook working directory for path normalization (defaults to the process cwd)
        #[arg(long)]
        cwd: Option<PathBuf>,
    },
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        ffi::OsString,
        path::{Path, PathBuf},
    };

    use clap::Parser;
    use tempfile::TempDir;

    use super::{
        Cli, Command, GraphsCommand, HooksCommand, PiCommand, RulesCommand, parse_date_timezone,
        resolve_claude_logs_dir, resolve_pi_sessions_dir,
    };
    use crate::{cost_report::DateTimezone, test_helpers::TestEnvVarGuard};

    fn with_home<R>(home: Option<&Path>, f: impl FnOnce() -> R) -> R {
        let _guard = match home {
            Some(home) => TestEnvVarGuard::set("HOME", home),
            None => TestEnvVarGuard::unset("HOME"),
        };
        f()
    }

    #[test]
    fn resolve_pi_sessions_dir_prefers_override() {
        let temp = TempDir::new().unwrap();
        let override_dir = temp.path().join("custom/pi/sessions");
        std::fs::create_dir_all(&override_dir).unwrap();

        let resolved = with_home(None, || {
            resolve_pi_sessions_dir(Some(override_dir.clone())).unwrap()
        });

        assert_eq!(resolved, override_dir);
    }

    #[test]
    fn resolve_pi_sessions_dir_uses_home_default_path() {
        let home = TempDir::new().unwrap();
        let expected = home.path().join(".pi/agent/sessions");
        std::fs::create_dir_all(&expected).unwrap();

        let resolved = with_home(Some(home.path()), || resolve_pi_sessions_dir(None).unwrap());

        assert_eq!(resolved, expected);
    }

    #[test]
    fn resolve_pi_sessions_dir_errors_when_home_is_missing() {
        let error = with_home(None, || resolve_pi_sessions_dir(None).unwrap_err());

        assert!(
            error.to_string().contains("HOME is not set"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn resolve_pi_sessions_dir_errors_when_override_is_missing() {
        let temp = TempDir::new().unwrap();
        let missing = temp.path().join("missing");

        let error = resolve_pi_sessions_dir(Some(missing.clone())).unwrap_err();

        assert!(
            error.to_string().contains("does not exist"),
            "unexpected error: {error}"
        );
        assert!(error.to_string().contains(&missing.display().to_string()));
    }

    #[test]
    fn resolve_pi_sessions_dir_errors_when_override_is_file() {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("sessions.jsonl");
        std::fs::write(&file, "[]").unwrap();

        let error = resolve_pi_sessions_dir(Some(file.clone())).unwrap_err();

        assert!(
            error.to_string().contains("is not a directory"),
            "unexpected error: {error}"
        );
        assert!(error.to_string().contains(&file.display().to_string()));
    }

    #[test]
    fn with_home_restores_original_value() {
        let original = env::var_os("HOME");
        let temp = TempDir::new().unwrap();

        with_home(Some(temp.path()), || {
            assert_eq!(env::var_os("HOME"), Some(OsString::from(temp.path())));
        });

        assert_eq!(env::var_os("HOME"), original);
    }

    #[test]
    fn parse_date_timezone_accepts_supported_values_case_insensitively() {
        assert_eq!(parse_date_timezone("local").unwrap(), DateTimezone::Local);
        assert_eq!(parse_date_timezone("UTC").unwrap(), DateTimezone::Utc);
    }

    #[test]
    fn parse_date_timezone_rejects_invalid_values() {
        let error = parse_date_timezone("pst").unwrap_err();

        assert!(
            error.to_string().contains("Invalid timezone"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn cli_parses_nested_pi_cost_command() {
        let cli = Cli::try_parse_from([
            "moriarty",
            "pi",
            "cost",
            "--dir",
            "logs/pi",
            "--timezone",
            "utc",
            "--conversations",
            "--tokens",
            "--start-time",
            "2025-01-01",
            "--end-time",
            "2025-01-02",
        ])
        .unwrap();

        match cli.command {
            Command::Pi {
                subcommand: PiCommand::Cost { dir, cost_args },
            } => {
                assert_eq!(dir, Some(PathBuf::from("logs/pi")));
                assert_eq!(cost_args.timezone, "utc");
                assert!(cost_args.conversations);
                assert!(cost_args.tokens);
                assert_eq!(cost_args.start_time.as_deref(), Some("2025-01-01"));
                assert_eq!(cost_args.end_time.as_deref(), Some("2025-01-02"));
            }
            other => panic!("expected nested pi cost command, got {other:?}"),
        }
    }

    #[test]
    fn cli_parses_graphs_claude_command() {
        let cli = Cli::try_parse_from([
            "moriarty",
            "graphs",
            "claude",
            "--dir",
            "logs/api",
            "--timezone",
            "utc",
            "--tokens",
        ])
        .unwrap();

        match cli.command {
            Command::Graphs {
                subcommand: GraphsCommand::Claude { dir, cost_args },
            } => {
                assert_eq!(dir, Some(PathBuf::from("logs/api")));
                assert_eq!(cost_args.timezone, "utc");
                assert!(cost_args.tokens);
                assert!(!cost_args.conversations);
            }
            other => panic!("expected graphs claude command, got {other:?}"),
        }
    }

    #[test]
    fn cli_parses_graphs_pi_command() {
        let cli = Cli::try_parse_from([
            "moriarty",
            "graphs",
            "pi",
            "--dir",
            "logs/pi",
            "--conversations",
        ])
        .unwrap();

        match cli.command {
            Command::Graphs {
                subcommand: GraphsCommand::Pi { dir, cost_args },
            } => {
                assert_eq!(dir, Some(PathBuf::from("logs/pi")));
                assert!(cost_args.conversations);
                assert!(!cost_args.tokens);
            }
            other => panic!("expected graphs pi command, got {other:?}"),
        }
    }

    #[test]
    fn resolve_claude_logs_dir_prefers_override() {
        let temp = TempDir::new().unwrap();
        let override_dir = temp.path().join("custom/claude/logs");
        std::fs::create_dir_all(&override_dir).unwrap();

        let resolved = with_home(None, || {
            resolve_claude_logs_dir(Some(override_dir.clone())).unwrap()
        });

        assert_eq!(resolved, override_dir);
    }

    #[test]
    fn resolve_claude_logs_dir_uses_home_default_path() {
        let home = TempDir::new().unwrap();
        let expected = home.path().join(".claude/projects");
        std::fs::create_dir_all(&expected).unwrap();

        let resolved = with_home(Some(home.path()), || resolve_claude_logs_dir(None).unwrap());

        assert_eq!(resolved, expected);
    }

    #[test]
    fn resolve_claude_logs_dir_errors_when_home_is_missing() {
        let error = with_home(None, || resolve_claude_logs_dir(None).unwrap_err());

        assert!(
            error.to_string().contains("HOME is not set"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn resolve_claude_logs_dir_errors_when_default_is_missing() {
        let home = TempDir::new().unwrap();
        let expected = home.path().join(".claude/projects");

        let error = with_home(Some(home.path()), || {
            resolve_claude_logs_dir(None).unwrap_err()
        });

        assert!(
            error.to_string().contains("does not exist"),
            "unexpected error: {error}"
        );
        assert!(
            error.to_string().contains(&expected.display().to_string()),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn resolve_claude_logs_dir_errors_when_override_is_missing() {
        let temp = TempDir::new().unwrap();
        let missing = temp.path().join("missing");

        let error = resolve_claude_logs_dir(Some(missing.clone())).unwrap_err();

        assert!(
            error.to_string().contains("does not exist"),
            "unexpected error: {error}"
        );
        assert!(error.to_string().contains(&missing.display().to_string()));
    }

    #[test]
    fn resolve_claude_logs_dir_errors_when_override_is_file() {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("history.jsonl");
        std::fs::write(&file, "[]").unwrap();

        let error = resolve_claude_logs_dir(Some(file.clone())).unwrap_err();

        assert!(
            error.to_string().contains("is not a directory"),
            "unexpected error: {error}"
        );
        assert!(error.to_string().contains(&file.display().to_string()));
    }

    #[test]
    fn cli_parses_api_pricing_without_dir() {
        let cli = Cli::try_parse_from(["moriarty", "api-pricing"]).unwrap();

        match cli.command {
            Command::ApiPricing { dir, .. } => {
                assert_eq!(dir, None);
            }
            other => panic!("expected api-pricing command, got {other:?}"),
        }
    }

    #[test]
    fn cli_parses_api_pricing_with_dir() {
        let cli = Cli::try_parse_from(["moriarty", "api-pricing", "--dir", "logs/api"]).unwrap();

        match cli.command {
            Command::ApiPricing { dir, .. } => {
                assert_eq!(dir, Some(PathBuf::from("logs/api")));
            }
            other => panic!("expected api-pricing command, got {other:?}"),
        }
    }

    #[test]
    fn cli_parses_graphs_claude_without_dir() {
        let cli = Cli::try_parse_from(["moriarty", "graphs", "claude"]).unwrap();

        match cli.command {
            Command::Graphs {
                subcommand: GraphsCommand::Claude { dir, .. },
            } => {
                assert_eq!(dir, None);
            }
            other => panic!("expected graphs claude command, got {other:?}"),
        }
    }

    #[test]
    fn cli_parses_hooks_report_with_timezone() {
        let cli = Cli::try_parse_from([
            "moriarty",
            "hooks",
            "report",
            "--timezone",
            "utc",
            "--start-time",
            "2026-01-01",
            "--tool",
            "Bash",
        ])
        .unwrap();

        match cli.command {
            Command::Hooks {
                subcommand: HooksCommand::Report(args),
            } => {
                assert_eq!(args.timezone, "utc");
                assert_eq!(args.start_time.as_deref(), Some("2026-01-01"));
                assert_eq!(args.tool.as_deref(), Some("Bash"));
            }
            other => panic!("expected hooks report command, got {other:?}"),
        }
    }

    #[test]
    fn cli_parses_rules_suggest_with_timezone() {
        use crate::rules::MatchMode;

        let cli = Cli::try_parse_from([
            "moriarty",
            "rules",
            "suggest",
            "--timezone",
            "utc",
            "--start-time",
            "2026-01-01",
            "--match",
            "prefix",
        ])
        .unwrap();

        match cli.command {
            Command::Rules {
                subcommand:
                    RulesCommand::Suggest {
                        timezone,
                        start_time,
                        match_mode,
                        ..
                    },
            } => {
                assert_eq!(timezone, "utc");
                assert_eq!(start_time.as_deref(), Some("2026-01-01"));
                assert_eq!(match_mode, MatchMode::Prefix);
            }
            other => panic!("expected rules suggest command, got {other:?}"),
        }
    }

    #[test]
    fn cli_parses_rules_replay_with_timezone() {
        let cli = Cli::try_parse_from([
            "moriarty",
            "rules",
            "replay",
            "--timezone",
            "utc",
            "--end-time",
            "2026-06-01",
        ])
        .unwrap();

        match cli.command {
            Command::Rules {
                subcommand:
                    RulesCommand::Replay {
                        timezone, end_time, ..
                    },
            } => {
                assert_eq!(timezone, "utc");
                assert_eq!(end_time.as_deref(), Some("2026-06-01"));
            }
            other => panic!("expected rules replay command, got {other:?}"),
        }
    }

    #[test]
    fn cli_rejects_flat_pi_cost_spelling() {
        let error = Cli::try_parse_from(["moriarty", "pi-cost"]).unwrap_err();

        assert_eq!(error.kind(), clap::error::ErrorKind::InvalidSubcommand);
    }
}
