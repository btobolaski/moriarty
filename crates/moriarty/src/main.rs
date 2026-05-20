use std::{
    env,
    path::{Path, PathBuf},
};

use clap::{Args, Parser, Subcommand};
use mcp::McpServers;
use miette::{IntoDiagnostic, WrapErr};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod api_pricing;
mod approval_tui;
mod cost_report;
mod hashing;
mod hooks;
mod mcp;
mod persistence;
mod pi_cost;
mod project_config;
mod repository;
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
            let timezone = parse_date_timezone(&cost_args.timezone)?;
            let filter = cost_args.time_filter()?;
            let report_mode = cost_args.report_mode();
            print_time_range_filter(&filter);
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
                let timezone = parse_date_timezone(&cost_args.timezone)?;
                let filter = cost_args.time_filter()?;
                let report_mode = cost_args.report_mode();
                print_time_range_filter(&filter);
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
                let filter = cost_args.time_filter()?;
                let report_mode = cost_args.report_mode();
                print_time_range_filter(&filter);
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
                let filter = cost_args.time_filter()?;
                let report_mode = cost_args.report_mode();
                print_time_range_filter(&filter);
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
        Command::Test { subcommand } => {
            test_runner::exec_test(subcommand).await?;
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Args)]
struct CostCommandArgs {
    /// Timezone to use for date determination (local or utc)
    #[arg(long, default_value = "local")]
    timezone: String,
    /// Aggregate by conversation/session instead of by date
    #[arg(long)]
    conversations: bool,
    /// Show token counts instead of dollar costs
    #[arg(long)]
    tokens: bool,
    /// Start time for filtering messages (ISO 8601 format, e.g., "2025-01-01T00:00:00Z" or "2025-01-01")
    /// If no timezone specified, UTC is assumed
    #[arg(long, value_name = "DATETIME")]
    start_time: Option<String>,
    /// End time for filtering messages (ISO 8601 format, e.g., "2025-01-01T23:59:59Z" or "2025-01-01")
    /// If no timezone specified, UTC is assumed
    #[arg(long, value_name = "DATETIME")]
    end_time: Option<String>,
}

impl CostCommandArgs {
    fn time_filter(&self) -> miette::Result<cost_report::TimeRangeFilter> {
        cost_report::TimeRangeFilter::new(self.start_time.clone(), self.end_time.clone())
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
    match timezone.to_ascii_lowercase().as_str() {
        "local" => Ok(cost_report::DateTimezone::Local),
        "utc" => Ok(cost_report::DateTimezone::Utc),
        _ => Err(miette::miette!(
            "Invalid timezone '{}'. Must be 'local' or 'utc'",
            timezone
        )),
    }
}

fn print_time_range_filter(filter: &cost_report::TimeRangeFilter) {
    if filter.is_unrestricted() {
        return;
    }

    println!("Applying time range filter:");
    if let Some(start) = filter.start {
        println!("  Start: {}", start.to_rfc3339());
    }
    if let Some(end) = filter.end {
        println!("  End:   {}", end.to_rfc3339());
    }
    println!();
}

fn resolve_pi_sessions_dir(override_dir: Option<PathBuf>) -> miette::Result<PathBuf> {
    let dir = if let Some(dir) = override_dir {
        dir
    } else {
        let Some(home) = env::var_os("HOME") else {
            return Err(miette::miette!(
                "HOME is not set; pass --dir to specify the pi sessions directory"
            ));
        };

        PathBuf::from(home).join(".pi/agent/sessions")
    };

    validate_pi_sessions_dir(&dir)?;
    Ok(dir)
}

fn validate_pi_sessions_dir(dir: &Path) -> miette::Result<()> {
    if !dir
        .try_exists()
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to check pi sessions directory '{}'", dir.display()))?
    {
        return Err(miette::miette!(
            "Pi sessions directory '{}' does not exist",
            dir.display()
        ));
    }

    if !dir.is_dir() {
        return Err(miette::miette!(
            "Pi sessions path '{}' is not a directory",
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
        /// The directory to analyze for API usage
        #[arg(short, long)]
        dir: PathBuf,
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
    /// Run project tests and tools
    Test {
        #[command(subcommand)]
        subcommand: TestCommand,
    },
}

#[derive(Debug, Subcommand)]
enum GraphsCommand {
    /// Render Claude/API usage graphs
    Claude {
        /// The directory to analyze for API usage
        #[arg(short, long)]
        dir: PathBuf,
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
        parse_date_timezone, resolve_pi_sessions_dir, Cli, Command, GraphsCommand, PiCommand,
    };
    use crate::cost_report::DateTimezone;

    struct HomeGuard {
        original: Option<OsString>,
    }

    impl HomeGuard {
        fn set(home: Option<&Path>) -> Self {
            let original = env::var_os("HOME");

            match home {
                Some(home) => {
                    // These tests run under `cargo nextest`, so each test has its own process and can
                    // safely mutate process-global environment variables.
                    unsafe { env::set_var("HOME", home) };
                }
                None => {
                    // See comment above about `cargo nextest` process isolation.
                    unsafe { env::remove_var("HOME") };
                }
            }

            Self { original }
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            match self.original.as_ref() {
                Some(value) => {
                    // See comment above about `cargo nextest` process isolation.
                    unsafe { env::set_var("HOME", value) };
                }
                None => {
                    // See comment above about `cargo nextest` process isolation.
                    unsafe { env::remove_var("HOME") };
                }
            }
        }
    }

    fn with_home<R>(home: Option<&Path>, f: impl FnOnce() -> R) -> R {
        let _guard = HomeGuard::set(home);
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
                assert_eq!(dir, PathBuf::from("logs/api"));
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
    fn cli_rejects_flat_pi_cost_spelling() {
        let error = Cli::try_parse_from(["moriarty", "pi-cost"]).unwrap_err();

        assert_eq!(error.kind(), clap::error::ErrorKind::InvalidSubcommand);
    }
}
