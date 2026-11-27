//! Read-only report of overlapping file claims between managed packages.
//!
//! Two enabled packages of one profile may claim the same relative path. The
//! higher priority wins, and rank 1 is the winner. This module reads
//! `file_claims`, `profile_packages` and `managed_files`; it never writes.
//!
//! The stored on-disk winner exists only for the active profile. An inactive
//! profile is not applied, so it has no disk winner and no drift to report.
//! `consistent` compares the stored winner with the expected winner of the
//! selected profile. A false value means `modn doctor` should check the
//! installation.

use crate::error::{coded, ErrorCode};
use crate::state::Store;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

/// The conflict report of one profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct ConflictReport {
    /// User-facing name of the profile this report describes.
    pub profile: String,
    /// Whether this profile decides the files on disk.
    pub active: bool,
    /// Overlapping paths in ownership-key order.
    pub conflicts: Vec<PathConflict>,
}

/// One path claimed by at least two enabled packages of the profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct PathConflict {
    /// Installation root that holds the path, such as `game` or `content`.
    pub root: String,
    /// Game-relative path as the claims record it.
    pub path: String,
    /// Package whose bytes are on disk, or `null` for an inactive profile.
    pub winner: Option<String>,
    /// Whether the stored winner is the expected winner.
    pub consistent: bool,
    /// Claims in priority order; the first claim is the priority winner.
    pub claims: Vec<PackageClaim>,
}

/// One package's claim on an overlapping path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct PackageClaim {
    /// Managed package identifier.
    pub package: String,
    /// One-based position in priority order; rank 1 is the winner.
    pub rank: usize,
    /// Stored priority; a higher value wins.
    pub priority: i64,
    /// Whether this claim is the profile's priority winner.
    pub winner: bool,
    /// BLAKE3 digest of the claimed bytes.
    pub blake3: String,
    /// Size of the claimed bytes.
    pub bytes: u64,
}

/// Read every path that more than one enabled package of `profile` claims.
///
/// `profile` defaults to the active profile. An unknown profile fails with
/// `PROFILE_NOT_FOUND`. Paths are sorted by root and ownership key so repeated
/// reports are deterministic.
///
/// # Errors
///
/// Returns `PROFILE_NOT_FOUND` when the installation has no such profile, and
/// a state read error when the database cannot be queried.
pub fn conflicts(store: &Store, game: &Path, profile: Option<&str>) -> Result<ConflictReport> {
    let game_text = game.to_string_lossy();
    let selected = select_profile(&store.connection, &game_text, profile)?;
    let claims = read_claims(&store.connection, game_text.as_ref(), &selected.id)?;
    let stored = match selected.active {
        true => read_winners(&store.connection, game_text.as_ref())?,
        false => HashMap::new(),
    };
    let mut conflicts = Vec::new();
    for group in group_claims(claims) {
        if group.len() < 2 {
            continue;
        }
        conflicts.push(path_conflict(&group, &stored, selected.active));
    }
    Ok(ConflictReport {
        profile: selected.name,
        active: selected.active,
        conflicts,
    })
}

struct ProfileRow {
    id: String,
    name: String,
    active: bool,
}

fn select_profile(
    connection: &Connection,
    game_path: &str,
    name: Option<&str>,
) -> Result<ProfileRow> {
    let row = match name {
        Some(name) => connection
            .query_row(
                "SELECT id, name, active FROM profiles WHERE game_path=?1 AND name=?2",
                params![game_path, name],
                read_profile_row,
            )
            .optional()?
            .ok_or_else(|| {
                coded(
                    ErrorCode::ProfileNotFound,
                    format!("profile {name:?} does not exist for {game_path}"),
                )
            })?,
        None => connection
            .query_row(
                "SELECT id, name, active FROM profiles WHERE game_path=?1 AND active=1",
                [game_path],
                read_profile_row,
            )
            .optional()?
            .ok_or_else(|| {
                coded(
                    ErrorCode::ProfileNotFound,
                    format!("no active profile exists for {game_path}"),
                )
            })?,
    };
    Ok(row)
}

fn read_profile_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProfileRow> {
    Ok(ProfileRow {
        id: row.get(0)?,
        name: row.get(1)?,
        active: row.get::<_, i64>(2)? != 0,
    })
}

struct ClaimRow {
    root: String,
    path_key: String,
    path: String,
    package: String,
    priority: i64,
    blake3: String,
    bytes: u64,
}

fn read_claims(
    connection: &Connection,
    game_path: &str,
    profile_id: &str,
) -> Result<Vec<ClaimRow>> {
    let mut statement = connection.prepare(
        "SELECT claim.root, claim.path_key, claim.relative_path, claim.package_id,
                member.priority, claim.blake3, claim.bytes
         FROM file_claims claim
         JOIN profile_packages member ON member.package_id = claim.package_id
         WHERE claim.game_path=?1 AND member.profile_id=?2 AND member.enabled=1
         ORDER BY claim.root, claim.path_key, member.priority DESC, claim.package_id",
    )?;
    let rows = statement
        .query_map(params![game_path, profile_id], |row| {
            Ok(ClaimRow {
                root: row.get(0)?,
                path_key: row.get(1)?,
                path: row.get(2)?,
                package: row.get(3)?,
                priority: row.get(4)?,
                blake3: row.get(5)?,
                bytes: row.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn read_winners(
    connection: &Connection,
    game_path: &str,
) -> Result<HashMap<(String, String), String>> {
    let mut statement = connection
        .prepare("SELECT root, path_key, package_id FROM managed_files WHERE game_path=?1")?;
    let rows = statement
        .query_map([game_path], |row| {
            Ok((
                (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.into_iter().collect())
}

fn group_claims(claims: Vec<ClaimRow>) -> Vec<Vec<ClaimRow>> {
    let mut groups: Vec<Vec<ClaimRow>> = Vec::new();
    for claim in claims {
        let starts_group = groups
            .last()
            .is_none_or(|group| group[0].root != claim.root || group[0].path_key != claim.path_key);
        if starts_group {
            groups.push(Vec::new());
        }
        groups
            .last_mut()
            .expect("a claim group was just created")
            .push(claim);
    }
    groups
}

fn path_conflict(
    group: &[ClaimRow],
    stored: &HashMap<(String, String), String>,
    active: bool,
) -> PathConflict {
    let expected = group[0].package.clone();
    let stored_winner = stored
        .get(&(group[0].root.clone(), group[0].path_key.clone()))
        .cloned();
    let winner = if active { stored_winner } else { None };
    let consistent = if active {
        winner.as_deref() == Some(expected.as_str())
    } else {
        true
    };
    PathConflict {
        root: group[0].root.clone(),
        path: group[0].path.clone(),
        winner,
        consistent,
        claims: group
            .iter()
            .enumerate()
            .map(|(index, claim)| PackageClaim {
                package: claim.package.clone(),
                rank: index + 1,
                priority: claim.priority,
                winner: index == 0,
                blake3: claim.blake3.clone(),
                bytes: claim.bytes,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests;
