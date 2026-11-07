use crate::state::{record_root, RootKind, Store};
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use rusqlite::params;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
/// Installed package and its manager-owned file manifest.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct ManagedPackage {
    pub id: String,
    pub pinned: bool,
    pub game_installation: PathBuf,
    pub archive_sha256: String,
    /// Exact owned files installed through the recorded source ZIP.
    pub files: Vec<OwnedFile>,
    /// Exclusive game-relative directory roots, owned in their entirety.
    pub directories: Vec<String>,
    /// Catalog identity, when the package was installed from a bucket.
    pub catalog: Option<super::CatalogReference>,
}

/// Manifest record for a file owned by a managed package.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct OwnedFile {
    /// Installation root that contains the destination: `game` or `content`.
    pub root: String,
    /// Root-relative destination path.
    pub destination: String,
    /// Verified installed SHA-256 digest.
    pub sha256: String,
    /// Verified installed file size.
    pub bytes: u64,
}

#[derive(Clone)]
pub(super) struct JournalFile {
    pub(super) ordinal: usize,
    pub(super) root: RootKind,
    pub(super) destination: String,
    pub(super) sha256: String,
    pub(super) bytes: u64,
}

pub(super) struct JournalDirectory {
    pub(super) root: RootKind,
    pub(super) path: String,
    pub(super) identity: Option<String>,
}

/// Resolved physical paths for one installation's roots.
#[derive(Clone)]
pub(super) struct Roots {
    pub(super) game: PathBuf,
    pub(super) content: Option<PathBuf>,
}

impl Roots {
    pub(super) fn path(&self, root: RootKind) -> Result<&Path> {
        match root {
            RootKind::Game => Ok(&self.game),
            RootKind::Content => self
                .content
                .as_deref()
                .context("installation has no content root"),
        }
    }

    pub(super) fn target(&self, root: RootKind, relative: &str) -> Result<PathBuf> {
        target_path(self.path(root)?, relative)
    }
}

pub(super) struct Journal {
    pub(super) id: String,
    pub(super) action: String,
    pub(super) status: String,
    pub(super) game: PathBuf,
    pub(super) files: Vec<JournalFile>,
    pub(super) directories: Vec<JournalDirectory>,
}

