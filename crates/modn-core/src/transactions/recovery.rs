//! Recovery of interrupted journaled transactions.
//!
//! Recovery reads each pending journal and either completes it or rolls it
//! back. It never deletes a file whose digest it cannot verify.
use super::journal::{
    digest, lock_store, target_path, temp_path, Journal, JournalDirectory, JournalFile,
};
use super::validation::validate_transaction_root;
use crate::state::Store;
use anyhow::{bail, Context, Result};
use std::fs;
use std::io;
use std::path::PathBuf;

impl Store {
    pub(super) fn pending(&self) -> Result<Vec<Journal>> {
        let mut statement = self.connection.prepare(
            "SELECT id, action, status, game_path FROM pending_transactions ORDER BY id",
        )?;
        let entries = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        entries
            .into_iter()
            .map(|(id, action, status, game)| {
                let mut q = self.connection.prepare(
                    "SELECT ord, relative_path, sha256, bytes FROM pending_files
                 WHERE transaction_id=?1 ORDER BY ord",
                )?;
                let files = q
                    .query_map([&id], |row| {
                        Ok(JournalFile {
                            ordinal: row.get::<_, i64>(0)? as usize,
                            destination: row.get(1)?,
                            sha256: row.get(2)?,
                            bytes: row.get::<_, i64>(3)? as u64,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let mut q = self.connection.prepare(
                     "SELECT relative_path,identity FROM pending_directories WHERE transaction_id=?1 ORDER BY ord")?;
                let directories = q.query_map([&id], |row| {
                    Ok(JournalDirectory { root: row.get(0)?, identity: row.get(1)? })
                })?.collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(Journal { id, action, status, game: PathBuf::from(game), files, directories })
            })
            .collect()
    }

    fn finish_journal(&mut self, id: &str) -> Result<()> {
        let staging = self.state_root.join(id);
        if staging.exists() {
            fs::remove_dir_all(&staging).with_context(|| {
                format!(
                    "cannot clean staged transaction {}; retry 'modn recover'",
                    staging.display()
                )
            })?;
        }
        let tx = self.connection.transaction()?;
        tx.execute(
            "DELETE FROM pending_directories WHERE transaction_id=?1",
            [id],
        )?;
        tx.execute("DELETE FROM pending_files WHERE transaction_id=?1", [id])?;
        tx.execute("DELETE FROM pending_transactions WHERE id=?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    fn recover_one(&mut self, journal: &Journal) -> Result<()> {
        if journal.action == "install" && journal.status == "staging" {
            if !journal.files.is_empty() {
                bail!("staging journal contains unexpected files");
            }
            return self.finish_journal(&journal.id);
        }
        let game = validate_transaction_root(&journal.game)?;
        for f in journal.files.iter().rev() {
            let dest = target_path(&game, &f.destination, /* create_parents= */ false)?;
            let temp = temp_path(&dest, &journal.id, f.ordinal)?;
            let witness = match fs::symlink_metadata(&temp) {
                Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
                    if digest(&temp)? != f.sha256 {
                        bail!("transaction {} has a changed or incomplete witness {}; preserve it and repair manually",
                            journal.id, temp.display());
                    }
                    true
                }
                Ok(_) => bail!("unsafe transaction witness: {}", temp.display()),
                Err(e) if e.kind() == io::ErrorKind::NotFound => false,
                Err(e) => return Err(e.into()),
            };
            let present = fs::symlink_metadata(&dest).is_ok();
            match (journal.action.as_str(), journal.status.as_str()) {
                ("install", "pending") => {
                    if witness && present {
                        if !same_file::is_same_file(&temp, &dest)? || digest(&dest)? != f.sha256 {
                            bail!(
                                "transaction {} target changed; preserve {}",
                                journal.id,
                                dest.display()
                            );
                        }
                        fs::remove_file(&dest)?;
                    }
                    if witness {
                        fs::remove_file(&temp)?;
                    }
                }
                ("uninstall", "pending") => {
                    if witness {
                        if present {
                            if !same_file::is_same_file(&temp, &dest)? || digest(&dest)? != f.sha256
                            {
                                bail!(
                                    "transaction {} destination changed; preserve {}",
                                    journal.id,
                                    dest.display()
                                );
                            }
                        } else {
                            let checked = target_path(
                                &game,
                                &f.destination,
                                /* create_parents= */ false,
                            )?;
                            if let Some(parent) = checked.parent() {
                                if !parent.is_dir() {
                                    bail!("recovery parent missing: {}", parent.display());
                                }
                            }
                            fs::hard_link(&temp, &checked)?;
                        }
                        fs::remove_file(&temp)?;
                    } else if !present || digest(&dest)? != f.sha256 {
                        bail!(
                            "transaction {} has no safe uninstall backup for {}",
                            journal.id,
                            dest.display()
                        );
                    }
                }
                (_, "committed") => {
                    if witness {
                        fs::remove_file(&temp)?;
                    }
                }
                _ => bail!("unknown transaction state for {}", journal.id),
            }
        }
        if journal.action == "uninstall" {
            for (ordinal, claim) in journal.directories.iter().enumerate() {
                super::directory::recover_directory(
                    &game,
                    &claim.root,
                    &journal.id,
                    ordinal,
                    claim
                        .identity
                        .as_deref()
                        .context("missing uninstall directory identity")?,
                    journal.status == "committed",
                )?;
            }
        } else if journal.action == "install" && journal.status == "pending" {
            for (ordinal, claim) in journal.directories.iter().enumerate().rev() {
                super::directory::recover_install(
                    &game,
                    &claim.root,
                    &journal.id,
                    ordinal,
                    claim.identity.as_deref(),
                )?;
            }
        }
        self.finish_journal(&journal.id)
    }

    /// Recover interrupted transactions, then roll back unfinished installation
    /// stacks. Never deletes an unverified file.
    pub fn recover(&mut self) -> Result<usize> {
        let _guard = lock_store(self)?;
        Ok(self.recover_locked()? + self.recover_stacks_locked()?)
    }

    pub(super) fn recover_locked(&mut self) -> Result<usize> {
        let pending = self.pending()?;
        for entry in &pending {
            self.recover_one(entry)?;
        }
        Ok(pending.len())
    }
}
