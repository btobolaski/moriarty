//! Project tools approval system.
//!
//! This module provides the approval verification system that ensures project tools are
//! explicitly approved before execution. Instead of hashing the whole `tools.toml` (which fires
//! on comments, whitespace, reordering, and unrelated commands), each approval binds **exactly
//! what executes**: the command's argv from `tools.toml`, the resolved binary paths, and the
//! binary's SHA-256. Approvals are stored as a **match-any set of approved versions per command
//! name**, so only three things ever trigger re-approval — a new command/check name, an argument
//! change, or a binary change — while cosmetic config edits and deleted commands never do.
//!
//! # Roots
//!
//! Approvals are keyed by **repository root** (shared across jj workspaces and git worktrees) so
//! identical content needs no re-approval per worktree. The configuration that verification parses
//! and the binaries it resolves come from the caller's **workspace root** (`detect_workspace_root`),
//! which is what gives each worktree tooling independence: a worktree on a branch with different
//! tooling runs its own `tools.toml`, while approvals stay shared by repo root.
//!
//! Stored binary paths are normalized relative to the workspace root when they live inside it
//! ([`normalize_path_for_storage`]), so an approval made in worktree A matches byte-identical
//! content in worktree B. PATH-resolved binaries (`which cargo`) live outside the workspace and
//! stay absolute.
//!
//! # Security Model
//!
//! - **Explicit approval**: All project tools must be approved via the TUI.
//! - **Exact binding**: Verification runs the parsed config through verification (no file re-read),
//!   closing a load/verify TOCTOU window — what you verify is literally what you run.
//! - **Match-any**: Approved versions accrete additively; stale ones only match if that exact
//!   argv+binary reappears. Running an approved version does not refresh its timestamp.
//! - **Anti-shadowing**: Verification spawns the resolved+hashed program path, never the raw
//!   `command[0]`, so a workspace-local file cannot shadow the approved copy.
//! - **Legacy migration**: Old files (single-table commands, `tools_config_hash` present) load
//!   with `argv: None` and verify on paths + binary hash; the next approval upgrades them in
//!   place to full argv binding plus `approved_at`. A binary-change re-approval retires the stale
//!   legacy version for those paths (fail-closed on binary revert) rather than leaving the old
//!   binary wildcard-approved under any argv.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

// Unix-specific permission checking. Windows code uses different APIs (see is_writable implementation).
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use chrono::{DateTime, Utc};
use fs2::FileExt;
use miette::{Context, IntoDiagnostic};
use serde::{Deserialize, Serialize};

use crate::{hashing, persistence::FileType, repository};

use super::config::ProjectConfig;

const APPROVALS_FILE: &str = "project_approvals.toml";

/// All project approvals stored in ~/.config/moriarty/project_approvals.toml
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectApprovals {
    /// Map of canonical project path to approval data
    #[serde(default)]
    pub projects: HashMap<String, ProjectApproval>,
}

/// Approval data for a single project (repository root).
///
/// `commands` and `checks` map each item name to the set of approved versions for that item.
/// A version matches the current config iff its argv (when bound), normalized paths, and binary
/// hash all equal the current values — see [`ProjectApprovals::verify_item_for_workspace`].
/// Legacy files predate argv binding and load each name as a single `argv: None` entry (see
/// [`one_or_many`]); the next approval upgrades such an entry in place.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectApproval {
    /// Timestamp when this project was last approved. Records only that the repo was touched;
    /// versions accrete and (in the follow-up expiration feature) expire independently via
    /// [`CommandApproval::approved_at`].
    pub last_approved: DateTime<Utc>,
    /// Match-any set of approved versions per command name.
    #[serde(default, deserialize_with = "one_or_many")]
    pub commands: HashMap<String, Vec<CommandApproval>>,
    /// Match-any set of approved versions per check name.
    #[serde(default, deserialize_with = "one_or_many")]
    pub checks: HashMap<String, Vec<CommandApproval>>,
}

