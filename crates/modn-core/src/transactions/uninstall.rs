use super::cleanup::RemovalMode;
use super::directory::{covers, quarantine, quarantine_path};
use super::install::Fault;
use super::journal::{
    digest, lock_store, restore_path, target_path, temp_path, JournalFile, Roots,
};
use crate::error::{coded, ErrorCode};
use crate::progress::ProgressEvent;
use crate::state::{resolve_root, RootKind, Store};
use anyhow::{bail, Context, Result};
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::Path;
use tempfile::Builder;

#[derive(Clone, Copy, PartialEq, Eq)]
enum PendingPolicy {
    RequireIdle,
    Allowed,
}

/// Whether a withdrawal may replace a shared winner with the next claimant.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum RestorePolicy {
    /// Refuse a winner that another package claims; undo and update use this.
    Refuse,
    /// Restore the next claimant's verified bytes; uninstall uses this.
    Replace,
}

impl Store {
    /// Uninstall only unchanged manager-owned files, keeping reversible hard-link backups until commit.
    pub fn uninstall(&mut self, game: &Path, package_id: &str) -> Result<()> {
        let _guard = lock_store(self)?;
        self.ensure_idle()?;
        self.begin_removal(game, package_id)?;
        self.uninstall_locked(game, package_id)
    }

    pub(crate) fn begin_removal(&self, game: &Path, package_id: &str) -> Result<()> {
        let catalog = self
            .installed(game)
            .ok()
            .and_then(|packages| packages.into_iter().find(|p| p.id == package_id))
            .and_then(|package| package.catalog);
        let package = catalog.map_or_else(|| package_id.to_owned(), |c| c.to_string());
        self.control.advance(ProgressEvent::Commit {
            package: Some(package),
        })?;
        Ok(())
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
        let content = if self
            .installed(&game)?
            .iter()
            .find(|package| package.id == package_id)
            .is_some_and(|package| package.files.iter().any(|file| file.root == "content"))
        {
            Some(resolve_root(&self.connection, &game, RootKind::Content)?)
        } else {
            None
        };
        let roots = Roots { game, content };
        let id = self.withdraw_with_mode_locked(
            &roots,
            package_id,
            Dependents::Refuse,
            mode,
            fault,
            &std::collections::BTreeSet::new(),
            PendingPolicy::RequireIdle,
            RestorePolicy::Replace,
        )?;
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
        self.prune_created(&roots)
            .context("uninstall committed; folder cleanup incomplete, run 'modn recover'")?;
        Ok(())
    }

    pub(super) fn withdraw_locked(
        &mut self,
        game: &Path,
        package_id: &str,
        dependents: Dependents,
    ) -> Result<String> {
        let game = self.transaction_root(game)?;
        let content = if self
            .installed(&game)?
            .iter()
            .find(|package| package.id == package_id)
            .is_some_and(|package| package.files.iter().any(|file| file.root == "content"))
        {
            Some(resolve_root(&self.connection, &game, RootKind::Content)?)
        } else {
            None
        };
        let roots = Roots { game, content };
        self.withdraw_with_mode_locked(
            &roots,
            package_id,
            dependents,
            RemovalMode::Strict,
            None,
            &std::collections::BTreeSet::new(),
            PendingPolicy::RequireIdle,
            RestorePolicy::Refuse,
        )
    }

