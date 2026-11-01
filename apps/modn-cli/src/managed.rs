//! `modn list`, `uninstall` and `recover`: manager-owned packages and journals.
use crate::commands::UninstallArgs;
use crate::selection::selected_installation;
use crate::state_access::{ManagerState, ProtectedTarget};
use anyhow::{bail, Result};
use modn_core::transactions::UninstallScope;
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
            match &package.catalog {
                Some(catalog) => println!(
                    "{}: {catalog}, {} owned files",
                    package.id,
                    package.files.len()
                ),
                None => println!("{}: {} owned files", package.id, package.files.len()),
            }
        }
    }
    Ok(())
}

pub(super) fn uninstall(
    args: &UninstallArgs,
    state_path: &Path,
    requested: Option<&str>,
    json: bool,
) -> Result<()> {
    let state = ManagerState::load(state_path)?;
    let target = state.target_for(args.path.as_deref(), requested)?;
    let scope = UninstallScope::default()
        .including_dependencies(args.include_deps)
        .including_dependents(args.cascade);
    if args.dry_run {
        let store = state.open_preview(&target)?;
        let (_, game) = selected_installation(Some(&store), args.path.clone(), requested)?;
        let plan = store.plan_uninstall(&game, &args.package, scope)?;
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"dry_run": true, "packages": plan})
                )?
            );
        } else {
            println!("Would uninstall, in this order:");
            for step in &plan {
                match &step.catalog {
                    Some(catalog) => {
                        println!("  {} {catalog} ({} owned files)", step.id, step.files);
                    }
                    None => println!("  {} ({} owned files)", step.id, step.files),
                }
            }
        }
        return Ok(());
    }
    if !args.yes {
        bail!("uninstall removes game files; rerun with --yes, or --dry-run to preview");
    }
    let mut store = state.open_writable(&target)?;
    let (_, game) = selected_installation(Some(&store), args.path.clone(), requested)?;
    let removed = store.uninstall_with(&game, &args.package, scope)?;
    if json {
        println!(
            "{}",
            serde_json::json!({"uninstalled": args.package, "packages": removed})
        );
    } else {
        for id in &removed {
            println!("Uninstalled {id}");
        }
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
