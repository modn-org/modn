//! The read-only diff of one profile switch.
//!
//! The current rows are the winners that the active profile materialized in
//! `managed_files`. The target winners are the highest-priority enabled
//! claimants of the target profile in `file_claims`. A path whose digest is
//! equal needs no write, whatever package owns the row.

use super::super::restore::{self, RestoreCandidate};
use super::refuse;
use super::{PathAction, PlannedPath, ProfileSwitchPlan, UnchangedPath};
use crate::archive::{ArchiveFile, ArchiveFormat, Inspection};
use crate::error::{coded, ErrorCode};
use crate::state::{RootKind, Store};
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

struct CurrentRow {
    relative_path: String,
    package_id: String,
    blake3: String,
    bytes: u64,
}

struct TargetRow {
    relative_path: String,
    package_id: String,
    archive_entry: String,
    blake3: String,
    bytes: u64,
}

/// One write that a later execution stages from the package archive.
struct Write {
    root: RootKind,
    relative_path: String,
    package_id: String,
    archive_entry: String,
    blake3: String,
    bytes: u64,
    replaces: Option<String>,
    target: PathBuf,
}

pub(super) fn plan(store: &Store, game: &Path, target: &str) -> Result<ProfileSwitchPlan> {
    let context = refuse::switchable(store, game, target)?;
    let game_path = context.game.to_string_lossy().into_owned();
    let game_id = game_id(store, &game_path)?;
    let current = read_current(store, &game_path, &context.from.id)?;
    let winners = read_winners(store, &game_path, &context.to.id)?;
    let mut remove = Vec::new();
    let mut writes = Vec::new();
    let mut unchanged = Vec::new();
    let keys: BTreeSet<(String, String)> = current.keys().chain(winners.keys()).cloned().collect();
    for key in keys {
        let root = RootKind::from_label(&key.0).unwrap_or(RootKind::Game);
        match (winners.get(&key), current.get(&key)) {
            (Some(winner), Some(row)) if row.blake3 == winner.blake3 => {
                unchanged.push(UnchangedPath {
                    root: key.0.clone(),
                    path: row.relative_path.clone(),
                });
            }
            (Some(winner), row) => {
                let target = context.roots.target(root, &winner.relative_path)?;
                writes.push(Write {
                    root,
                    relative_path: winner.relative_path.clone(),
                    package_id: winner.package_id.clone(),
                    archive_entry: winner.archive_entry.clone(),
                    blake3: winner.blake3.clone(),
                    bytes: winner.bytes,
                    replaces: row.map(|row| row.package_id.clone()),
                    target,
                });
            }
            (None, Some(row)) => remove.push(PlannedPath {
                root: key.0.clone(),
                path: row.relative_path.clone(),
                action: PathAction::Remove,
                package: Some(row.package_id.clone()),
                replaces: None,
                blake3: Some(row.blake3.clone()),
                bytes: Some(row.bytes),
            }),
            (None, None) => {}
        }
    }
    refuse_archives(store, &writes)?;
    refuse_collisions(&writes)?;
    let from_members = members(store, &context.from.id)?;
    let to_members = members(store, &context.to.id)?;
    let plan = ProfileSwitchPlan {
        game: game_id,
        path: game_path,
        from: context.from.name,
        to: context.to.name,
        remove,
        write: writes.iter().map(planned_write).collect(),
        unchanged: unchanged.len(),
        unchanged_paths: unchanged,
        entering: to_members.difference(&from_members).cloned().collect(),
        leaving: from_members.difference(&to_members).cloned().collect(),
        bytes_to_write: writes.iter().map(|write| write.bytes).sum(),
    };
    Ok(plan)
}

fn planned_write(write: &Write) -> PlannedPath {
    PlannedPath {
        root: write.root.label().to_owned(),
        path: write.relative_path.clone(),
        action: if write.replaces.is_some() {
            PathAction::Replace
        } else {
            PathAction::Write
        },
        package: Some(write.package_id.clone()),
        replaces: write.replaces.clone(),
        blake3: Some(write.blake3.clone()),
        bytes: Some(write.bytes),
    }
}