pub(super) fn digest(path: &Path) -> Result<String> {
    let mut reader = File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut reader, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

pub(crate) fn lock_store(store: &Store) -> Result<File> {
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
pub(super) fn target_path(game: &Path, relative: &str) -> Result<PathBuf> {
    let canonical = game.canonicalize()?;
    let mut current = canonical.clone();
    let parts: Vec<_> = relative.split('/').collect();
    if parts
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
    pub(crate) fn ensure_no_pending(&self) -> Result<()> {
        self.ensure_pending_only(None)
    }

    /// Refuse to continue while any journal other than `allowed` is pending.
    pub(super) fn ensure_pending_only(&self, allowed: Option<&str>) -> Result<()> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM pending_transactions WHERE id IS NOT ?1",
            [allowed],
            |row| row.get(0),
        )?;
        if count > 0 {
            bail!("unfinished transaction detected; run 'modn recover' before another write");
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "journal keeps file and directory witnesses separately"
    )]
    pub(super) fn insert_journal(
        &mut self,
        id: &str,
        action: &str,
        game: &Path,
        package_id: &str,
        archive_hash: Option<&str>,
        files: &[JournalFile],
        directories: &[String],
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
                "INSERT INTO pending_files(transaction_id,ord,root,relative_path,sha256,bytes)
                VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    id,
                    f.ordinal as i64,
                    f.root.label(),
                    f.destination,
                    f.sha256,
                    f.bytes as i64
                ],
            )?;
        }
        for (ordinal, directory) in directories.iter().enumerate() {
            let identity: Option<String> = if matches!(action, "uninstall" | "cleanup") {
                Some(tx.query_row(
                    "SELECT identity FROM managed_directories WHERE package_id=?1 AND root=?2 AND relative_path=?3",
                    params![package_id, RootKind::Game.label(), directory],
                    |row| row.get::<_, Option<String>>(0),
                )?.context("owned directory identity missing")?)
            } else {
                None
            };
            tx.execute(
                "INSERT INTO pending_directories(transaction_id,ord,root,relative_path,identity) VALUES (?1,?2,?3,?4,?5)",
                params![id, ordinal as i64, RootKind::Game.label(), directory, identity])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(super) fn begin_staging(
        &mut self,
        id: &str,
        game: &Path,
        hash: &str,
        allowed: Option<&str>,
    ) -> Result<()> {
        self.ensure_pending_only(allowed)?;
        let tx = self.connection.transaction()?;
        tx.execute(
            "INSERT INTO pending_transactions(id,action,status,game_path,package_id,archive_sha256)
             VALUES (?1,'install','staging',?2,?1,?3)",
            params![id, game.to_string_lossy().as_ref(), hash],
        )?;
        // Recovery may treat a missing `pending_links` row as proof that no
        // link exists only for a journal that carries this marker.
        tx.execute(
            "INSERT INTO link_recorded_journals(transaction_id) VALUES (?1)",
            [id],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub(super) fn stage_ready(
        &mut self,
        id: &str,
        files: &[JournalFile],
        directories: &[JournalDirectory],
    ) -> Result<()> {
        let tx = self.connection.transaction()?;
        for f in files {
            tx.execute(
                "INSERT INTO pending_files(transaction_id,ord,root,relative_path,sha256,bytes)
                VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    id,
                    f.ordinal as i64,
                    f.root.label(),
                    f.destination,
                    f.sha256,
                    f.bytes as i64
                ],
            )?;
        }
        for (index, directory) in directories.iter().enumerate() {
            tx.execute(
                "INSERT INTO pending_directories(transaction_id,ord,root,relative_path) VALUES (?1,?2,?3,?4)",
                params![id, index as i64, directory.root.label(), directory.path])?;
        }
        tx.execute(
            "UPDATE pending_transactions SET status='pending' WHERE id=?1 AND status='staging'",
            [id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub(super) fn record_directory_identity(
        &mut self,
        journal_id: &str,
        kind: RootKind,
        root: &str,
        identity: &str,
    ) -> Result<()> {
        let updated = self.connection.execute(
            "UPDATE pending_directories SET identity=?4
             WHERE transaction_id=?1 AND root=?2 AND relative_path=?3 AND identity IS NULL",
            params![journal_id, kind.label(), root, identity],
        )?;
        if updated != 1 {
            bail!("cannot persist exclusive directory identity: {root}");
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the commit names the root and catalog context separately"
    )]
    pub(super) fn commit_install(
        &mut self,
        id: &str,
        game: &Path,
        hash: &str,
        files: &[JournalFile],
        directories: &[JournalDirectory],
        content: Option<&Path>,
        catalog: Option<&super::catalog::CatalogRecord>,
    ) -> Result<()> {
        let tx = self.connection.transaction()?;
        let replaced = catalog.and_then(|catalog| catalog.replacing);
        if let Some(replaced) = replaced {
            super::update::vacate(&tx, replaced)?;
        }
        record_root(&tx, game, RootKind::Game, game)?;
        if let Some(content) = content {
            record_root(&tx, game, RootKind::Content, content)?;
        }
        tx.execute(
            "INSERT INTO managed_packages(id,game_path,archive_sha256) VALUES (?1,?2,?3)",
            params![id, game.to_string_lossy().as_ref(), hash],
        )?;
        for f in files {
            tx.execute("INSERT INTO managed_files(game_path,root,path_key,relative_path,package_id,sha256,bytes)
                VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![game.to_string_lossy().as_ref(), f.root.label(),
                    super::ownership_key(&f.destination),
                    f.destination, id, f.sha256, f.bytes as i64])?;
        }
        for directory in directories {
            let identity: String = tx.query_row(
                "SELECT identity FROM pending_directories WHERE transaction_id=?1 AND root=?2 AND relative_path=?3",
                params![id, directory.root.label(), directory.path],
                |row| row.get::<_, Option<String>>(0),
            )?.context("exclusive directory identity must be persisted before commit")?;
            tx.execute(
                "INSERT INTO managed_directories(game_path,root,path_key,relative_path,package_id,identity)
                 VALUES (?1,?2,?3,?4,?5,?6)",
                params![game.to_string_lossy().as_ref(), directory.root.label(),
                    super::ownership_key(&directory.path), directory.path, id, identity],
            )?;
        }
        if let Some(catalog) = catalog {
            super::catalog::record(&tx, id, catalog)?;
        }
        if let Some(replaced) = replaced {
            super::update::retire(&tx, replaced, id)?;
        }
        super::created_dirs::commit(&tx, id, game, content)?;
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
            "DELETE FROM managed_directories WHERE package_id=?1",
            [package_id],
        )?;
        tx.execute(
            "DELETE FROM managed_files WHERE package_id=?1",
            [package_id],
        )?;
        super::catalog::forget(&tx, package_id)?;
        tx.execute("DELETE FROM managed_packages WHERE id=?1", [package_id])?;
        tx.execute(
            "UPDATE pending_transactions SET status='committed' WHERE id=?1",
            [journal_id],
        )?;
        tx.commit()?;
        Ok(())
    }
}
