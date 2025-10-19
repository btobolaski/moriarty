use std::path::PathBuf;

use clap::{Parser, Subcommand};

mod logs;
mod tui;

#[tokio::main]
async fn main() -> miette::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Logs { file } => {
            // Read and parse the log file
            let log_lines = logs::parser::read_file(file).await?;

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
    },
}
