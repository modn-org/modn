//! Planning the file changes of one profile apply.
//!
//! The expected winner of each claimed path is the highest-priority enabled
//! claimant. A path whose stored winner or digest differs is checked against
//! the current bytes and the verified archive cache before it becomes an entry.

use super::journal::JournalFile;
use super::profile_apply::ProfileApply;
use super::profile_toggle::{
    PreparedToggle, ProfileToggle, ToggleAction, ToggleChange, ToggleEntry,
};
use super::profile_toggle_plan::{require_absent, require_current};
use super::restore::{self, RestoreCandidate};
use crate::error::{coded, ErrorCode};
use crate::state::{recorded_root, RootKind, Store};
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

struct Claim {
    root: RootKind,
    relative_path: String,
    package_id: String,
    archive_entry: String,
    blake3: String,
    bytes: u64,
}

struct Winner {
    relative_path: String,
    package_id: String,
    blake3: String,
    bytes: u64,
}

pub(super) struct PreparedApply {
    pub(super) report: ProfileApply,
    pub(super) profile_id: String,
    pub(super) prepared: PreparedToggle,
}

impl Store {
    pub(super) fn prepare_apply(
        &self,
        game: &Path,
        profile: Option<&str>,
        order: Option<&[String]>,
    ) -> Result<PreparedApply> {
        let game = self.transaction_root(game)?;
        let game_text = game.to_string_lossy();
        let row: Option<(String, String, i64)> = match profile {
            Some(name) => self
                .connection
                .query_row(
                    "SELECT id, name, active FROM profiles WHERE game_path=?1 AND name=?2",
                    params![game_text.as_ref(), name],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?,
            None => self
                .connection
                .query_row(
                    "SELECT id, name, active FROM profiles WHERE game_path=?1 AND active=1",
                    [game_text.as_ref()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?,
        };
        let Some((profile_id, name, active)) = row else {
            let wanted = profile.map_or_else(
                || "an active profile".to_owned(),
                |name| format!("profile {name:?}"),
            );
            return Err(coded(
                ErrorCode::ProfileNotFound,
                format!("{wanted} does not exist for {}", game.display()),
            ));
        };
        let active = active != 0;
        let mut result = PreparedApply {
            report: ProfileApply {
                profile: name,
                active,
                changes: Vec::new(),
            },
            profile_id,
            prepared: PreparedToggle {
                toggle: ProfileToggle {
                    profile: String::new(),
                    package: String::new(),
                    enabled: false,
                    active,
                    changes: Vec::new(),
                },
                profile_id: String::new(),
                current_enabled: false,
                roots: super::journal::Roots {
                    game: game.clone(),
                    content: None,
                },
                entries: Vec::new(),
            },
        };
        result.prepared.profile_id.clone_from(&result.profile_id);
        result
            .prepared
            .toggle
            .profile
            .clone_from(&result.report.profile);
        if !active {
            return Ok(result);
        }
        let claims = self.read_claims(&game, &result.profile_id, order)?;
        let winners = self.read_winners(&game)?;
        let cache = restore::cache(self);
        let mut keys: Vec<(String, String)> = winners.keys().cloned().collect();
        for key in claims.keys() {
            if !winners.contains_key(key) {
                keys.push(key.clone());
            }
        }
        keys.sort();
        keys.dedup();
        let mut entries = Vec::new();
        for (root, path_key) in keys {
            let group = claims.get(&(root.clone(), path_key.clone()));
            let stored = winners.get(&(root.clone(), path_key.clone()));
            let change = match (group.and_then(|group| group.first()), stored) {
                (Some(claim), Some(winner))
                    if winner.package_id == claim.package_id && winner.blake3 == claim.blake3 =>
                {
                    continue;
                }
                (Some(claim), Some(winner)) => {
                    let destination =
                        self.target(&result.prepared, claim.root, &claim.relative_path)?;
                    require_current(&destination, &winner.blake3)?;
                    Some(self.replace_entry(&cache, entries.len(), claim, Some(winner))?)
                }
                (Some(claim), None) => {
                    let destination =
                        self.target(&result.prepared, claim.root, &claim.relative_path)?;
                    require_absent(&destination)?;
                    Some(self.replace_entry(&cache, entries.len(), claim, None)?)
                }
                (None, Some(winner)) => {
                    let kind = RootKind::from_label(&root).unwrap_or(RootKind::Game);
                    let destination = self.target(&result.prepared, kind, &winner.relative_path)?;
                    require_current(&destination, &winner.blake3)?;
                    Some(remove_entry(entries.len(), kind, winner))
                }
                (None, None) => None,
            };
            if let Some((entry, change)) = change {
                entries.push(entry);
                result.report.changes.push(change);
            }
        }
        if entries
            .iter()
            .any(|entry| entry.file.root == RootKind::Content)
        {
            result.prepared.roots.content =
                recorded_root(&self.connection, &game, RootKind::Content)?;
        }
        result.prepared.toggle.changes = result.report.changes.clone();
        result.prepared.entries = entries;
        Ok(result)
    }

    fn target(&self, prepared: &PreparedToggle, root: RootKind, relative: &str) -> Result<PathBuf> {
        prepared.roots.target(root, relative)
    }

    fn replace_entry(
        &self,
        cache: &crate::acquisition::ArchiveCache,
        ordinal: usize,
        claim: &Claim,
        winner: Option<&Winner>,
    ) -> Result<(ToggleEntry, ToggleChange)> {
        let file = JournalFile {
            ordinal,
            root: claim.root,
            destination: claim.relative_path.clone(),
            archive_entry: claim.archive_entry.clone(),
            blake3: winner.map_or_else(String::new, |winner| winner.blake3.clone()),
            bytes: winner.map_or(0, |winner| winner.bytes),
        };
        let candidate = RestoreCandidate {
            package_id: claim.package_id.clone(),
            destination: claim.relative_path.clone(),
            archive_entry: claim.archive_entry.clone(),
            blake3: claim.blake3.clone(),
            bytes: claim.bytes,
        };
        let archive = restore::verified_archive(self, cache, &candidate)?
            .ok_or_else(|| restore::missing_archive(&candidate, &claim.relative_path))?;
        let action = if winner.is_some() {
            ToggleAction::Replace
        } else {
            ToggleAction::Create
        };
        let change = ToggleChange {
            path: claim.relative_path.clone(),
            action,
            from: winner.map(|winner| winner.package_id.clone()),
            to: Some(claim.package_id.clone()),
        };
        Ok((
            ToggleEntry {
                change: change.clone(),
                file,
                new_bytes: Some(candidate),
                archive: Some(archive),
            },
            change,
        ))
    }

    fn read_claims(
        &self,
        game: &Path,
        profile_id: &str,
        order: Option<&[String]>,
    ) -> Result<BTreeMap<(String, String), Vec<Claim>>> {
        let mut statement = self.connection.prepare(
            "SELECT claim.root, claim.path_key, claim.relative_path, claim.package_id,
                    claim.archive_entry, claim.blake3, claim.bytes, member.priority
             FROM file_claims claim
             JOIN profile_packages member ON member.package_id = claim.package_id
             WHERE claim.game_path=?1 AND member.profile_id=?2 AND member.enabled=1
             ORDER BY claim.root, claim.path_key, member.priority DESC, claim.package_id",
        )?;
        let rows = statement
            .query_map(
                params![game.to_string_lossy().as_ref(), profile_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)? as u64,
                        row.get::<_, i64>(7)?,
                    ))
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut groups: BTreeMap<(String, String), Vec<Claim>> = BTreeMap::new();
        for (root, path_key, relative_path, package_id, archive_entry, blake3, bytes, _priority) in
            rows
        {
            let root = RootKind::from_label(&root).unwrap_or(RootKind::Game);
            groups
                .entry((root.label().to_owned(), path_key))
                .or_default()
                .push(Claim {
                    root,
                    relative_path,
                    package_id,
                    archive_entry,
                    blake3,
                    bytes,
                });
        }
        if let Some(order) = order {
            let positions: BTreeMap<&str, usize> = order
                .iter()
                .enumerate()
                .map(|(index, package)| (package.as_str(), index))
                .collect();
            for group in groups.values_mut() {
                group.sort_by_key(|claim| {
                    (
                        positions
                            .get(claim.package_id.as_str())
                            .copied()
                            .unwrap_or(usize::MAX),
                        claim.package_id.clone(),
                    )
                });
            }
        }
        Ok(groups)
    }

    fn read_winners(&self, game: &Path) -> Result<BTreeMap<(String, String), Winner>> {
        let mut statement = self.connection.prepare(
            "SELECT root, path_key, relative_path, package_id, blake3, bytes
             FROM managed_files WHERE game_path=?1",
        )?;
        let rows = statement
            .query_map([game.to_string_lossy().as_ref()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    Winner {
                        relative_path: row.get(2)?,
                        package_id: row.get(3)?,
                        blake3: row.get(4)?,
                        bytes: row.get::<_, i64>(5)? as u64,
                    },
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .map(|(root, path_key, winner)| ((root, path_key), winner))
            .collect())
    }
}

fn remove_entry(ordinal: usize, root: RootKind, winner: &Winner) -> (ToggleEntry, ToggleChange) {
    let change = ToggleChange {
        path: winner.relative_path.clone(),
        action: ToggleAction::Remove,
        from: Some(winner.package_id.clone()),
        to: None,
    };
    (
        ToggleEntry {
            change: change.clone(),
            file: JournalFile {
                ordinal,
                root,
                destination: winner.relative_path.clone(),
                archive_entry: String::new(),
                blake3: winner.blake3.clone(),
                bytes: winner.bytes,
            },
            new_bytes: None,
            archive: None,
        },
        change,
    )
}