/// Approval data for a single version of one command or check.
///
/// `argv` is the `tools.toml` command array this approval binds. `None` marks a migrated legacy
/// entry that predates argv binding; it matches on paths + binary hash only and is upgraded in
/// place to full argv binding on the next approval of that item. `approved_at` is the per-version
/// approval timestamp (the hook for the follow-up expiration/pruning feature); `None` marks a
/// migrated legacy entry and is treated as oldest.
///
/// Invariant: post-migration `argv` and `approved_at` co-vary — both `Some` for a fully bound
/// version, both `None` only for a not-yet-re-approved legacy entry. The TUI's transient pre-save
/// state (`argv: Some`, `approved_at: None`) is the only intermediate and is never persisted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandApproval {
    /// The tools.toml argv this approval binds. `None` = migrated legacy entry (arguments not
    /// bound until the next approval upgrades it in place).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argv: Option<Vec<String>>,
    /// When this version was (last) approved. `None` = migrated legacy entry (unknown approval
    /// time; treated as oldest).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<DateTime<Utc>>,
    /// Original path specified in tools.toml (may be a symlink), normalized for storage via
    /// [`normalize_path_for_storage`].
    pub original_path: String,
    /// Canonical path to the binary (symlinks resolved), normalized for storage via
    /// [`normalize_path_for_storage`].
    pub canonical_path: String,
    /// SHA-256 hash of the binary file
    pub binary_hash: String,
}

/// Deserialize a `HashMap<String, Vec<CommandApproval>>` whose values are each either a legacy
/// single-table entry (one `CommandApproval`) or a new array-of-tables entry (a `Vec`), so old
/// approval files (one approval per name) and new files (a `Vec` per name) both load. Per-value
/// single-or-many handling is required because the map value type is `Vec`, which a single legacy
/// table would otherwise fail to deserialize into.
fn one_or_many<'de, D>(deserializer: D) -> Result<HashMap<String, Vec<CommandApproval>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // Each value is either a legacy single-table entry (one `CommandApproval`) or a new
    // array-of-tables entry (a `Vec`); the untagged enum lets serde pick per value, so a single
    // legacy table loads as a one-element vec and a new array loads as-is. The derive handles the
    // map iteration, so per-value single-or-many handling is the only custom part.
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(CommandApproval),
        Many(Vec<CommandApproval>),
    }

    let map: HashMap<String, OneOrMany> = Deserialize::deserialize(deserializer)?;
    Ok(map
        .into_iter()
        .map(|(name, value)| {
            (
                name,
                match value {
                    OneOrMany::One(approval) => vec![approval],
                    OneOrMany::Many(approvals) => approvals,
                },
            )
        })
        .collect())
}

/// Type of item being verified
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemType {
    Command,
    Check,
}

/// Result of verifying project approval status
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationResult {
    /// Project is approved and an entry matches the current argv+binary. Carries the item's
    /// program resolved against the workspace root so callers spawn exactly the file that was
    /// verified instead of re-resolving `command[0]` against a possibly different base (a
    /// workspace cwd, where an unapproved local copy could shadow the approved one). This is the
    /// *original* resolved path, not its canonicalization — both name the same hashed file, but
    /// canonicalizing breaks symlinked multi-call binaries (nix coreutils, busybox) that dispatch
    /// on `argv[0]`.
    Approved { program: PathBuf },
    /// Project has not been approved yet
    NotApproved,
    /// Item (command or check) is configured but has no approved version at all — e.g. a brand-new
    /// name added to tools.toml, or a name absent from approvals. (An approved item whose argv
    /// changed returns [`VerificationResult::ItemChanged`] instead, not this variant.)
    ItemNotApproved { item: String },
    /// Item is approved but no approved version matches the current argv/binary. `args_changed`
    /// is true when no approved version even carries the current argv (used only for message
    /// wording); `approved_versions` is how many versions exist for the item.
    ItemChanged {
        item: String,
        args_changed: bool,
        approved_versions: usize,
    },
}

