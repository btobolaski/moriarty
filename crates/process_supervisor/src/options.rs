use std::{ffi::OsString, path::PathBuf, time::Duration};

use miette::Diagnostic;
use thiserror::Error;

/// Raw OS strings preserve verified program spelling and literal argv without shell parsing.
/// The environment is inherited and stdin is always null; see [usage](crate#usage-and-ownership).
#[derive(Debug, Clone)]
pub struct ProcessSpec {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub cwd: Option<PathBuf>,
}

/// Caller configuration crosses validation here, not inside process execution.
/// See [defaults and resource bounds](crate#budgets-and-results).
#[derive(Debug, Clone)]
pub struct ProcessOptions {
    pub(crate) stdout_limit: usize,
    pub(crate) stderr_limit: usize,
    pub(crate) startup_timeout: Duration,
    pub(crate) execution_timeout: Option<Duration>,
    pub(crate) term_grace: Duration,
    pub(crate) post_kill_grace: Duration,
}

/// Invalid raw configuration cannot produce an options value or become a process execution error.
#[derive(Debug, Error, Diagnostic, PartialEq, Eq)]
pub enum OptionsError {
    #[error("capture limits must not exceed isize::MAX bytes per stream")]
    CaptureTooLarge,
}

impl Default for ProcessOptions {
    fn default() -> Self {
        Self {
            stdout_limit: 1024 * 1024,
            stderr_limit: 1024 * 1024,
            startup_timeout: Duration::from_secs(5),
            execution_timeout: None,
            term_grace: Duration::from_secs(2),
            post_kill_grace: Duration::from_secs(1),
        }
    }
}

impl ProcessOptions {
    /// Zero discards while draining; impossible buffer sizes fail before any allocation or launch.
    pub fn with_capture_limits(
        mut self,
        stdout: usize,
        stderr: usize,
    ) -> Result<Self, OptionsError> {
        if stdout > isize::MAX as usize || stderr > isize::MAX as usize {
            return Err(OptionsError::CaptureTooLarge);
        }
        self.stdout_limit = stdout;
        self.stderr_limit = stderr;
        Ok(self)
    }

    /// Admission and blocked creation share this budget, including abandoned handoffs.
    pub fn with_startup_timeout(mut self, timeout: Duration) -> Self {
        self.startup_timeout = timeout;
        self
    }

    /// Starts after successful startup, so a slow launch does not consume running time.
    pub fn with_execution_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.execution_timeout = timeout;
        self
    }

    /// Post-KILL time is shared by reaping, group checks and both EOFs, not renewed per operation.
    pub fn with_cleanup_graces(mut self, term: Duration, post_kill: Duration) -> Self {
        self.term_grace = term;
        self.post_kill_grace = post_kill;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setters_validate_capture_limits_and_accept_independent_graces() {
        let defaults = ProcessOptions::default();
        assert_eq!(
            defaults
                .clone()
                .with_capture_limits(usize::MAX, 0)
                .unwrap_err(),
            OptionsError::CaptureTooLarge
        );
        let options = defaults
            .with_capture_limits(0, 0)
            .unwrap()
            .with_cleanup_graces(Duration::MAX, Duration::MAX);
        assert_eq!(
            (options.term_grace, options.post_kill_grace),
            (Duration::MAX, Duration::MAX)
        );
    }
}
