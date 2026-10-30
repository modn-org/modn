//! Game-agnostic, ownership-aware ZIP operation executor.
use super::directory::exclusive_roots;
use super::journal::{
    digest, lock_store, target_path, temp_path, JournalFile, ManagedPackage, OwnedFile,
};
use super::validation::{
    validate_directory_reservations, validate_execution_plan, validate_ownership,
    validate_transaction_root,
};
use crate::archive::inspect;
use crate::planning::InstallPlan;
use crate::state::Store;
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;
use tempfile::Builder;

impl Store {
    /// Execute a caller-approved proposal, independently checked against the current archive
    /// and filesystem. This is a write API, not exposed to untrusted Luau modules.
    /// For now, only ZIP file-copy operations are supported.
    pub fn install_operations_zip(
        &mut self,
        game: &Path,
        archive: &Path,
        proposal: &InstallPlan,
    ) -> Result<ManagedPackage> {
        self.install_operations_inner(game, archive, proposal, None)
    }

    pub(super) fn install_operations_inner(
        &mut self,
        game: &Path,
        archive: &Path,
        proposal: &InstallPlan,
        fail_after: Option<usize>,
    ) -> Result<ManagedPackage> {
        let _guard = lock_store(self)?;
        self.ensure_no_pending()?;
        let game = validate_transaction_root(game)?;
        if self.state_root.starts_with(&game) || game.starts_with(&self.state_root) {
            bail!("manager state must be separate from the game directory");
        }
        let inspection = inspect(archive)?;
        validate_execution_plan(&inspection, &game, proposal)?;
        validate_ownership(self, &game, proposal)?;
        let directories = exclusive_roots(&game, proposal)?;
        validate_directory_reservations(self, &game, &directories)?;
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
            let original_names: HashMap<_, _> = inspection
                .files
                .iter()
                .map(|file| {
                    (
                        file.entry.as_str(),
                        file.original_entry.as_deref().unwrap_or(&file.entry),
                    )
                })
                .collect();
            let mut files = Vec::new();
            for (n, op) in proposal.operations.iter().enumerate() {
                let archive_name = original_names
                    .get(op.source.as_str())
                    .context("source is not present in inspected ZIP")?;
                let mut input = zip.by_name(archive_name)?;
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
                    bail!("{err:#}; staging recovery incomplete: {recovery_error:#}; run 'modn recover'");
                }
                return Err(err);
            }
        };
        if let Err(err) = self.stage_ready(&id, &files, &directories) {
            drop(staging);
            if let Err(recovery_error) = self.recover_locked() {
                bail!(
                    "{err:#}; staging recovery incomplete: {recovery_error:#}; run 'modn recover'"
                );
            }
            return Err(err);
        }
        let stage_path = staging.keep();
        let attempt = (|| -> Result<()> {
            // Atomically create each exclusive ownership boundary; a concurrent
            // creator must never be silently adopted by this transaction.
            for (ordinal, root) in directories.iter().enumerate() {
                let identity = super::directory::prepare_install(&game, root, &id, ordinal)?;
                self.record_directory_identity(&id, root, &identity)?;
                super::directory::activate_install(&game, root, &id, ordinal, &identity)?;
            }
            for f in &files {
                let dest = target_path(&game, &f.destination, /* create_parents= */ true)?;
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
            self.commit_install(&id, &game, &inspection.sha256, &files, &directories)
        })();
        if let Err(err) = attempt {
            let recover = self.recover_locked();
            if let Err(recovery_error) = recover {
                bail!(
                    "{err:#}; automatic rollback incomplete: {recovery_error:#}; run 'modn recover'"
                );
            }
            return Err(err);
        }
        self.recover_locked()?;
        Ok(ManagedPackage {
            id,
            game_installation: game,
            archive_sha256: Some(inspection.sha256),
            origin: "archive".into(),
            directories,
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
}
