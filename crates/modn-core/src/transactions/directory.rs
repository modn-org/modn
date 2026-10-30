//! Game-agnostic exclusive directory identities and recoverable same-volume moves.
use super::journal::target_path;
use crate::planning::InstallPlan;
use anyhow::{bail, Context, Result};
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Uppercasing each scalar avoids Rust's contextual lowercasing (Greek final
/// sigma) when comparing *missing* Windows paths. This is a conservative
/// collision guard, not proof of identity of an existing directory.
pub(super) fn case_key(path: &str) -> String {
    if cfg!(windows) {
        path.chars().flat_map(char::to_uppercase).collect()
    } else {
        path.to_owned()
    }
}

pub(super) fn covers(roots: &[String], relative: &str) -> bool {
    let key = case_key(relative);
    roots
        .iter()
        .any(|root| key.starts_with(&format!("{}/", case_key(root))))
}

pub(super) fn exclusive_roots(game: &Path, plan: &InstallPlan) -> Result<Vec<String>> {
    let mut roots = BTreeSet::new();
    for op in &plan.operations {
        if let Some(root) = &op.owned_directory {
            if !op.destination.starts_with(&format!("{root}/"))
                || !root.contains('/')
                || root
                    .split('/')
                    .any(|c| c.is_empty() || c == "." || c == "..")
            {
                bail!("invalid exclusive directory claim: {root}");
            }
            roots.insert(root.clone());
        }
    }
    for root in &roots {
        let target = target_path(game, root, false)?;
        if fs::symlink_metadata(&target).is_ok() {
            bail!(
                "exclusive directory must not preexist: {}",
                target.display()
            );
        }
        let parent = target.parent().context("directory has no parent")?;
        let meta = fs::symlink_metadata(parent)
            .with_context(|| format!("exclusive parent missing: {}", parent.display()))?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            bail!("exclusive parent is unsafe: {}", parent.display());
        }
        let key = case_key(root);
        if roots.iter().any(|other| {
            other != root
                && (case_key(other) == key
                    || case_key(other).starts_with(&format!("{key}/"))
                    || key.starts_with(&format!("{}/", case_key(other))))
        }) {
            bail!("overlapping exclusive directory claims: {root}");
        }
        for op in &plan.operations {
            if case_key(&op.destination).starts_with(&format!("{key}/"))
                && op.owned_directory.as_deref() != Some(root)
            {
                bail!("mixed ownership in exclusive directory: {root}");
            }
        }
    }
    Ok(roots.into_iter().collect())
}

/// Persist the directory object's native identity, never a file planted inside
/// it. The identity survives rename and partial recursive deletion.
#[cfg(windows)]
pub(super) fn identity(path: &Path) -> Result<String> {
    use std::os::windows::fs::MetadataExt;
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        bail!("unsafe directory identity: {}", path.display());
    }
    let handle = winapi_util::Handle::from_path_any(path)?;
    let info = winapi_util::file::information(&handle)?;
    Ok(format!(
        "win:{}:{}:{}",
        info.volume_serial_number(),
        info.file_index(),
        meta.creation_time()
    ))
}

#[cfg(unix)]
pub(super) fn identity(path: &Path) -> Result<String> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        bail!("unsafe directory identity: {}", path.display());
    }
    Ok(format!("unix:{}:{}", meta.dev(), meta.ino()))
}

#[cfg(not(any(windows, unix)))]
pub(super) fn identity(_path: &Path) -> Result<String> {
    bail!("directory ownership needs native filesystem identity support")
}

pub(super) fn verify_directory(path: &Path, expected: &str) -> Result<()> {
    if expected.is_empty() || identity(path)? != expected {
        bail!("owned directory identity changed: {}", path.display());
    }
    Ok(())
}

/// The same-volume temporary folder is empty until its identity has been
/// persisted. A crash before recording identity therefore cannot leave an
/// unverified live mod installation.
pub(super) fn quarantine_path(
    game: &Path,
    root: &str,
    id: &str,
    ordinal: usize,
) -> Result<PathBuf> {
    let path = target_path(game, root, false)?;
    Ok(path
        .parent()
        .context("directory has no parent")?
        .join(format!(".modn-txn-{id}-dir-{ordinal}.tmp")))
}

