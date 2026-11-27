//! Execution, commit and recovery of a profile enable or disable.
//!
//! Bytes are staged and verified before the journal appears. Execution keeps
//! the old bytes in a `.tmp` witness and the new bytes in a `.restore` witness
//! until the SQLite commit. Recovery restores the old bytes while the journal
//! is pending and keeps the new bytes once it is committed.

use super::journal::{digest, restore_path, temp_path, Journal, JournalFile, Roots};
use super::profile_toggle::{active_profile_id, PreparedToggle, ProfileToggle};
use super::restore::{self, PendingRestore, StagedRestore};
use crate::state::Store;
use anyhow::{bail, Context, Result};
use rusqlite::params;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::Path;
use tempfile::Builder;

/// Stage, place and commit a checked profile toggle.
pub(super) fn execute_toggle(
    store: &mut Store,
    prepared: PreparedToggle,
    fault: Option<super::install::Fault>,
) -> Result<ProfileToggle> {
    let staging = Builder::new()
        .prefix("txn-")
        .tempdir_in(&store.state_root)?;
    let id = staging
        .path()
        .file_name()
        .context("missing transaction id")?
        .to_string_lossy()
        .to_string();
    let restore_dir = staging.path().join("restore");
    let mut restores = BTreeMap::new();
    for entry in &prepared.entries {
        let Some(candidate) = &entry.new_bytes else {
            continue;
        };
        fs::create_dir_all(&restore_dir)?;
        let archive = entry
            .archive
            .as_deref()
            .context("planned profile toggle archive is missing")?;
        let staged =
            restore::stage_member(store, archive, candidate, &restore_dir, entry.file.ordinal)?;
        restores.insert(
            entry.file.ordinal,
            StagedRestore {
                candidate: candidate.clone(),
                staged,
            },
        );
    }
    let files: Vec<JournalFile> = prepared
        .entries
        .iter()
        .map(|entry| entry.file.clone())
        .collect();
    store.insert_journal(
        &id,
        "profile_toggle",
        &prepared.roots.game,
        &prepared.toggle.package,
        None,
        &files,
        &[],
    )?;
    restore::record_restores(store, &id, &restores)?;
    let _ = staging.keep();
    let attempt = place(&prepared, &restores, &id, fault);
    if let Err(err) = attempt {
        if matches!(fault, Some(super::install::Fault::CrashAfter(_))) {
            return Err(err);
        }
        if let Err(recovery_error) = store.recover_locked() {
            bail!(
                "{err:#}; profile toggle recovery incomplete: {recovery_error:#}; run 'modn recover'"
            );
        }
        return Err(err);
    }
    if let Err(err) = commit_toggle(
        store,
        &id,
        &prepared.profile_id,
        &prepared.toggle.package,
        prepared.toggle.enabled,
    ) {
        if let Err(recovery_error) = store.recover_locked() {
            bail!(
                "{err:#}; profile toggle recovery incomplete: {recovery_error:#}; run 'modn recover'"
            );
        }
        return Err(err);
    }
    store
        .recover_locked()
        .context("profile toggle committed; cleanup incomplete, run 'modn recover'")?;
    Ok(prepared.toggle)
}

fn place(
    prepared: &PreparedToggle,
    restores: &BTreeMap<usize, StagedRestore>,
    id: &str,
    fault: Option<super::install::Fault>,
) -> Result<()> {
    for (index, entry) in prepared.entries.iter().enumerate() {
        place_one(prepared, entry, restores, id)?;
        if fault.is_some_and(|fault| fault.after_files() == index + 1) {
            bail!("injected profile toggle stop");
        }
    }
    Ok(())
}

fn place_one(
    prepared: &PreparedToggle,
    entry: &super::profile_toggle::ToggleEntry,
    restores: &BTreeMap<usize, StagedRestore>,
    id: &str,
) -> Result<()> {
    let target = prepared
        .roots
        .target(entry.file.root, &entry.file.destination)?;
    if entry.file.blake3.is_empty() {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        if fs::symlink_metadata(&target).is_ok() {
            return Err(anyhow::anyhow!(
                "PROFILE_DRIFT: an unmanaged file appeared at {}; preserve it and run 'modn doctor'",
                target.display()
            ));
        }
    } else {
        if !target.is_file() || digest(&target)? != entry.file.blake3 {
            bail!(
                "PROFILE_DRIFT: managed file changed during the profile toggle; run 'modn doctor': {}",
                target.display()
            );
        }
        let temp = temp_path(&target, id, entry.file.ordinal)?;
        fs::hard_link(&target, &temp)?;
        if !same_file::is_same_file(&target, &temp)?
            || digest(&temp)? != entry.file.blake3
            || digest(&target)? != entry.file.blake3
        {
            bail!(
                "PROFILE_DRIFT: managed file changed while it was backed up: {}",
                target.display()
            );
        }
        fs::remove_file(&target)?;
    }
    if let Some(restore) = restores.get(&entry.file.ordinal) {
        let witness = restore_path(&target, id, entry.file.ordinal)?;
        let mut input = File::open(&restore.staged)?;
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&witness)?;
        if io::copy(&mut input, &mut output)? != restore.candidate.bytes {
            bail!("restore staging size mismatch");
        }
        output.sync_all()?;
        drop(output);
        if digest(&witness)? != restore.candidate.blake3 {
            bail!("restore staging digest mismatch");
        }
        fs::hard_link(&witness, &target)?;
        if digest(&target)? != restore.candidate.blake3 {
            bail!("restored file digest mismatch: {}", target.display());
        }
    }
    Ok(())
}

