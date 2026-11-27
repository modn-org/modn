//! Restoring the next claimant's verified bytes when a winning package leaves.
//!
//! An uninstall of a package that wins a shared file must put the
//! next-highest-priority enabled claimant's bytes on disk. The archive that
//! supplies those bytes is checked in the plan phase: a missing or changed
//! archive refuses the whole operation before any disk change. Execution keeps
//! the removed winner's bytes in a `.tmp` witness and the restored bytes in a
//! `.restore` witness until the SQLite commit. Recovery puts the removed
//! winner back while the journal is pending and keeps the restored bytes once
//! it is committed.

use super::journal::{digest, JournalFile};
use crate::acquisition::{source_cache_url, ArchiveCache, CacheKey};
use crate::archive::{ArchiveFormat, Inspection};
use crate::error::{coded, ErrorCode};
use crate::planning::{InstallPlan, PlanStatus, PlannedFile};
use crate::state::Store;
use anyhow::Result;
use rusqlite::params;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// One shadowed claim that takes over a path when its winner leaves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RestoreCandidate {
    /// Package that keeps the claim.
    pub(super) package_id: String,
    /// Root-relative destination the claim covers.
    pub(super) destination: String,
    /// Archive member that supplies the verified bytes.
    pub(super) archive_entry: String,
    /// BLAKE3 digest the restored bytes must have.
    pub(super) blake3: String,
    /// Exact size of the restored bytes.
    pub(super) bytes: u64,
}

/// One restore row of a pending uninstall journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PendingRestore {
    /// Claimant package whose bytes are written.
    pub(super) package_id: String,
    /// Archive member that supplies the bytes.
    pub(super) archive_entry: String,
    /// BLAKE3 digest of the bytes that were written.
    pub(super) blake3: String,
    /// Size of the bytes that were written.
    pub(super) bytes: u64,
}

/// A restore whose bytes are staged and verified, ready to place.
pub(super) struct StagedRestore {
    pub(super) candidate: RestoreCandidate,
    pub(super) staged: PathBuf,
}

/// The archive cache of the manager profile that holds `store`.
///
/// `MODN_CACHE` replaces the profile cache for every command, as it does for
/// downloads; an empty value keeps the profile cache.
pub(super) fn cache(store: &Store) -> ArchiveCache {
    if let Some(root) = std::env::var_os("MODN_CACHE").filter(|value| !value.is_empty()) {
        return ArchiveCache::new(root.into());
    }
    ArchiveCache::new(store.state_root.join("cache"))
}

