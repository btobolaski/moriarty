use std::path::PathBuf;

use clap::{Parser, Subcommand};
use mcp::McpServers;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod api_pricing;
mod approval_tui;
mod hashing;
mod hooks;
mod mcp;
mod persistence;
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
        Command::ApiPricing {
            dir,
            timezone,
            conversations,
            start_time,
            end_time,
        } => {
            // Default to info-level so cost_analyzer's parse-failure and
            // unrecognized-model events reach the operator without setting RUST_LOG.
            let filter =
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
            let _ = tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
                .try_init();

            // Parse timezone argument
            let tz = match timezone.to_lowercase().as_str() {
                "local" => api_pricing::DateTimezone::Local,
                "utc" => api_pricing::DateTimezone::Utc,
                _ => {
                    eprintln!(
                        "Error: Invalid timezone '{}'. Must be 'local' or 'utc'",
                        timezone
                    );
                    std::process::exit(1);
                }
            };

            // Parse time range filter
            let filter = api_pricing::TimeRangeFilter::new(start_time, end_time)?;

            // Display filter info if set
            if !filter.is_unrestricted() {
                println!("Applying time range filter:");
                if let Some(start) = filter.start {
                    println!("  Start: {}", start.to_rfc3339());
                }
                if let Some(end) = filter.end {
                    println!("  End:   {}", end.to_rfc3339());
                }
                println!();
            }

            // Run the API pricing analysis
            api_pricing::run(&dir, tz, conversations, &filter).await?;
        }
        Command::Mcp { server } => {
            server.run().await?;
        }
        Command::ApproveProject { project_dir } => {
            // Initialize the terminal
            let terminal = ratatui::init();

            // Create and run the approval app
            let app = approval_tui::ApprovalApp::new(project_dir).await?;
            let approved = app.run(terminal).await;

            // Restore the terminal
            ratatui::restore();

            // Exit with appropriate code
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

#[derive(Debug, Parser)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    ApiPricing {
        /// The directory to analyze for API usage
        #[arg(short, long)]
        dir: PathBuf,
        /// Timezone to use for date determination (local or utc)
        #[arg(long, default_value = "local")]
        timezone: String,
        /// Aggregate by conversation/session instead of by date
        #[arg(long)]
        conversations: bool,
        /// Start time for filtering messages (ISO 8601 format, e.g., "2025-01-01T00:00:00Z" or "2025-01-01")
        /// If no timezone specified, UTC is assumed
        #[arg(long, value_name = "DATETIME")]
        start_time: Option<String>,
        /// End time for filtering messages (ISO 8601 format, e.g., "2025-01-01T23:59:59Z" or "2025-01-01")
        /// If no timezone specified, UTC is assumed
        #[arg(long, value_name = "DATETIME")]
        end_time: Option<String>,
    },
    /// Runs one of the mcp servers
    Mcp {
        #[command(subcommand)]
        server: McpServers,
    },
    /// Approve project tools for MCP server execution
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
pub enum HooksCommand {
    /// Execute hook with input and log the results
    Exec,
}

#[derive(Debug, Subcommand)]
pub enum TestCommand {
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
