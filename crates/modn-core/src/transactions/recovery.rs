use super::journal::{
    digest, lock_store, temp_path, Journal, JournalDirectory, JournalFile, Roots,
};
use crate::state::Store;
use crate::state::{recorded_root, resolve_root, RootKind};
use anyhow::{bail, Context, Result};
use std::fs;
use std::io;
use std::path::PathBuf;

impl Store {
    pub(super) fn pending(&self) -> Result<Vec<Journal>> {
        let mut statement = self.connection.prepare(
            "SELECT id, action, status, game_path FROM pending_transactions
             ORDER BY action = 'uninstall', id",
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
                    "SELECT ord, root, relative_path, sha256, bytes FROM pending_files
                 WHERE transaction_id=?1 ORDER BY ord",
                )?;
                let files = q
                    .query_map([&id], |row| {
                        Ok(JournalFile {
                            ordinal: row.get::<_, i64>(0)? as usize,
                            root: RootKind::from_label(&row.get::<_, String>(1)?)
                                .unwrap_or(RootKind::Game),
                            destination: row.get(2)?,
                            sha256: row.get(3)?,
                            bytes: row.get::<_, i64>(4)? as u64,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let mut q = self.connection.prepare(
                     "SELECT root,relative_path,identity FROM pending_directories WHERE transaction_id=?1 ORDER BY ord")?;
                let directories = q.query_map([&id], |row| {
                    Ok(JournalDirectory {
                        root: RootKind::from_label(&row.get::<_, String>(0)?)
                            .unwrap_or(RootKind::Game),
                        path: row.get(1)?,
                        identity: row.get(2)?,
                    })
                })?.collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(Journal { id, action, status, game: PathBuf::from(game), files, directories })
            })
            .collect()
    }

    /// Returns true only when the journal records link attempts and has no
    /// attempt for this file. A journal from an older Modn has no marker, so
    /// missing evidence proves nothing for it.
    fn never_linked(&self, id: &str, ordinal: usize) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM link_recorded_journals WHERE transaction_id=?1)
               AND NOT EXISTS(SELECT 1 FROM pending_links WHERE transaction_id=?1 AND ord=?2)",
            rusqlite::params![id, ordinal as i64],
            |row| row.get(0),
        )?)
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
        tx.execute(
            "DELETE FROM pending_created_directories WHERE transaction_id=?1",
            [id],
        )?;
        tx.execute("DELETE FROM pending_links WHERE transaction_id=?1", [id])?;
        tx.execute(
            "DELETE FROM link_recorded_journals WHERE transaction_id=?1",
            [id],
        )?;
        tx.execute("DELETE FROM pending_files WHERE transaction_id=?1", [id])?;
        tx.execute("DELETE FROM pending_transactions WHERE id=?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    fn recover_one(&mut self, journal: &Journal) -> Result<()> {
        let game = resolve_root(&self.connection, &journal.game, RootKind::Game)?;
        let has_content = journal
            .files
            .iter()
            .any(|file| file.root == RootKind::Content)
            || journal
                .directories
                .iter()
                .any(|directory| directory.root == RootKind::Content);
        let content = if has_content {
            recorded_root(&self.connection, &journal.game, RootKind::Content)?
        } else {
            None
        };
        let roots = Roots { game, content };
        if journal.action == "install" && journal.status == "staging" {
            if !journal.files.is_empty() {
                bail!("staging journal contains unexpected files");
            }
            self.remove_journal_directories(&roots, &journal.id)?;
            return self.finish_journal(&journal.id);
        }
        if journal.action == "cleanup" {
            for claim in &journal.directories {
                super::directory::verify_directory(
                    &roots.target(claim.root, &claim.path)?,
                    claim
                        .identity
                        .as_deref()
                        .context("missing cleanup directory identity")?,
                )?;
            }
        }
        for f in journal.files.iter().rev() {
            let dest = roots.target(f.root, &f.destination)?;
            let temp = temp_path(&dest, &journal.id, f.ordinal)?;
            let present = fs::symlink_metadata(&dest).is_ok();
            let witness = match fs::symlink_metadata(&temp) {
                Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
                    let never_linked = journal.action == "install"
                        && journal.status == "pending"
                        && self.never_linked(&journal.id, f.ordinal)?;
                    if never_linked {
                        fs::remove_file(&temp)?;
                        continue;
                    }
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
                ("uninstall" | "cleanup", "pending") => {
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
                            let checked = roots.target(f.root, &f.destination)?;
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
                    roots.path(claim.root)?,
                    &claim.path,
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
                    roots.path(claim.root)?,
                    &claim.path,
                    &journal.id,
                    ordinal,
                    claim.identity.as_deref(),
                )?;
            }
            self.remove_journal_directories(&roots, &journal.id)?;
        }
        self.finish_journal(&journal.id)
    }

    /// Recover interrupted transactions, roll back unfinished installation
    /// stacks, then remove empty directories that Modn created. Never deletes
    /// an unverified file or a directory that Modn did not create.
    pub fn recover(&mut self) -> Result<usize> {
        let _guard = lock_store(self)?;
        let recovered = self.recover_locked()? + self.recover_stacks_locked()?;
        self.prune_all_created()?;
        Ok(recovered)
    }

    pub(crate) fn recover_locked(&mut self) -> Result<usize> {
        let pending = self.pending()?;
        for entry in &pending {
            self.recover_one(entry)?;
        }
        Ok(pending.len())
    }
}
