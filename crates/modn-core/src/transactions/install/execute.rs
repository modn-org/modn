//! Execution of a locked install: staging, file placement and journal steps.
use super::super::catalog::{CatalogInstall, CatalogRecord};
use super::super::directory::exclusive_roots;
use super::super::journal::{
    digest, temp_path, JournalDirectory, JournalFile, ManagedPackage, OwnedFile, Roots,
};
use super::super::native_injector::NativeInjectorPolicy;
use super::super::validation::{
    validate_directory_reservations, validate_execution_plan, validate_ownership,
};
use super::Fault;
use crate::archive::inspect;
use crate::error::{coded, ErrorCode};
use crate::planning::InstallPlan;
use crate::progress::ProgressEvent;
use crate::state::Store;
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;
use tempfile::Builder;

impl Store {
    #[expect(
        clippy::too_many_arguments,
        reason = "the locked install path keeps the catalog policy explicit beside the proposal"
    )]
    pub(in crate::transactions) fn install_locked(
        &mut self,
        game: &Path,
        archive: &Path,
        proposal: &InstallPlan,
        fault: Option<Fault>,
        approved: &[String],
        catalog: Option<CatalogInstall<'_>>,
        policy: NativeInjectorPolicy,
        stack_id: Option<&str>,
    ) -> Result<ManagedPackage> {
        if let Some(release) = catalog.map(|catalog| catalog.release) {
            release.ensure_native_injector_allowed(policy)?;
        }
        let replacing = catalog.and_then(|catalog| catalog.replacing);
        let replaced_journal = replacing.map(|replaced| replaced.journal_id);
        let replaced_package = replacing.map(|replaced| replaced.package_id);
        self.ensure_pending_only(replaced_journal)?;
        let game = self.transaction_root(game)?;
        if self.state_root.starts_with(&game) || game.starts_with(&self.state_root) {
            bail!("manager state must be separate from the game directory");
        }
        let content = match (&proposal.content_base, &proposal.content_root) {
            (Some(base), Some(root)) => Some(crate::content_root::validate(&game, base, root)?),
            _ => None,
        };
        let roots = Roots {
            game: game.clone(),
            content,
        };
        if let Some(content) = &roots.content {
            crate::state::check_root(
                &self.connection,
                &roots.game,
                crate::state::RootKind::Content,
                content,
            )?;
        }
        if self.state_root.starts_with(roots.game.as_path())
            || roots.game.starts_with(&self.state_root)
            || roots.content.as_ref().is_some_and(|content| {
                self.state_root.starts_with(content) || content.starts_with(&self.state_root)
            })
        {
            bail!("manager state must be separate from every installation root");
        }
        let inspection = inspect(archive)?;
        let archive_blake3 = self
            .history
            .is_some()
            .then(|| format!("blake3:{}", inspection.blake3));
        let overrides = validate_ownership(self, &game, proposal, replaced_package)?;
        validate_execution_plan(&inspection, &roots, proposal, approved, &overrides)?;
        let directories = exclusive_roots(&game, proposal)?;
        validate_directory_reservations(self, &game, &directories, replaced_package)?;
        let release = catalog.map(|catalog| catalog.release);
        if replacing.is_none() {
            if let Some(release) = &release {
                if self.installed_from(&game, &release.reference)? {
                    return Err(coded(
                        ErrorCode::AlreadyInstalled,
                        format!(
                            "{} is already installed; uninstall it before installing it again",
                            release.reference
                        ),
                    ));
                }
            }
        }
        let mut conflicts = self.conflicting_packages(&game, release, &inspection.blake3)?;
        if let Some(replaced) = replaced_package {
            conflicts.retain(|label| label.split(' ').next() != Some(replaced));
        }
        if !conflicts.is_empty() {
            let subject = release.map_or_else(
                || format!("archive {}", inspection.blake3),
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
                    replacing: catalog.replacing,
                })
            })
            .transpose()?;
        let package = release.map(|release| release.reference.to_string());
        let control = self.control.clone();
        control.advance(ProgressEvent::Extract {
            package: package.clone(),
        })?;
        let staging = Builder::new().prefix("txn-").tempdir_in(&self.state_root)?;
        let id = staging
            .path()
            .file_name()
            .context("missing transaction id")?
            .to_string_lossy()
            .to_string();
        self.begin_staging(&id, &game, &inspection.blake3, replaced_journal)?;
        self.ensure_content_root(&id, &roots, proposal.content_base.as_deref())?;
        let staged = (|| -> Result<Vec<JournalFile>> {
            if inspection.format == crate::archive::ArchiveFormat::SevenZ {
                let files = super::super::staging::stage_7z(
                    archive,
                    &inspection,
                    proposal,
                    staging.path(),
                    &control,
                )?;
                if inspect(archive)?.blake3 != inspection.blake3 {
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
                control.checkpoint()?;
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
                    root: crate::state::RootKind::from_label(&op.root)
                        .context("plan operation has an unknown root")?,
                    destination: op.destination.clone(),
                    archive_entry: op.source.clone(),
                    blake3: digest(&stage)?,
                    bytes: copied,
                });
            }
            drop(zip);
            if inspect(archive)?.blake3 != inspection.blake3 {
                bail!("archive changed between inspection and extraction");
            }
            Ok(files)
        })();
        let staged = staged.and_then(|files| {
            control.advance(ProgressEvent::Stage {
                package: package.clone(),
            })?;
            control.checkpoint()?;
            super::super::content::validate_staged_content(proposal, staging.path())?;
            Ok(files)
        });
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
        let journal_directories: Vec<JournalDirectory> = directories
            .iter()
            .map(|path| JournalDirectory {
                root: crate::state::RootKind::Game,
                path: path.clone(),
                identity: None,
            })
            .collect();
        let ready = (|| -> Result<()> {
            control.advance(ProgressEvent::Commit {
                package: package.clone(),
            })?;
            if let Some(content) = &roots.content {
                crate::state::record_root(
                    &self.connection,
                    &roots.game,
                    crate::state::RootKind::Content,
                    content,
                )?;
            }
            self.stage_ready(&id, &files, &journal_directories)
        })();
        if let Err(err) = ready {
            drop(staging);
            if let Err(recovery_error) = self.recover_locked() {
                bail!(
                    "{err:#}; staging recovery incomplete: {recovery_error:#}; run 'modn recover'"
                );
            }
            return Err(err);
        }
        let stage_path = staging.keep();
        let by_destination: HashMap<String, &crate::transactions::claim::PlanOverride> = overrides
            .iter()
            .map(|entry| {
                (
                    format!(
                        "{}:{}",
                        entry.root,
                        super::super::ownership_key(&entry.destination)
                    ),
                    entry,
                )
            })
            .collect();
        let mut backups = Vec::new();
        let attempt = (|| -> Result<()> {
            for (ordinal, root) in directories.iter().enumerate() {
                let identity = super::super::directory::prepare_install(&game, root, &id, ordinal)?;
                self.record_directory_identity(&id, crate::state::RootKind::Game, root, &identity)?;
                super::super::directory::activate_install(&game, root, &id, ordinal, &identity)?;
            }
            for f in &files {
                self.create_parents(&id, &roots, f.root, &f.destination)?;
                let dest = roots.target(f.root, &f.destination)?;
                let key = format!(
                    "{}:{}",
                    f.root.label(),
                    super::super::ownership_key(&f.destination)
                );
                if let Some(entry) = by_destination.get(&key) {
                    let metadata = fs::symlink_metadata(&dest).with_context(|| {
                        format!(
                            "CONFLICT: owned file is missing; run 'modn doctor': {}",
                            dest.display()
                        )
                    })?;
                    if !metadata.is_file() || metadata.file_type().is_symlink() {
                        return Err(coded(
                            crate::error::ErrorCode::Conflict,
                            format!(
                                "CONFLICT: owned destination is not a regular file: {}",
                                dest.display()
                            ),
                        ));
                    }
                    if digest(&dest)? != entry.replaced_blake3 {
                        return Err(coded(
                            crate::error::ErrorCode::Conflict,
                            format!(
                                "CONFLICT: owned file changed; run 'modn doctor': {}",
                                dest.display()
                            ),
                        ));
                    }
                    let backup = super::super::journal::backup_path(&dest, &id, f.ordinal)?;
                    fs::hard_link(&dest, &backup)?;
                    if !same_file::is_same_file(&dest, &backup)?
                        || digest(&backup)? != entry.replaced_blake3
                    {
                        return Err(coded(
                            crate::error::ErrorCode::Conflict,
                            format!(
                                "CONFLICT: owned file changed while backing it up: {}",
                                dest.display()
                            ),
                        ));
                    }
                    self.connection.execute(
                        "INSERT INTO pending_overrides
                         (transaction_id,ord,root,relative_path,package_id,blake3,bytes)
                         VALUES (?1,?2,?3,?4,?5,?6,?7)",
                        rusqlite::params![
                            id,
                            f.ordinal as i64,
                            f.root.label(),
                            f.destination,
                            entry.replaced_package,
                            entry.replaced_blake3,
                            f.bytes as i64
                        ],
                    )?;
                    backups.push(backup);
                    fs::remove_file(&dest)?;
                } else if fs::symlink_metadata(&dest).is_ok() {
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
                if digest(&temp)? != f.blake3 {
                    bail!("staging digest mismatch");
                }
                self.connection.execute(
                    "INSERT INTO pending_links(transaction_id,ord) VALUES (?1,?2)",
                    rusqlite::params![id, f.ordinal as i64],
                )?;
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
                &inspection.blake3,
                &files,
                &journal_directories,
                roots.content.as_deref(),
                catalog.as_ref(),
                archive_blake3.as_deref(),
                stack_id,
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
        for backup in backups {
            match fs::remove_file(&backup) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "installed; cannot remove recovery witness {}; run 'modn recover'",
                            backup.display()
                        )
                    });
                }
            }
        }
        self.recover_locked()?;
        let metadata = self.history.as_ref().map(|context| {
            crate::history::metadata_for(
                context,
                &id,
                catalog.as_ref().map(|catalog| catalog.release),
                archive_blake3.as_deref(),
            )
        });
        Ok(ManagedPackage {
            id,
            pinned: false,
            game_installation: game,
            archive_blake3: inspection.blake3,
            directories,
            catalog: catalog.as_ref().map(|c| c.release.reference.clone()),
            mod_id: metadata.as_ref().map(|metadata| metadata.mod_id.clone()),
            version: metadata
                .as_ref()
                .and_then(|metadata| metadata.version.clone()),
            source: metadata
                .as_ref()
                .and_then(|metadata| metadata.source.clone()),
            declarations: release.map(super::super::catalog::ReleaseDeclarations::of),
            files: files
                .into_iter()
                .map(|f| OwnedFile {
                    root: f.root.label().to_owned(),
                    destination: f.destination,
                    blake3: f.blake3,
                    bytes: f.bytes,
                })
                .collect(),
        })
    }
}
