//! Project configuration and approval system.
//!
//! This module combines two related concerns:
//!
//! - **[`config`]**: Loading and parsing `.config/tools.toml` files into typed structures.
//!   Provides [`ProjectConfig`] and [`Commands`] types, along with the [`load_project_settings`]
//!   function for reading configuration files.
//!
//! - **[`approvals`]**: Security approval system for verifying that project tools haven't changed.
//!   Tracks SHA-256 hashes of configuration files and binaries, provides verification logic,
//!   and manages approval persistence with file locking.
//!
//! ## Usage
//!
//! Most consumers should use the re-exported types from the module root, which provides a
//! convenient flat namespace. For example:
//!
//! ```no_run
//! use moriarty::project_config::{load_project_settings, ProjectApprovals};
//!
//! # async fn example() -> miette::Result<()> {
//! // Load configuration
//! let config = load_project_settings("/path/to/project".into()).await?;
//!
//! // Verify approvals
//! let approvals = ProjectApprovals::load().await?;
//! let result = approvals.verify_project(
//!     std::path::Path::new("/path/to/project"),
//!     "lint"
//! ).await?;
//! # Ok(())
//! # }
//! ```
//!
//! Direct access to submodules ([`config`] and [`approvals`]) is provided for advanced use cases
//! where you need access to non-re-exported functionality.

pub mod approvals;
pub mod config;

// Re-export commonly used types and functions
pub use approvals::{
    is_script, is_within_project, is_writable, read_script_contents,
    resolve_binary_path_with_original, CommandApproval, ProjectApprovals, VerificationResult,
};
pub use config::{load_project_settings, ProjectConfig};
