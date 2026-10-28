//! Exclusive, journaled installs of ZIP legacy archives. All paths are game-relative.
use crate::{archive::inspect, discovery::validate_installation, planning::plan, state::Store};
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use rusqlite::params;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
use tempfile::Builder;

const MAX_TOTAL: u64 = 8 * 1024 * 1024 * 1024;
const MAX_FILE: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct ManagedPackage {
    pub id: String,
    pub game_installation: PathBuf,
    pub archive_sha256: String,
    pub files: Vec<OwnedFile>,
}

#[derive(Clone, Debug, Serialize)]
pub struct OwnedFile {
    pub destination: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone)]
struct JournalFile {
    ordinal: usize,
    destination: String,
    sha256: String,
    bytes: u64,
}

struct Journal {
    id: String,
    action: String,
    status: String,
    game: PathBuf,
    files: Vec<JournalFile>,
}

fn digest(path: &Path) -> Result<String> {
    let mut reader = File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut reader, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn lock_store(store: &Store) -> Result<File> {
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

fn target_path(game: &Path, relative: &str, create_parents: bool) -> Result<PathBuf> {
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

fn temp_path(target: &Path, id: &str, n: usize) -> Result<PathBuf> {
    Ok(target
        .parent()
        .context("target has no parent")?
        .join(format!(".mods-txn-{id}-{n}.tmp")))
}

impl Store {
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

    fn ensure_no_pending(&self) -> Result<()> {
        let count: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM pending_transactions", [], |row| {
                    row.get(0)
                })?;
        if count > 0 {
            bail!("unfinished transaction detected; run 'mod recover' before another write");
        }
        Ok(())
    }

    fn insert_journal(
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

    fn begin_staging(&mut self, id: &str, game: &Path, hash: &str) -> Result<()> {
        self.ensure_no_pending()?;
        self.connection.execute(
            "INSERT INTO pending_transactions(id,action,status,game_path,package_id,archive_sha256)
             VALUES (?1,'install','staging',?2,?1,?3)",
            params![id, game.to_string_lossy().as_ref(), hash],
        )?;
        Ok(())
    }

    fn stage_ready(&mut self, id: &str, files: &[JournalFile]) -> Result<()> {
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

    fn pending(&self) -> Result<Vec<Journal>> {
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
                    "cannot clean staged transaction {}; retry 'mod recover'",
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
            let dest = target_path(&game, &f.destination, false)?;
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
                            let checked = target_path(&game, &f.destination, false)?;
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

    fn recover_locked(&mut self) -> Result<usize> {
        let pending = self.pending()?;
        for entry in &pending {
            self.recover_one(entry)?;
        }
        Ok(pending.len())
    }

    fn commit_install(
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

    fn commit_uninstall(&mut self, journal_id: &str, package_id: &str) -> Result<()> {
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

    /// Install a ZIP of legacy archives; never adopts or replaces existing files.
    pub fn install_zip(&mut self, game: &Path, archive: &Path) -> Result<ManagedPackage> {
        self.install_inner(game, archive, None)
    }

    fn install_inner(
        &mut self,
        game: &Path,
        archive: &Path,
        fail_after: Option<usize>,
    ) -> Result<ManagedPackage> {
        let _guard = lock_store(self)?;
        self.ensure_no_pending()?;
        let game = validate_installation(game)?;
        if self.state_root.starts_with(&game) || game.starts_with(&self.state_root) {
            bail!("manager state must be separate from the game directory");
        }
        let inspection = inspect(archive)?;
        if inspection.format != "zip" {
            bail!("installation supports ZIP packages only");
        }
        let proposal = plan(&inspection, &game);
        if proposal.status != "reviewable" {
            bail!("installation plan blocked: {:?}", proposal.blockers);
        }
        if proposal
            .operations
            .iter()
            .any(|f| f.kind != "legacy-archive")
        {
            bail!("installation currently supports legacy .archive files only");
        }
        if proposal.operations.iter().any(|f| f.bytes > MAX_FILE)
            || proposal
                .operations
                .iter()
                .try_fold(0u64, |n, f| n.checked_add(f.bytes))
                .is_none_or(|n| n > MAX_TOTAL)
        {
            bail!("installation exceeds file (4 GiB) or total (8 GiB) limit");
        }
        let staging = Builder::new().prefix("txn-").tempdir_in(&self.state_root)?;
        let id = staging
            .path()
            .file_name()
            .context("missing transaction id")?
            .to_string_lossy()
            .to_string();
        self.begin_staging(&id, &game, &inspection.sha256)?;
        let staged = (|| -> Result<Vec<JournalFile>> {
            let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
            let mut files = Vec::new();
            for (n, op) in proposal.operations.iter().enumerate() {
                let mut input = zip.by_name(&op.source)?;
                if input.is_dir() || input.size() != op.bytes {
                    bail!("archive metadata changed for {}", op.source);
                }
                let stage = staging.path().join(format!("{n}.bin"));
                let mut output = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&stage)?;
                let copied = io::copy(&mut input.by_ref().take(op.bytes + 1), &mut output)?;
                output.flush()?;
                output.sync_all()?;
                if copied != op.bytes {
                    bail!("archive extraction size mismatch for {}", op.source);
                }
                files.push(JournalFile {
                    ordinal: n,
                    destination: op.destination.clone(),
                    sha256: digest(&stage)?,
                    bytes: copied,
                });
            }
            drop(zip);
            if inspect(archive)?.sha256 != inspection.sha256 {
                bail!("archive changed between inspection and extraction");
            }
            Ok(files)
        })();
        let files = match staged {
            Ok(files) => files,
            Err(err) => {
                drop(staging);
                if let Err(recovery_error) = self.recover_locked() {
                    bail!("{err:#}; staging recovery incomplete: {recovery_error:#}; run 'mod recover'");
                }
                return Err(err);
            }
        };
        if let Err(err) = self.stage_ready(&id, &files) {
            drop(staging);
            if let Err(recovery_error) = self.recover_locked() {
                bail!(
                    "{err:#}; staging recovery incomplete: {recovery_error:#}; run 'mod recover'"
                );
            }
            return Err(err);
        }
        let stage_path = staging.keep();
        let attempt = (|| -> Result<()> {
            for f in &files {
                let dest = target_path(&game, &f.destination, true)?;
                if fs::symlink_metadata(&dest).is_ok() {
                    bail!("existing unmanaged destination: {}", dest.display());
                }
                let temp = temp_path(&dest, &id, f.ordinal)?;
                let mut output = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&temp)?;
                let mut input = File::open(stage_path.join(format!("{}.bin", f.ordinal)))?;
                if io::copy(&mut input, &mut output)? != f.bytes {
                    bail!("staging size mismatch");
                }
                output.sync_all()?;
                drop(output);
                if digest(&temp)? != f.sha256 {
                    bail!("staging digest mismatch");
                }
                fs::hard_link(&temp, &dest).with_context(|| {
                    format!(
                        "cannot atomically link destination {}; existing files never overwritten",
                        dest.display()
                    )
                })?;
                if fail_after.is_some_and(|limit| f.ordinal + 1 == limit) {
                    bail!("injected interruption after {} files", f.ordinal + 1);
                }
            }
            self.commit_install(&id, &game, &inspection.sha256, &files)
        })();
        if let Err(err) = attempt {
            let recover = self.recover_locked();
            if let Err(recovery_error) = recover {
                bail!(
                    "{err:#}; automatic rollback incomplete: {recovery_error:#}; run 'mod recover'"
                );
            }
            return Err(err);
        }
        self.recover_locked()?;
        Ok(ManagedPackage {
            id,
            game_installation: game,
            archive_sha256: inspection.sha256,
            files: files
                .into_iter()
                .map(|f| OwnedFile {
                    destination: f.destination,
                    sha256: f.sha256,
                    bytes: f.bytes,
                })
                .collect(),
        })
    }

    /// Uninstall only unchanged manager-owned files, keeping reversible hard-link backups until commit.
    pub fn uninstall(&mut self, game: &Path, package_id: &str) -> Result<()> {
        let _guard = lock_store(self)?;
        self.ensure_no_pending()?;
        let game = validate_installation(game)?;
        let package = self
            .installed(&game)?
            .into_iter()
            .find(|p| p.id == package_id)
            .with_context(|| format!("managed package {package_id} not found for selected game"))?;
        let files: Vec<_> = package
            .files
            .iter()
            .enumerate()
            .map(|(ordinal, f)| JournalFile {
                ordinal,
                destination: f.destination.clone(),
                sha256: f.sha256.clone(),
                bytes: f.bytes,
            })
            .collect();
        for f in &files {
            let target = target_path(&game, &f.destination, false)?;
            if !target.is_file() || digest(&target)? != f.sha256 {
                bail!(
                    "owned file missing or modified; refusing uninstall: {}",
                    target.display()
                );
            }
        }
        let staging = Builder::new().prefix("txn-").tempdir_in(&self.state_root)?;
        let id = staging
            .path()
            .file_name()
            .context("missing transaction id")?
            .to_string_lossy()
            .to_string();
        let _ = staging.keep();
        self.insert_journal(&id, "uninstall", &game, package_id, None, &files)?;
        let attempt = (|| -> Result<()> {
            for f in &files {
                let target = target_path(&game, &f.destination, false)?;
                if !target.is_file() || digest(&target)? != f.sha256 {
                    bail!("owned file modified during uninstall: {}", target.display());
                }
                let temp = temp_path(&target, &id, f.ordinal)?;
                fs::hard_link(&target, &temp)?;
                if !same_file::is_same_file(&target, &temp)?
                    || digest(&temp)? != f.sha256
                    || digest(&target)? != f.sha256
                {
                    bail!(
                        "owned file changed while backing it up: {}",
                        target.display()
                    );
                }
                fs::remove_file(&target)?;
            }
            self.commit_uninstall(&id, package_id)
        })();
        if let Err(err) = attempt {
            if let Err(recovery_error) = self.recover_locked() {
                bail!(
                    "{err:#}; uninstall recovery incomplete: {recovery_error:#}; run 'mod recover'"
                );
            }
            return Err(err);
        }
        self.recover_locked()
            .context("uninstall committed; cleanup incomplete, run 'mod recover'")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;

    fn fixture() -> (tempfile::TempDir, Store, PathBuf) {
        let root = tempdir().unwrap();
        let game = root.path().join("game");
        fs::create_dir_all(game.join("bin/x64")).unwrap();
        fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").unwrap();
        let store = Store::open(&root.path().join("state/state.sqlite")).unwrap();
        (root, store, game)
    }

    fn archive(root: &Path, entries: &[(&str, &[u8])]) -> PathBuf {
        let path = root.join("fixture.zip");
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        for (name, contents) in entries {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(contents).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    fn prepared_file(dest: &str, data: &[u8]) -> JournalFile {
        let mut hasher = Sha256::new();
        hasher.update(data);
        JournalFile {
            ordinal: 0,
            destination: dest.into(),
            sha256: format!("{:x}", hasher.finalize()),
            bytes: data.len() as u64,
        }
    }

    #[test]
    fn install_list_and_uninstall_preserve_unmanaged_files() {
        let (dir, mut store, game) = fixture();
        let path = archive(
            dir.path(),
            &[
                ("Wrapper/archive/pc/mod/HD Reworked.archive", b"new mod"),
                ("Wrapper/README.md", b"ignored"),
            ],
        );
        fs::create_dir_all(game.join("archive/pc/mod")).unwrap();
        let unmanaged = game.join("archive/pc/mod/unmanaged.archive");
        fs::write(&unmanaged, b"keep").unwrap();
        let installed = store.install_zip(&game, &path).unwrap();
        assert_eq!(installed.files.len(), 1);
        assert_eq!(
            fs::read(game.join(&installed.files[0].destination)).unwrap(),
            b"new mod"
        );
        assert_eq!(store.installed(&game).unwrap().len(), 1);
        store.uninstall(&game, &installed.id).unwrap();
        assert!(store.installed(&game).unwrap().is_empty());
        assert!(!game.join(&installed.files[0].destination).exists());
        assert_eq!(fs::read(unmanaged).unwrap(), b"keep");
    }

    #[test]
    fn existing_file_blocks_install_without_changing_it() {
        let (dir, mut store, game) = fixture();
        let path = archive(dir.path(), &[("archive/pc/mod/x.archive", b"replacement")]);
        fs::create_dir_all(game.join("archive/pc/mod")).unwrap();
        let dest = game.join("archive/pc/mod/x.archive");
        fs::write(&dest, b"unmanaged").unwrap();
        assert!(store.install_zip(&game, &path).is_err());
        assert_eq!(fs::read(dest).unwrap(), b"unmanaged");
        assert!(store.installed(&game).unwrap().is_empty());
    }

    #[test]
    fn modified_managed_content_cannot_be_uninstalled() {
        let (dir, mut store, game) = fixture();
        let path = archive(dir.path(), &[("archive/pc/mod/x.archive", b"original")]);
        let package = store.install_zip(&game, &path).unwrap();
        let dest = game.join(&package.files[0].destination);
        fs::write(&dest, b"user edits").unwrap();
        assert!(store.uninstall(&game, &package.id).is_err());
        assert_eq!(fs::read(dest).unwrap(), b"user edits");
        assert_eq!(store.installed(&game).unwrap().len(), 1);
    }

    #[test]
    fn injected_partial_install_rolls_back_all_owned_files() {
        let (dir, mut store, game) = fixture();
        let path = archive(
            dir.path(),
            &[
                ("archive/pc/mod/one.archive", b"first"),
                ("archive/pc/mod/two.archive", b"second"),
            ],
        );
        assert!(store.install_inner(&game, &path, Some(1)).is_err());
        assert!(!game.join("archive/pc/mod/one.archive").exists());
        assert!(!game.join("archive/pc/mod/two.archive").exists());
        assert!(store.installed(&game).unwrap().is_empty());
        assert_eq!(store.recover().unwrap(), 0);
        assert!(store.install_zip(&game, &path).is_ok());
    }

    #[test]
    fn recovering_crashed_install_removes_only_proven_hardlinks() {
        let (_dir, mut store, game) = fixture();
        let f = prepared_file("archive/pc/mod/crashed.archive", b"owned");
        let id = "txn-test-crash";
        store
            .insert_journal(
                id,
                "install",
                &game,
                id,
                Some("archive"),
                std::slice::from_ref(&f),
            )
            .unwrap();
        let dest = target_path(&game, &f.destination, true).unwrap();
        let temp = temp_path(&dest, id, 0).unwrap();
        fs::write(&temp, b"owned").unwrap();
        fs::hard_link(&temp, &dest).unwrap();
        assert_eq!(store.recover().unwrap(), 1);
        assert!(!dest.exists());
        assert!(!temp.exists());
        assert_eq!(store.recover().unwrap(), 0);
    }

    #[test]
    fn recovering_crashed_uninstall_restores_original_file() {
        let (_dir, mut store, game) = fixture();
        let f = prepared_file("archive/pc/mod/crashed.archive", b"owned");
        let id = "txn-test-uninstall";
        let dest = target_path(&game, &f.destination, true).unwrap();
        fs::write(&dest, b"owned").unwrap();
        store
            .insert_journal(id, "uninstall", &game, id, None, std::slice::from_ref(&f))
            .unwrap();
        let temp = temp_path(&dest, id, 0).unwrap();
        fs::hard_link(&dest, &temp).unwrap();
        fs::remove_file(&dest).unwrap();
        assert_eq!(store.recover().unwrap(), 1);
        assert_eq!(fs::read(&dest).unwrap(), b"owned");
        assert!(!temp.exists());
    }

    #[test]
    fn changed_witness_blocks_recovery_and_preserves_files() {
        let (_dir, mut store, game) = fixture();
        let f = prepared_file("archive/pc/mod/crashed.archive", b"owned");
        let id = "txn-test-tamper";
        store
            .insert_journal(id, "install", &game, id, None, std::slice::from_ref(&f))
            .unwrap();
        let dest = target_path(&game, &f.destination, true).unwrap();
        let temp = temp_path(&dest, id, 0).unwrap();
        fs::write(&temp, b"owned").unwrap();
        fs::hard_link(&temp, &dest).unwrap();
        fs::write(&dest, b"changed").unwrap();
        assert!(store.recover().is_err());
        assert_eq!(fs::read(&dest).unwrap(), b"changed");
        assert!(store
            .install_zip(
                &game,
                &archive(_dir.path(), &[("archive/pc/mod/other.archive", b"x")])
            )
            .is_err());
    }

    #[test]
    fn recovering_crashed_extraction_removes_staging_without_game_writes() {
        let (_dir, mut store, game) = fixture();
        let id = "txn-crashed-extraction";
        let stage = store.state_root.join(id);
        fs::create_dir_all(&stage).unwrap();
        store.begin_staging(id, &game, "sha").unwrap();
        fs::write(stage.join("0.bin"), vec![17u8; 4096]).unwrap();
        assert_eq!(store.recover().unwrap(), 1);
        assert!(!stage.exists());
        assert!(store.pending().unwrap().is_empty());
        assert!(!game.join("archive").exists());
    }

    #[cfg(windows)]
    #[test]
    fn locked_staging_cleanup_keeps_journal_for_retry() {
        use std::os::windows::fs::OpenOptionsExt;
        let (_dir, mut store, game) = fixture();
        let f = prepared_file("archive/pc/mod/never-created.archive", b"not installed");
        let id = "txn-locked-staging";
        store
            .insert_journal(id, "install", &game, id, None, std::slice::from_ref(&f))
            .unwrap();
        let stage = store.state_root.join(id);
        fs::create_dir_all(&stage).unwrap();
        let file = stage.join("locked.bin");
        fs::write(&file, b"staged").unwrap();
        let handle = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&file)
            .unwrap();
        assert!(store.recover().is_err());
        assert_eq!(store.pending().unwrap().len(), 1);
        drop(handle);
        assert_eq!(store.recover().unwrap(), 1);
        assert!(!stage.exists());
        assert!(store.pending().unwrap().is_empty());
    }

    #[test]
    fn recovery_never_deletes_preexisting_empty_directories() {
        let (_dir, mut store, game) = fixture();
        let directory = game.join("archive/pc/mod");
        fs::create_dir_all(&directory).unwrap();
        let file = prepared_file("archive/pc/mod/not-written.archive", b"never written");
        store
            .insert_journal(
                "txn-empty-dir",
                "install",
                &game,
                "txn-empty-dir",
                None,
                std::slice::from_ref(&file),
            )
            .unwrap();
        assert_eq!(store.recover().unwrap(), 1);
        assert!(directory.is_dir());
    }

    #[test]
    fn rejects_non_zip_or_mixed_framework_package() {
        let (dir, mut store, game) = fixture();
        let path = archive(
            dir.path(),
            &[
                ("archive/pc/mod/x.archive", b"new"),
                ("r6/scripts/script.reds", b"script"),
            ],
        );
        assert!(store.install_zip(&game, &path).is_err());
        assert!(!game.join("archive").exists());
        let other = dir.path().join("fake.7z");
        fs::write(&other, b"wrong format").unwrap();
        assert!(store.install_zip(&game, &other).is_err());
    }
}
