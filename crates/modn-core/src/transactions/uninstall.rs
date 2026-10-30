use super::directory::{covers, quarantine, quarantine_path};
use super::journal::{digest, lock_store, target_path, temp_path, JournalFile};
use super::validation::validate_transaction_root;
use crate::state::Store;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;
use tempfile::Builder;

impl Store {
    /// Uninstall only unchanged manager-owned files, keeping reversible hard-link backups until commit.
    pub fn uninstall(&mut self, game: &Path, package_id: &str) -> Result<()> {
        let _guard = lock_store(self)?;
        self.ensure_no_pending()?;
        let game = validate_transaction_root(game)?;
        let package = self
            .installed(&game)?
            .into_iter()
            .find(|p| p.id == package_id)
            .with_context(|| format!("managed package {package_id} not found for selected game"))?;
        let files: Vec<_> = package
            .files
            .iter()
            .filter(|f| !covers(&package.directories, &f.destination))
            .enumerate()
            .map(|(ordinal, f)| JournalFile {
                ordinal,
                destination: f.destination.clone(),
                sha256: f.sha256.clone(),
                bytes: f.bytes,
            })
            .collect();
        for f in &files {
            if package.origin == "manual" {
                let (_, bytes, sha) = crate::adoption::checked_file(&game, &f.destination)?;
                if bytes != f.bytes || sha != f.sha256 {
                    bail!(
                        "adopted file changed; refusing uninstall: {}",
                        f.destination
                    );
                }
            }
            let target = target_path(&game, &f.destination, /* create_parents= */ false)?;
            if !target.is_file() || digest(&target)? != f.sha256 {
                bail!(
                    "owned file missing or modified; refusing uninstall: {}",
                    target.display()
                );
            }
        }
        // Refuse absent, linked or replaced directories before recording a
        // persistent transaction; recovery must never need to guess ownership.
        for (index, root) in package.directories.iter().enumerate() {
            let path = target_path(&game, root, false)?;
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || fs::symlink_metadata(quarantine_path(&game, root, "preflight", index)?).is_ok()
            {
                bail!("unsafe owned directory: {}", path.display());
            }
            let identity = self.managed_directory_identity(package_id, root)?;
            super::directory::verify_directory(&path, &identity)?;
        }
        let staging = Builder::new().prefix("txn-").tempdir_in(&self.state_root)?;
        let id = staging
            .path()
            .file_name()
            .context("missing transaction id")?
            .to_string_lossy()
            .to_string();
        let _ = staging.keep();
        self.insert_journal(
            &id,
            "uninstall",
            &game,
            package_id,
            None,
            &files,
            &package.directories,
        )?;
        let attempt = (|| -> Result<()> {
            for (index, root) in package.directories.iter().enumerate() {
                let identity = self.managed_directory_identity(package_id, root)?;
                quarantine(&game, root, &id, index, &identity)?;
            }
            for f in &files {
                if package.origin == "manual" {
                    let (_, bytes, sha) = crate::adoption::checked_file(&game, &f.destination)?;
                    if bytes != f.bytes || sha != f.sha256 {
                        bail!("adopted file changed during uninstall: {}", f.destination);
                    }
                }
                let target = target_path(&game, &f.destination, /* create_parents= */ false)?;
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
                    "{err:#}; uninstall recovery incomplete: {recovery_error:#}; run 'modn recover'"
                );
            }
            return Err(err);
        }
        self.recover_locked()
            .context("uninstall committed; cleanup incomplete, run 'modn recover'")?;
        Ok(())
    }
}