/// The highest-priority enabled claimant of `file` in the active profile,
/// ignoring the packages in `excluding`.
pub(super) fn next_claim(
    store: &Store,
    game: &Path,
    file: &JournalFile,
    excluding: &BTreeSet<String>,
) -> Result<Option<RestoreCandidate>> {
    let path_key = super::ownership_key(&file.destination);
    let mut statement = store.connection.prepare(
        "SELECT claim.package_id, claim.relative_path, claim.archive_entry,
                claim.blake3, claim.bytes
         FROM file_claims claim
         JOIN profile_packages member ON member.package_id = claim.package_id
         JOIN profiles profile ON profile.id = member.profile_id
         WHERE claim.game_path=?1 AND claim.root=?2 AND claim.path_key=?3
           AND profile.game_path=?1 AND profile.active=1 AND member.enabled=1
         ORDER BY member.priority DESC, claim.package_id",
    )?;
    let rows = statement
        .query_map(
            params![game.to_string_lossy().as_ref(), file.root.label(), path_key],
            |row| {
                Ok(RestoreCandidate {
                    package_id: row.get(0)?,
                    destination: row.get(1)?,
                    archive_entry: row.get(2)?,
                    blake3: row.get(3)?,
                    bytes: row.get::<_, i64>(4)? as u64,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .find(|candidate| !excluding.contains(&candidate.package_id)))
}

/// The verified archive that supplies `candidate`, if the cache holds one.
///
/// A catalog package is looked up by its recorded cache key. Any other package
/// is looked up by its pinned archive BLAKE3, so a verified payload with the
/// same bytes serves a manual or standalone install. Every hit is re-hashed by
/// the cache before it is returned.
pub(super) fn verified_archive(
    store: &Store,
    cache: &ArchiveCache,
    candidate: &RestoreCandidate,
) -> Result<Option<PathBuf>> {
    let pin: String = store.connection.query_row(
        "SELECT archive_blake3 FROM managed_packages WHERE id=?1",
        [&candidate.package_id],
        |row| row.get(0),
    )?;
    let pin = pin.strip_prefix("blake3:").unwrap_or(&pin);
    if !valid_digest(pin) {
        return Ok(None);
    }
    if let Some(path) = keyed_archive(store, cache, &candidate.package_id, pin)? {
        return Ok(Some(path));
    }
    scan_by_pin(cache, pin)
}

fn keyed_archive(
    store: &Store,
    cache: &ArchiveCache,
    package_id: &str,
    pin: &str,
) -> Result<Option<PathBuf>> {
    let Some(metadata) = crate::history::package_metadata(&store.connection, package_id)? else {
        return Ok(None);
    };
    let Some(catalog) = metadata.catalog else {
        return Ok(None);
    };
    let Some(version) = metadata.version else {
        return Ok(None);
    };
    let Some(source) = metadata.source else {
        return Ok(None);
    };
    let Some(url) = source_cache_url(&source)? else {
        return Ok(None);
    };
    let Ok(digest) = crate::acquisition::ArchiveDigest::new(pin) else {
        return Ok(None);
    };
    let key = CacheKey::new(
        &catalog.game_id,
        &catalog.mod_id,
        &version,
        &url,
        digest.as_str(),
    );
    cache.cached(&key)
}

fn scan_by_pin(cache: &ArchiveCache, pin: &str) -> Result<Option<PathBuf>> {
    for entry in cache.entries(None)? {
        if entry.retired {
            continue;
        }
        let Ok(current) = digest(&entry.path) else {
            continue;
        };
        if current == pin {
            return Ok(Some(entry.path));
        }
    }
    Ok(None)
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Stage and verify the bytes of every restorable file, before any disk change.
///
/// `files` are the winner's journal files. A file without another claimant is
/// skipped. A needed archive that is missing or does not reproduce the claim's
/// digest refuses with [`ErrorCode::ProfileArchiveMissing`].
pub(super) fn stage_restores(
    store: &Store,
    game: &Path,
    files: &[JournalFile],
    staging: &Path,
    excluding: &BTreeSet<String>,
) -> Result<BTreeMap<usize, StagedRestore>> {
    let cache = cache(store);
    let restore_staging = staging.join("restore");
    let mut restores = BTreeMap::new();
    for file in files {
        let Some(candidate) = next_claim(store, game, file, excluding)? else {
            continue;
        };
        let Some(archive) = verified_archive(store, &cache, &candidate)? else {
            return Err(missing_archive(&candidate, &file.destination));
        };
        fs::create_dir_all(&restore_staging)?;
        let staged = stage_member(store, &archive, &candidate, &restore_staging, file.ordinal)?;
        restores.insert(file.ordinal, StagedRestore { candidate, staged });
    }
    Ok(restores)
}

fn stage_member(
    store: &Store,
    archive: &Path,
    candidate: &RestoreCandidate,
    staging: &Path,
    ordinal: usize,
) -> Result<PathBuf> {
    let inspection = crate::archive::inspect(archive)?;
    match inspection.format {
        ArchiveFormat::Zip => stage_zip(archive, &inspection, candidate, staging, ordinal),
        ArchiveFormat::SevenZ => stage_sevenz(store, archive, &inspection, candidate, staging),
        _ => Err(missing_entry(
            candidate,
            "archive format cannot be installed",
        )),
    }
}

fn stage_zip(
    archive: &Path,
    inspection: &Inspection,
    candidate: &RestoreCandidate,
    staging: &Path,
    ordinal: usize,
) -> Result<PathBuf> {
    let original = inspection
        .files
        .iter()
        .find(|file| file.entry == candidate.archive_entry)
        .ok_or_else(|| missing_entry(candidate, "archive entry is missing"))?;
    let name = original
        .original_entry
        .as_deref()
        .unwrap_or(&original.entry);
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    let mut input = zip
        .by_name(name)
        .map_err(|_| missing_entry(candidate, "archive entry is unreadable"))?;
    let staged = staging.join(format!("{ordinal}.bin"));
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&staged)?;
    let copied = io::copy(&mut input.by_ref().take(candidate.bytes + 1), &mut output)?;
    output.flush()?;
    output.sync_all()?;
    drop(output);
    if copied != candidate.bytes {
        return Err(missing_entry(candidate, "archive entry changed"));
    }
    verify_staged(&staged, candidate)?;
    Ok(staged)
}

fn stage_sevenz(
    store: &Store,
    archive: &Path,
    inspection: &Inspection,
    candidate: &RestoreCandidate,
    staging: &Path,
) -> Result<PathBuf> {
    let plan = InstallPlan {
        archive_blake3: inspection.blake3.clone(),
        game_installation: PathBuf::new(),
        status: PlanStatus::Reviewable,
        metadata_only: true,
        operations: vec![PlannedFile {
            source: candidate.archive_entry.clone(),
            destination: candidate.destination.clone(),
            bytes: candidate.bytes,
            kind: "file-copy".into(),
            owned_directory: None,
            root: "game".into(),
        }],
        blockers: Vec::new(),
        skipped_generated_files: Vec::new(),
        content_base: None,
        content_root: None,
        overrides: Vec::new(),
    };
    let files = super::staging::stage_7z(archive, inspection, &plan, staging, &store.control)?;
    let file = files
        .into_iter()
        .next()
        .ok_or_else(|| missing_entry(candidate, "archive entry is missing"))?;
    if file.blake3 != candidate.blake3 || file.bytes != candidate.bytes {
        return Err(missing_entry(candidate, "archive entry changed"));
    }
    Ok(staging.join("0.bin"))
}

fn verify_staged(path: &Path, candidate: &RestoreCandidate) -> Result<()> {
    let metadata = fs::metadata(path)?;
    if metadata.len() != candidate.bytes || digest(path)? != candidate.blake3 {
        return Err(missing_entry(candidate, "archive entry changed"));
    }
    Ok(())
}

fn missing_archive(candidate: &RestoreCandidate, destination: &str) -> anyhow::Error {
    coded(
        ErrorCode::ProfileArchiveMissing,
        format!(
            "PROFILE_ARCHIVE_MISSING: package {} has no verified archive in the cache to restore {}; run 'modn cache show' or reacquire the package",
            candidate.package_id, destination
        ),
    )
}

fn missing_entry(candidate: &RestoreCandidate, reason: &str) -> anyhow::Error {
    coded(
        ErrorCode::ProfileArchiveMissing,
        format!(
            "PROFILE_ARCHIVE_MISSING: package {} cannot restore {} from archive entry {}: {reason}; run 'modn cache show' or reacquire the package",
            candidate.package_id, candidate.destination, candidate.archive_entry
        ),
    )
}

/// Write the journal rows of the restores that will be placed.
pub(super) fn record_restores(
    store: &mut Store,
    id: &str,
    restores: &BTreeMap<usize, StagedRestore>,
) -> Result<()> {
    let tx = store.connection.transaction()?;
    for (ordinal, restore) in restores {
        tx.execute(
            "INSERT INTO pending_restores
             (transaction_id,ord,package_id,archive_entry,blake3,bytes)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                id,
                *ordinal as i64,
                restore.candidate.package_id,
                restore.candidate.archive_entry,
                restore.candidate.blake3,
                restore.candidate.bytes as i64
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// The restore rows of one pending journal.
pub(super) fn pending_restores(store: &Store, id: &str) -> Result<BTreeMap<usize, PendingRestore>> {
    let mut statement = store.connection.prepare(
        "SELECT ord, package_id, archive_entry, blake3, bytes FROM pending_restores
         WHERE transaction_id=?1",
    )?;
    let rows = statement
        .query_map([id], |row| {
            Ok((
                row.get::<_, i64>(0)? as usize,
                PendingRestore {
                    package_id: row.get(1)?,
                    archive_entry: row.get(2)?,
                    blake3: row.get(3)?,
                    bytes: row.get::<_, i64>(4)? as u64,
                },
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.into_iter().collect())
}
