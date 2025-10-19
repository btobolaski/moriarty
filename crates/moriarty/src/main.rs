use std::path::PathBuf;

use clap::{Parser, Subcommand};

mod logs;

#[tokio::main]
async fn main() -> miette::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Logs { file } => {
            logs::read_file(file).await?;
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
