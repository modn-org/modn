//! Game-agnostic evidence that managed packages remain in place.
//!
//! Ownership claims are checked against the ledger and the filesystem alone,
//! never through game discovery, so readiness checks do not depend on Lua.
use crate::scan::{IntegrityStatus, OwnedFileCheck};
use crate::state::Store;
use crate::transactions::ManagedPackage;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathState {
    Present,
    Missing,
    Unsafe,
    Unreadable,
}

/// Integrity of one exclusive directory claimed by a managed package.
pub(crate) struct DirectoryRootCheck {
    /// Recorded game-relative directory.
    pub(crate) root: String,
    /// Package that owns the directory.
    pub(crate) package_id: String,
    /// `Present` only when the native directory identity still matches.
    pub(crate) status: IntegrityStatus,
}

// Walk each component without trusting reparse points, symlinks or directory redirects.
// A failed check is reported as drift, never used to infer or transfer ownership.
pub(crate) fn checked_path(
    root: &Path,
    relative: &str,
    expect_directory: bool,
) -> (PathState, PathBuf) {
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

fn directory_root_checks(
    store: &Store,
    game: &Path,
    package: &ManagedPackage,
) -> Vec<DirectoryRootCheck> {
    package
        .directories
        .iter()
        .map(|root| {
            let (state, path) = checked_path(game, root, true);
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
            DirectoryRootCheck {
                root: root.clone(),
                package_id: package.id.clone(),
                status,
            }
        })
        .collect()
}

// The first of the package's own exclusive directories containing `destination`.
fn covering_root<'a>(
    roots: &'a [DirectoryRootCheck],
    destination: &str,
) -> Option<&'a DirectoryRootCheck> {
    roots
        .iter()
        .find(|check| destination.starts_with(&format!("{}/", check.root)))
}

/// Check every owned file and exclusive directory of `packages`.
///
/// Files beneath a verified exclusive directory report the directory's status
/// without hashing, because runtime output inside it stays mutable.
pub(crate) fn owned_file_checks(
    store: &Store,
    game: &Path,
    packages: Vec<ManagedPackage>,
) -> (Vec<OwnedFileCheck>, Vec<DirectoryRootCheck>) {
    let mut owned_files = Vec::new();
    let mut directory_roots = Vec::new();
    for package in packages {
        let roots = directory_root_checks(store, game, &package);
        for file in package.files {
            let status = match covering_root(&roots, &file.destination) {
                Some(check) => check.status,
                None => verify_owned(game, &file.destination, file.bytes, &file.sha256),
            };
            owned_files.push(OwnedFileCheck {
                package_id: package.id.clone(),
                relative_path: file.destination,
                status,
                expected_sha256: file.sha256,
                expected_bytes: file.bytes,
            });
        }
        directory_roots.extend(roots);
    }
    (owned_files, directory_roots)
}

/// Whether a managed package's ownership claims and original payload hold.
///
/// Every exclusive directory covering an original file must keep its native
/// identity, and every original file, including those beneath such a
/// directory, must keep its recorded size and digest. A package without
/// recorded files cannot prove anything and is never intact.
pub(crate) fn package_intact(store: &Store, game: &Path, package: &ManagedPackage) -> bool {
    let roots = directory_root_checks(store, game, package);
    !package.files.is_empty()
        && package.files.iter().all(|file| {
            covering_root(&roots, &file.destination)
                .is_none_or(|check| check.status == IntegrityStatus::Present)
                && verify_owned(game, &file.destination, file.bytes, &file.sha256)
                    == IntegrityStatus::Intact
        })
}
