use super::cleanup::RemovalMode;
use super::directory::{covers, quarantine, quarantine_path};
use super::install::Fault;
use super::journal::{digest, lock_store, target_path, temp_path, JournalFile};
use crate::state::Store;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;
use tempfile::Builder;

impl Store {
    /// Uninstall only unchanged manager-owned files, keeping reversible hard-link backups until commit.
    pub fn uninstall(&mut self, game: &Path, package_id: &str) -> Result<()> {
        let _guard = lock_store(self)?;
        self.ensure_idle()?;
        self.uninstall_locked(game, package_id)
    }

    pub(super) fn uninstall_locked(&mut self, game: &Path, package_id: &str) -> Result<()> {
        self.uninstall_with_mode_locked(game, package_id, RemovalMode::Strict, None)
    }

    pub(super) fn uninstall_with_mode_locked(
        &mut self,
        game: &Path,
        package_id: &str,
        mode: RemovalMode,
        fault: Option<Fault>,
    ) -> Result<()> {
        let game = self.transaction_root(game)?;
        let id =
            self.withdraw_with_mode_locked(&game, package_id, Dependents::Refuse, mode, fault)?;
        if let Err(err) = self.commit_uninstall(&id, package_id) {
            if let Err(recovery_error) = self.recover_locked() {
                bail!(
                    "{err:#}; uninstall recovery incomplete: {recovery_error:#}; run 'modn recover'"
                );
            }
            return Err(err);
        }
        self.recover_locked()
            .context("uninstall committed; cleanup incomplete, run 'modn recover'")?;
        self.prune_created(&game)
            .context("uninstall committed; folder cleanup incomplete, run 'modn recover'")?;
        Ok(())
    }

    pub(super) fn withdraw_locked(
        &mut self,
        game: &Path,
        package_id: &str,
        dependents: Dependents,
    ) -> Result<String> {
        self.withdraw_with_mode_locked(game, package_id, dependents, RemovalMode::Strict, None)
    }

    fn withdraw_with_mode_locked(
        &mut self,
        game: &Path,
        package_id: &str,
        dependents: Dependents,
        mode: RemovalMode,
        fault: Option<Fault>,
    ) -> Result<String> {
        self.ensure_no_pending()?;
        let package = self
            .installed(game)?
            .into_iter()
            .find(|p| p.id == package_id)
            .with_context(|| format!("managed package {package_id} not found for selected game"))?;
        let needed_by = super::catalog::dependents(self, &package.id)?;
        if matches!(dependents, Dependents::Refuse) && !needed_by.is_empty() {
            bail!(
                "managed package {package_id} is required by {}; uninstall dependents first",
                needed_by.join(", ")
            );
        }
        let forced = matches!(mode, RemovalMode::ModifiedOwned);
        let mut files: Vec<_> = package
            .files
            .iter()
            .filter(|f| forced || !covers(&package.directories, &f.destination))
            .enumerate()
            .map(|(ordinal, f)| JournalFile {
                ordinal,
                destination: f.destination.clone(),
                sha256: f.sha256.clone(),
                bytes: f.bytes,
            })
            .collect();
        for f in &mut files {
            let target = target_path(game, &f.destination)?;
            if forced {
                let metadata = fs::symlink_metadata(&target)?;
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    bail!("unsafe owned target: {}", target.display());
                }
                f.sha256 = digest(&target)?;
                f.bytes = metadata.len();
            }
            if !target.is_file() || digest(&target)? != f.sha256 {
                bail!(
                    "owned file missing or modified; refusing uninstall: {}",
                    target.display()
                );
            }
        }
        for (index, root) in package.directories.iter().enumerate() {
            let path = target_path(game, root)?;
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || fs::symlink_metadata(quarantine_path(game, root, "preflight", index)?).is_ok()
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
            if forced { "cleanup" } else { "uninstall" },
            game,
            package_id,
            None,
            &files,
            &package.directories,
        )?;
        let attempt = (|| -> Result<()> {
            if !forced {
                for (index, root) in package.directories.iter().enumerate() {
                    let identity = self.managed_directory_identity(package_id, root)?;
                    quarantine(game, root, &id, index, &identity)?;
                }
            }
            for f in &files {
                if forced {
                    for root in &package.directories {
                        let identity = self.managed_directory_identity(package_id, root)?;
                        super::directory::verify_directory(&target_path(game, root)?, &identity)?;
                    }
                }
                let target = target_path(game, &f.destination)?;
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
                if fault.is_some_and(|fault| fault.after_files() == f.ordinal + 1) {
                    bail!("injected cleanup stop");
                }
            }
            Ok(())
        })();
        if let Err(err) = attempt {
            if matches!(fault, Some(Fault::CrashAfter(_))) {
                return Err(err);
            }
            if let Err(recovery_error) = self.recover_locked() {
                bail!(
                    "{err:#}; uninstall recovery incomplete: {recovery_error:#}; run 'modn recover'"
                );
            }
            return Err(err);
        }
        Ok(id)
    }
}

#[derive(Clone, Copy)]
pub(super) enum Dependents {
    Refuse,
    Repoint,
}
