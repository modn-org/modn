use crate::commands::PinArgs;
use crate::commands::UninstallArgs;
use crate::progress::Progress;
use crate::state_access::{InstallationAccess, ManagerState, ProtectedTarget};
use anyhow::{bail, Context, Result};
use modn_core::error::{coded, ErrorCode};
use modn_core::history::HistoryContext;
use modn_core::manager_roots::ManagerRoots;
use modn_core::transactions::{CleanupApproval, UninstallScope};
use std::collections::HashMap;
use std::path::PathBuf;

pub(super) fn list(
    path: Option<PathBuf>,
    roots: &ManagerRoots,
    requested: Option<&str>,
    json: bool,
) -> Result<()> {
    let selected = ManagerState::load(roots)?
        .for_installation(roots, path, requested)?
        .open(InstallationAccess::Write)?;
    let store = selected.store.expect("write access opens manager state");
    let game = selected.root;
    let packages = store.installed(&game)?;
    let profiles = modn_core::profiles::list(&store, &game)?;
    let active = profiles.iter().find(|profile| profile.active);
    let profile_name = active.map(|profile| profile.name.clone());
    let mut members = HashMap::new();
    let mut ranks = HashMap::new();
    if let Some(profile) = active {
        for member in &profile.packages {
            if member.enabled {
                let rank = ranks.len() + 1;
                ranks.insert(member.package_id.as_str(), rank);
            }
            members.insert(member.package_id.as_str(), member);
        }
    }
    if json {
        let mut entries = Vec::with_capacity(packages.len());
        for package in &packages {
            let mut value = serde_json::to_value(package)?;
            let Some(entry) = value.as_object_mut() else {
                bail!(
                    "managed package {package_id} is not a JSON object",
                    package_id = package.id
                );
            };
            let member = members.get(package.id.as_str());
            entry.insert("profile".into(), name_value(profile_name.as_deref()));
            entry.insert(
                "rank".into(),
                ranks
                    .get(package.id.as_str())
                    .map_or(serde_json::Value::Null, |rank| serde_json::json!(rank)),
            );
            entry.insert(
                "enabled".into(),
                serde_json::json!(member.is_some_and(|member| member.enabled)),
            );
            entry.insert(
                "shadowed_files".into(),
                serde_json::json!(member.map_or(0, |member| member.shadowed_files)),
            );
            entry.insert(
                "priority".into(),
                member.map_or(serde_json::Value::Null, |member| {
                    serde_json::json!(member.priority)
                }),
            );
            entries.push(value);
        }
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }
    match &profile_name {
        Some(name) => println!("Profile: {name}"),
        None => println!("Profile: (none)"),
    }
    if packages.is_empty() {
        println!("No manager-owned packages installed.");
    } else {
        for package in packages {
            let member = members.get(package.id.as_str());
            let tag = match ranks.get(package.id.as_str()) {
                Some(rank) => format!("#{rank} "),
                None => "disabled ".to_owned(),
            };
            let shadowed = member
                .filter(|member| member.shadowed_files > 0)
                .map(|member| format!(" ({} shadowed)", member.shadowed_files))
                .unwrap_or_default();
            let pin = if package.pinned { " (pinned)" } else { "" };
            match &package.catalog {
                Some(catalog) => println!(
                    "{tag}{}{pin}: {catalog}, {} owned files{shadowed}",
                    package.id,
                    package.files.len()
                ),
                None => println!(
                    "{tag}{}{pin}: {} owned files{shadowed}",
                    package.id,
                    package.files.len()
                ),
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

fn name_value(name: Option<&str>) -> serde_json::Value {
    name.map_or(serde_json::Value::Null, |name| name.to_owned().into())
}

pub(super) fn uninstall(
    args: &UninstallArgs,
    roots: &ManagerRoots,
    requested: Option<&str>,
    json: bool,
    progress: &Progress,
) -> Result<()> {
    progress.begin()?;
    let state = ManagerState::load(roots)?;
    let state = state.for_installation(roots, args.path.clone(), requested)?;
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
                for restore in &step.restores {
                    println!(
                        "  restores {} from {}",
                        restore.destination, restore.package_id
                    );
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
    let game_id = selected.game_id;
    store.set_history_context(HistoryContext::new(game_id.clone(), game.clone()));
    let removed = if args.force {
        let plan = store.plan_uninstall(&game, &args.package, scope)?;
        let id = &plan.first().expect("a removal plan selects one package").id;
        store.cleanup_owned(&game, id, CleanupApproval::Approved)?;
        vec![id.clone()]
    } else {
        store.uninstall_with(&game, &args.package, scope)?
    };
    let operation_id = store.committed_operation_id().map(str::to_owned);
    if json {
        let mut result = serde_json::json!({"uninstalled": args.package, "packages": removed});
        if let Some(operation_id) = operation_id {
            result["operation_id"] = serde_json::Value::String(operation_id);
        }
        println!("{}", result);
    } else {
        for id in &removed {
            println!("Uninstalled {id}");
        }
    }
    Ok(())
}

pub(super) fn pin(
    args: &PinArgs,
    roots: &ManagerRoots,
    requested: Option<&str>,
    pinned: bool,
) -> Result<()> {
    let selected = ManagerState::load(roots)?
        .for_installation(roots, args.path.clone(), requested)?
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
    roots: &ManagerRoots,
    requested: Option<&str>,
    json: bool,
) -> Result<()> {
    let mut store = ManagerState::load(roots)?
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
