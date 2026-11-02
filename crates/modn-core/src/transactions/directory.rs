use super::journal::target_path;
use crate::planning::InstallPlan;
use anyhow::{bail, Context, Result};
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub(crate) fn case_key(path: &str) -> String {
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
        let target = target_path(game, root)?;
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

#[cfg(target_os = "linux")]
pub(super) fn identity(path: &Path) -> Result<String> {
    use rustix::fs::{AtFlags, StatxFlags};
    use std::os::unix::fs::MetadataExt;

    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        bail!("unsafe directory identity: {}", path.display());
    }
    let stat = rustix::fs::statx(
        rustix::fs::CWD,
        path,
        AtFlags::SYMLINK_NOFOLLOW,
        StatxFlags::BASIC_STATS | StatxFlags::BTIME,
    )
    .map_err(io::Error::from)?;
    if (stat.stx_mask & StatxFlags::BTIME.bits()) == 0 || stat.stx_ino != meta.ino() {
        bail!(
            "filesystem cannot prove directory birth identity: {}",
            path.display()
        );
    }
    // Device and inode alone can be reused after deletion. An actual birth
    // timestamp makes the reallocated directory a different identity.
    Ok(format!(
        "linux:{}:{}:{}:{}:{}",
        stat.stx_dev_major,
        stat.stx_dev_minor,
        stat.stx_ino,
        stat.stx_btime.tv_sec,
        stat.stx_btime.tv_nsec,
    ))
}

#[cfg(not(any(windows, target_os = "linux")))]
pub(super) fn identity(_path: &Path) -> Result<String> {
    bail!("exclusive directory ownership requires reliable native birth identity")
}

pub(super) fn verify_directory(path: &Path, expected: &str) -> Result<()> {
    if expected.is_empty() || identity(path)? != expected {
        bail!("owned directory identity changed: {}", path.display());
    }
    Ok(())
}

pub(super) fn quarantine_path(
    game: &Path,
    root: &str,
    id: &str,
    ordinal: usize,
) -> Result<PathBuf> {
    let path = target_path(game, root)?;
    Ok(path
        .parent()
        .context("directory has no parent")?
        .join(format!(".modn-txn-{id}-dir-{ordinal}.tmp")))
}

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
    let active = target_path(game, root)?;
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
    let active = target_path(game, root)?;
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
    let active = target_path(game, root)?;
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

pub(super) fn recover_directory(
    game: &Path,
    root: &str,
    id: &str,
    n: usize,
    expected: &str,
    committed: bool,
) -> Result<()> {
    let active = target_path(game, root)?;
    let staged = quarantine_path(game, root, id, n)?;
    let saved = present_directory(&staged)?;
    let live = present_directory(&active)?;
    if committed {
        if live && identity(&active)? == expected {
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
    let active = target_path(game, root)?;
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
            fs::remove_dir(&staged).with_context(|| {
                format!("unproven staging directory not empty: {}", staged.display())
            })?;
        }
    }
    Ok(())
}
