//! Execute a checked profile switch as one recoverable transaction.
//!
//! The plan is re-run under the transaction lock. Every written or replaced
//! path is staged and verified from its package archive before the journal
//! appears. Execution then quarantines each path it removes or replaces with
//! a hard-link `.old` witness, moves the staged bytes into place through a
//! `.tmp` witness, checks cancellation once more before the commit point, and
//! commits the active profile, the winner rows, the history record and the
//! journal status in one SQLite transaction. A failure before that commit
//! rolls back through the normal recovery path, so the old profile stays
//! active and every game byte is unchanged.

use super::plan::{prepare, PreparedSwitch, Remove, Write};
use super::ProfileSwitchPlan;
use crate::error::ErrorCode;
use crate::progress::{OperationControl, ProgressEvent};
use crate::state::Store;
use anyhow::{bail, Context, Result};
use rusqlite::params;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::Path;
use tempfile::Builder;

/// How one profile switch is controlled and reported.
///
/// Progress arrives through the control's reporter, as it does for an
/// install: `extract`, then `stage`, then `commit`. A cancellation before the
/// `commit` event stops the switch with [`ErrorCode::Cancelled`] and rolls it
/// back.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct ProfileSwitchOptions {
    /// Cancellation and progress control of this switch.
    pub control: OperationControl,
}

impl ProfileSwitchOptions {
    /// Options that report to `control` and follow its cancellation.
    #[must_use]
    pub fn new(control: OperationControl) -> Self {
        Self { control }
    }
}

/// The committed state and counts of one profile switch.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProfileSwitchOutcome {
    /// The plan that the switch executed.
    pub plan: ProfileSwitchPlan,
    /// Transaction identifier, also the history operation identifier.
    pub transaction_id: String,
    /// Number of managed paths the switch removed.
    pub removed: usize,
    /// Number of managed paths the switch wrote or replaced.
    pub written: usize,
    /// Number of managed paths whose bytes already matched the target.
    pub unchanged: usize,
}

/// A crash point that a test injects to prove recovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SwitchFault {
    /// The journal exists, but no game file moved.
    AfterJournal,
    /// This many paths are quarantined.
    AfterQuarantines(usize),
    /// Every destination holds its new bytes, but nothing committed.
    AfterMoves,
    /// The SQLite commit succeeded, but cleanup did not start.
    AfterCommit,
    /// Every destination moved, then the caller cancels before the commit.
    CancelAfterMoves,
}

/// Switch `game` from its active profile to `target`.
///
/// The routine takes the transaction lock, re-runs the read-only plan under
/// it, and changes nothing when a refusal, a cancellation or a write failure
/// happens before the commit point.
///
/// # Errors
///
/// Returns the plan refusals (`PENDING_TRANSACTION`, `PROFILE_NOT_FOUND`,
/// `PROFILE_ACTIVE`, `PROFILE_DRIFT`, `PROFILE_ARCHIVE_MISSING`, `CONFLICT`)
/// before any write, [`ErrorCode::Cancelled`] for a cancellation before the
/// commit point, and an I/O or state error when staging or the commit fails.
pub fn profile_switch(
    store: &mut Store,
    game: &Path,
    target: &str,
    options: ProfileSwitchOptions,
) -> Result<ProfileSwitchOutcome> {
    let _guard = super::super::lock_store(store)?;
    switch_locked(store, game, target, options, None)
}

/// [`profile_switch`] with an injected crash point for recovery tests.
#[cfg(test)]
pub(super) fn switch_with_fault(
    store: &mut Store,
    game: &Path,
    target: &str,
    options: ProfileSwitchOptions,
    fault: SwitchFault,
) -> Result<ProfileSwitchOutcome> {
    let _guard = super::super::lock_store(store)?;
    switch_locked(store, game, target, options, Some(fault))
}

