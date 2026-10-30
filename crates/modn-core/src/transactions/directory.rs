//! Exclusive directory claims and reversible same-volume quarantine.
use super::journal::target_path;
use crate::planning::InstallPlan;
use anyhow::{bail, Context, Result};
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// A directory can be exclusively owned only when the containing directory
/// already exists and no other planned operation uses it as a shared root.
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
        if roots
            .iter()
            .any(|other| other != root && other.starts_with(&format!("{root}/")))
        {
            bail!("overlapping exclusive directory claims: {root}");
        }
        for op in &plan.operations {
            if op.destination.starts_with(&format!("{root}/"))
                && op.owned_directory.as_deref() != Some(root)
            {
                bail!("mixed ownership in exclusive directory: {root}");
            }
        }
    }
    Ok(roots.into_iter().collect())
}

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

pub(super) fn quarantine(game: &Path, root: &str, id: &str, ordinal: usize) -> Result<()> {
    let path = target_path(game, root, false)?;
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("owned directory missing or linked: {}", path.display());
    }
    let stage = quarantine_path(game, root, id, ordinal)?;
    if fs::symlink_metadata(&stage).is_ok() {
        bail!(
            "directory quarantine destination occupied: {}",
            stage.display()
        );
    }
    fs::rename(&path, &stage).with_context(|| format!("cannot quarantine {}", path.display()))
}

/// Recover pending removal by rename, or purge committed removal without
/// traversing directory links (std remove_dir_all does not follow symlinks).
pub(super) fn recover_directory(
    game: &Path,
    root: &str,
    id: &str,
    ordinal: usize,
    committed: bool,
) -> Result<()> {
    let path = target_path(game, root, false)?;
    let stage = quarantine_path(game, root, id, ordinal)?;
    let check = |p: &Path| -> Result<bool> {
        match fs::symlink_metadata(p) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => Ok(true),
            Ok(_) => bail!("unsafe directory in recovery: {}", p.display()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    };
    let saved = check(&stage)?;
    let active = check(&path)?;
    if committed {
        if saved {
            fs::remove_dir_all(&stage)?;
        }
    } else if saved {
        if active {
            bail!("directory reappeared during recovery: {}", path.display());
        }
        fs::rename(&stage, &path)?;
    } else if !active {
        bail!("no safe recovery witness for {}", path.display());
    }
    Ok(())
}

pub(crate) fn covers(roots: &[String], relative: &str) -> bool {
    roots
        .iter()
        .any(|root| relative.starts_with(&format!("{root}/")))
}