fn game_id(store: &Store, game_path: &str) -> Result<String> {
    Ok(store
        .connection
        .query_row(
            "SELECT game_id FROM game_installations WHERE path=?1 ORDER BY game_id",
            [game_path],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or_default())
}

fn members(store: &Store, profile_id: &str) -> Result<BTreeSet<String>> {
    let mut statement = store
        .connection
        .prepare("SELECT package_id FROM profile_packages WHERE profile_id=?1 AND enabled=1")?;
    let rows = statement
        .query_map([profile_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;
    Ok(rows)
}

fn read_current(
    store: &Store,
    game_path: &str,
    profile_id: &str,
) -> Result<BTreeMap<(String, String), CurrentRow>> {
    let mut statement = store.connection.prepare(
        "SELECT root, path_key, relative_path, package_id, blake3, bytes
         FROM managed_files WHERE game_path=?1 AND profile_id=?2",
    )?;
    let rows = statement
        .query_map(params![game_path, profile_id], |row| {
            Ok((
                (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                CurrentRow {
                    relative_path: row.get(2)?,
                    package_id: row.get(3)?,
                    blake3: row.get(4)?,
                    bytes: row.get::<_, i64>(5)? as u64,
                },
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.into_iter().collect())
}

fn read_winners(
    store: &Store,
    game_path: &str,
    profile_id: &str,
) -> Result<BTreeMap<(String, String), TargetRow>> {
    let mut statement = store.connection.prepare(
        "SELECT claim.root, claim.path_key, claim.relative_path, claim.package_id,
                claim.archive_entry, claim.blake3, claim.bytes
         FROM file_claims claim
         JOIN profile_packages member ON member.package_id = claim.package_id
         WHERE claim.game_path=?1 AND member.profile_id=?2 AND member.enabled=1
         ORDER BY claim.root, claim.path_key, member.priority DESC, claim.package_id",
    )?;
    let rows = statement
        .query_map(params![game_path, profile_id], |row| {
            Ok((
                (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                TargetRow {
                    relative_path: row.get(2)?,
                    package_id: row.get(3)?,
                    archive_entry: row.get(4)?,
                    blake3: row.get(5)?,
                    bytes: row.get::<_, i64>(6)? as u64,
                },
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut winners = BTreeMap::new();
    for (key, row) in rows {
        winners.entry(key).or_insert(row);
    }
    Ok(winners)
}

/// Refuse when a written path has no verified archive member.
///
/// Every package that cannot supply its bytes is named once, so the user sees
/// the whole reacquisition list before any write. A ZIP member is read through
/// a hasher to compare its digest; a 7z member is checked by entry and size,
/// because extraction to disk is an execution step.
fn refuse_archives(store: &Store, writes: &[Write]) -> Result<()> {
    let cache = restore::cache(store);
    let mut archives: BTreeMap<String, Option<PathBuf>> = BTreeMap::new();
    let mut inspections: BTreeMap<PathBuf, Inspection> = BTreeMap::new();
    let mut missing: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for write in writes {
        if !archives.contains_key(&write.package_id) {
            let candidate = RestoreCandidate {
                package_id: write.package_id.clone(),
                destination: write.relative_path.clone(),
                archive_entry: write.archive_entry.clone(),
                blake3: write.blake3.clone(),
                bytes: write.bytes,
            };
            let archive = restore::verified_archive(store, &cache, &candidate)?;
            archives.insert(write.package_id.clone(), archive);
        }
        let archive = archives.get(&write.package_id).and_then(Option::as_ref);
        let present = match archive {
            Some(archive) => {
                if !inspections.contains_key(archive) {
                    inspections.insert(archive.clone(), crate::archive::inspect(archive)?);
                }
                let inspection = inspections.get(archive).expect("archive inspected above");
                entry_matches(archive, inspection, write)?
            }
            None => false,
        };
        if !present {
            missing
                .entry(write.package_id.clone())
                .or_default()
                .push(write.relative_path.clone());
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    let lines: Vec<String> = missing
        .iter()
        .map(|(package, paths)| {
            let first = paths.first().map_or("", String::as_str);
            let more = match paths.len() {
                0 | 1 => String::new(),
                count => format!(" and {} more path(s)", count - 1),
            };
            format!(
                "PROFILE_ARCHIVE_MISSING: package {package} has no verified archive member for {first}{more}; run 'modn cache show' or reacquire the package"
            )
        })
        .collect();
    Err(coded(ErrorCode::ProfileArchiveMissing, lines.join("\n")))
}

fn entry_matches(archive: &Path, inspection: &Inspection, write: &Write) -> Result<bool> {
    let Some(entry) = inspection
        .files
        .iter()
        .find(|entry| entry.entry == write.archive_entry)
    else {
        return Ok(false);
    };
    if entry.bytes != write.bytes {
        return Ok(false);
    }
    if inspection.format != ArchiveFormat::Zip {
        return Ok(true);
    }
    zip_member_matches(archive, entry, &write.blake3)
}

fn zip_member_matches(archive: &Path, entry: &ArchiveFile, expected: &str) -> Result<bool> {
    let name = entry.original_entry.as_deref().unwrap_or(&entry.entry);
    let mut zip = zip::ZipArchive::new(fs::File::open(archive)?)?;
    let Ok(mut input) = zip.by_name(name) else {
        return Ok(false);
    };
    let mut hasher = blake3::Hasher::new();
    let copied = io::copy(&mut input, &mut hasher)?;
    Ok(copied == entry.bytes && hasher.finalize().to_hex().as_str() == expected)
}

/// Refuse to overwrite a file that no managed row owns.
fn refuse_collisions(writes: &[Write]) -> Result<()> {
    for write in writes.iter().filter(|write| write.replaces.is_none()) {
        match fs::symlink_metadata(&write.target) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Ok(_) => {
                return Err(coded(
                    ErrorCode::Conflict,
                    format!(
                        "CONFLICT: an unmanaged file occupies {} ({}); move it away before switching profiles",
                        write.relative_path,
                        write.target.display()
                    ),
                ))
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
