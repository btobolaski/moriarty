//! Tracing configuration for hooks execution
//!
//! Provides structured logging to track hook execution, including:
//! - Hook triggers and event data
//! - Command execution and output
//! - Timing information
//! - Errors and failures
//!
//! Logs are written in JSON format to XDG_STATE_HOME/moriarty/hooks/hooks.log with daily rotation.

use crate::persistence::FileType;
use miette::{IntoDiagnostic, Result};
#[cfg(test)]
use std::path::PathBuf;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Initialize tracing for hooks with file-based JSON logging
///
/// Returns a `WorkerGuard` that must be held for the lifetime of the application.
/// When dropped, the guard flushes any remaining logs.
///
/// # Log Location
///
/// Logs are written in JSON format to `$XDG_STATE_HOME/moriarty/hooks/hooks.log` (or
/// `~/.local/state/moriarty/hooks/hooks.log` on most systems).
///
/// # Log Rotation
///
/// Log files are rotated daily with the naming pattern `hooks.log.YYYY-MM-DD`.
///
/// # Environment Variables
///
/// - `MORIARTY_HOOKS_LOG`: Override default log level filter (default: "info")
///   Examples: "debug", "trace", "moriarty::hooks=trace"
///
/// # Examples
///
/// ```no_run
/// use moriarty::hooks::tracing::init_tracing;
///
/// #[tokio::main]
/// async fn main() -> miette::Result<()> {
///     let _guard = init_tracing().await?;
///
///     // Hook execution with tracing
///     tracing::info!("Starting hook execution");
///
///     Ok(())
/// }
/// ```
pub async fn init_tracing() -> Result<WorkerGuard> {
    // Get the log directory by building path to a dummy file and taking its parent
    let log_file_path = FileType::State.build_path("hooks/hooks.log").await?;
    let log_dir = log_file_path
        .parent()
        .ok_or_else(|| miette::miette!("Invalid log path"))?;

    // Create daily rotating file appender
    let file_appender = tracing_appender::rolling::daily(log_dir, "hooks.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    // Configure filter from environment or default to "info"
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"))
        .add_directive("moriarty::hooks=debug".parse().into_diagnostic()?);

    // Build subscriber with JSON file output
    let file_layer = fmt::layer()
        .json()
        .with_writer(non_blocking)
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true);

    // Global singleton with try_init (vs panic-on-reinit) chosen because multiple
    // initialization attempts are expected in test scenarios where test isolation doesn't extend
    // to global state. Production code always initializes once, making this a test-only concern.
    match tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .try_init()
    {
        Ok(_) => {
            tracing::debug!(
                log_dir = %log_dir.display(),
                "Hooks tracing initialized"
            );
            Ok(guard)
        }
        Err(e) => Err(miette::miette!(
            "Failed to initialize tracing subscriber: {}",
            e
        )),
    }
}

/// Get the current hooks log file path
///
/// Returns the path to the active log file (today's log).
#[cfg(test)]
pub async fn get_current_log_path() -> Result<PathBuf> {
    FileType::State.build_path("hooks/hooks.log").await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Safe to use std::env::set_var because cargo nextest isolates each test in a separate process.
    fn setup_isolated_xdg_state() -> TempDir {
        let temp_dir = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_STATE_HOME", temp_dir.path());
        temp_dir
    }

    #[tokio::test]
    async fn test_get_current_log_path() {
        let _xdg_dir = setup_isolated_xdg_state();

        let path = get_current_log_path().await.unwrap();
        let path_str = path.to_string_lossy();
        assert!(
            path_str.contains("moriarty"),
            "Path should contain 'moriarty': {}",
            path_str
        );
        assert!(
            path_str.contains("hooks"),
            "Path should contain 'hooks': {}",
            path_str
        );
        assert!(
            path_str.contains("hooks.log"),
            "Path should contain 'hooks.log': {}",
            path_str
        );
    }

    #[tokio::test]
    async fn test_init_tracing_creates_log_directory() {
        let _xdg_dir = setup_isolated_xdg_state();

        let _guard = init_tracing().await.unwrap();

        let log_path = get_current_log_path().await.unwrap();
        let log_dir = log_path
            .parent()
            .expect("Log path should have a parent directory");
        assert!(
            log_dir.exists(),
            "Log directory should exist: {}",
            log_dir.display()
        );
    }

    #[tokio::test]
    async fn test_init_tracing_logs_correctly() {
        let _xdg_dir = setup_isolated_xdg_state();
        let _guard = init_tracing().await.unwrap();

        // Write a test log message
        tracing::info!("Test message from hooks tracing");

        // Force flush by dropping the guard
        drop(_guard);

        // Get the log directory and find the actual log file
        let log_path = get_current_log_path().await.unwrap();
        let log_dir = log_path.parent().expect("Log path should have parent");

        // Find log files in the directory (daily rotation creates dated files)
        let entries: Vec<_> = std::fs::read_dir(log_dir)
            .expect("Failed to read log directory")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("hooks.log"))
            .collect();

        assert!(!entries.is_empty(), "Should have at least one log file");

        // Read the first log file found
        let log_file = &entries[0];
        let content = std::fs::read_to_string(log_file.path()).expect("Failed to read log file");

        assert!(
            content.contains("Test message from hooks tracing"),
            "Log file should contain test message. Content:\n{}",
            content
        );
        assert!(
            content.contains("Hooks tracing initialized"),
            "Log file should contain initialization message. Content:\n{}",
            content
        );
    }

    #[tokio::test]
    async fn test_log_level_from_env() {
        let _xdg_dir = setup_isolated_xdg_state();
        std::env::set_var("RUST_LOG", "debug");

        let _guard = init_tracing().await.unwrap();

        // Write debug and info messages
        tracing::debug!("Debug level message");
        tracing::info!("Info level message");

        // Force flush
        drop(_guard);

        // Get the log directory and find the actual log file
        let log_path = get_current_log_path().await.unwrap();
        let log_dir = log_path.parent().expect("Log path should have parent");

        // Find log files in the directory
        let entries: Vec<_> = std::fs::read_dir(log_dir)
            .expect("Failed to read log directory")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("hooks.log"))
            .collect();

        assert!(!entries.is_empty(), "Should have at least one log file");

        // Read the first log file found
        let log_file = &entries[0];
        let content = std::fs::read_to_string(log_file.path()).expect("Failed to read log file");

        assert!(
            content.contains("Debug level message"),
            "Debug message should appear when RUST_LOG=debug. Content:\n{}",
            content
        );
        assert!(
            content.contains("Info level message"),
            "Info message should appear when RUST_LOG=debug. Content:\n{}",
            content
        );
    }
}
