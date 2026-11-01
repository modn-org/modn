//! Route `modn install` to catalog resolution or standalone archive installation.
use crate::catalog_install;
use crate::commands::InstallArgs;
use crate::selection::selected_installation;
use crate::state_access::ManagerState;
use anyhow::{bail, Result};
use modn_core::planning::preview_install_zip_with_permissions;
use std::path::{Path, PathBuf};

/// Bare catalog IDs win over same-named files in the working directory.
/// A standalone archive is explicit through its extension or path syntax.
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
    archive_extension || (args.file.is_none() && explicit_path && source.is_file())
}

pub(super) fn run(
    args: &InstallArgs,
    state_path: &Path,
    requested: Option<&str>,
    json: bool,
) -> Result<()> {
    if !is_standalone_archive(args) {
        return catalog_install::run(args, state_path, requested, json);
    }
    if args.file.is_some() {
        bail!("--file applies only to cataloged mods, not standalone archives");
    }
    install_archive(args, state_path, requested, json)
}

fn install_archive(
    args: &InstallArgs,
    state_path: &Path,
    requested: Option<&str>,
    json: bool,
) -> Result<()> {
    let state = ManagerState::load(state_path)?;
    // An explicit-path preview needs no manager state; it never creates one.
    let mut store = if args.dry_run && args.path.is_some() && state.existing().is_none() {
        None
    } else {
        let target = state.target_for(args.path.as_deref(), requested)?;
        Some(if args.dry_run {
            state.open_preview(&target)?
        } else {
            state.open_writable(&target)?
        })
    };
    let archive = PathBuf::from(&args.target);
    if !args.yes && !args.dry_run {
        bail!("installation writes game files; choose --dry-run or explicitly approve with --yes");
    }
    let (game_id, game) = selected_installation(store.as_ref(), args.path.clone(), requested)?;
    let report =
        preview_install_zip_with_permissions(&game_id, &game, &archive, &args.allow_executable)?;
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
    let package = store
        .as_mut()
        .expect("approved installation opens writable state")
        .install_operations_zip_with_permissions(
            &game,
            &archive,
            &report,
            &args.allow_executable,
        )?;
    if json {
        println!("{}", serde_json::to_string_pretty(&package)?);
    } else {
        println!(
            "Installed {} ({} owned files)",
            package.id,
            package.files.len()
        );
    }
    Ok(())
}
