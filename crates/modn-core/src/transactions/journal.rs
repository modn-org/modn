//! Transaction journal persistence, locking, rollback and recovery.
use crate::discovery::validate_installation;
use crate::state::Store;
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use rusqlite::params;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

/// Installed package and its manager-owned file manifest.
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct ManagedPackage {
    /// Package ID assigned during installation.
    pub id: String,
    /// Destination game installation.
    pub game_installation: PathBuf,
    /// Digest of the source archive.
    pub archive_sha256: String,
    /// Owned files; unmanaged content is never adopted.
    pub files: Vec<OwnedFile>,
}

/// Manifest record for a file owned by a managed package.
#[derive(Clone, Debug, Serialize)]
#[non_exhaustive]
pub struct OwnedFile {
    /// Game-relative destination path.
    pub destination: String,
    /// Verified installed SHA-256 digest.
    pub sha256: String,
    /// Verified installed file size.
    pub bytes: u64,
}

#[derive(Clone)]
pub(super) struct JournalFile {
    pub(super) ordinal: usize,
    pub(super) destination: String,
    pub(super) sha256: String,
    pub(super) bytes: u64,
}

pub(super) struct Journal {
    id: String,
    action: String,
    status: String,
    game: PathBuf,
    files: Vec<JournalFile>,
}

pub(super) fn digest(path: &Path) -> Result<String> {
    let mut reader = File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut reader, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

pub(super) fn lock_store(store: &Store) -> Result<File> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(store.state_root.join(".transactions.lock"))?;
    lock.lock_exclusive()
        .context("cannot acquire exclusive transaction lock")?;
    Ok(lock)
}

pub(super) fn target_path(game: &Path, relative: &str, create_parents: bool) -> Result<PathBuf> {
    let canonical = game.canonicalize()?;
    let mut current = canonical.clone();
    let parts: Vec<_> = relative.split('/').collect();
    if parts.len() < 2
        || parts
            .iter()
            .any(|p| p.is_empty() || *p == "." || *p == ".." || p.contains(['\\', ':']))
    {
        bail!("invalid destination path: {relative}");
    }
    for part in &parts[..parts.len() - 1] {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => bail!("linked or non-directory ancestor: {}", current.display()),
            Err(e) if e.kind() == io::ErrorKind::NotFound && create_parents => {
                fs::create_dir(&current)
                    .with_context(|| format!("cannot create {}", current.display()))?;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(canonical.join(relative)),
            Err(e) => return Err(e.into()),
        }
        if !current.canonicalize()?.starts_with(&canonical) {
            bail!("destination escapes game root");
        }
    }
    let target = canonical.join(relative);
    if let Ok(meta) = fs::symlink_metadata(&target) {
        if meta.file_type().is_symlink() {
            bail!("linked destination: {}", target.display());
        }
    }
    Ok(target)
}

pub(super) fn temp_path(target: &Path, id: &str, n: usize) -> Result<PathBuf> {
    Ok(target
        .parent()
        .context("target has no parent")?
        .join(format!(".modn-txn-{id}-{n}.tmp")))
}

