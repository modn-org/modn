use crate::state_access::{InstallationAccess, ManagerState};
use anyhow::Result;
use modn_core::inventory::{doctor_with_ownership_for, scan_with_ownership_for};
use modn_core::scan::{IntegrityStatus, ScanRoot};
use modn_core::state::Store;
use std::path::{Path, PathBuf};

fn selected_store(
    path: Option<PathBuf>,
    state_path: &Path,
    requested: Option<&str>,
) -> Result<(Store, String, PathBuf)> {
    let selected = ManagerState::load(state_path)?
        .for_installation(path, requested)?
        .open(InstallationAccess::Write)?;
    Ok((
        selected.store.expect("write access opens manager state"),
        selected.game_id,
        selected.root,
    ))
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
    if let Some(content_root) = &report.content_root {
        println!("Content root: {}", content_root.display());
    }
    println!("REDmod executable present: {}", report.redmod_available);
    for entry in report.mods {
        let root = if entry.root == ScanRoot::Content {
            "content "
        } else {
            ""
        };
        println!(
            "{root}{}: {} [{}{}]",
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
            let path = if file.root == ScanRoot::Content {
                format!("content/{}", file.relative_path)
            } else {
                file.relative_path
            };
            println!("OWNED {}: {} ({})", file.status, path, file.package_id);
        }
    }
    for signal in report.framework_signals {
        let root = if signal.root == ScanRoot::Content {
            "content "
        } else {
            ""
        };
        println!(
            "{root}FRAMEWORK SIGNAL {}: {}",
            signal.name, signal.relative_path
        );
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
    for entry in report.scan.mods.iter().filter(|entry| !entry.managed) {
        let root = if entry.root == ScanRoot::Content {
            "content "
        } else {
            ""
        };
        println!(
            "UNMANAGED {root}{}: {}",
            entry.kind,
            entry.relative_path.display()
        );
    }
    for diagnostic in report.diagnostics {
        println!(
            "{} [{}]: {}",
            diagnostic.severity, diagnostic.code, diagnostic.message
        );
        if let Some(action) = diagnostic.action {
            println!("  Next: {action}");
        }
    }
    Ok(())
}