fn switch_locked(
    store: &mut Store,
    game: &Path,
    target: &str,
    options: ProfileSwitchOptions,
    fault: Option<SwitchFault>,
) -> Result<ProfileSwitchOutcome> {
    let prepared = prepare(store, game, target)?;
    let control = options.control;
    store.set_control(control.clone());
    control.advance(ProgressEvent::Extract { package: None })?;
    let staging = Builder::new()
        .prefix("txn-")
        .tempdir_in(&store.state_root)?;
    let id = staging
        .path()
        .file_name()
        .context("missing transaction id")?
        .to_string_lossy()
        .to_string();
    let staged = match stage_writes(store, &prepared, staging.path(), &control) {
        Ok(()) => {
            control.advance(ProgressEvent::Stage { package: None })?;
            Ok(())
        }
        Err(error) => Err(error),
    };
    if let Err(error) = staged {
        drop(staging);
        return rollback(store, error, fault);
    }
    if let Err(error) = insert_journal(store, &id, &prepared) {
        return rollback(store, error, fault);
    }
    let _staging = staging.keep();
    simulate(fault, SwitchFault::AfterJournal)?;
    let attempt = place(store, &prepared, &id, &control, fault).and_then(|()| {
        control.advance(ProgressEvent::Commit { package: None })?;
        store.commit_profile_switch(&id, &prepared).map(|_| ())
    });
    match attempt {
        Ok(()) => {}
        Err(error) => return rollback(store, error, fault),
    }
    simulate(fault, SwitchFault::AfterCommit)?;
    store
        .recover_locked()
        .context("profile switch committed; cleanup incomplete, run 'modn recover'")?;
    Ok(outcome(prepared, id))
}

fn outcome(prepared: PreparedSwitch, transaction_id: String) -> ProfileSwitchOutcome {
    ProfileSwitchOutcome {
        removed: prepared.plan.remove.len(),
        written: prepared.plan.write.len(),
        unchanged: prepared.plan.unchanged,
        plan: prepared.plan,
        transaction_id,
    }
}

/// Report `error`, rolling back a pending journal unless a test injected a
/// crash. A cancellation keeps its error type, so it reports `CANCELLED`.
fn rollback(
    store: &mut Store,
    error: anyhow::Error,
    fault: Option<SwitchFault>,
) -> Result<ProfileSwitchOutcome> {
    if fault.is_some() && crate::error::code_of(&error) != Some(ErrorCode::Cancelled) {
        return Err(error);
    }
    if let Err(recovery_error) = store.recover_locked() {
        bail!(
            "{error:#}; profile switch recovery incomplete: {recovery_error:#}; run 'modn recover'"
        );
    }
    Err(error)
}

/// Stage and verify every written member before the journal appears.
fn stage_writes(
    store: &Store,
    prepared: &PreparedSwitch,
    staging: &Path,
    control: &OperationControl,
) -> Result<()> {
    let cache = super::super::restore::cache(store);
    for write in &prepared.writes {
        control.checkpoint()?;
        let candidate = super::super::restore::RestoreCandidate {
            package_id: write.package_id.clone(),
            destination: write.relative_path.clone(),
            archive_entry: write.archive_entry.clone(),
            blake3: write.blake3.clone(),
            bytes: write.bytes,
        };
        let archive = super::super::restore::verified_archive(store, &cache, &candidate)?
            .ok_or_else(|| {
                super::super::restore::missing_archive(&candidate, &write.relative_path)
            })?;
        super::super::restore::stage_member(store, &archive, &candidate, staging, write.ordinal)?;
        control.checkpoint()?;
    }
    Ok(())
}

/// Record the pending journal in one SQLite transaction.
fn insert_journal(store: &mut Store, id: &str, prepared: &PreparedSwitch) -> Result<()> {
    let game = prepared.game.to_string_lossy();
    let tx = store.connection.transaction()?;
    tx.execute(
        "INSERT INTO pending_transactions(id,action,status,game_path,package_id,archive_blake3)
         VALUES (?1,'profile-switch','pending',?2,'',NULL)",
        params![id, game.as_ref()],
    )?;
    tx.execute(
        "INSERT INTO pending_profile_switches(transaction_id,game_path,from_profile,to_profile)
         VALUES (?1,?2,?3,?4)",
        params![id, game.as_ref(), prepared.from.name, prepared.to.name],
    )?;
    for write in &prepared.writes {
        tx.execute(
            "INSERT INTO pending_files(transaction_id,ord,root,relative_path,blake3,bytes)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                id,
                write.ordinal as i64,
                write.root.label(),
                write.relative_path,
                write.blake3,
                write.bytes as i64
            ],
        )?;
    }
    for path in overrides(prepared) {
        tx.execute(
            "INSERT INTO pending_overrides
             (transaction_id,ord,root,relative_path,package_id,blake3,bytes)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                id,
                path.ordinal as i64,
                path.root.label(),
                path.relative_path,
                path.package_id,
                path.blake3,
                path.bytes as i64
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// One path whose current bytes leave their place: a removal or a replacement.
struct Override<'a> {
    ordinal: usize,
    root: crate::state::RootKind,
    relative_path: &'a str,
    package_id: &'a str,
    blake3: &'a str,
    bytes: u64,
    target: &'a Path,
}

/// Every removed or replaced path, in execution order.
fn overrides(prepared: &PreparedSwitch) -> impl Iterator<Item = Override<'_>> {
    let replaced = prepared.writes.iter().filter_map(|write| {
        Some(Override {
            ordinal: write.ordinal,
            root: write.root,
            relative_path: &write.relative_path,
            package_id: write.replaces.as_deref()?,
            blake3: write.replaces_blake3.as_deref()?,
            bytes: write.replaces_bytes?,
            target: &write.target,
        })
    });
    let removed = prepared.removes.iter().map(|remove: &Remove| Override {
        ordinal: remove.ordinal,
        root: remove.root,
        relative_path: &remove.relative_path,
        package_id: &remove.package_id,
        blake3: &remove.blake3,
        bytes: remove.bytes,
        target: &remove.target,
    });
    replaced.chain(removed)
}