/// A same-volume move must never replace a concurrently created directory.
fn rename_unoccupied(from: &Path, to: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            from,
            rustix::fs::CWD,
            to,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(io::Error::from)?;
    }
    #[cfg(windows)]
    {
        // MoveFileEx cannot replace an existing directory with another directory.
        fs::rename(from, to)?;
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        bail!("exclusive folder moves require a no-replace rename on this platform");
    }
    Ok(())
}

pub(super) fn prepare_install(game: &Path, root: &str, id: &str, n: usize) -> Result<String> {
    let active = target_path(game, root, false)?;
    let staged = quarantine_path(game, root, id, n)?;
    if fs::symlink_metadata(&active).is_ok() {
        bail!("exclusive directory already exists: {}", active.display());
    }
    fs::create_dir(&staged)?;
    identity(&staged)
}

pub(super) fn activate_install(
    game: &Path,
    root: &str,
    id: &str,
    n: usize,
    expected: &str,
) -> Result<()> {
    let active = target_path(game, root, false)?;
    let staged = quarantine_path(game, root, id, n)?;
    verify_directory(&staged, expected)?;
    if fs::symlink_metadata(&active).is_ok() {
        bail!("exclusive directory already exists: {}", active.display());
    }
    rename_unoccupied(&staged, &active)
        .with_context(|| format!("cannot activate owned directory {}", active.display()))
}

pub(super) fn quarantine(
    game: &Path,
    root: &str,
    id: &str,
    n: usize,
    expected: &str,
) -> Result<()> {
    let active = target_path(game, root, false)?;
    verify_directory(&active, expected)?;
    let staged = quarantine_path(game, root, id, n)?;
    if fs::symlink_metadata(&staged).is_ok() {
        bail!(
            "directory quarantine destination occupied: {}",
            staged.display()
        );
    }
    rename_unoccupied(&active, &staged)?;
    Ok(())
}

fn present_directory(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => Ok(true),
        Ok(_) => bail!("unsafe directory in recovery: {}", path.display()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// Committed deletion is retryable: the directory ID persists even when its
/// contents were partially removed, and a substituted directory is refused.
pub(super) fn recover_directory(
    game: &Path,
    root: &str,
    id: &str,
    n: usize,
    expected: &str,
    committed: bool,
) -> Result<()> {
    let active = target_path(game, root, false)?;
    let staged = quarantine_path(game, root, id, n)?;
    let saved = present_directory(&staged)?;
    let live = present_directory(&active)?;
    if committed {
        if live {
            bail!(
                "owned directory reappeared after commit: {}",
                active.display()
            );
        }
        if saved {
            verify_directory(&staged, expected)?;
            fs::remove_dir_all(&staged)?;
        }
    } else if saved {
        verify_directory(&staged, expected)?;
        if live {
            bail!(
                "owned directory reappeared during recovery: {}",
                active.display()
            );
        }
        rename_unoccupied(&staged, &active)?;
    } else if live {
        verify_directory(&active, expected)?;
    } else {
        bail!(
            "missing owned directory recovery witness: {}",
            active.display()
        );
    }
    Ok(())
}

/// Only empty nested directories may be removed during an uncommitted install.
/// Unknown files/links preserve the journal and require manual resolution.
fn prune_empty(path: &Path) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let child = entry?;
        let kind = child.file_type()?;
        if !kind.is_dir() || kind.is_symlink() {
            bail!(
                "unknown content blocks directory rollback: {}",
                child.path().display()
            );
        }
        prune_empty(&child.path())?;
    }
    fs::remove_dir(path)?;
    Ok(())
}

pub(super) fn recover_install(
    game: &Path,
    root: &str,
    id: &str,
    n: usize,
    expected: Option<&str>,
) -> Result<()> {
    let active = target_path(game, root, false)?;
    let staged = quarantine_path(game, root, id, n)?;
    let saved = present_directory(&staged)?;
    let live = present_directory(&active)?;
    if saved && live {
        bail!("both live and staged owned directories exist: {root}");
    }
    if let Some(expected) = expected {
        if saved {
            verify_directory(&staged, expected)?;
            prune_empty(&staged)?;
        }
        if live {
            verify_directory(&active, expected)?;
            prune_empty(&active)?;
        }
    } else {
        if live {
            bail!("unproven active directory during recovery: {root}");
        }
        if saved {
            // There are no game payload writes before recording identity.
            // Do not recurse into an unproven directory.
            fs::remove_dir(&staged).with_context(|| {
                format!("unproven staging directory not empty: {}", staged.display())
            })?;
        }
    }
    Ok(())
}