impl Store {
    /// List packages installed by this manager for the selected game.
    pub fn installed(&self, game: &Path) -> Result<Vec<ManagedPackage>> {
        let game = validate_installation(game)?;
        let name = game.to_string_lossy().into_owned();
        let mut query = self.connection.prepare(
            "SELECT id, archive_sha256 FROM managed_packages WHERE game_path=?1 ORDER BY created_at, id")?;
        let packages = query
            .query_map([&name], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        packages.into_iter().map(|(id, archive_sha256)| {
            let mut statement = self.connection.prepare(
                "SELECT relative_path, sha256, bytes FROM managed_files WHERE package_id=?1 ORDER BY relative_path")?;
            let files = statement.query_map([&id], |row| Ok(OwnedFile {
                destination: row.get(0)?, sha256: row.get(1)?, bytes: row.get::<_,i64>(2)? as u64
            }))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(ManagedPackage { id, game_installation: game.clone(), archive_sha256, files })
        }).collect()
    }

    pub(super) fn ensure_no_pending(&self) -> Result<()> {
        let count: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM pending_transactions", [], |row| {
                    row.get(0)
                })?;
        if count > 0 {
            bail!("unfinished transaction detected; run 'modn recover' before another write");
        }
        Ok(())
    }

    pub(super) fn insert_journal(
        &mut self,
        id: &str,
        action: &str,
        game: &Path,
        package_id: &str,
        archive_hash: Option<&str>,
        files: &[JournalFile],
    ) -> Result<()> {
        self.ensure_no_pending()?;
        let tx = self.connection.transaction()?;
        tx.execute(
            "INSERT INTO pending_transactions(id,action,status,game_path,package_id,archive_sha256)
            VALUES (?1,?2,'pending',?3,?4,?5)",
            params![
                id,
                action,
                game.to_string_lossy().as_ref(),
                package_id,
                archive_hash
            ],
        )?;
        for f in files {
            tx.execute(
                "INSERT INTO pending_files(transaction_id,ord,relative_path,sha256,bytes)
                VALUES (?1,?2,?3,?4,?5)",
                params![
                    id,
                    f.ordinal as i64,
                    f.destination,
                    f.sha256,
                    f.bytes as i64
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(super) fn begin_staging(&mut self, id: &str, game: &Path, hash: &str) -> Result<()> {
        self.ensure_no_pending()?;
        self.connection.execute(
            "INSERT INTO pending_transactions(id,action,status,game_path,package_id,archive_sha256)
             VALUES (?1,'install','staging',?2,?1,?3)",
            params![id, game.to_string_lossy().as_ref(), hash],
        )?;
        Ok(())
    }

    pub(super) fn stage_ready(&mut self, id: &str, files: &[JournalFile]) -> Result<()> {
        let tx = self.connection.transaction()?;
        for f in files {
            tx.execute(
                "INSERT INTO pending_files(transaction_id,ord,relative_path,sha256,bytes)
                VALUES (?1,?2,?3,?4,?5)",
                params![
                    id,
                    f.ordinal as i64,
                    f.destination,
                    f.sha256,
                    f.bytes as i64
                ],
            )?;
        }
        tx.execute(
            "UPDATE pending_transactions SET status='pending' WHERE id=?1 AND status='staging'",
            [id],
        )?;
        tx.commit()?;
        Ok(())
    }

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
                Ok(Journal {
                    id,
                    action,
                    status,
                    game: PathBuf::from(game),
                    files,
                })
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
        let game = validate_installation(&journal.game)?;
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
        self.finish_journal(&journal.id)
    }

    /// Recover interrupted transactions. Never deletes an unverified file.
    pub fn recover(&mut self) -> Result<usize> {
        let _guard = lock_store(self)?;
        self.recover_locked()
    }

    pub(super) fn recover_locked(&mut self) -> Result<usize> {
        let pending = self.pending()?;
        for entry in &pending {
            self.recover_one(entry)?;
        }
        Ok(pending.len())
    }

    pub(super) fn commit_install(
        &mut self,
        id: &str,
        game: &Path,
        hash: &str,
        files: &[JournalFile],
    ) -> Result<()> {
        let tx = self.connection.transaction()?;
        tx.execute(
            "INSERT INTO managed_packages(id,game_path,archive_sha256) VALUES (?1,?2,?3)",
            params![id, game.to_string_lossy().as_ref(), hash],
        )?;
        for f in files {
            tx.execute("INSERT INTO managed_files(game_path,path_key,relative_path,package_id,sha256,bytes)
                VALUES (?1,?2,?3,?4,?5,?6)",
                params![game.to_string_lossy().as_ref(), f.destination.to_lowercase(),
                    f.destination, id, f.sha256, f.bytes as i64])?;
        }
        tx.execute(
            "UPDATE pending_transactions SET status='committed' WHERE id=?1",
            [id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub(super) fn commit_uninstall(&mut self, journal_id: &str, package_id: &str) -> Result<()> {
        let tx = self.connection.transaction()?;
        tx.execute(
            "DELETE FROM managed_files WHERE package_id=?1",
            [package_id],
        )?;
        tx.execute("DELETE FROM managed_packages WHERE id=?1", [package_id])?;
        tx.execute(
            "UPDATE pending_transactions SET status='committed' WHERE id=?1",
            [journal_id],
        )?;
        tx.commit()?;
        Ok(())
    }
}
