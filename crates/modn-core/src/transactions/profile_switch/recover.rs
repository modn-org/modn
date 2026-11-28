//! Recovery of an interrupted profile switch.
//!
//! A switch keeps the bytes it takes out of place in a `.old` witness and the
//! bytes it places in a `.tmp` witness until the SQLite commit. A pending
//! journal puts every quarantined file back and keeps the old profile active.
//! A committed journal keeps the new bytes and removes the witnesses. A
//! destination that matches neither journalled digest is never overwritten.

use super::super::journal::{temp_path, Journal, JournalFile, Roots};
use super::super::override_recovery::{recover_override, OverrideWitness};
use crate::error::{coded, ErrorCode};
use crate::state::{recorded_root, RootKind, Store};
use anyhow::{bail, Result};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

/// Recover one interrupted profile switch.
pub(crate) fn recover_switch(store: &Store, journal: &Journal, roots: &Roots) -> Result<()> {
    let overrides = store.pending_overrides(&journal.id)?;
    let roots = resolve_roots(store, journal, roots, &overrides)?;
    if journal.status == "committed" {
        for (ordinal, old) in &overrides {
            let synthetic = synthetic_file(*ordinal, old);
            let file = file_at(journal, *ordinal).unwrap_or(&synthetic);
            let dest = roots.target(file.root, &file.destination)?;
            recover_override(&journal.id, "committed", file, old, &dest)?;
        }
        for file in &journal.files {
            if overrides.contains_key(&file.ordinal) {
                continue;
            }
            remove_witness(&temp_path(
                &roots.target(file.root, &file.destination)?,
                &journal.id,
                file.ordinal,
            )?)?;
        }
        return Ok(());
    }
    if journal.status != "pending" {
        bail!("unknown transaction state for {}", journal.id);
    }
    for file in journal.files.iter().rev() {
        let dest = roots.target(file.root, &file.destination)?;
        match overrides.get(&file.ordinal) {
            Some(old) => {
                check_match(&journal.id, file, old)?;
                recover_override(&journal.id, "pending", file, old, &dest)?;
            }
            None => recover_created(&journal.id, file, &dest)?,
        }
    }
    for (ordinal, old) in &overrides {
        if file_at(journal, *ordinal).is_some() {
            continue;
        }
        let file = synthetic_file(*ordinal, old);
        let dest = roots.target(file.root, &file.destination)?;
        recover_override(&journal.id, "pending", &file, old, &dest)?;
    }
    Ok(())
}

/// Resolve the content root when only a removal names it.
fn resolve_roots(
    store: &Store,
    journal: &Journal,
    roots: &Roots,
    overrides: &BTreeMap<usize, OverrideWitness>,
) -> Result<Roots> {
    let needs_content = journal
        .files
        .iter()
        .any(|file| file.root == RootKind::Content)
        || overrides
            .values()
            .any(|old| RootKind::from_label(&old.root) == Some(RootKind::Content));
    let content = if needs_content && roots.content.is_none() {
        recorded_root(&store.connection, &journal.game, RootKind::Content)?
    } else {
        roots.content.clone()
    };
    Ok(Roots {
        game: roots.game.clone(),
        content,
    })
}

fn file_at(journal: &Journal, ordinal: usize) -> Option<&JournalFile> {
    journal.files.iter().find(|file| file.ordinal == ordinal)
}

fn check_match(id: &str, file: &JournalFile, old: &OverrideWitness) -> Result<()> {
    if old.root != file.root.label() || old.destination != file.destination {
        bail!("transaction {id} override witness does not match its file");
    }
    Ok(())
}

fn synthetic_file(ordinal: usize, old: &OverrideWitness) -> JournalFile {
    JournalFile {
        ordinal,
        root: RootKind::from_label(&old.root).unwrap_or(RootKind::Game),
        destination: old.destination.clone(),
        archive_entry: String::new(),
        blake3: String::new(),
        bytes: 0,
    }
}

/// Undo a file that this switch created: a destination with the new bytes
/// goes away, and a changed destination is preserved and reported.
fn recover_created(id: &str, file: &JournalFile, dest: &Path) -> Result<()> {
    match fs::symlink_metadata(dest) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
            if super::super::journal::digest(dest)? == file.blake3 {
                fs::remove_file(dest)?;
            } else {
                return Err(conflict(id, dest));
            }
        }
        Ok(_) => return Err(conflict(id, dest)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let temp = temp_path(dest, id, file.ordinal)?;
    if temp.is_file() {
        if super::super::journal::digest(&temp)? != file.blake3 {
            return Err(coded(
                ErrorCode::Conflict,
                format!(
                    "CONFLICT: transaction {id} has a changed profile switch witness {}; preserve it and repair manually",
                    temp.display()
                ),
            ));
        }
        remove_witness(&temp)?;
    }
    Ok(())
}

fn conflict(id: &str, dest: &Path) -> anyhow::Error {
    coded(
        ErrorCode::Conflict,
        format!(
            "CONFLICT: transaction {id} destination changed after the interrupted profile switch; preserve {} and repair manually",
            dest.display()
        ),
    )
}

fn remove_witness(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
