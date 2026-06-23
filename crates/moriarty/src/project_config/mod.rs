//! Project configuration and approval system.
//!
//! This module combines three related concerns:
//!
//! - **[`config`]**: Loading and parsing `.config/tools.toml` files into typed structures.
//!   Provides [`ProjectConfig`] and [`config::Commands`] types, along with the [`load_project_settings`]
//!   function for reading configuration files.
//!
//! - **[`approvals`]**: Security approval system for verifying that project tools haven't changed.
//!   Tracks SHA-256 hashes of configuration files and binaries, provides verification logic,
//!   and manages approval persistence with file locking.
//!
//! - **[`runner`]**: Verified command execution for project tools. Combines configuration loading
//!   and approval verification into a safe execution model. Provides [`verify_and_load_project`]
//!   for loading verified projects and [`VerifiedProject`] for running commands.
//!
//! ## Usage
//!
//! Most consumers should use the re-exported types from the module root, which provides a
//! convenient flat namespace. For example:
//!
//! ```no_run
//! use moriarty::project_config::{verify_and_load_project};
//!
//! # async fn example() -> miette::Result<()> {
//! // Verify and load project (combines config + approval verification)
//! let project = verify_and_load_project("/path/to/project".into()).await?;
//!
//! // Run a single command
//! let output = project.run_command("lint").await?;
//!
//! // Or run all commands in parallel
//! let results = project.run_all_commands().await?;
//! # Ok(())
//! # }
//! ```
//!
//! Direct access to submodules ([`config`], [`approvals`], and [`runner`]) is provided for
//! advanced use cases where you need access to non-re-exported functionality.

pub mod approvals;
pub mod config;
pub mod runner;

// Re-export commonly used types and functions
pub use approvals::{
    CommandApproval, ProjectApprovals, VerificationResult, is_script, is_within_project,
    is_writable, read_script_contents, resolve_binary_path_with_original,
};
pub use config::{ProjectConfig, load_project_settings};
// Allow unused imports warning for re-exports that are part of the public API
// but not used within this crate
#[allow(unused_imports)]
pub use runner::{CommandOutput, VerifiedProject, verify_and_load_project};
