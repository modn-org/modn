use crate::commands::UndoArgs;
use crate::download;
use crate::install_consent;
use crate::modules::content_root_appdata;
use crate::progress::Progress;
use crate::selection::selected_installation;
use crate::state_access::{ManagerState, ProtectedTarget};
use anyhow::{bail, Context, Result};
use modn_core::archive::inspect;
use modn_core::buckets::PackageSource;
use modn_core::error::{coded, ErrorCode};
use modn_core::history::{
    HistoryContext, HistoryKind, HistoryOperation, HistoryPackage, HistoryRelease,
};
use modn_core::planning::{
    preview_install_zip_with_permissions, preview_update_with_appdata,
    preview_update_with_permissions,
};
use modn_core::transactions::{
    CatalogReference, CatalogRelease, ManagedPackage, NativeInjectorPolicy, ReleaseDeclarations,
};
use modn_core::undo::{conflict, inverse, UndoRestore};
use std::collections::BTreeSet;
use std::path::Path;

/// The recorded release declarations of `package`.
fn release_declarations(
    package: &HistoryPackage,
) -> Result<(&HistoryRelease, &ReleaseDeclarations)> {
    let release = package.release_before.as_ref().ok_or_else(|| {
        coded(
            ErrorCode::UndoFailed,
            format!(
                "{} has no recorded release to restore; install the release again",
                package.mod_id
            ),
        )
    })?;
    let declarations = release.declarations.as_ref().ok_or_else(|| {
        coded(
            ErrorCode::UndoFailed,
            format!(
                "{} was recorded before release declarations were stored; install the release again",
                package.mod_id
            ),
        )
    })?;
    Ok((release, declarations))
}

/// Rebuild the catalog release that `package` recorded.
fn recorded_release(
    package: &HistoryPackage,
    release: &HistoryRelease,
    declarations: &ReleaseDeclarations,
) -> Result<CatalogRelease> {
    let reference = release.catalog.clone().ok_or_else(|| {
        coded(
            ErrorCode::UndoFailed,
            format!(
                "{} has no recorded catalog identity; install the release again",
                package.mod_id
            ),
        )
    })?;
    let mut rebuilt = CatalogRelease::new(reference, declarations.dependencies.clone())
        .with_conflicts(declarations.conflicts.clone())
        .with_native_injector(declarations.native_injector);
    if let Some(version) = &release.version {
        rebuilt = rebuilt.with_version(version.clone());
    }
    Ok(rebuilt)
}

/// The stored approval and explicit flags for one restored release.
fn approval(
    store: &modn_core::state::Store,
    installation: &Path,
    reference: Option<&CatalogReference>,
    args: &UndoArgs,
) -> Result<(Vec<String>, bool)> {
    let flags = install_consent::flags(args.allow_native_injector, &args.allow_executable);
    let stored = match reference {
        Some(reference) => install_consent::stored_approval(Some(store), installation, reference)?,
        None => None,
    };
    let approved = stored
        .as_ref()
        .map_or_else(|| flags.clone(), |stored| stored.union(&flags));
    Ok((
        approved.executables.into_iter().collect(),
        approved.native_injector,
    ))
}

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
        let (release, declarations) = release_declarations(package)?;
        let rebuilt = match target.kind {
            HistoryKind::Update => Some(recorded_release(package, release, declarations)?),
            HistoryKind::Uninstall if release.catalog.is_some() => {
                Some(recorded_release(package, release, declarations)?)
            }
            _ => None,
        };
        let reference = rebuilt.as_ref().map(|rebuilt| &rebuilt.reference);
        let (approved, native_allowed) = approval(store, &target.installation, reference, args)?;
        let policy = if rebuilt
            .as_ref()
            .is_some_and(|rebuilt| rebuilt.native_injector && native_allowed)
        {
            NativeInjectorPolicy::Allow
        } else {
            NativeInjectorPolicy::Refuse
        };
        if let Some(rebuilt) = &rebuilt {
            rebuilt.ensure_native_injector_allowed(policy)?;
        }
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
                        &approved,
                        &replaced,
                        appdata,
                    )?,
                    None => preview_update_with_permissions(
                        &target.game_id,
                        &target.installation,
                        &archive,
                        &approved,
                        &replaced,
                    )?,
                }
            }
            HistoryKind::Uninstall => match content_root_appdata(&target.game_id)? {
                Some(appdata) => preview_update_with_appdata(
                    &target.game_id,
                    &target.installation,
                    &archive,
                    &approved,
                    &[],
                    &appdata,
                )?,
                None => preview_install_zip_with_permissions(
                    &target.game_id,
                    &target.installation,
                    &archive,
                    &approved,
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
        store.verify_restored_bytes_with_control(&archive, &plan, &expected, progress.control())?;
        restores.push(UndoRestore::new(
            archive, plan, expected, rebuilt, approved, policy,
        ));
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
    let known = crate::operation_id::KnownOperations::read(&store, &target.game_id)?;
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
            let mut restored = restores(&store, state_path, &target, args, progress)?;
            for (package, restore) in target.packages.iter().zip(restored.drain(..)) {
                let release = restore
                    .release
                    .as_ref()
                    .context("history record has no catalog release to restore")?;
                let current = current_package(&store, &target, package)?;
                let source = package
                    .release_before
                    .as_ref()
                    .and_then(|release| release.source.as_ref())
                    .context("history record has no release source")?;
                store.set_history_context(
                    HistoryContext::new(target.game_id.clone(), target.installation.clone())
                        .with_undo_of(target.id.clone())
                        .with_source(&release.reference, source.clone()),
                );
                installed.push(store.update_catalog_release_checked(
                    &target.installation,
                    &current.id,
                    &restore.archive,
                    &restore.plan,
                    release,
                    restore.policy,
                    &restore.approved_executables,
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
    let operation_id = known.committed(&store, &target.game_id)?;
    if json {
        let mut result = serde_json::json!({
            "undo": target.id,
            "kind": effect.as_str(),
            "removed": removed,
            "installed": installed.iter().map(|package| package.id.clone()).collect::<Vec<_>>(),
        });
        if let Some(operation_id) = operation_id {
            result["operation_id"] = serde_json::Value::String(operation_id);
        }
        println!("{}", serde_json::to_string_pretty(&result)?);
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
