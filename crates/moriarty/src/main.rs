use std::path::PathBuf;

use clap::{Parser, Subcommand};
use mcp::McpServers;

mod api_pricing;
mod logs;
mod mcp;
mod tui;

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
        Command::ApiPricing { dir, timezone } => {
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

            // Run the API pricing analysis
            api_pricing::run(&dir, tz).await?;
        }
        Command::Mcp { server } => {
            server.run().await?;
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
    },
    /// Runs one of the mcp servers
    Mcp {
        #[command(subcommand)]
        server: McpServers,
    },
}
