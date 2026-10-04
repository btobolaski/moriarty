use std::{fmt, io, process::ExitStatus};

use miette::Diagnostic;
use thiserror::Error;

/// Cause and actual reaped status stay inseparable; see [completion guarantees](crate#budgets-and-results).
#[derive(Debug)]
pub enum Completion {
    /// Nonzero and signal exits remain ordinary completed reports.
    Exited(ExitStatus),
    /// Cooperative TERM handling may produce a successful status despite cancellation.
    Cancelled(ExitStatus),
    /// The deadline initiates cleanup; this is the child's status, not an invented timeout code.
    TimedOut(ExitStatus),
}

/// Raw bytes need not be UTF-8, and truncation is not an execution error.
/// EOF remains separate because reaching a byte limit must not stop draining.
#[derive(Debug, Default)]
pub struct Capture {
    /// Bounded by the configured retention limit; partial bytes may be invalid UTF-8.
    pub bytes: Vec<u8>,
    /// Requires discarded bytes, not merely reaching the retention limit.
    pub truncated: bool,
    /// False also covers failed or uninitialized readers, not just a still-open pipe.
    pub eof: bool,
}

/// Returned only after reaping, group absence and both pipe EOFs are confirmed;
/// incomplete supervision instead carries [`PartialState`] in a [`ProcessError`].
#[derive(Debug)]
pub struct ProcessReport {
    pub completion: Completion,
    pub stdout: Capture,
    pub stderr: Capture,
}

/// Successful outcomes keep this cause in [`Completion`]; errors also retain its phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptionReason {
    Cancelled,
    TimedOut,
}

/// Cleanup uncertainty must not erase whether startup or execution was interrupted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interruption {
    Startup(InterruptionReason),
    Execution(InterruptionReason),
}

// One list supplies discriminants, storage length, and exhaustive regression coverage.
macro_rules! stages {
    ($($stage:ident),+ $(,)?) => {
        /// A fixed vocabulary bounds retained diagnostics to one failure per stage.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[repr(usize)]
        pub enum Stage { $($stage),+ }

        impl Stage {
            const ALL: &[Self] = &[$(Self::$stage),+];
        }
    };
}

stages!(
    Allocation,
    Launcher,
    Spawn,
    PipeSetup,
    Stdout,
    Stderr,
    Observation,
    Term,
    Kill,
    Reap,
    GroupProbe,
    Driver,
);

/// Retains the original I/O source so callers need not parse display text to classify a failure.
#[derive(Debug, Error, Diagnostic)]
#[error("{stage:?}: {source}")]
pub struct StageFailure {
    pub stage: Stage,
    #[source]
    pub source: io::Error,
}

/// Error-only evidence may lack child identity or status; it is never a completed report.
/// `Display` excludes captured bytes, while `Debug` intentionally exposes the fields.
#[derive(Debug, Default)]
pub struct PartialState {
    /// Missing identity is not proof that no child exists or can still be created.
    pub child_id: Option<u32>,
    /// Only observed reaping supplies a status; absence is not proof of no child.
    pub status: Option<ExitStatus>,
    /// Creation may still finish outside the driver; false does not establish cleanup.
    pub launch_unconfirmed: bool,
    /// Confirmed absence is retained monotonically rather than probing a possibly recycled PGID.
    pub group_absent: bool,
    /// Records entry into the TERM path, including preflight: not proof a signal syscall
    /// ran, succeeded, or killed anything. False means no such entry was recorded.
    pub term_attempted: bool,
    /// KILL-path entry, with the same evidence limits as [`Self::term_attempted`].
    pub kill_attempted: bool,
    /// Cleanup uncertainty does not erase the initiating phase and reason.
    pub interruption: Option<Interruption>,
    pub stdout: Capture,
    pub stderr: Capture,
    failures: [Option<StageFailure>; Stage::ALL.len()],
}

impl PartialState {
    /// Stage order is stable; repeated failures retain the first source without growing storage.
    pub fn failures(&self) -> impl Iterator<Item = &StageFailure> {
        self.failures.iter().flatten()
    }

    pub(crate) fn record(&mut self, stage: Stage, source: impl Into<io::Error>) {
        self.failures[stage as usize].get_or_insert_with(|| StageFailure {
            stage,
            source: source.into(),
        });
    }

    pub(crate) fn failed(&self) -> bool {
        self.failures().next().is_some()
    }
}

// Never include raw child output in diagnostics: the caller may itself be an MCP stdio host.
impl fmt::Display for PartialState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "child={:?}, reaped={}, group_absent={}, term_attempted={}, kill_attempted={}, stdout_eof={}, stderr_eof={}, launch_unconfirmed={}, interruption={:?}",
            self.child_id,
            self.status.is_some(),
            self.group_absent,
            self.term_attempted,
            self.kill_attempted,
            self.stdout.eof,
            self.stderr.eof,
            self.launch_unconfirmed,
            self.interruption
        )?;
        for failure in self.failures() {
            write!(f, "; {failure}")?;
        }
        Ok(())
    }
}

