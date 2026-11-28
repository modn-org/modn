//! Recovery of an install that replaces a lower-priority managed file.
//!
//! An interrupted override leaves two witnesses beside the destination:
//! `.old` holds the previous owner's verified bytes, `.tmp` holds the staged
//! new bytes. The journal's `pending_overrides` row records the previous
//! owner's digest. A pending journal puts the previous bytes back and keeps
//! the previous owner the winner. A committed journal keeps the new file and
//! deletes both witnesses. A destination that matches neither journalled
//! digest is never changed.

use super::journal::{backup_path, digest, temp_path, JournalFile};
use crate::error::{coded, ErrorCode};
use crate::state::Store;
use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

/// The previous owner's bytes for one file of an interrupted override.
pub(super) struct OverrideWitness {
    pub(super) root: String,
    pub(super) destination: String,
    pub(super) blake3: String,
}

/// The two states of a witness path that recovery may act on.
enum Witness {
    Missing,
    Verified,
}

/// The two states of a destination that recovery may act on.
enum Destination {
    Missing,
    /// The previous owner's bytes are still in place.
    Old,
    /// The staged new bytes were placed but never committed.
    New,
}

impl Store {
    pub(super) fn pending_overrides(&self, id: &str) -> Result<BTreeMap<usize, OverrideWitness>> {
        let mut statement = self.connection.prepare(
            "SELECT ord, root, relative_path, blake3 FROM pending_overrides
             WHERE transaction_id=?1",
        )?;
        let rows = statement
            .query_map([id], |row| {
                Ok((
                    row.get::<_, i64>(0)? as usize,
                    OverrideWitness {
                        root: row.get(1)?,
                        destination: row.get(2)?,
                        blake3: row.get(3)?,
                    },
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().collect())
    }
}

/// Recover one file of an interrupted override: undo an uncommitted
/// replacement, or keep a committed one and remove its witnesses.
pub(super) fn recover_override(
    id: &str,
    status: &str,
    file: &JournalFile,
    old: &OverrideWitness,
    dest: &Path,
) -> Result<()> {
    if old.root != file.root.label() || old.destination != file.destination {
        bail!(
            "transaction {id} override witness does not match its file: {}",
            dest.display()
        );
    }
    let temp = temp_path(dest, id, file.ordinal)?;
    let backup = backup_path(dest, id, file.ordinal)?;
    if status == "committed" {
        remove_witness(&backup)?;
        remove_witness(&temp)?;
        return Ok(());
    }
    if status != "pending" {
        bail!("unknown transaction state for {id}");
    }
    let witness = witness_state(&backup, old, id)?;
    if matches!(witness, Witness::Missing) {
        return without_witness(id, old, file, &temp, dest, &backup);
    }
    match destination_state(dest, old, file, id)? {
        Destination::New => {
            // The destination may be the hard link to `.tmp`, so remove both
            // before the previous bytes go back. A repeated run sees either
            // the old bytes or a missing destination and still recovers.
            fs::remove_file(dest)?;
            remove_witness(&temp)?;
            restore(&backup, dest, &old.blake3, id)?;
            remove_witness(&backup)?;
        }
        Destination::Old => {
            remove_witness(&temp)?;
            remove_witness(&backup)?;
        }
        Destination::Missing => {
            remove_witness(&temp)?;
            restore(&backup, dest, &old.blake3, id)?;
            remove_witness(&backup)?;
        }
    }
    Ok(())
}

/// A journal without `.old` can only complete when the previous bytes are
/// already in place. It never guesses at or deletes unverified content.
fn without_witness(
    id: &str,
    old: &OverrideWitness,
    file: &JournalFile,
    temp: &Path,
    dest: &Path,
    backup: &Path,
) -> Result<()> {
    // The previous bytes must already be at the destination. New or missing
    // bytes prove that recovery cannot restore the previous owner safely.
    if matches!(destination_state(dest, old, file, id)?, Destination::Old) {
        remove_witness(temp)?;
        return Ok(());
    }
    Err(conflict(id, dest, backup))
}

fn witness_state(path: &Path, old: &OverrideWitness, id: &str) -> Result<Witness> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
            if digest(path)? != old.blake3 {
                return Err(coded(
                    ErrorCode::Conflict,
                    format!(
                        "CONFLICT: transaction {id} has a changed override witness {}; preserve it and repair manually",
                        path.display()
                    ),
                ));
            }
            Ok(Witness::Verified)
        }
        Ok(_) => Err(coded(
            ErrorCode::Conflict,
            format!(
                "CONFLICT: transaction {id} has an unsafe override witness {}; preserve it and repair manually",
                path.display()
            ),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Witness::Missing),
        Err(error) => Err(error.into()),
    }
}

fn destination_state(
    dest: &Path,
    old: &OverrideWitness,
    file: &JournalFile,
    id: &str,
) -> Result<Destination> {
    match fs::symlink_metadata(dest) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
            let current = digest(dest)?;
            if current == old.blake3 {
                Ok(Destination::Old)
            } else if current == file.blake3 {
                Ok(Destination::New)
            } else {
                Err(coded(
                    ErrorCode::Conflict,
                    format!(
                        "CONFLICT: transaction {id} destination changed after the interrupted install; preserve {} and repair manually",
                        dest.display()
                    ),
                ))
            }
        }
        Ok(_) => Err(coded(
            ErrorCode::Conflict,
            format!(
                "CONFLICT: transaction {id} destination is not a regular file; preserve {} and repair manually",
                dest.display()
            ),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Destination::Missing),
        Err(error) => Err(error.into()),
    }
}

fn restore(backup: &Path, dest: &Path, expected: &str, id: &str) -> Result<()> {
    fs::hard_link(backup, dest).with_context(|| {
        format!(
            "cannot restore {} from its override witness",
            dest.display()
        )
    })?;
    if digest(dest)? != expected {
        bail!(
            "transaction {id} restored a changed file; run 'modn doctor': {}",
            dest.display()
        );
    }
    Ok(())
}

fn remove_witness(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn conflict(id: &str, dest: &Path, backup: &Path) -> anyhow::Error {
    coded(
        ErrorCode::Conflict,
        format!(
            "CONFLICT: transaction {id} has no override witness {backup} for {}; preserve the destination and repair manually",
            dest.display(),
            backup = backup.display()
        ),
    )
}
