//! File claims and the overrides a plan reports for shared managed files.
//!
//! A claim is one package's statement that it writes a destination. Two
//! packages may claim one destination: the higher-priority package wins. The
//! planning layer returns a [`PlanOverride`] for each destination a plan takes
//! from another package instead of refusing the plan.

use crate::state::{RootKind, Store};
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// One managed destination a plan takes from another package.
///
/// The installing package's identifier and digest are unknown while the plan is
/// metadata only, so they stay [`None`] until an install assigns them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct PlanOverride {
    /// Installation root label: `game` or `content`.
    pub root: String,
    /// Game-relative destination the plan writes.
    pub destination: String,
    /// Installing package, once a transaction names it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_id: Option<String>,
    /// BLAKE3 digest of the installed bytes, once staging computes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blake3: Option<String>,
    /// Declared bytes of the installed file.
    pub bytes: u64,
    /// Archive entry that supplies the installed file.
    pub archive_entry: String,
    /// Package whose file the plan replaces.
    pub replaced_package: String,
    /// BLAKE3 digest of the file the plan replaces.
    pub replaced_blake3: String,
}

/// One destination of an installation plan with its source entry and size.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ClaimedPath<'a> {
    pub(crate) destination: &'a str,
    pub(crate) archive_entry: &'a str,
    pub(crate) bytes: u64,
}

/// A file that an installed package owns on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Reservation {
    pub(crate) destination: String,
    pub(crate) package_id: String,
    pub(crate) blake3: String,
}

impl Store {
    /// Destination keys (`root:ownership_key`) of files the active profile's
    /// packages own on disk, excluding the package `replaced` names.
    pub(crate) fn managed_claim_keys(
        &self,
        game: &Path,
        replaced: Option<&str>,
    ) -> Result<BTreeSet<String>> {
        let mut statement = self.connection.prepare(
            "SELECT file.root, file.relative_path
             FROM managed_files file
             JOIN profile_packages member ON member.package_id = file.package_id
             JOIN profiles profile ON profile.id = member.profile_id
             WHERE file.game_path=?1 AND profile.game_path=?1 AND profile.active=1
               AND file.package_id IS NOT ?2",
        )?;
        let keys = statement
            .query_map(params![game.to_string_lossy().as_ref(), replaced], |row| {
                let root: String = row.get(0)?;
                let destination: String = row.get(1)?;
                Ok(format!(
                    "{root}:{}",
                    crate::transactions::ownership_key(&destination)
                ))
            })?
            .collect::<rusqlite::Result<BTreeSet<_>>>()?;
        Ok(keys)
    }

    /// Files of `root` that installed packages own on disk, by ownership key,
    /// excluding the package `replaced` names.
    pub(crate) fn file_reservations(
        &self,
        game: &Path,
        root: RootKind,
        replaced: Option<&str>,
    ) -> Result<BTreeMap<String, Reservation>> {
        let mut statement = self.connection.prepare(
            "SELECT relative_path, package_id, blake3 FROM managed_files
             WHERE game_path=?1 AND root=?2 AND package_id IS NOT ?3",
        )?;
        let rows = statement
            .query_map(
                params![game.to_string_lossy().as_ref(), root.label(), replaced],
                |row| {
                    let destination: String = row.get(0)?;
                    Ok(Reservation {
                        destination,
                        package_id: row.get(1)?,
                        blake3: row.get(2)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .map(|reservation| {
                (
                    crate::transactions::ownership_key(&reservation.destination),
                    reservation,
                )
            })
            .collect())
    }

    /// Whether `package_id` belongs to the active profile of `game`.
    pub(crate) fn claims_in_active_profile(&self, game: &Path, package_id: &str) -> Result<bool> {
        Ok(self
            .connection
            .query_row(
                "SELECT 1 FROM profile_packages member
                 JOIN profiles profile ON profile.id = member.profile_id
                 WHERE member.package_id=?1 AND profile.game_path=?2 AND profile.active=1",
                params![package_id, game.to_string_lossy().as_ref()],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }
}
