//! Reconcile on-disk discovery against manager-owned SQLite records without writing to a game.
use crate::scan::{self, DoctorReport, IntegrityStatus, OwnedFileCheck, ScanReport};
use crate::state::Store;
use anyhow::Result;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
enum PathState {
    Present,
    Missing,
    Unsafe,
    Unreadable,
}

// Walk each component without trusting reparse points, symlinks or directory redirects.
// A failed check is reported as drift, never used to infer or transfer ownership.
fn checked_path(root: &Path, relative: &str, expect_directory: bool) -> (PathState, PathBuf) {
    let mut current = root.to_path_buf();
    let parts: Vec<_> = relative.split('/').collect();
    if parts.is_empty()
        || parts.iter().any(|part| {
            part.is_empty()
                || *part == "."
                || *part == ".."
                || part.contains([':', '\\'])
                || part.chars().any(|c| c.is_control())
        })
    {
        return (PathState::Unsafe, current);
    }

    for (i, part) in parts.iter().enumerate() {
        current.push(part);
        let last = i + 1 == parts.len();
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return (PathState::Missing, current);
            }
            Err(_) => return (PathState::Unreadable, current),
        };
        if metadata.file_type().is_symlink()
            || (last && metadata.is_dir() != expect_directory)
            || (!last && !metadata.is_dir())
        {
            return (PathState::Unsafe, current);
        }
        match current.canonicalize() {
            Ok(canonical) if canonical.starts_with(root) => {}
            Ok(_) => return (PathState::Unsafe, current),
            Err(_) => return (PathState::Unreadable, current),
        }
    }
    (PathState::Present, current)
}

fn verify_owned(
    root: &Path,
    relative: &str,
    expected_bytes: u64,
    expected_sha: &str,
) -> IntegrityStatus {
    let (status, path) = checked_path(root, relative, false);
    match status {
        PathState::Missing => return IntegrityStatus::Missing,
        PathState::Unsafe => return IntegrityStatus::Unsafe,
        PathState::Unreadable => return IntegrityStatus::Unreadable,
        PathState::Present => {}
    }
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(_) => return IntegrityStatus::Unreadable,
    };
    if !metadata.is_file() {
        return IntegrityStatus::Unsafe;
    }
    if metadata.len() != expected_bytes {
        return IntegrityStatus::Modified;
    }
    let input = match File::open(&path) {
        Ok(file) => file,
        Err(_) => return IntegrityStatus::Unreadable,
    };
    let mut hasher = Sha256::new();
    match io::copy(
        &mut input.take(expected_bytes.saturating_add(1)),
        &mut hasher,
    ) {
        Ok(read) if read == expected_bytes => {
            if format!("{:x}", hasher.finalize()) == expected_sha {
                IntegrityStatus::Intact
            } else {
                IntegrityStatus::Modified
            }
        }
        Ok(_) => IntegrityStatus::Modified,
        Err(_) => IntegrityStatus::Unreadable,
    }
}

/// Check original package files, even beneath exclusive owned directories.
///
/// Directory-owned runtime output stays mutable; this function only proves
/// that the original installed prerequisite bytes remain present.
#[must_use]
pub fn original_package_payloads_intact(
    game: &Path,
    package: &crate::transactions::ManagedPackage,
) -> bool {
    !package.files.is_empty()
        && package.files.iter().all(|file| {
            verify_owned(game, &file.destination, file.bytes, &file.sha256)
                == IntegrityStatus::Intact
        })
}

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

    let mut directory_roots = Vec::new();
    for package in installed {
        for root in &package.directories {
            let (state, path) = checked_path(&game, root, true);
            let status = match state {
                PathState::Present
                    if store.verified_managed_directory(&package.id, root, &path) =>
                {
                    IntegrityStatus::Present
                }
                PathState::Present | PathState::Unsafe => IntegrityStatus::Unsafe,
                PathState::Missing => IntegrityStatus::Missing,
                PathState::Unreadable => IntegrityStatus::Unreadable,
            };
            directory_roots.push((root.clone(), package.id.clone(), status));
        }
        for file in package.files {
            let root_status = directory_roots.iter().find(|(root, id, _)| {
                id == &package.id && file.destination.starts_with(&format!("{root}/"))
            });
            let status = match root_status {
                Some((_, _, status)) => *status,
                None => verify_owned(&game, &file.destination, file.bytes, &file.sha256),
            };
            report.owned_files.push(OwnedFileCheck {
                package_id: package.id.clone(),
                relative_path: file.destination,
                status,
                expected_sha256: file.sha256,
                expected_bytes: file.bytes,
            });
        }
    }
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
        let candidate = directory_roots.iter().find(|(root, _, status)| {
            if *status != IntegrityStatus::Present
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
        if let Some((_, package_id, _)) = candidate {
            if checked_path(&game, &relative, false).0 == PathState::Present {
                detected.managed = true;
                detected.package_id = Some(package_id.clone());
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
    scan::diagnose_checked(scan_with_ownership_for(store, game_id, installation)?)
}
