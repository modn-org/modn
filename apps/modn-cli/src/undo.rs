use crate::commands::UndoArgs;
use crate::download;
use crate::modules::content_root_appdata;
use crate::progress::Progress;
use crate::selection::selected_installation;
use crate::state_access::{ManagerState, ProtectedTarget};
use anyhow::{bail, Context, Result};
use modn_core::archive::inspect;
use modn_core::buckets::PackageSource;
use modn_core::error::{coded, ErrorCode};
use modn_core::history::{HistoryContext, HistoryKind, HistoryOperation, HistoryPackage};
use modn_core::planning::{
    preview_install_zip_with_permissions, preview_update_with_appdata,
    preview_update_with_permissions,
};
use modn_core::transactions::{CatalogRelease, ManagedPackage};
use modn_core::undo::{conflict, inverse, UndoRestore};
use std::collections::BTreeSet;
use std::path::Path;

/// The packages an undo must install again, with their recorded files.
fn restores(
    store: &modn_core::state::Store,
    state_path: &Path,
    target: &HistoryOperation,
    args: &UndoArgs,
    progress: &Progress,
) -> Result<Vec<UndoRestore>> {
    let cache = download::cache_for(state_path)?;
    let mut restores = Vec::new();
    for package in &target.packages {
        let release = package
            .release_before
            .as_ref()
            .with_context(|| format!("{} has no recorded release to restore", package.mod_id))?;
        let source = release.source.as_ref().with_context(|| {
            format!(
                "{} was installed before releases were recorded; install it again",
                package.mod_id
            )
        })?;
        if matches!(source, PackageSource::Manual { .. }) {
            return Err(coded(
                ErrorCode::UndoSourceManual,
                format!(
                    "{} has a manual source; put its archive in place with `modn install --file` instead",
                    package.mod_id
                ),
            ));
        }
        let sha256 = release
            .archive_sha256
            .as_deref()
            .with_context(|| format!("{} has no recorded archive digest", package.mod_id))?;
        let version = release.version.as_deref().unwrap_or("0");
        let archive = download::acquire_source(
            &cache,
            &target.game_id,
            &package.mod_id,
            version,
            source,
            sha256,
            progress,
        )?;
        if inspect(&archive)?.sha256 != sha256 {
            return Err(coded(
                ErrorCode::DigestMismatch,
                format!(
                    "{}: restored archive does not match the recorded SHA-256",
                    package.mod_id
                ),
            ));
        }
        let expected: Vec<_> = package
            .files
            .iter()
            .filter(|file| file.digest_before.is_some())
            .cloned()
            .collect();
        let plan = match target.kind {
            HistoryKind::Update => {
                let current = current_package(store, target, package)?;
                let mut replaced: Vec<String> = current
                    .files
                    .iter()
                    .map(|file| file.destination.clone())
                    .collect();
                replaced.extend(current.directories.iter().cloned());
                let appdata = content_root_appdata(&target.game_id)?;
                match &appdata {
                    Some(appdata) => preview_update_with_appdata(
                        &target.game_id,
                        &target.installation,
                        &archive,
                        &args.allow_executable,
                        &replaced,
                        appdata,
                    )?,
                    None => preview_update_with_permissions(
                        &target.game_id,
                        &target.installation,
                        &archive,
                        &args.allow_executable,
                        &replaced,
                    )?,
                }
            }
            HistoryKind::Uninstall => match content_root_appdata(&target.game_id)? {
                Some(appdata) => preview_update_with_appdata(
                    &target.game_id,
                    &target.installation,
                    &archive,
                    &args.allow_executable,
                    &[],
                    &appdata,
                )?,
                None => preview_install_zip_with_permissions(
                    &target.game_id,
                    &target.installation,
                    &archive,
                    &args.allow_executable,
                )?,
            },
            HistoryKind::Install => bail!("an install is undone by removing its packages"),
            _ => bail!("unknown history kind"),
        };
        let planned: BTreeSet<(&str, &str)> = plan
            .operations
            .iter()
            .map(|operation| (operation.root.as_str(), operation.destination.as_str()))
            .collect();
        let recorded: BTreeSet<(&str, &str)> = expected
            .iter()
            .map(|file| (file.root.as_str(), file.destination.as_str()))
            .collect();
        if planned != recorded {
            return Err(coded(
                ErrorCode::UndoFailed,
                format!(
                    "{}: the current game policy maps this release to other roots or files than the record",
                    package.mod_id
                ),
            ));
        }
        restores.push(UndoRestore::new(archive, plan, expected));
    }
    Ok(restores)
}

fn current_package(
    store: &modn_core::state::Store,
    target: &HistoryOperation,
    package: &HistoryPackage,
) -> Result<ManagedPackage> {
    let id = package
        .package_id_after
        .as_deref()
        .context("history record has no installed package ID")?;
    store
        .installed(&target.installation)?
        .into_iter()
        .find(|installed| installed.id == id)
        .with_context(|| format!("managed package {id} is no longer installed"))
}

fn print_plan(target: &HistoryOperation, json: bool, dry_run: bool) -> Result<()> {
    let packages: Vec<&str> = target
        .packages
        .iter()
        .map(|package| package.mod_id.as_str())
        .collect();
    if json {
        let mut report = serde_json::json!({
            "undo": target.id,
            "kind": inverse(target.kind).as_str(),
            "packages": packages,
        });
        if dry_run {
            report["dry_run"] = serde_json::Value::Bool(true);
        }
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "Would undo {} ({}) by {}: {}",
            target.id,
            target.kind.as_str(),
            if target.kind == HistoryKind::Install {
                "removing packages"
            } else {
                "installing packages"
            },
            packages.join(", ")
        );
    }
    Ok(())
}

