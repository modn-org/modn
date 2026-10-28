use super::journal::{digest, lock_store, target_path, temp_path, JournalFile};
use crate::discovery::validate_installation;
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
            let target = target_path(&game, &f.destination, /* create_parents= */ false)?;
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