pub(super) fn commit_toggle(
    store: &mut Store,
    id: &str,
    profile_id: &str,
    package: &str,
    enabled: bool,
) -> Result<()> {
    let files = store.toggle_journal_files(id)?;
    let restores = restore::pending_restores(store, id)?;
    let game = store_game(store, id)?;
    let active = active_profile_id(store, &game)?;
    let tx = store.connection.transaction()?;
    for file in &files {
        if let Some(restore) = restores.get(&file.ordinal) {
            let updated = tx.execute(
                "UPDATE managed_files
                 SET package_id=?1, blake3=?2, bytes=?3, archive_entry=?4
                 WHERE game_path=?5 AND root=?6 AND path_key=?7 AND package_id<>?1",
                params![
                    restore.package_id,
                    restore.blake3,
                    restore.bytes as i64,
                    restore.archive_entry,
                    file.game_path,
                    file.root,
                    file.path_key
                ],
            )?;
            if updated == 0 {
                tx.execute(
                    "INSERT INTO managed_files
                     (game_path,root,path_key,relative_path,package_id,blake3,bytes,profile_id,archive_entry)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![
                        file.game_path,
                        file.root,
                        file.path_key,
                        file.relative_path,
                        restore.package_id,
                        restore.blake3,
                        restore.bytes as i64,
                        active,
                        restore.archive_entry
                    ],
                )?;
            }
        } else {
            let deleted = tx.execute(
                "DELETE FROM managed_files
                 WHERE game_path=?1 AND root=?2 AND path_key=?3 AND package_id=?4",
                params![file.game_path, file.root, file.path_key, package],
            )?;
            if deleted != 1 {
                bail!(
                    "profile toggle winner row is missing for {}",
                    file.relative_path
                );
            }
        }
    }
    let updated = tx.execute(
        "UPDATE profile_packages SET enabled=?1 WHERE profile_id=?2 AND package_id=?3",
        params![enabled, profile_id, package],
    )?;
    if updated != 1 {
        bail!("profile membership changed while the toggle committed; run 'modn doctor'");
    }
    let committed = tx.execute(
        "UPDATE pending_transactions SET status='committed' WHERE id=?1 AND status='pending'",
        [id],
    )?;
    if committed != 1 {
        bail!("profile toggle journal {id} is no longer pending; run 'modn recover'");
    }
    tx.commit()?;
    Ok(())
}

fn store_game(store: &Store, id: &str) -> Result<std::path::PathBuf> {
    let game: String = store.connection.query_row(
        "SELECT game_path FROM pending_transactions WHERE id=?1",
        [id],
        |row| row.get(0),
    )?;
    Ok(game.into())
}

/// Recover one interrupted profile toggle.
pub(super) fn recover_toggle(store: &Store, journal: &Journal, roots: &Roots) -> Result<()> {
    let restores = restore::pending_restores(store, &journal.id)?;
    for file in journal.files.iter().rev() {
        let dest = roots.target(file.root, &file.destination)?;
        if let Some(restore) = restores.get(&file.ordinal) {
            if file.blake3.is_empty() {
                recover_created(&journal.id, &journal.status, file, restore, &dest)?;
            } else {
                super::restore_recovery::recover_restore(
                    &journal.id,
                    &journal.status,
                    file,
                    restore,
                    &dest,
                )?;
            }
        } else if file.blake3.is_empty() {
            remove_witness(&restore_path(&dest, &journal.id, file.ordinal)?)?;
        } else {
            recover_removed(&journal.id, &journal.status, file, &dest)?;
        }
    }
    Ok(())
}

fn recover_created(
    id: &str,
    status: &str,
    file: &JournalFile,
    restore: &PendingRestore,
    dest: &Path,
) -> Result<()> {
    let witness = restore_path(dest, id, file.ordinal)?;
    if status == "committed" {
        remove_witness(&witness)?;
        return Ok(());
    }
    if status != "pending" {
        bail!("unknown transaction state for {id}");
    }
    match fs::symlink_metadata(dest) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
            if digest(dest)? != restore.blake3 {
                bail!(
                    "CONFLICT: transaction {id} destination changed after the interrupted profile toggle; preserve {} and repair manually",
                    dest.display()
                );
            }
            fs::remove_file(dest)?;
        }
        Ok(_) => bail!(
            "CONFLICT: transaction {id} destination is not a regular file; preserve {} and repair manually",
            dest.display()
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    remove_witness(&witness)?;
    Ok(())
}

fn recover_removed(id: &str, status: &str, file: &JournalFile, dest: &Path) -> Result<()> {
    let temp = temp_path(dest, id, file.ordinal)?;
    if status == "committed" {
        remove_witness(&temp)?;
        return Ok(());
    }
    if status != "pending" {
        bail!("unknown transaction state for {id}");
    }
    if temp.is_file() {
        if digest(&temp)? != file.blake3 {
            bail!(
                "transaction {id} has a changed profile toggle witness {}; preserve it and repair manually",
                temp.display()
            );
        }
        match fs::symlink_metadata(dest) {
            Ok(_) if file_matches(dest, &file.blake3)? => {}
            Ok(_) => bail!(
                "transaction {id} destination changed after the interrupted profile toggle; preserve {}",
                dest.display()
            ),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::hard_link(&temp, dest).with_context(|| {
                    format!(
                        "cannot restore {} from its profile toggle witness",
                        dest.display()
                    )
                })?;
            }
            Err(error) => return Err(error.into()),
        }
        remove_witness(&temp)?;
        return Ok(());
    }
    if !file_matches(dest, &file.blake3)? {
        bail!(
            "transaction {id} has no safe profile toggle backup for {}",
            dest.display()
        );
    }
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
