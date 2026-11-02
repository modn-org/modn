//! Reconcile on-disk discovery against manager-owned SQLite records without writing to a game.
use crate::integrity::{self, PathState};
use crate::scan::{self, DoctorReport, IntegrityStatus, ScanReport};
use crate::state::Store;
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

// Non-contextual per-character case folding is only a Windows candidate search.
// Different names can fold identically on NTFS; actual file identity still decides ownership.
fn ownership_key(relative: &str) -> String {
    if cfg!(windows) {
        relative.chars().flat_map(char::to_lowercase).collect()
    } else {
        relative.to_owned()
    }
}

/// Reconcile owned files using the selected game's ordinary Lua scan policy.
pub fn scan_with_ownership_for(
    store: &Store,
    game_id: &str,
    installation: &Path,
) -> Result<ScanReport> {
    let game = crate::discovery::validate_installation_for(game_id, installation)?;
    let installed = store.installed(&game)?;
    let mut report = crate::game_modules::policy::scan(game_id, &game, 20_000)?;
    let mut matched: HashMap<String, Vec<usize>> = HashMap::new();

    let (owned_files, directory_roots) = integrity::owned_file_checks(store, &game, installed);
    report.owned_files.extend(owned_files);
    report.owned_files.sort_by(|a, b| {
        a.relative_path
            .cmp(&b.relative_path)
            .then(a.package_id.cmp(&b.package_id))
    });
    for (index, file) in report.owned_files.iter().enumerate() {
        matched
            .entry(ownership_key(&file.relative_path))
            .or_default()
            .push(index);
    }

    // Gather actual matching directory entries before choosing a managed attribution.
    // Distinct hard links may have identical file IDs; the exact recorded name wins.
    let mut candidates = vec![Vec::<usize>::new(); report.owned_files.len()];
    for (detected_index, detected) in report.mods.iter().enumerate() {
        let relative = detected.relative_path.to_string_lossy().replace('\\', "/");
        if let Some(indices) = matched.get(&ownership_key(&relative)) {
            for &owned_index in indices {
                let owned = &report.owned_files[owned_index];
                if matches!(
                    owned.status,
                    IntegrityStatus::Missing | IntegrityStatus::Unsafe
                ) {
                    continue;
                }
                // An unreadable file cannot prove identity, so require exact spelling.
                // Neither branch may ever infer an intact hash from an unrelated alias.
                if owned.status == IntegrityStatus::Unreadable {
                    if relative == owned.relative_path {
                        candidates[owned_index].push(detected_index);
                    }
                } else if same_file::is_same_file(
                    game.join(&owned.relative_path),
                    game.join(&detected.relative_path),
                )
                .unwrap_or(false)
                {
                    candidates[owned_index].push(detected_index);
                }
            }
        }
    }

    for (owned_index, entries) in candidates.iter().enumerate() {
        let owned = &report.owned_files[owned_index];
        let exact = entries.iter().copied().find(|&index| {
            report.mods[index]
                .relative_path
                .to_string_lossy()
                .replace('\\', "/")
                == owned.relative_path
        });
        // Ambiguous aliases must remain unmanaged rather than claim extra hard links.
        let selected = exact.or_else(|| (entries.len() == 1).then(|| entries[0]));
        if let Some(detected_index) = selected {
            let detected = &mut report.mods[detected_index];
            if detected.managed {
                continue;
            }
            detected.managed = true;
            detected.package_id = Some(owned.package_id.clone());
            detected.integrity = Some(owned.status);
        }
    }
    for detected in &mut report.mods {
        if detected.managed {
            continue;
        }
        let relative = detected.relative_path.to_string_lossy().replace('\\', "/");
        let candidate = directory_roots.iter().find(|check| {
            let root = &check.root;
            if check.status != IntegrityStatus::Present
                || !crate::transactions::directory_candidate_key(&relative).starts_with(&format!(
                    "{}/",
                    crate::transactions::directory_candidate_key(root)
                ))
            {
                return false;
            }
            // Folded names are candidates, not proof: NTFS may distinguish
            // Unicode lookalikes even when Rust's lowercase mapping does not.
            let prefix = relative
                .split('/')
                .take(root.split('/').count())
                .collect::<Vec<_>>()
                .join("/");
            same_file::is_same_file(game.join(root), game.join(prefix)).unwrap_or(false)
        });
        if let Some(check) = candidate {
            if integrity::checked_path(&game, &relative, false).0 == PathState::Present {
                detected.managed = true;
                detected.package_id = Some(check.package_id.clone());
                detected.integrity = Some(IntegrityStatus::Present);
            }
        }
    }
    Ok(report)
}

/// Evaluate the selected game's Lua doctor policy over the reconciled inventory.
pub fn doctor_with_ownership_for(
    store: &Store,
    game_id: &str,
    installation: &Path,
) -> Result<DoctorReport> {
    let mut report =
        scan::diagnose_checked(scan_with_ownership_for(store, game_id, installation)?)?;
    report.created_directories = store.created_directories(installation)?;
    Ok(report)
}
