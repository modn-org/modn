use super::catalog::{CatalogInstall, CatalogRecord, CatalogRelease};
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

/// A test-injected stop after the given number of linked files.
#[derive(Clone, Copy)]
pub(super) enum Fault {
    /// Fail, then roll back automatically.
    FailAfter(usize),
    /// Stop as if the process died: no automatic rollback.
    #[cfg_attr(not(test), expect(dead_code, reason = "only tests simulate a crash"))]
    CrashAfter(usize),
}

impl Fault {
    fn after_files(self) -> usize {
        match self {
            Self::FailAfter(files) | Self::CrashAfter(files) => files,
        }
    }
}

impl Store {
    /// Execute a caller-approved proposal, independently checked against the current archive
    /// and filesystem. This is a write API, not exposed to untrusted Lua modules.
    /// ZIP and 7z file-copy operations share the same journal and recovery.
    pub fn install_operations_zip(
        &mut self,
        game: &Path,
        archive: &Path,
        proposal: &InstallPlan,
    ) -> Result<ManagedPackage> {
        self.install_operations_inner(game, archive, proposal, None)
    }

    /// Install with independent caller-authorized tool executable destinations.
    /// Approval is never read from the module's untrusted operation labels.
    pub fn install_operations_zip_with_permissions(
        &mut self,
        game: &Path,
        archive: &Path,
        proposal: &InstallPlan,
        approved: &[String],
    ) -> Result<ManagedPackage> {
        self.install_inner_with_permissions(game, archive, proposal, None, approved, None)
    }

    /// Install a catalog release, recording its provenance and dependency links.
    ///
    /// Dependency readiness is checked while the transaction lock is held, so a
    /// concurrent uninstall cannot remove a prerequisite between the check and
    /// the ownership commit. Recorded dependencies then block their uninstall.
    pub fn install_catalog_release(
        &mut self,
        game: &Path,
        archive: &Path,
        proposal: &InstallPlan,
        release: &CatalogRelease,
    ) -> Result<ManagedPackage> {
        self.install_catalog_release_with_permissions(game, archive, proposal, release, &[])
    }

    /// Install a catalog release with caller-authorized tool executable destinations.
    ///
    /// Approval comes from the user's invocation, never from the bucket
    /// manifest or the module's operation labels; the archive is revalidated
    /// against the same approvals while the transaction lock is held.
    pub fn install_catalog_release_with_permissions(
        &mut self,
        game: &Path,
        archive: &Path,
        proposal: &InstallPlan,
        release: &CatalogRelease,
        approved: &[String],
    ) -> Result<ManagedPackage> {
        self.install_inner_with_permissions(game, archive, proposal, None, approved, Some(release))
    }

    pub(super) fn install_operations_inner(
        &mut self,
        game: &Path,
        archive: &Path,
        proposal: &InstallPlan,
        fail_after: Option<usize>,
    ) -> Result<ManagedPackage> {
        let fault = fail_after.map(Fault::FailAfter);
        self.install_inner_with_permissions(game, archive, proposal, fault, &[], None)
    }

    fn install_inner_with_permissions(
        &mut self,
        game: &Path,
        archive: &Path,
        proposal: &InstallPlan,
        fault: Option<Fault>,
        approved: &[String],
        release: Option<&CatalogRelease>,
    ) -> Result<ManagedPackage> {
        let _guard = lock_store(self)?;
        self.ensure_idle()?;
        let catalog = release.map(|release| CatalogInstall {
            release,
            stack_id: None,
            replacing: None,
        });
        self.install_locked(game, archive, proposal, fault, approved, catalog)
    }

    pub(super) fn install_locked(
        &mut self,
        game: &Path,
        archive: &Path,
        proposal: &InstallPlan,
        fault: Option<Fault>,
        approved: &[String],
        catalog: Option<CatalogInstall<'_>>,
    ) -> Result<ManagedPackage> {
        let replacing = catalog.and_then(|catalog| catalog.replacing);
        let replaced_journal = replacing.map(|replaced| replaced.journal_id);
        let replaced_package = replacing.map(|replaced| replaced.package_id);
        self.ensure_pending_only(replaced_journal)?;
        let game = validate_transaction_root(game)?;
        if self.state_root.starts_with(&game) || game.starts_with(&self.state_root) {
            bail!("manager state must be separate from the game directory");
        }
        let inspection = inspect(archive)?;
        validate_execution_plan(&inspection, &game, proposal, approved)?;
        validate_ownership(self, &game, proposal, replaced_package)?;
        let directories = exclusive_roots(&game, proposal)?;
        validate_directory_reservations(self, &game, &directories, replaced_package)?;
        let release = catalog.map(|catalog| catalog.release);
        let mut conflicts = self.conflicting_packages(&game, release, &inspection.sha256)?;
        if let Some(replaced) = replaced_package {
            conflicts.retain(|label| label.split(' ').next() != Some(replaced));
        }
        if !conflicts.is_empty() {
            let subject = release.map_or_else(
                || format!("archive {}", inspection.sha256),
                |release| release.reference.to_string(),
            );
            bail!(
                "{subject} conflicts with installed {}; uninstall it first",
                conflicts.join(", ")
            );
        }
        let catalog = catalog
            .map(|catalog| -> Result<CatalogRecord> {
                let dependency_ids =
                    self.ready_dependencies(&game, &catalog.release.dependencies)?;
                Ok(CatalogRecord {
                    release: catalog.release,
                    dependency_ids,
                    stack_id: catalog.stack_id,
                    replacing: catalog.replacing,
                })
            })
            .transpose()?;
        let staging = Builder::new().prefix("txn-").tempdir_in(&self.state_root)?;
        let id = staging
            .path()
            .file_name()
            .context("missing transaction id")?
            .to_string_lossy()
            .to_string();
        self.begin_staging(&id, &game, &inspection.sha256, replaced_journal)?;
        let staged = (|| -> Result<Vec<JournalFile>> {
            if inspection.format == crate::archive::ArchiveFormat::SevenZ {
                let files =
                    super::staging::stage_7z(archive, &inspection, proposal, staging.path())?;
                if inspect(archive)?.sha256 != inspection.sha256 {
                    bail!("archive changed between inspection and extraction");
                }
                return Ok(files);
            }
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
            for (ordinal, root) in directories.iter().enumerate() {
                let identity = super::directory::prepare_install(&game, root, &id, ordinal)?;
                self.record_directory_identity(&id, root, &identity)?;
                super::directory::activate_install(&game, root, &id, ordinal, &identity)?;
            }
            for f in &files {
                self.create_parents(&id, &game, &f.destination)?;
                let dest = target_path(&game, &f.destination)?;
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
                if fault.is_some_and(|fault| fault.after_files() == f.ordinal + 1) {
                    bail!("injected interruption after {} files", f.ordinal + 1);
                }
            }
            self.commit_install(
                &id,
                &game,
                &inspection.sha256,
                &files,
                &directories,
                catalog.as_ref(),
            )
        })();
        if let Err(err) = attempt {
            if matches!(fault, Some(Fault::CrashAfter(_))) {
                return Err(err);
            }
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
            archive_sha256: inspection.sha256,
            directories,
            catalog: catalog.as_ref().map(|c| c.release.reference.clone()),
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