impl VerificationResult {
    /// The shared "`<kind> '<item>' has not been approved in its current form — its
    /// {arguments|binary} changed since approval (N approved version(s) exist)" sentence — the one
    /// wording both the command runner and the checks runner use, so they cannot drift. `kind` is
    /// typed ([`ItemType`]) so a check can never be labeled "Command". Callers pass the destructured
    /// [`ItemChanged`](Self::ItemChanged) fields directly, so the call is infallible — the variant
    /// is already matched and there is no `Option` to `.expect()` away.
    pub fn item_changed_sentence(
        kind: ItemType,
        item: &str,
        args_changed: bool,
        approved_versions: usize,
    ) -> String {
        format!(
            "{} '{}' has not been approved in its current form — its {} changed since approval \
             ({} approved version(s) exist)",
            match kind {
                ItemType::Command => "Command",
                ItemType::Check => "Check",
            },
            item,
            if args_changed { "arguments" } else { "binary" },
            approved_versions,
        )
    }
}

impl ProjectApprovals {
    /// Load approvals from disk
    pub async fn load() -> miette::Result<Self> {
        match FileType::Config.load::<Self>(APPROVALS_FILE).await {
            Ok(approvals) => Ok(approvals),
            Err(e) => {
                let error_msg = format!("{:?}", e);
                if error_msg.contains("No such file or directory")
                    || error_msg.contains("cannot find the file")
                    || error_msg.contains("NotFound")
                {
                    Ok(Self::default())
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Save approvals to disk
    pub async fn save(&self) -> miette::Result<()> {
        FileType::Config.persist(APPROVALS_FILE, self).await
    }

    /// Atomically update the approvals file with proper file locking
    ///
    /// This method ensures that concurrent modifications to the approvals file
    /// don't race by using file locking to make the load-modify-save cycle atomic.
    /// Approval insertion happens inside this method, so concurrent approvals from two worktrees
    /// stay atomic.
    pub async fn update<F>(f: F) -> miette::Result<()>
    where
        F: FnOnce(&mut Self) -> miette::Result<()>,
    {
        let approvals_path = FileType::Config.build_path(APPROVALS_FILE).await?;
        let lock_path = approvals_path.with_extension("lock");

        if let Some(parent) = lock_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .into_diagnostic()
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        let lock_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .into_diagnostic()
            .with_context(|| format!("Failed to open lock file: {}", lock_path.display()))?;

        lock_file
            .lock_exclusive()
            .into_diagnostic()
            .with_context(|| "Failed to acquire exclusive lock on approvals file")?;

        let mut approvals = Self::load().await?;
        f(&mut approvals)?;
        approvals.save().await
    }

    /// Verify a command against the already-detected repository root, loading nothing: callers
    /// pass the workspace root (for binary resolution) and the already-parsed `ProjectConfig`
    /// (for the command array). See [`Self::verify_item_for_workspace`] for the matching rules.
    pub async fn verify_project_for_workspace(
        &self,
        repository_root: &Path,
        workspace_root: &Path,
        config: &ProjectConfig,
        command_name: &str,
    ) -> miette::Result<VerificationResult> {
        self.verify_item_for_workspace(
            repository_root,
            workspace_root,
            config,
            command_name,
            ItemType::Command,
        )
        .await
    }

    /// Verify a check against the already-detected repository root. The check analogue of
    /// [`Self::verify_project_for_workspace`].
    pub async fn verify_check_for_workspace(
        &self,
        repository_root: &Path,
        workspace_root: &Path,
        config: &ProjectConfig,
        check_name: &str,
    ) -> miette::Result<VerificationResult> {
        self.verify_item_for_workspace(
            repository_root,
            workspace_root,
            config,
            check_name,
            ItemType::Check,
        )
        .await
    }

    /// Generic verification for commands or checks against an already-detected repository root
    /// and workspace root, taking the already-parsed config (no file re-read) so what is verified
    /// is literally what runs.
    ///
    /// Matching, per approved version of the item:
    /// - `argv` matches iff the version's `argv` is `None` (legacy, binds no argv) or equals the
    ///   current command array;
    /// - `original_path` and `canonical_path` match after normalizing both the stored and the
    ///   freshly resolved paths against `workspace_root` ([`normalize_path_for_storage`]), so an
    ///   approval made in one worktree matches byte-identical content in another;
    /// - `binary_hash` matches the hashed binary.
    ///
    /// The argument roots must be roots as returned by `detect_repository_root` /
    /// `detect_workspace_root`; approvals are keyed by repository root, so any other path fails
    /// closed as `NotApproved` rather than bypassing anything.
    async fn verify_item_for_workspace(
        &self,
        repository_root: &Path,
        workspace_root: &Path,
        config: &ProjectConfig,
        item_name: &str,
        item_type: ItemType,
    ) -> miette::Result<VerificationResult> {
        let project_key = repository_root.to_string_lossy().to_string();

        let Some(approval) = self.projects.get(&project_key) else {
            return Ok(VerificationResult::NotApproved);
        };

        let versions = match item_type {
            ItemType::Command => approval.commands.get(item_name),
            ItemType::Check => approval.checks.get(item_name),
        };
        let Some(versions) = versions else {
            return Ok(VerificationResult::ItemNotApproved {
                item: item_name.to_string(),
            });
        };

        // Look up the current command array from the parsed config. A missing entry means the
        // item name was added to tools.toml since approval (or removed) — either way it is not
        // approved in its current form.
        let command_array = match item_type {
            ItemType::Command => match item_name {
                "lint" => config.commands.lint.as_deref(),
                "test" => config.commands.test.as_deref(),
                "build" => config.commands.build.as_deref(),
                "format" => config.commands.format.as_deref(),
                _ => None,
            },
            ItemType::Check => config
                .checks
                .as_ref()
                .and_then(|checks| checks.iter().find(|c| c.name == item_name))
                .map(|c| c.command.as_slice()),
        };
        let Some(command_array) = command_array else {
            return Ok(VerificationResult::ItemNotApproved {
                item: item_name.to_string(),
            });
        };

        let binary_name = &command_array[0];
        let (original_path, canonical_path) =
            resolve_binary_path_with_original(binary_name, workspace_root)?;

        // Hash immediately after resolution to keep the TOCTOU property.
        let current_binary_hash = hashing::hash_file(&canonical_path).await?;

        let norm_original = normalize_path_for_storage(&original_path, workspace_root);
        let norm_canonical = normalize_path_for_storage(&canonical_path, workspace_root);

        let argv_matches = |v: &CommandApproval| {
            v.argv.as_deref().is_none() || v.argv.as_deref() == Some(command_array)
        };

        // Normalize the *stored* paths against the current workspace root at comparison time too,
        // so a real legacy entry (which stored absolute paths like `<ws>/./check.sh`) matches the
        // freshly-normalized relative form (`check.sh`) without forced re-approval (plan decision 2).
        // New approvals already stored relative paths, which normalize to themselves. A stored path
        // from a *different* worktree root (legacy absolute) is not inside this workspace and stays
        // absolute — failing closed, which preserves legacy's pre-existing same-workspace-only scope.
        for version in versions {
            let stored_original =
                normalize_path_for_storage(Path::new(&version.original_path), workspace_root);
            let stored_canonical =
                normalize_path_for_storage(Path::new(&version.canonical_path), workspace_root);
            if argv_matches(version)
                && stored_original == norm_original
                && stored_canonical == norm_canonical
                && version.binary_hash == current_binary_hash
            {
                return Ok(VerificationResult::Approved {
                    program: original_path,
                });
            }
        }

        // No version matched. `args_changed` reports whether any approved version even carries
        // the current argv, for message wording only. A legacy `argv: None` entry matches any argv,
        // so it counts as carrying the current argv — otherwise a binary change on a not-yet-upgraded
        // legacy entry would misleadingly report an arguments change.
        // Reuse the same predicate as the approval loop so the legacy wildcard rule
        // (an `argv: None` version matches any argv) cannot drift between the two sites.
        let args_changed = !versions.iter().any(argv_matches);

        Ok(VerificationResult::ItemChanged {
            item: item_name.to_string(),
            args_changed,
            approved_versions: versions.len(),
        })
    }

    /// Add or update approvals for a project, keyed by repository root.
    ///
    /// Callers (the TUI save and test helpers) pass fully-built approvals: `argv` included and
    /// paths already normalized via [`normalize_path_for_storage`] against `workspace_root`. For
    /// each item, an existing legacy (`argv: None`) entry matching on paths + binary hash is
    /// upgraded in place (its `argv` set and `approved_at` refreshed) rather than grown; an
    /// identical existing entry has only its `approved_at` refreshed (re-approval is a
    /// re-confirmation — the signal the follow-up expiration feature will want); otherwise a new
    /// version is pushed with `approved_at = now`, and any stale legacy (`argv: None`) versions for
    /// the same paths are retired so the old binary cannot stay wildcard-approved after a binary
    /// change re-approval.
    pub fn approve_project(
        &mut self,
        workspace_root: PathBuf,
        commands: HashMap<String, CommandApproval>,
        checks: HashMap<String, CommandApproval>,
    ) -> miette::Result<()> {
        // Detect repository root (jj workspace root, git root, or canonicalized path) — approvals
        // stay keyed by repository root so identical content across worktrees needs no re-approval.
        let repository_root = repository::detect_repository_root(&workspace_root)?;
        let project_key = repository_root.to_string_lossy().to_string();

        let entry = self.projects.entry(project_key).or_default();
        let now = Utc::now();
        entry.last_approved = now;

        for (name, approval) in commands {
            upsert_version(&mut entry.commands, name, approval, now, &workspace_root);
        }
        for (name, approval) in checks {
            upsert_version(&mut entry.checks, name, approval, now, &workspace_root);
        }
        Ok(())
    }
}

/// Insert or refresh a single approved version within `map[name]`:
/// - upgrade an existing legacy (`argv: None`) entry matching on paths + binary hash in place;
/// - else refresh `approved_at` on an identical existing entry;
/// - else retire stale legacy (`argv: None`) versions for the same paths (the new argv-bound version
///   supersedes their old-binary wildcard) and push a new version with `approved_at = now`.
///
/// Stored paths are normalized against `workspace_root` at comparison time so a real legacy
/// entry (absolute paths) matches the caller's already-normalized new entry and upgrades in place
/// rather than accreting a dead second version (plan decision 2). The retirement on a binary
/// change is fail-closed: the old binary stops being approved under any argv once the user
/// re-approves the new one, so reverting the binary cannot ride the stale wildcard.
fn upsert_version(
    map: &mut HashMap<String, Vec<CommandApproval>>,
    name: String,
    new: CommandApproval,
    now: DateTime<Utc>,
    workspace_root: &Path,
) {
    let versions = map.entry(name).or_default();
    let norm = |p: &str| normalize_path_for_storage(Path::new(p), workspace_root);

    // Upgrade a legacy argv-less entry whose (normalized) paths + binary hash match: bind argv
    // and refresh approved_at, without growing the set.
    for version in versions.iter_mut() {
        if version.argv.is_none()
            && norm(&version.original_path) == new.original_path
            && norm(&version.canonical_path) == new.canonical_path
            && version.binary_hash == new.binary_hash
        {
            // Upgrade in place: bind argv, stamp approved_at, and converge the stored paths to the
            // normalized form so storage no longer carries the raw legacy absolute paths (future
            // readers still normalize at comparison time, but storage converges).
            version.argv = new.argv.clone();
            version.approved_at = Some(now);
            version.original_path = new.original_path.clone();
            version.canonical_path = new.canonical_path.clone();
            return;
        }
    }

    // Re-approval of an identical version: refresh approved_at (the renewal signal), no growth.
    for version in versions.iter_mut() {
        if version.argv == new.argv
            && norm(&version.original_path) == new.original_path
            && norm(&version.canonical_path) == new.canonical_path
            && version.binary_hash == new.binary_hash
        {
            version.approved_at = Some(now);
            return;
        }
    }

    // No existing version matched on (paths, hash). The new approval binds an explicit argv for
    // the current binary, so retire any stale legacy (`argv: None`) versions for the same paths:
    // they wildcard-matched the *old* binary under any argv, and the user has now moved on. Without
    // this, swapping the binary back to the old hash would re-approve it under any argv — a
    // fail-open regression vector. Cross-workspace argv-bound versions (different hashes, bound
    // argv) are not `argv: None` and are preserved.
    if new.argv.is_some() {
        versions.retain(|v| {
            !(v.argv.is_none()
                && norm(&v.original_path) == new.original_path
                && norm(&v.canonical_path) == new.canonical_path)
        });
    }

    let mut new = new;
    new.approved_at = Some(now);
    versions.push(new);
}

/// Normalize a resolved binary path for cross-worktree storage.
///
/// Returns the path relative to `workspace_root` when the path is inside it, else the absolute
/// path string. Both the original (symlink) path and the canonical path are normalized this way,
/// so an approval made in worktree A matches byte-identical content in worktree B: a relative
/// program like `./tool.sh` stores as `tool.sh` regardless of which worktree resolved it, while a
/// PATH-resolved binary (`which cargo`) lives outside the workspace and stays absolute.
pub fn normalize_path_for_storage(path: &Path, workspace_root: &Path) -> String {
    match path.strip_prefix(workspace_root) {
        Ok(relative) => relative.to_string_lossy().into_owned(),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

/// Resolve a binary name to both its original and canonical paths
///
/// Returns (original_path, canonical_path) where:
/// - original_path: The resolved but not canonicalized path (may be a symlink)
/// - canonical_path: The fully resolved path with all symlinks followed
///
/// This tracks symlinks at multiple levels to detect if any intermediate symlink changes.
pub fn resolve_binary_path_with_original(
    binary_name: &str,
    project_dir: &Path,
) -> miette::Result<(PathBuf, PathBuf)> {
    let path = Path::new(binary_name);

    // Determine the original (non-canonicalized) path
    let original_path = if path.is_absolute() {
        path.to_path_buf()
    } else if binary_name.contains('/') {
        // Relative path - resolve relative to project dir
        project_dir.join(binary_name)
    } else {
        // Look up in PATH
        which::which(binary_name)
            .into_diagnostic()
            .with_context(|| format!("Failed to find binary '{}' in PATH", binary_name))?
    };

    // Canonicalize to get the final path with all symlinks resolved
    let canonical_path = original_path
        .canonicalize()
        .into_diagnostic()
        .with_context(|| {
            format!(
                "Failed to canonicalize binary path: {}",
                original_path.display()
            )
        })?;

    Ok((original_path, canonical_path))
}

/// Check if a file is a script by reading its first bytes for a shebang.
///
/// Scripts are treated specially in the approval flow: if a script is also writable,
/// its full contents are displayed to the user during approval. This prevents hidden
/// malicious code execution by ensuring users can review what will actually run.
pub async fn is_script(path: &Path) -> miette::Result<bool> {
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(path)
        .await
        .into_diagnostic()
        .with_context(|| format!("Failed to open file: {}", path.display()))?;

    let mut buffer = [0u8; 2];

    match file.read_exact(&mut buffer).await {
        Ok(_) => Ok(buffer == *b"#!"),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(miette::miette!("Failed to read file header: {}", e)),
    }
}

/// Check if a file is writable by the current user
#[cfg(unix)]
pub async fn is_writable(path: &Path) -> miette::Result<bool> {
    let metadata = tokio::fs::metadata(path)
        .await
        .into_diagnostic()
        .with_context(|| format!("Failed to get metadata for: {}", path.display()))?;

    let permissions = metadata.permissions();
    let mode = permissions.mode();

    // Check only owner write bit (0o200) for security: if the current user can modify the binary,
    // an attacker with access to this user account can inject malicious code before execution,
    // bypassing our hash-based approval system. Group/other write bits are irrelevant to this threat.
    Ok(mode & 0o200 != 0)
}

/// Check if a path is within a project directory
pub fn is_within_project(binary_path: &Path, project_dir: &Path) -> bool {
    binary_path.starts_with(project_dir)
}

/// Read script contents for display in TUI
pub async fn read_script_contents(path: &Path) -> miette::Result<String> {
    tokio::fs::read_to_string(path)
        .await
        .into_diagnostic()
        .with_context(|| format!("Failed to read script: {}", path.display()))
}

#[cfg(test)]
mod tests;

/// Test helper function re-exported for use in integration tests.
///
/// This function is used by other test modules (e.g., `hooks::tests`, `mcp::tool_runner::tests`)
/// to create approved project configurations for testing purposes.
#[cfg(test)]
pub use tests::approve_project_config;
