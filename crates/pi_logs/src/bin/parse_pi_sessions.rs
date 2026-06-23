//! Recursively parse every `*.jsonl` file under a pi sessions directory.
//!
//! This binary is a smoke test / coverage tool for [`pi_logs::parser`]. It
//! defaults to `~/.pi/agent/sessions`, walks it in a deterministic order, and
//! reports per-file parse failures. It exits non-zero if any file fails to
//! parse so coverage gaps are visible in CI.

use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::Parser;
use pi_logs::parser::{ParseError, parse_file};
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(
    about = "Parse every *.jsonl file under a pi sessions directory.",
    long_about = "Recursively walks the given directory (defaults to \
                  ~/.pi/agent/sessions), attempts to parse every *.jsonl \
                  file, and reports failures. Exits non-zero if any file \
                  fails to parse."
)]
struct Args {
    /// Directory to walk. Defaults to `~/.pi/agent/sessions`.
    #[arg(value_name = "SESSIONS_DIR")]
    sessions_dir: Option<PathBuf>,
}

fn default_sessions_dir() -> Option<PathBuf> {
    // We avoid pulling in an extra `dirs` crate by reading HOME directly.
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".pi/agent/sessions"))
}

fn collect_jsonl_files(root: &Path) -> Result<Vec<PathBuf>, walkdir::Error> {
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in WalkDir::new(root).sort_by_file_name() {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            files.push(path.to_path_buf());
        }
    }
    Ok(files)
}

fn main() -> ExitCode {
    let args = Args::parse();
    let sessions_dir = match args.sessions_dir.or_else(default_sessions_dir) {
        Some(dir) => dir,
        None => {
            eprintln!(
                "error: no sessions directory provided and $HOME is not set; \
                 pass a path explicitly"
            );
            return ExitCode::from(2);
        }
    };

    if !sessions_dir.exists() {
        eprintln!(
            "error: sessions directory does not exist: {}",
            sessions_dir.display()
        );
        return ExitCode::from(2);
    }

    let files = match collect_jsonl_files(&sessions_dir) {
        Ok(files) => files,
        Err(err) => {
            eprintln!("error: failed to walk {}: {err}", sessions_dir.display());
            return ExitCode::from(2);
        }
    };

    println!(
        "scanning {} *.jsonl file(s) under {}",
        files.len(),
        sessions_dir.display()
    );

    let mut failures: Vec<ParseError> = Vec::new();
    let mut parsed_lines: usize = 0;

    for path in &files {
        match parse_file(path) {
            Ok(lines) => {
                parsed_lines += lines.len();
            }
            Err(err) => {
                println!("  FAIL {}", path.display());
                eprintln!("{err}");
                failures.push(err);
            }
        }
    }

    println!();
    println!(
        "parsed {} line(s) across {} file(s); {} failure(s)",
        parsed_lines,
        files.len(),
        failures.len()
    );

    if failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