pub(super) fn run(
    args: &UndoArgs,
    state_path: &Path,
    requested: Option<&str>,
    json: bool,
    progress: &Progress,
) -> Result<()> {
    progress.begin()?;
    let state = ManagerState::load(state_path)?
        .require_existing("no manager state database; there is no operation to undo")?;
    let read = state
        .existing()
        .context("existing manager state is readable")?;
    let (game_id, installation) = selected_installation(Some(read), args.path.clone(), requested)?;
    let operations = read.history(None)?;
    let belongs = |operation: &HistoryOperation| {
        operation.game_id == game_id && operation.installation == installation
    };
    let target = match args.id.as_deref() {
        Some(id) => match operations.iter().find(|operation| operation.id == id) {
            Some(operation) if belongs(operation) => Some(operation.clone()),
            Some(operation) => {
                return Err(coded(
                    ErrorCode::UndoNotFound,
                    format!(
                        "operation {id} belongs to {} at {}, not to {game_id} at {}",
                        operation.game_id,
                        operation.installation.display(),
                        installation.display()
                    ),
                ));
            }
            None => None,
        },
        None => operations
            .iter()
            .find(|operation| belongs(operation))
            .cloned(),
    }
    .ok_or_else(|| {
        coded(
            ErrorCode::UndoNotFound,
            args.id.as_deref().map_or_else(
                || {
                    format!(
                        "no committed operation to undo for {game_id} at {}",
                        installation.display()
                    )
                },
                |id| {
                    format!(
                        "no committed operation {id} for {game_id} at {}",
                        installation.display()
                    )
                },
            ),
        )
    })?;
    if let Some(found) = conflict(&operations, &target) {
        return Err(coded(
            ErrorCode::UndoConflict,
            format!(
                "operation {} ({}) {}; undo it first",
                found.operation_id,
                found.kind.as_str(),
                found.reason
            ),
        ));
    }
    let restore = target.kind != HistoryKind::Install;
    if restore && progress.enabled() && !args.yes && !args.dry_run {
        return Err(coded(
            ErrorCode::ApprovalRequired,
            "undo writes game files; pass --yes with --progress json",
        ));
    }
    if args.dry_run {
        let plan = if restore {
            restores(read, state_path, &target, args, progress)?
        } else {
            Vec::new()
        };
        print_plan(&target, json, true)?;
        if !json && !plan.is_empty() {
            println!("Archives verified; no game file or manager record changes.");
        }
        return Ok(());
    }
    if !args.yes {
        print_plan(&target, json, false)?;
        return Err(coded(
            ErrorCode::InvalidArguments,
            "undo changes game files; rerun with --yes",
        ));
    }
    let mut store = state.open_writable(&ProtectedTarget {
        game: Some(target.game_id.clone()),
        root: Some(target.installation.clone()),
    })?;
    store.set_control(progress.control().clone());
    store.set_history_context(
        HistoryContext::new(target.game_id.clone(), target.installation.clone())
            .with_undo_of(target.id.clone()),
    );
    let mut removed = Vec::new();
    let mut installed: Vec<ManagedPackage> = Vec::new();
    match target.kind {
        HistoryKind::Install => {
            removed = store.uninstall_for_undo(&target.installation, &target)?;
        }
        HistoryKind::Update => {
            let mut restores = restores(&store, state_path, &target, args, progress)?;
            for (package, restore) in target.packages.iter().zip(restores.drain(..)) {
                let release = package
                    .release_before
                    .as_ref()
                    .context("history record has no release to restore")?;
                let current = current_package(&store, &target, package)?;
                let reference = current.catalog.clone().context(
                    "the installed package has no catalog identity; reinstall it with `modn install --file`",
                )?;
                let source = release
                    .source
                    .as_ref()
                    .context("history record has no release source")?;
                let mut catalog_release = CatalogRelease::new(reference.clone(), Vec::new());
                catalog_release =
                    catalog_release.with_version(release.version.clone().unwrap_or_default());
                store.set_history_context(
                    HistoryContext::new(target.game_id.clone(), target.installation.clone())
                        .with_undo_of(target.id.clone())
                        .with_source(&reference, source.clone()),
                );
                installed.push(store.update_catalog_release_checked(
                    &target.installation,
                    &current.id,
                    &restore.archive,
                    &restore.plan,
                    &catalog_release,
                    if args.allow_native_injector {
                        modn_core::transactions::NativeInjectorPolicy::Allow
                    } else {
                        modn_core::transactions::NativeInjectorPolicy::Refuse
                    },
                    &args.allow_executable,
                    &restore.expected,
                )?);
            }
        }
        HistoryKind::Uninstall => {
            let archives = restores(&store, state_path, &target, args, progress)?;
            installed = store.restore_for_undo(&target.installation, &target, &archives)?;
        }
        _ => bail!("unknown history kind"),
    }
    let effect = inverse(target.kind);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "undo": target.id,
                "kind": effect.as_str(),
                "removed": removed,
                "installed": installed.iter().map(|package| package.id.clone()).collect::<Vec<_>>(),
            }))?
        );
    } else if removed.is_empty() {
        println!(
            "Undid {} by installing {} package(s).",
            target.id,
            installed.len()
        );
    } else {
        println!("Undid {} by removing {}.", target.id, removed.join(", "));
    }
    Ok(())
}