/// Downcastable from `miette::Report` so callers can inspect bounded partial evidence.
/// See [host-lifetime limits](crate#stalled-startup-and-host-lifetime) before retrying uncertain launches.
#[derive(Debug, Error, Diagnostic)]
pub enum ProcessError {
    /// Includes failures before a child exists as well as cleaned-up setup failures.
    #[error("process start failed: {state}")]
    StartFailed { state: Box<PartialState> },
    /// An expired or cancelled launch may still be blocked in the OS; inspect `launch_unconfirmed`.
    #[error("process startup interrupted: {state}")]
    StartupInterrupted { state: Box<PartialState> },
    /// A child status cannot hide I/O, observation, or driver failure.
    #[error("process supervision failed: {state}")]
    SupervisionFailed { state: Box<PartialState> },
    /// Process cleanup or capture settlement lacks proof; inspect the individual evidence fields.
    #[error("process cleanup or capture settlement unconfirmed: {state}")]
    CleanupUnconfirmed { state: Box<PartialState> },
}

pub(crate) enum DriverChannel {
    Startup,
    Completion,
}

impl ProcessError {
    /// Explicit access keeps output available to the caller without leaking it into diagnostic display.
    pub fn partial(&self) -> &PartialState {
        match self {
            Self::StartFailed { state }
            | Self::StartupInterrupted { state }
            | Self::SupervisionFailed { state }
            | Self::CleanupUnconfirmed { state } => state,
        }
    }

    pub(crate) fn from_startup(
        mut state: PartialState,
        reason: Option<InterruptionReason>,
        unconfirmed: bool,
    ) -> Self {
        state.interruption = reason.map(Interruption::Startup);
        state.launch_unconfirmed = unconfirmed;
        let state = Box::new(state);
        if reason.is_some() {
            Self::StartupInterrupted { state }
        } else if unconfirmed {
            Self::CleanupUnconfirmed { state }
        } else {
            Self::StartFailed { state }
        }
    }

    pub(crate) fn driver_closed(channel: DriverChannel) -> Self {
        let mut state = PartialState {
            launch_unconfirmed: matches!(channel, DriverChannel::Startup),
            ..PartialState::default()
        };
        state.record(
            Stage::Driver,
            io::Error::other(
                "driver ended without a result; only nonwaiting drop cleanup was possible",
            ),
        );
        Self::CleanupUnconfirmed {
            state: Box::new(state),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_failure_projects_outcome_facts_over_stale_metadata() {
        for (reason, unconfirmed, kind) in [
            (None, false, "start"),
            (Some(InterruptionReason::Cancelled), false, "interrupted"),
            (None, true, "unconfirmed"),
            (Some(InterruptionReason::TimedOut), true, "interrupted"),
        ] {
            let state = PartialState {
                launch_unconfirmed: !unconfirmed,
                interruption: Some(Interruption::Execution(InterruptionReason::Cancelled)),
                ..PartialState::default()
            };
            let error = ProcessError::from_startup(state, reason, unconfirmed);
            let actual_kind = match &error {
                ProcessError::StartFailed { .. } => "start",
                ProcessError::StartupInterrupted { .. } => "interrupted",
                ProcessError::CleanupUnconfirmed { .. } => "unconfirmed",
                ProcessError::SupervisionFailed { .. } => "supervision",
            };
            assert_eq!(actual_kind, kind);
            assert_eq!(error.partial().launch_unconfirmed, unconfirmed);
            assert_eq!(
                error.partial().interruption,
                reason.map(Interruption::Startup)
            );
        }
    }

    #[test]
    fn every_stage_retains_only_its_first_source_in_order() {
        let mut state = PartialState::default();
        assert!(!state.failed());
        for &stage in Stage::ALL.iter().rev() {
            state.record(stage, io::Error::other(format!("first {stage:?}")));
            state.record(stage, io::Error::other("duplicate"));
        }
        assert!(state.failed());
        assert_eq!(state.failures().count(), Stage::ALL.len());
        for (failure, &stage) in state.failures().zip(Stage::ALL) {
            assert_eq!(failure.stage, stage);
            assert_eq!(failure.source.to_string(), format!("first {stage:?}"));
        }
    }

    #[test]
    fn diagnostic_display_keeps_metadata_but_excludes_captured_output() {
        let mut state = PartialState {
            child_id: Some(7),
            group_absent: true,
            term_attempted: true,
            kill_attempted: true,
            ..PartialState::default()
        };
        state.stdout.bytes = b"stdout-private-marker".to_vec();
        state.stderr.bytes = b"stderr-private-marker".to_vec();
        state.record(Stage::Stdout, io::Error::other("read failed"));
        state.record(
            Stage::Stdout,
            io::Error::other("later error must not replace the first"),
        );
        let error = ProcessError::CleanupUnconfirmed {
            state: Box::new(state),
        };
        let diagnostic = error.to_string();
        assert!(diagnostic.contains("child=Some(7)") && diagnostic.contains("group_absent=true"));
        assert!(
            diagnostic.contains("term_attempted=true")
                && diagnostic.contains("kill_attempted=true")
        );
        let empty = PartialState::default().to_string();
        assert!(empty.contains("term_attempted=false") && empty.contains("kill_attempted=false"));
        assert!(diagnostic.contains("Stdout: read failed"));
        assert!(!diagnostic.contains("stdout-private-marker"));
        assert!(!diagnostic.contains("stderr-private-marker"));
        assert!(!diagnostic.contains("later error"));
        assert_eq!(error.partial().failures().count(), 1);
    }
}
