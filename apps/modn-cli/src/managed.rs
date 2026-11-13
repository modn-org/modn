use crate::commands::PinArgs;
use crate::commands::UninstallArgs;
use crate::progress::Progress;
use crate::state_access::{InstallationAccess, ManagerState, ProtectedTarget};
use anyhow::{bail, Context, Result};
use modn_core::error::{coded, ErrorCode};
use modn_core::history::HistoryContext;
use modn_core::transactions::{CleanupApproval, UninstallScope};
use std::path::{Path, PathBuf};

pub(super) fn list(
    path: Option<PathBuf>,
    state_path: &Path,
    requested: Option<&str>,
    json: bool,
) -> Result<()> {
    let selected = ManagerState::load(state_path)?
        .for_installation(path, requested)?
        .open(InstallationAccess::Write)?;
    let store = selected.store.expect("write access opens manager state");
    let game = selected.root;
    let packages = store.installed(&game)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&packages)?);
        return Ok(());
    }
    if packages.is_empty() {
        println!("No manager-owned packages installed.");
    } else {
        for package in packages {
            let pin = if package.pinned { " (pinned)" } else { "" };
            match &package.catalog {
                Some(catalog) => println!(
                    "{}{pin}: {catalog}, {} owned files",
                    package.id,
                    package.files.len()
                ),
                None => println!("{}{pin}: {} owned files", package.id, package.files.len()),
            }
            for directory in &package.directories {
                println!("  exclusive folder: {directory}");
            }
        }
    }
    let created = store.created_directories(&game)?;
    if !created.is_empty() {
        println!("Folders Modn created, removed when empty:");
        for directory in created {
            println!("  {directory}");
        }
    }
    Ok(())
}

pub(super) fn uninstall(
    args: &UninstallArgs,
    state_path: &Path,
    requested: Option<&str>,
    json: bool,
    progress: &Progress,
) -> Result<()> {
    progress.begin()?;
    let state = ManagerState::load(state_path)?;
    let state = state.for_installation(args.path.clone(), requested)?;
    let scope = UninstallScope::default()
        .including_dependencies(args.include_deps)
        .including_dependents(args.cascade);
    if args.dry_run {
        let selected = state.open(InstallationAccess::Preview)?;
        let store = selected
            .store
            .expect("preview access requires manager state");
        let game = selected.root;
        let plan = store.plan_uninstall(&game, &args.package, scope)?;
        if args.force {
            let id = &plan.first().expect("a removal plan selects one package").id;
            let package = store
                .installed(&game)?
                .into_iter()
                .find(|p| &p.id == id)
                .context("managed package changed during preview; retry the command")?;
            let pending = "Current paths, missing files, directory identities, and pending transactions are checked under the commit lock.";
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "dry_run": true, "force": true, "status": "verification_required",
                        "packages": plan, "owned_files": package.files,
                        "checks_pending_until_commit": pending,
                    }))?
                );
            } else {
                println!("Would clean recorded files of {id}, including modified files:");
                for file in &package.files {
                    println!("  {}", file.destination);
                }
                println!("{pending}");
            }
            return Ok(());
        }
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
        return Err(coded(
            ErrorCode::ApprovalRequired,
            "uninstall removes game files; rerun with --yes, or --dry-run to preview",
        ));
    }
    let selected = state.open(InstallationAccess::Write)?;
    let mut store = selected.store.expect("write access opens manager state");
    store.set_control(progress.control().clone());
    let game = selected.root;
    store.set_history_context(HistoryContext::new(selected.game_id, game.clone()));
    let removed = if args.force {
        let plan = store.plan_uninstall(&game, &args.package, scope)?;
        let id = &plan.first().expect("a removal plan selects one package").id;
        store.cleanup_owned(&game, id, CleanupApproval::Approved)?;
        vec![id.clone()]
    } else {
        store.uninstall_with(&game, &args.package, scope)?
    };
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

pub(super) fn pin(
    args: &PinArgs,
    state_path: &Path,
    requested: Option<&str>,
    pinned: bool,
) -> Result<()> {
    let selected = ManagerState::load(state_path)?
        .for_installation(args.path.clone(), requested)?
        .open(InstallationAccess::Write)?;
    let mut store = selected.store.expect("write access opens manager state");
    let game = selected.root;
    let (id, changed) = store.set_package_pin(&game, &args.package, pinned)?;
    let status = if pinned { "pinned" } else { "unpinned" };
    let unchanged = if changed { "" } else { " (unchanged)" };
    println!("{id}: {status}{unchanged}");
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
