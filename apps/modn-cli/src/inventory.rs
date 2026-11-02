//! `modn scan` and `doctor`: read-only reconciliation of game content and ownership.
use crate::selection::selected_installation;
use crate::state_access::ManagerState;
use anyhow::Result;
use modn_core::inventory::{doctor_with_ownership_for, scan_with_ownership_for};
use modn_core::scan::IntegrityStatus;
use modn_core::state::Store;
use std::path::{Path, PathBuf};

// Scan and doctor never write game files; they still open (or create) manager
// state after validating its location, as before.
fn selected_store(
    path: Option<PathBuf>,
    state_path: &Path,
    requested: Option<&str>,
) -> Result<(Store, String, PathBuf)> {
    let state = ManagerState::load(state_path)?;
    let target = state.target_for(path.as_deref(), requested)?;
    let store = state.open_writable(&target)?;
    let (game_id, game) = selected_installation(Some(&store), path, requested)?;
    Ok((store, game_id, game))
}

pub(super) fn scan(
    path: Option<PathBuf>,
    state_path: &Path,
    requested: Option<&str>,
    json: bool,
) -> Result<()> {
    let (store, game_id, path) = selected_store(path, state_path, requested)?;
    let report = scan_with_ownership_for(&store, &game_id, &path)?;
    if report.incomplete {
        eprintln!("warning: modn scan incomplete (see doctor diagnostics)");
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    println!("{}: {}", game_id, report.installation.display());
    println!("REDmod executable present: {}", report.redmod_available);
    for entry in report.mods {
        println!(
            "{}: {} [{}{}]",
            entry.kind,
            entry.relative_path.display(),
            if entry.managed {
                "managed"
            } else {
                "unmanaged"
            },
            entry
                .integrity
                .map(|status| format!(", {status}"))
                .unwrap_or_default()
        );
    }
    for file in report.owned_files {
        if file.status != IntegrityStatus::Intact {
            println!(
                "OWNED {}: {} ({})",
                file.status, file.relative_path, file.package_id
            );
        }
    }
    for signal in report.framework_signals {
        println!("FRAMEWORK SIGNAL {}: {}", signal.name, signal.relative_path);
    }
    Ok(())
}

pub(super) fn doctor(
    path: Option<PathBuf>,
    state_path: &Path,
    requested: Option<&str>,
    json: bool,
) -> Result<()> {
    let (store, game_id, path) = selected_store(path, state_path, requested)?;
    let report = doctor_with_ownership_for(&store, &game_id, &path)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    println!("{}: {}", game_id, report.scan.installation.display());
    println!(
        "REDmod executable present: {}",
        report.scan.redmod_available
    );
    println!(
        "Detected unmanaged entries: {}",
        report
            .scan
            .mods
            .iter()
            .filter(|entry| !entry.managed)
            .count()
    );
    println!("Owned files checked: {}", report.scan.owned_files.len());
    println!("Folders Modn created: {}", report.created_directories.len());
    for diagnostic in report.diagnostics {
        println!(
            "{} [{}]: {}",
            diagnostic.severity, diagnostic.code, diagnostic.message
        );
    }
    Ok(())
}