/// Quarantine every removed or replaced path, then place every staged write.
fn place(
    store: &Store,
    prepared: &PreparedSwitch,
    id: &str,
    control: &OperationControl,
    fault: Option<SwitchFault>,
) -> Result<()> {
    for (index, path) in overrides(prepared).enumerate() {
        control.checkpoint()?;
        quarantine(path.target, id, path.ordinal, path.blake3)?;
        simulate(fault, SwitchFault::AfterQuarantines(index + 1))?;
    }
    for write in &prepared.writes {
        control.checkpoint()?;
        place_write(store, id, write)?;
    }
    if fault == Some(SwitchFault::CancelAfterMoves) {
        control.token().cancel();
    }
    simulate(fault, SwitchFault::AfterMoves)
}

/// Keep the current bytes of `dest` in a hard-link witness and remove it.
fn quarantine(dest: &Path, id: &str, ordinal: usize, blake3: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(dest).with_context(|| {
        format!(
            "PROFILE_DRIFT: a managed file is missing during the profile switch; run 'modn doctor': {}",
            dest.display()
        )
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!(
            "PROFILE_DRIFT: a managed path is not a regular file during the profile switch; run 'modn doctor': {}",
            dest.display()
        );
    }
    if super::super::journal::digest(dest)? != blake3 {
        bail!(
            "PROFILE_DRIFT: a managed file changed during the profile switch; run 'modn doctor': {}",
            dest.display()
        );
    }
    let backup = super::super::journal::backup_path(dest, id, ordinal)?;
    fs::hard_link(dest, &backup)?;
    if !same_file::is_same_file(dest, &backup)? || super::super::journal::digest(&backup)? != blake3
    {
        bail!(
            "PROFILE_DRIFT: a managed file changed while it was backed up: {}",
            dest.display()
        );
    }
    fs::remove_file(dest)?;
    Ok(())
}

/// Move one staged member into place through a `.tmp` witness.
fn place_write(store: &Store, id: &str, write: &Write) -> Result<()> {
    if let Some(parent) = write.target.parent() {
        if !parent.is_dir() {
            fs::create_dir_all(parent)?;
        }
    }
    let temp = super::super::journal::temp_path(&write.target, id, write.ordinal)?;
    let staged = store
        .state_root
        .join(id)
        .join(format!("{}.bin", write.ordinal));
    let mut input = File::open(&staged)?;
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)?;
    if io::copy(&mut input, &mut output)? != write.bytes {
        bail!(
            "profile switch staging size mismatch for {}",
            write.relative_path
        );
    }
    output.sync_all()?;
    drop(output);
    if super::super::journal::digest(&temp)? != write.blake3 {
        bail!(
            "profile switch staging digest mismatch for {}",
            write.relative_path
        );
    }
    fs::hard_link(&temp, &write.target)?;
    if super::super::journal::digest(&write.target)? != write.blake3 {
        bail!(
            "profile switch destination digest mismatch: {}",
            write.target.display()
        );
    }
    Ok(())
}

/// Fail the way a crash would when `fault` names `point`.
fn simulate(fault: Option<SwitchFault>, point: SwitchFault) -> Result<()> {
    if fault.is_some_and(|fault| fault == point) {
        bail!("injected profile switch stop at {point:?}");
    }
    Ok(())
}
