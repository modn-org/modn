//! `modn list`, `uninstall` and `recover`: manager-owned packages and journals.
use crate::selection::selected_installation;
use crate::state_access::{ManagerState, ProtectedTarget};
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

pub(super) fn list(
    path: Option<PathBuf>,
    state_path: &Path,
    requested: Option<&str>,
    json: bool,
) -> Result<()> {
    let state = ManagerState::load(state_path)?;
    let target = state.target_for(path.as_deref(), requested)?;
    let store = state.open_writable(&target)?;
    let (_, game) = selected_installation(Some(&store), path, requested)?;
    let packages = store.installed(&game)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&packages)?);
    } else if packages.is_empty() {
        println!("No manager-owned packages installed.");
    } else {
        for package in packages {
            println!("{}: {} owned files", package.id, package.files.len());
        }
    }
    Ok(())
}

pub(super) fn uninstall(
    package_id: &str,
    path: Option<PathBuf>,
    yes: bool,
    state_path: &Path,
    requested: Option<&str>,
    json: bool,
) -> Result<()> {
    let state = ManagerState::load(state_path)?;
    let target = state.target_for(path.as_deref(), requested)?;
    let mut store = state.open_writable(&target)?;
    if !yes {
        bail!("uninstall removes game files; rerun with --yes");
    }
    let (_, game) = selected_installation(Some(&store), path, requested)?;
    store.uninstall(&game, package_id)?;
    if json {
        println!("{}", serde_json::json!({"uninstalled": package_id}));
    } else {
        println!("Uninstalled {package_id}");
    }
    Ok(())
}

pub(super) fn recover(
    yes: bool,
    state_path: &Path,
    requested: Option<&str>,
    json: bool,
) -> Result<()> {
    let mut store = ManagerState::load(state_path)?
        .require_existing("no manager state database to recover")?
        .open_writable(&ProtectedTarget {
            game: requested.map(str::to_owned),
            root: None,
        })?;
    if !yes {
        bail!("recovery may roll back interrupted game changes; rerun with --yes");
    }
    let count = store.recover()?;
    if json {
        println!("{}", serde_json::json!({"recovered": count}));
    } else {
        println!("Recovered {count} interrupted transactions.");
    }
    Ok(())
}
