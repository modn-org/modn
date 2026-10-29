use super::journal::{
    digest, lock_store, target_path, temp_path, JournalFile, ManagedPackage, OwnedFile,
};
use crate::archive::inspect;
use crate::discovery::validate_installation;
use crate::planning::{plan, resolve, InstallPlan, PlanIssue};
use crate::state::Store;
use crate::CYBERPUNK_ID;
use anyhow::{bail, Context, Result};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;
use tempfile::Builder;

const MAX_TOTAL: u64 = 8 * 1024 * 1024 * 1024;
const MAX_FILE: u64 = 4 * 1024 * 1024 * 1024;

/// Preview only operations the current legacy ZIP transaction engine can execute.
///
/// The Luau mapping is untrusted. Until the installer consumes the same
/// validated mapping in a later milestone, reject previews that disagree with
/// the current Rust executor instead of showing a misleading dry run.
pub fn preview_install_zip(game: &Path, archive: &Path) -> Result<InstallPlan> {
    let inspection = inspect(archive)?;
    let mut preview = resolve(CYBERPUNK_ID, &inspection, game)?;
    let actual = plan(&inspection, &preview.game_installation);
    let agrees = actual.operations.len() == preview.operations.len()
        && actual
            .operations
            .iter()
            .zip(&preview.operations)
            .all(|(a, b)| {
                a.source == b.source
                    && a.destination == b.destination
                    && a.bytes == b.bytes
                    && a.kind == b.kind
            });
    if inspection.format != "zip"
        || actual.status != "reviewable"
        || actual
            .operations
            .iter()
            .any(|file| file.kind != "legacy-archive")
        || !agrees
    {
        preview.blockers.push(PlanIssue {
            code: "NOT_INSTALLABLE".into(),
            message: "This archive or proposed mapping is not supported by the current installer"
                .into(),
        });
        preview.status = "blocked".into();
    }
    Ok(preview)
}

impl Store {
    /// Install a ZIP of legacy archives; never adopts or replaces existing files.
    pub fn install_zip(&mut self, game: &Path, archive: &Path) -> Result<ManagedPackage> {
        self.install_inner(game, archive, None)
    }

    pub(super) fn install_inner(
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
                    bail!("{err:#}; staging recovery incomplete: {recovery_error:#}; run 'modn recover'");
                }
                return Err(err);
            }
        };
        if let Err(err) = self.stage_ready(&id, &files) {
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
            self.commit_install(&id, &game, &inspection.sha256, &files)
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
}
