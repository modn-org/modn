//! Exclusive directory claims and reversible same-volume quarantine.
use super::journal::target_path;
use crate::planning::InstallPlan;
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const OWNER_MARKER: &str = ".modn-ownership";

fn case_key(path: &str) -> String {
    if cfg!(windows) {
        path.to_lowercase()
    } else {
        path.to_owned()
    }
}

/// An exclusive claim cannot overlap file-owned paths or an aliased claim.
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

/// A sidecar in the shared parent is hard-linked to a marker in the mod folder.
/// Both names must refer to the original file object even across renames.
fn witness_path(directory: &Path, root: &str) -> Result<PathBuf> {
    let digest = format!("{:x}", Sha256::digest(root.as_bytes()));
    Ok(directory
        .parent()
        .context("directory has no parent")?
        .join(format!(".modn-dir-{}.witness", &digest[..24])))
}

fn witness_prefix(root: &str) -> String {
    format!("modn-owned-directory-v1\n{root}\n")
}

fn verify_sidecar(path: &Path, root: &str) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        bail!("unsafe ownership witness: {}", path.display());
    }
    let bytes = fs::read(path)?;
    if !bytes.starts_with(witness_prefix(root).as_bytes()) || bytes.len() > 4096 {
        bail!("ownership witness changed: {}", path.display());
    }
    Ok(())
}

pub(super) fn create_witness(directory: &Path, root: &str, id: &str) -> Result<()> {
    let sidecar = witness_path(directory, root)?;
    let mut witness = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&sidecar)?;
    witness.write_all(format!("{}{}\n", witness_prefix(root), id).as_bytes())?;
    witness.sync_all()?;
    fs::hard_link(&sidecar, directory.join(OWNER_MARKER))
        .with_context(|| format!("cannot mark exclusive directory {}", directory.display()))?;
    verify_directory(directory, root)
}

pub(super) fn verify_directory(directory: &Path, root: &str) -> Result<()> {
    let meta = fs::symlink_metadata(directory)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        bail!("unsafe owned directory: {}", directory.display());
    }
    let sidecar = witness_path(directory, root)?;
    verify_sidecar(&sidecar, root)?;
    let marker = directory.join(OWNER_MARKER);
    let file_meta = fs::symlink_metadata(&marker)?;
    if !file_meta.is_file()
        || file_meta.file_type().is_symlink()
        || !same_file::is_same_file(&sidecar, &marker)?
    {
        bail!("owned directory identity changed: {}", directory.display());
    }
    Ok(())
}

fn remove_sidecar(directory: &Path, root: &str) -> Result<()> {
    let sidecar = witness_path(directory, root)?;
    match fs::symlink_metadata(&sidecar) {
        Ok(_) => {
            verify_sidecar(&sidecar, root)?;
            fs::remove_file(sidecar)?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
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
    verify_directory(&path, root)?;
    let stage = quarantine_path(game, root, id, ordinal)?;
    if fs::symlink_metadata(&stage).is_ok() {
        bail!(
            "directory quarantine destination occupied: {}",
            stage.display()
        );
    }
    fs::rename(&path, &stage).with_context(|| format!("cannot quarantine {}", path.display()))
}

/// Never restore or delete a substituted quarantine folder.
pub(super) fn recover_directory(
    game: &Path,
    root: &str,
    id: &str,
    ordinal: usize,
    committed: bool,
) -> Result<()> {
    let path = target_path(game, root, false)?;
    let stage = quarantine_path(game, root, id, ordinal)?;
    let exists = |p: &Path| -> Result<bool> {
        match fs::symlink_metadata(p) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => Ok(true),
            Ok(_) => bail!("unsafe directory in recovery: {}", p.display()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    };
    let saved = exists(&stage)?;
    let active = exists(&path)?;
    if committed {
        if active {
            bail!(
                "directory reappeared during committed recovery: {}",
                path.display()
            );
        }
        if saved {
            verify_directory(&stage, root)?;
            fs::remove_dir_all(&stage)?;
        }
        remove_sidecar(&path, root)?;
    } else if saved {
        verify_directory(&stage, root)?;
        if active {
            bail!("directory reappeared during recovery: {}", path.display());
        }
        fs::rename(&stage, &path)?;
    } else if !active {
        bail!("no safe recovery witness for {}", path.display());
    } else {
        verify_directory(&path, root)?;
    }
    Ok(())
}

pub(crate) fn covers(roots: &[String], relative: &str) -> bool {
    let key = case_key(relative);
    roots
        .iter()
        .any(|root| key.starts_with(&format!("{}/", case_key(root))))
}

/// Prune only empty nested folders after all verified file witnesses are removed.
fn prune_empty_nested(path: &Path) -> Result<()> {
    for child in fs::read_dir(path)? {
        let child = child?;
        let meta = child.file_type()?;
        if !meta.is_dir() || meta.is_symlink() {
            bail!(
                "unknown content blocks rollback: {}",
                child.path().display()
            );
        }
        prune_empty_nested(&child.path())?;
    }
    fs::remove_dir(path)?;
    Ok(())
}

pub(super) fn rollback_empty(game: &Path, roots: &[String]) -> Result<()> {
    for root in roots.iter().rev() {
        let path = target_path(game, root, false)?;
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                remove_sidecar(&path, root)?;
                continue;
            }
            Err(error) => return Err(error.into()),
            Ok(_) => verify_directory(&path, root)?,
        }
        for child in fs::read_dir(&path)? {
            let child = child?;
            if child.file_name() == OWNER_MARKER {
                continue;
            }
            let meta = child.file_type()?;
            if !meta.is_dir() || meta.is_symlink() {
                bail!(
                    "unknown content blocks rollback: {}",
                    child.path().display()
                );
            }
            prune_empty_nested(&child.path())?;
        }
        fs::remove_file(path.join(OWNER_MARKER))?;
        fs::remove_dir(&path)?;
        remove_sidecar(&path, root)?;
    }
    Ok(())
}
