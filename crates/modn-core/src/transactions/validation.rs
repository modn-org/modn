//! Core-side validation before any approved ZIP operation is staged.
//! Game policy never authorizes filesystem writes by itself.
use crate::archive::{validated_name, Inspection};
use crate::planning::{
    existing_destination_issue, insert_destination, prohibited_payload, InstallPlan,
};
use crate::state::Store;
use anyhow::{bail, Result};
use rusqlite::params;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const MAX_FILE: u64 = 4 * 1024 * 1024 * 1024;
const MAX_TOTAL: u64 = 8 * 1024 * 1024 * 1024;

/// Require an existing real directory; no game-specific executable or layout checks.
pub(crate) fn validate_transaction_root(game: &Path) -> Result<PathBuf> {
    let metadata = fs::symlink_metadata(game)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        bail!("transaction root must be a real directory, not a link");
    }
    Ok(game.canonicalize()?)
}

/// Revalidate a caller-approved proposal independently of its origin and
/// against fresh archive headers and live target paths under the store lock.
pub(super) fn validate_execution_plan(
    inspection: &Inspection,
    game: &Path,
    proposal: &InstallPlan,
) -> Result<()> {
    if inspection.format != "zip" {
        bail!("transaction executor currently supports ZIP file operations only");
    }
    if proposal.status != "reviewable" || !proposal.blockers.is_empty() || !proposal.metadata_only {
        bail!("blocked or invalid installation proposal cannot be executed");
    }
    if proposal.archive_sha256 != inspection.sha256 {
        bail!("archive changed since the proposed installation was reviewed");
    }
    if validate_transaction_root(&proposal.game_installation)? != game {
        bail!("installation proposal targets a different game root");
    }
    if proposal.operations.is_empty() {
        bail!("installation proposal contains no file operations");
    }

    let mut sources = HashMap::new();
    for entry in &inspection.files {
        if prohibited_payload(&entry.entry) {
            bail!("executable payload is not supported: {}", entry.entry);
        }
        sources.insert(entry.entry.as_str(), entry.bytes);
    }
    let mut selected = HashSet::new();
    let mut targets = BTreeSet::new();
    let mut total = 0u64;
    for operation in &proposal.operations {
        if operation.source.len() > 4_096 || !selected.insert(operation.source.as_str()) {
            bail!("invalid or duplicate source: {}", operation.source);
        }
        let Some(&actual_bytes) = sources.get(operation.source.as_str()) else {
            bail!("source is not present in ZIP: {}", operation.source);
        };
        if operation.bytes != actual_bytes || operation.bytes > MAX_FILE {
            bail!(
                "source size mismatch or file exceeds 4 GiB: {}",
                operation.source
            );
        }
        total = total
            .checked_add(operation.bytes)
            .ok_or_else(|| anyhow::anyhow!("total operation size overflow"))?;
        if total > MAX_TOTAL {
            bail!("total installation exceeds 8 GiB");
        }
        if operation.destination.len() > 1_024
            || !validated_name(&operation.destination)
                .is_ok_and(|name| name == operation.destination)
            || !operation.destination.contains('/')
            || prohibited_payload(&operation.destination)
        {
            bail!("invalid destination: {}", operation.destination);
        }
        if operation.kind.is_empty()
            || operation.kind.len() > 64
            || !operation
                .kind
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            bail!("invalid operation kind: {}", operation.kind);
        }
        if let Some(code) = insert_destination(&mut targets, &operation.destination) {
            bail!("{code}: {}", operation.destination);
        }
        if let Some(code) = existing_destination_issue(game, &operation.destination) {
            bail!("{code}: {}", operation.destination);
        }
    }
    Ok(())
}

/// Reserve even missing owned destinations before staging. SQLite ownership
/// must not be silently replaced if a managed file has disappeared on disk.
pub(super) fn validate_ownership(store: &Store, game: &Path, proposal: &InstallPlan) -> Result<()> {
    validate_ownership_paths(
        store,
        game,
        proposal
            .operations
            .iter()
            .map(|operation| operation.destination.as_str()),
    )
}

