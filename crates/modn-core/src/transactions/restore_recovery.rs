//! Recovery of an uninstall that restores the next claimant's bytes.
//!
//! The removed winner's bytes in `.tmp` define the rollback while the journal
//! is pending. The restored claimant's bytes in `.restore` and on disk define
//! the committed state. A destination that matches neither is preserved.

use super::journal::{digest, restore_path, temp_path, JournalFile};
use super::restore::PendingRestore;
use crate::error::{coded, ErrorCode};
use anyhow::{bail, Context, Result};
use rusqlite::{params, OptionalExtension, Transaction};
use std::fs;
use std::io;
use std::path::Path;

/// Recover one restored file of an interrupted uninstall.
pub(super) fn recover_restore(
    id: &str,
    status: &str,
    file: &JournalFile,
    restore: &PendingRestore,
    dest: &Path,
) -> Result<()> {
    let temp = temp_path(dest, id, file.ordinal)?;
    let staged = restore_path(dest, id, file.ordinal)?;
    if status == "committed" {
        remove_witness(&temp)?;
        remove_witness(&staged)?;
        return Ok(());
    }
    if status != "pending" {
        bail!("unknown transaction state for {id}");
    }
    if !temp.is_file() {
        if file_matches(dest, &file.blake3)? {
            remove_witness(&staged)?;
            return Ok(());
        }
        return Err(coded(
            ErrorCode::Conflict,
            format!(
                "CONFLICT: transaction {id} has no uninstall witness for {}; preserve it and repair manually",
                dest.display()
            ),
        ));
    }
    if digest(&temp)? != file.blake3 {
        return Err(coded(
            ErrorCode::Conflict,
            format!(
                "CONFLICT: transaction {id} has a changed uninstall witness {}; preserve it and repair manually",
                temp.display()
            ),
        ));
    }
    match fs::symlink_metadata(dest) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
            let current = digest(dest)?;
            if current == file.blake3 {
                remove_witness(&staged)?;
                remove_witness(&temp)?;
                return Ok(());
            }
            if current != restore.blake3 {
                return Err(coded(
                    ErrorCode::Conflict,
                    format!(
                        "CONFLICT: transaction {id} destination changed after the interrupted uninstall; preserve {} and repair manually",
                        dest.display()
                    ),
                ));
            }
            fs::remove_file(dest)?;
        }
        Ok(_) => {
            return Err(coded(
                ErrorCode::Conflict,
                format!(
                    "CONFLICT: transaction {id} destination is not a regular file; preserve {} and repair manually",
                    dest.display()
                ),
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    remove_witness(&staged)?;
    fs::hard_link(&temp, dest).with_context(|| {
        format!(
            "cannot restore {} from its uninstall witness",
            dest.display()
        )
    })?;
    if digest(dest)? != file.blake3 {
        bail!(
            "transaction {id} restored a changed file; run 'modn doctor': {}",
            dest.display()
        );
    }
    remove_witness(&temp)?;
    Ok(())
}

fn file_matches(path: &Path, expected: &str) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
            Ok(digest(path)? == expected)
        }
        Ok(_) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn remove_witness(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Move each restored path's winner row to its claimant at commit.
///
/// The row keeps its profile, so the winner stays in the active profile. The
/// restored claimant must already hold a claim with the journalled bytes.
pub(super) fn move_winners(
    tx: &Transaction<'_>,
    journal_id: &str,
    removed_package: &str,
) -> Result<()> {
    let mut statement = tx.prepare(
        "SELECT restore.package_id, restore.blake3, restore.bytes,
                restore.archive_entry, journal.game_path, file.root, file.relative_path
         FROM pending_restores restore
         JOIN pending_files file
           ON file.transaction_id = restore.transaction_id AND file.ord = restore.ord
         JOIN pending_transactions journal ON journal.id = restore.transaction_id
         WHERE restore.transaction_id=?1",
    )?;
    let rows = statement
        .query_map([journal_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? as u64,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (package_id, blake3, bytes, archive_entry, game_path, root, relative_path) in rows {
        let path_key = super::ownership_key(&relative_path);
        let claim: Option<(String, i64, String)> = tx
            .query_row(
                "SELECT blake3, bytes, archive_entry FROM file_claims
                 WHERE package_id=?1 AND game_path=?2 AND root=?3 AND path_key=?4",
                params![package_id, game_path, root, path_key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((claim_blake3, claim_bytes, claim_entry)) = claim else {
            bail!("restore claimant {package_id} no longer holds its claim for {relative_path}");
        };
        if claim_blake3 != blake3 || claim_bytes as u64 != bytes || claim_entry != archive_entry {
            bail!("restore claimant {package_id} changed its claim for {relative_path}");
        }
        let moved = tx.execute(
            "UPDATE managed_files
             SET package_id=?1, blake3=?2, bytes=?3, archive_entry=?4
             WHERE game_path=?5 AND root=?6 AND path_key=?7 AND package_id=?8",
            params![
                package_id,
                blake3,
                bytes as i64,
                archive_entry,
                game_path,
                root,
                path_key,
                removed_package
            ],
        )?;
        if moved != 1 {
            bail!("removed winner row is missing for {relative_path}; run 'modn doctor'");
        }
    }
    Ok(())
}
