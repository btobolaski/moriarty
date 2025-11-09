use std::path::PathBuf;

use clap::{Parser, Subcommand};
use mcp::McpServers;

mod api_pricing;
mod approval_tui;
mod hashing;
mod hooks;
mod logs;
mod mcp;
mod persistence;
mod project_config;
mod test_runner;
mod tui;
mod user_config;

#[tokio::main]
async fn main() -> miette::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Logs { file, validate } => {
            // Read and parse the log file
            let log_lines = logs::parser::read_file(file).await?;

            if !validate {
                // Initialize the terminal
                let terminal = ratatui::init();

                // Create and run the app
                let app = tui::app::App::new(log_lines);
                let result = app.run(terminal).await;

                // Restore the terminal
                ratatui::restore();

                result?;
            }
        }
        Command::ApiPricing {
            dir,
            timezone,
            conversations,
            start_time,
            end_time,
        } => {
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
    Logs {
        /// The specific log file to read
        #[arg(short, long)]
        file: PathBuf,
        /// instead of running the viewer, it simply parses the log and exits. It will produce a
        /// non-zero exit code if the parsing failed.
        #[arg(long)]
        validate: bool,
    },
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