/// Reject overlapping installation roots, owned destinations and aliases.
/// Reused by both installation and metadata-only adoption.
pub(crate) fn validate_ownership_paths<'a>(
    store: &Store,
    game: &Path,
    paths: impl IntoIterator<Item = &'a str>,
) -> Result<()> {
    let selected = PathBuf::from(game.to_string_lossy().to_lowercase());
    let mut roots = store
        .connection
        .prepare("SELECT DISTINCT game_path FROM managed_packages")?;
    let roots = roots
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for recorded in roots {
        let other = PathBuf::from(recorded.to_lowercase());
        if other != selected && (other.starts_with(&selected) || selected.starts_with(&other)) {
            bail!("installation root overlaps another managed installation");
        }
    }
    let mut query = store
        .connection
        .prepare("SELECT path_key FROM managed_files WHERE game_path=?1")?;
    let owned = query
        .query_map(params![game.to_string_lossy().as_ref()], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut reserved: BTreeSet<String> = owned.into_iter().collect();
    let mut q = store
        .connection
        .prepare("SELECT path_key FROM managed_directories WHERE game_path=?1")?;
    let directories = q
        .query_map(params![game.to_string_lossy().as_ref()], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for destination in paths {
        let folded = destination.to_lowercase();
        if directories
            .iter()
            .any(|root| folded == *root || folded.starts_with(&format!("{root}/")))
        {
            bail!("destination is inside an exclusively owned directory: {destination}");
        }
        if let Some(code) = insert_destination(&mut reserved, destination) {
            bail!("{code}: destination already reserved by manager: {destination}");
        }
    }
    Ok(())
}

// Resolve existing ancestors before comparing a missing registered root.
// An ancestor may have become a symlink or junction after registration.
fn resolve_registered_root(path: &Path) -> Result<PathBuf> {
    let mut missing = Vec::new();
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => {
                // A dangling link must fail closed rather than be skipped.
                let mut resolved = ancestor.canonicalize()?;
                for component in missing.into_iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Some(component) = ancestor.file_name() {
                    missing.push(component.to_os_string());
                } else if path.is_absolute() {
                    // No surviving prefix (for example, an offline drive).
                    return Ok(path.to_path_buf());
                } else {
                    return Err(error.into());
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
    bail!("registered installation root has no resolvable ancestor")
}

// Registered game roots must not overlap another game's adoption target.
// An existing game marker alone does not authorize cross-game file ownership.
pub(crate) fn validate_registered_adoption_roots(
    store: &Store,
    game_id: &str,
    game: &Path,
) -> Result<()> {
    for registration in store.list()? {
        let other = resolve_registered_root(&registration.path)?;
        if (other == game && registration.game_id != game_id)
            || (other != game && (other.starts_with(game) || game.starts_with(&other)))
        {
            bail!("adoption installation root overlaps another registered game");
        }
    }
    Ok(())
}

/// Directory claims must not cover an existing package's reserved paths, even
/// when its files have been removed from disk. This prevents ownership theft.
pub(super) fn validate_directory_reservations(
    store: &Store,
    game: &Path,
    directories: &[String],
) -> Result<()> {
    let mut files = store
        .connection
        .prepare("SELECT path_key FROM managed_files WHERE game_path=?1")?;
    let occupied = files
        .query_map(params![game.to_string_lossy().as_ref()], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut query = store
        .connection
        .prepare("SELECT path_key FROM managed_directories WHERE game_path=?1")?;
    let owned_dirs = query
        .query_map(params![game.to_string_lossy().as_ref()], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for directory in directories {
        let key = directory.to_lowercase();
        if occupied.iter().chain(owned_dirs.iter()).any(|owned| {
            owned == &key
                || owned.starts_with(&format!("{key}/"))
                || key.starts_with(&format!("{owned}/"))
        }) {
            bail!("exclusive directory overlaps existing ownership: {directory}");
        }
    }
    Ok(())
}
