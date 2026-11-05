use super::journal::{digest, lock_store, target_path, JournalFile, ManagedPackage};
use super::validation::{
    validate_ownership, validate_plan, validate_transaction_root, ExistingTargets,
};
use crate::archive::inspect;
use crate::planning::preview_adoption;
use crate::state::Store;
use anyhow::{bail, Context, Result};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;
use tempfile::Builder;

/// Independent authorization to record ownership of existing, matching files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AdoptionApproval {
    /// Do not record any ownership.
    Denied,
    /// The caller explicitly acknowledged adoption of the reviewed archive.
    Acknowledged,
}

impl Store {
    /// Adopt matching manual files without writing game bytes.
    ///
    /// The caller must supply the archive digest from its reviewed preview.
    /// The current game module selects files again under the manager lock.
    /// Every target must match its selected archive member by size and SHA-256.
    /// The atomic ownership commit records archive provenance and individual files.
    /// This API never claims a directory or unknown descendants.
    pub fn adopt_archive(
        &mut self,
        game_id: &str,
        game: &Path,
        archive: &Path,
        reviewed_sha256: &str,
        approval: AdoptionApproval,
    ) -> Result<ManagedPackage> {
        if approval != AdoptionApproval::Acknowledged {
            bail!("manual adoption requires explicit acknowledgement");
        }
        let _guard = lock_store(self)?;
        self.ensure_idle()?;
        let game = validate_transaction_root(game)?;
        if self.state_root.starts_with(&game) || game.starts_with(&self.state_root) {
            bail!("manager state must be separate from the game directory");
        }
        let inspection = inspect(archive)?;
        if inspection.sha256 != reviewed_sha256 {
            bail!("archive changed since adoption preview");
        }
        let proposal = preview_adoption(game_id, &game, archive)?;
        validate_plan(&inspection, &game, &proposal, &[], ExistingTargets::Adopt)?;
        validate_ownership(self, &game, &proposal, None)?;
        let conflicts = self.conflicting_packages(&game, None, &inspection.sha256)?;
        if !conflicts.is_empty() {
            bail!("adoption conflicts with installed {}", conflicts.join(", "));
        }
        let staging = Builder::new()
            .prefix("adopt-")
            .tempdir_in(&self.state_root)?;
        let files = if inspection.format == crate::archive::ArchiveFormat::SevenZ {
            super::staging::stage_7z(archive, &inspection, &proposal, staging.path())?
        } else {
            let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
            let mut files = Vec::new();
            for (ordinal, operation) in proposal.operations.iter().enumerate() {
                let member = inspection
                    .files
                    .iter()
                    .find(|entry| entry.entry == operation.source)
                    .context("selected archive member missing")?;
                let mut input =
                    zip.by_name(member.original_entry.as_deref().unwrap_or(&member.entry))?;
                if input.is_dir() || input.size() != operation.bytes {
                    bail!("archive member changed during adoption");
                }
                let path = staging.path().join(format!("{ordinal}.bin"));
                let mut output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)?;
                let bytes = io::copy(&mut input.by_ref().take(operation.bytes + 1), &mut output)?;
                output.flush()?;
                if bytes != operation.bytes {
                    bail!("archive extraction size mismatch");
                }
                files.push(JournalFile {
                    ordinal,
                    destination: operation.destination.clone(),
                    sha256: digest(&path)?,
                    bytes,
                });
            }
            files
        };
        super::content::validate_staged_content(&proposal, staging.path())?;
        if inspect(archive)?.sha256 != reviewed_sha256 {
            bail!("archive changed during adoption verification");
        }
        for file in &files {
            let target = target_path(&game, &file.destination)?;
            let metadata = fs::symlink_metadata(&target)?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() != file.bytes
                || digest(&target)? != file.sha256
            {
                bail!(
                    "manual file does not match selected archive: {}",
                    target.display()
                );
            }
        }
        let id = staging
            .path()
            .file_name()
            .context("missing adoption id")?
            .to_string_lossy()
            .to_string();
        self.commit_install(&id, &game, reviewed_sha256, &files, &[], None)?;
        self.installed(&game)?
            .into_iter()
            .find(|package| package.id == id)
            .context("adopted package missing after commit")
    }
}