    pub(crate) fn withdraw_member_locked(
        &mut self,
        game: &Path,
        package_id: &str,
        allowed: &std::collections::BTreeSet<String>,
    ) -> Result<String> {
        let game = self.transaction_root(game)?;
        let content = if self
            .installed(&game)?
            .iter()
            .find(|package| package.id == package_id)
            .is_some_and(|package| package.files.iter().any(|file| file.root == "content"))
        {
            Some(resolve_root(&self.connection, &game, RootKind::Content)?)
        } else {
            None
        };
        let roots = Roots { game, content };
        self.withdraw_with_mode_locked(
            &roots,
            package_id,
            Dependents::Refuse,
            RemovalMode::Strict,
            None,
            allowed,
            PendingPolicy::Allowed,
            RestorePolicy::Refuse,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the withdrawal keeps roots, dependent policy, mode and pending policy explicit"
    )]
    fn withdraw_with_mode_locked(
        &mut self,
        roots: &Roots,
        package_id: &str,
        dependents: Dependents,
        mode: RemovalMode,
        fault: Option<Fault>,
        allowed: &std::collections::BTreeSet<String>,
        pending: PendingPolicy,
        restore: RestorePolicy,
    ) -> Result<String> {
        if matches!(pending, PendingPolicy::RequireIdle) {
            self.ensure_no_pending()?;
        }
        let package = self
            .installed(&roots.game)?
            .into_iter()
            .find(|p| p.id == package_id)
            .with_context(|| format!("managed package {package_id} not found for selected game"))?;
        if matches!(restore, RestorePolicy::Refuse) {
            if let Some(path) = self.shadowing_claim_path(&package.id)? {
                return Err(coded(
                    ErrorCode::Conflict,
                    format!(
                        "CONFLICT: managed package {package_id} wins a shared file; this operation cannot restore the next claimant's bytes: {path}; uninstall it with 'modn uninstall' instead"
                    ),
                ));
            }
        }
        let needed_by = super::catalog::dependents(self, &package.id)?;
        if matches!(dependents, Dependents::Refuse)
            && needed_by
                .iter()
                .any(|label| !allowed.contains(label.split(' ').next().unwrap_or(label)))
        {
            return Err(coded(
                ErrorCode::PackageNeeded,
                format!(
                    "managed package {package_id} is required by {}; uninstall dependents first",
                    needed_by.join(", ")
                ),
            ));
        }
        let forced = matches!(mode, RemovalMode::ModifiedOwned);
        let mut files: Vec<_> = package
            .files
            .iter()
            .filter(|f| forced || !covers(&package.directories, &f.destination))
            .enumerate()
            .map(|(ordinal, f)| JournalFile {
                ordinal,
                root: RootKind::from_label(&f.root).unwrap_or(RootKind::Game),
                destination: f.destination.clone(),
                archive_entry: String::new(),
                blake3: f.blake3.clone(),
                bytes: f.bytes,
            })
            .collect();
        let excluding = BTreeSet::from([package.id.clone()]);
        for f in &mut files {
            let target = roots.target(f.root, &f.destination)?;
            if forced {
                let metadata = fs::symlink_metadata(&target)?;
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err(coded(
                        ErrorCode::Conflict,
                        format!("unsafe owned target: {}", target.display()),
                    ));
                }
                f.blake3 = digest(&target)?;
                f.bytes = metadata.len();
            }
            if !target.is_file() || digest(&target)? != f.blake3 {
                let claimed =
                    super::restore::next_claim(self, &roots.game, f, &excluding)?.is_some();
                return Err(if claimed {
                    coded(
                        ErrorCode::ProfileDrift,
                        format!(
                            "PROFILE_DRIFT: managed winner changed on disk; run 'modn doctor': {}",
                            target.display()
                        ),
                    )
                } else {
                    coded(
                        ErrorCode::Conflict,
                        format!(
                            "owned file missing or modified; refusing uninstall: {}",
                            target.display()
                        ),
                    )
                });
            }
        }
        for (index, root) in package.directories.iter().enumerate() {
            let path = target_path(&roots.game, root)?;
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || fs::symlink_metadata(quarantine_path(&roots.game, root, "preflight", index)?)
                    .is_ok()
            {
                return Err(coded(
                    ErrorCode::Conflict,
                    format!("unsafe owned directory: {}", path.display()),
                ));
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
        let restores = if matches!(restore, RestorePolicy::Replace) {
            super::restore::stage_restores(self, &roots.game, &files, staging.path(), &excluding)?
        } else {
            std::collections::BTreeMap::new()
        };
        self.insert_journal(
            &id,
            if forced { "cleanup" } else { "uninstall" },
            &roots.game,
            package_id,
            None,
            &files,
            &package.directories,
        )?;
        super::restore::record_restores(self, &id, &restores)?;
        let _ = staging.keep();
        let attempt = (|| -> Result<()> {
            if !forced {
                for (index, root) in package.directories.iter().enumerate() {
                    let identity = self.managed_directory_identity(package_id, root)?;
                    quarantine(&roots.game, root, &id, index, &identity)?;
                }
            }
            for f in &files {
                if forced {
                    for root in &package.directories {
                        let identity = self.managed_directory_identity(package_id, root)?;
                        super::directory::verify_directory(
                            &target_path(&roots.game, root)?,
                            &identity,
                        )?;
                    }
                }
                let target = roots.target(f.root, &f.destination)?;
                if !target.is_file() || digest(&target)? != f.blake3 {
                    return Err(coded(
                        ErrorCode::Conflict,
                        format!("owned file modified during uninstall: {}", target.display()),
                    ));
                }
                let temp = temp_path(&target, &id, f.ordinal)?;
                fs::hard_link(&target, &temp)?;
                if !same_file::is_same_file(&target, &temp)?
                    || digest(&temp)? != f.blake3
                    || digest(&target)? != f.blake3
                {
                    return Err(coded(
                        ErrorCode::Conflict,
                        format!(
                            "owned file changed while backing it up: {}",
                            target.display()
                        ),
                    ));
                }
                fs::remove_file(&target)?;
                if let Some(restore) = restores.get(&f.ordinal) {
                    let witness = restore_path(&target, &id, f.ordinal)?;
                    let mut input = File::open(&restore.staged)?;
                    let mut output = OpenOptions::new()
                        .create_new(true)
                        .write(true)
                        .open(&witness)?;
                    if io::copy(&mut input, &mut output)? != restore.candidate.bytes {
                        bail!("restore staging size mismatch");
                    }
                    output.sync_all()?;
                    drop(output);
                    if digest(&witness)? != restore.candidate.blake3 {
                        bail!("restore staging digest mismatch");
                    }
                    fs::hard_link(&witness, &target)?;
                    if digest(&target)? != restore.candidate.blake3 {
                        bail!("restored file digest mismatch: {}", target.display());
                    }
                }
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
