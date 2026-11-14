use crate::catalog_install;
use crate::commands::InstallArgs;
use crate::modules::content_root_appdata;
use crate::progress::Progress;
use crate::state_access::{InstallationAccess, ManagerState};
use anyhow::{bail, Context, Result};
use modn_core::buckets::PackageSource;
use modn_core::history::{HistoryContext, StandalonePackage};
use modn_core::planning::{preview_install_zip_with_appdata, preview_install_zip_with_permissions};
use std::path::{Path, PathBuf};

fn is_standalone_archive(args: &InstallArgs) -> bool {
    let source = Path::new(&args.target);
    let archive_extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["zip", "7z", "jar"].contains(&extension.to_ascii_lowercase().as_str())
        });
    let explicit_path =
        source.is_absolute() || args.target.starts_with('.') || args.target.contains('\\');
    archive_extension || (args.file.is_empty() && explicit_path && source.is_file())
}

pub(super) fn run(
    args: &InstallArgs,
    state_path: &Path,
    requested: Option<&str>,
    json: bool,
    progress: &Progress,
) -> Result<()> {
    progress.begin()?;
    if args.download && !args.dry_run {
        bail!("--download only prepares a --dry-run; an install downloads what it needs");
    }
    if !is_standalone_archive(args) {
        return catalog_install::run(args, state_path, requested, json, progress);
    }
    if !args.file.is_empty() {
        bail!("--file applies only to cataloged mods, not standalone archives");
    }
    if args.download {
        bail!("--download applies only to cataloged mods, not standalone archives");
    }
    install_archive(args, state_path, requested, json, progress)
}

fn install_archive(
    args: &InstallArgs,
    state_path: &Path,
    requested: Option<&str>,
    json: bool,
    progress: &Progress,
) -> Result<()> {
    let access = if args.dry_run {
        InstallationAccess::ArchivePreview
    } else {
        InstallationAccess::ArchiveInstall { approved: args.yes }
    };
    let selected = ManagerState::load(state_path)?
        .for_installation(args.path.clone(), requested)?
        .open(access)?;
    let mut store = selected.store;
    if let Some(store) = store.as_mut() {
        store.set_control(progress.control().clone());
    }
    let game_id = selected.game_id;
    let game = selected.root;
    let archive = PathBuf::from(&args.target);
    let report = match content_root_appdata(&game_id)? {
        Some(appdata) => preview_install_zip_with_appdata(&game_id, &game, &archive, &appdata)?,
        None => {
            preview_install_zip_with_permissions(&game_id, &game, &archive, &args.allow_executable)?
        }
    };
    if args.dry_run {
        if json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            println!("Dry run: {} (metadata only)", report.status);
            for file in report.operations {
                println!("{} => {}", file.source, file.destination);
            }
            for issue in report.blockers {
                println!("BLOCKER {}: {}", issue.code, issue.message);
            }
        }
        return Ok(());
    }
    let (package, operation_id) = {
        let filename = archive
            .file_name()
            .and_then(|name| name.to_str())
            .context("archive filename must be UTF-8")?
            .to_owned();
        let mod_id = archive
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("archive")
            .to_owned();
        let history = HistoryContext::new(game_id.clone(), game.clone()).with_standalone(
            StandalonePackage::new(mod_id, PackageSource::Manual { filename }),
        );
        let store = store
            .as_mut()
            .expect("approved installation opens writable state");
        store.set_history_context(history);
        let package = store.install_operations_zip_with_permissions(
            &game,
            &archive,
            &report,
            &args.allow_executable,
        )?;
        let operation_id = store.committed_operation_id().map(str::to_owned);
        (package, operation_id)
    };
    if json {
        let mut result = serde_json::to_value(&package)?;
        if let Some(operation_id) = operation_id {
            result["operation_id"] = serde_json::Value::String(operation_id);
        }
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!(
            "Installed {} ({} owned files)",
            package.id,
            package.files.len()
        );
    }
    Ok(())
}
