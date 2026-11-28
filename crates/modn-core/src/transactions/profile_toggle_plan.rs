//! Planning the file changes of one profile enable or disable.

use super::journal::{digest, JournalFile};
use super::profile_toggle::{PreparedToggle, ToggleAction, ToggleChange, ToggleEntry};
use super::restore::{self, RestoreCandidate};
use crate::error::{coded, ErrorCode};
use crate::state::{RootKind, Store};
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

impl Store {
    pub(super) fn plan_disable(
        &self,
        prepared: &PreparedToggle,
        package: &str,
    ) -> Result<Vec<ToggleEntry>> {
        let game_text = prepared.roots.game.to_string_lossy();
        let mut statement = self.connection.prepare(
            "SELECT root, relative_path, archive_entry, blake3, bytes
             FROM managed_files WHERE game_path=?1 AND package_id=?2
             ORDER BY root, relative_path",
        )?;
        let rows = statement
            .query_map(params![game_text.as_ref(), package], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)? as u64,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let excluding = BTreeSet::from([package.to_owned()]);
        let cache = restore::cache(self);
        let mut entries = Vec::new();
        for (root, destination, archive_entry, blake3, bytes) in rows {
            let root = RootKind::from_label(&root).unwrap_or(RootKind::Game);
            let ordinal = entries.len();
            let file = JournalFile {
                ordinal,
                root,
                destination: destination.clone(),
                archive_entry,
                blake3,
                bytes,
            };
            let target = prepared.roots.target(root, &destination)?;
            require_current(&target, &file.blake3)?;
            if let Some(candidate) =
                restore::next_claim(self, &prepared.roots.game, &file, &excluding)?
            {
                let archive = restore::verified_archive(self, &cache, &candidate)?
                    .ok_or_else(|| restore::missing_archive(&candidate, &destination))?;
                entries.push(ToggleEntry {
                    change: ToggleChange {
                        path: destination,
                        action: ToggleAction::Restore,
                        from: Some(package.to_owned()),
                        to: Some(candidate.package_id.clone()),
                    },
                    file,
                    new_bytes: Some(candidate),
                    archive: Some(archive),
                });
            } else {
                entries.push(ToggleEntry {
                    change: ToggleChange {
                        path: destination,
                        action: ToggleAction::Remove,
                        from: Some(package.to_owned()),
                        to: None,
                    },
                    file,
                    new_bytes: None,
                    archive: None,
                });
            }
        }
        Ok(entries)
    }

    pub(super) fn plan_enable(
        &self,
        prepared: &PreparedToggle,
        package: &str,
        priority: i64,
    ) -> Result<Vec<ToggleEntry>> {
        let game_text = prepared.roots.game.to_string_lossy();
        let mut statement = self.connection.prepare(
            "SELECT root, relative_path, archive_entry, blake3, bytes
             FROM file_claims WHERE game_path=?1 AND package_id=?2
             ORDER BY root, relative_path",
        )?;
        let claims = statement
            .query_map(params![game_text.as_ref(), package], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)? as u64,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let excluding = BTreeSet::from([package.to_owned()]);
        let cache = restore::cache(self);
        let mut entries = Vec::new();
        for (root, destination, archive_entry, blake3, bytes) in claims {
            let root = RootKind::from_label(&root).unwrap_or(RootKind::Game);
            let path_key = super::ownership_key(&destination);
            let winner: Option<(String, String, u64)> = self
                .connection
                .query_row(
                    "SELECT package_id, blake3, bytes FROM managed_files
                     WHERE game_path=?1 AND root=?2 AND path_key=?3",
                    params![game_text.as_ref(), root.label(), path_key],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get::<_, i64>(2)? as u64)),
                )
                .optional()?;
            let mut file = JournalFile {
                ordinal: entries.len(),
                root,
                destination: destination.clone(),
                archive_entry: archive_entry.clone(),
                blake3: String::new(),
                bytes: 0,
            };
            let candidate = restore::next_claim(self, &prepared.roots.game, &file, &excluding)?;
            if let Some(claimant) = &candidate {
                if priority <= profile_priority(self, &prepared.profile_id, &claimant.package_id)? {
                    continue;
                }
            }
            let target = prepared.roots.target(root, &destination)?;
            let change = match winner {
                Some((winner_id, winner_blake3, winner_bytes)) if winner_id != package => {
                    file.blake3 = winner_blake3;
                    file.bytes = winner_bytes;
                    require_current(&target, &file.blake3)?;
                    ToggleChange {
                        path: destination.clone(),
                        action: ToggleAction::Replace,
                        from: Some(winner_id),
                        to: Some(package.to_owned()),
                    }
                }
                Some(_) => continue,
                None => {
                    require_absent(&target)?;
                    ToggleChange {
                        path: destination.clone(),
                        action: ToggleAction::Create,
                        from: None,
                        to: Some(package.to_owned()),
                    }
                }
            };
            let new_bytes = RestoreCandidate {
                package_id: package.to_owned(),
                destination,
                archive_entry,
                blake3,
                bytes,
            };
            let archive = restore::verified_archive(self, &cache, &new_bytes)?
                .ok_or_else(|| restore::missing_archive(&new_bytes, &file.destination))?;
            entries.push(ToggleEntry {
                change,
                file,
                new_bytes: Some(new_bytes),
                archive: Some(archive),
            });
        }
        Ok(entries)
    }
}

fn profile_priority(store: &Store, profile_id: &str, package: &str) -> Result<i64> {
    Ok(store.connection.query_row(
        "SELECT priority FROM profile_packages WHERE profile_id=?1 AND package_id=?2",
        params![profile_id, package],
        |row| row.get(0),
    )?)
}

pub(super) fn require_current(target: &Path, expected: &str) -> Result<()> {
    match fs::symlink_metadata(target) {
        Ok(metadata)
            if metadata.is_file()
                && !metadata.file_type().is_symlink()
                && digest(target)? == expected =>
        {
            Ok(())
        }
        Ok(_) => Err(drift(target)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(drift(target)),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn require_absent(target: &Path) -> Result<()> {
    match fs::symlink_metadata(target) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(coded(
            ErrorCode::ProfileDrift,
            format!(
                "PROFILE_DRIFT: an unmanaged file occupies {}; move it away, then run 'modn doctor'",
                target.display()
            ),
        )),
        Err(error) => Err(error.into()),
    }
}

fn drift(target: &Path) -> anyhow::Error {
    coded(
        ErrorCode::ProfileDrift,
        format!(
            "PROFILE_DRIFT: managed file changed on disk; run 'modn doctor': {}",
            target.display()
        ),
    )
}
