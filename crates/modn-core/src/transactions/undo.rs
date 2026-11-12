//! Reverse one committed operation from the operation history.
//!
//! The inverse of an install is an uninstall, the inverse of an uninstall is
//! an install, and the inverse of an update is the earlier release. Every
//! inverse runs through the normal transactional paths and writes one new
//! history record whose `undoes` field names the target.

use super::{lock_store, ManagedPackage, NativeInjectorPolicy};
use crate::archive::{ArchiveFormat, Inspection};
use crate::error::{coded, ErrorCode};
use crate::history::{HistoryFile, HistoryKind, HistoryOperation, HistoryRelease};
use crate::planning::InstallPlan;
use crate::state::Store;
use anyhow::{bail, Context, Result};
use rusqlite::Transaction;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// What undoing an operation of `kind` does.
#[must_use]
pub const fn inverse(kind: HistoryKind) -> HistoryKind {
    match kind {
        HistoryKind::Install => HistoryKind::Uninstall,
        HistoryKind::Update | HistoryKind::Uninstall => HistoryKind::Install,
    }
}

/// A later operation that makes the target unsafe to undo.
#[expect(missing_docs, reason = "Field names state their meaning")]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct UndoConflict {
    pub operation_id: String,
    pub kind: HistoryKind,
    pub reason: String,
}

/// Find the newest later operation that touched a file or package of `target`.
///
/// `operations` must be newest first. Records of another installation never
/// conflict. The check refuses before any change, so a second undo of the
/// same target is also refused.
#[must_use]
pub fn conflict(
    operations: &[HistoryOperation],
    target: &HistoryOperation,
) -> Option<UndoConflict> {
    let position = operations
        .iter()
        .position(|operation| operation.id == target.id)?;
    let mut paths = BTreeMap::new();
    let mut packages = BTreeMap::new();
    for package in &target.packages {
        for file in &package.files {
            paths.insert((file.root.as_str(), file.destination.as_str()), ());
        }
        for id in [&package.package_id_before, &package.package_id_after]
            .into_iter()
            .flatten()
        {
            packages.insert(id.as_str(), ());
        }
    }
    for later in &operations[..position] {
        if later.installation != target.installation {
            continue;
        }
        for package in &later.packages {
            let changed_package = [
                package.package_id_before.as_deref(),
                package.package_id_after.as_deref(),
            ]
            .into_iter()
            .flatten()
            .any(|id| packages.contains_key(id));
            let changed_file = package
                .files
                .iter()
                .any(|file| paths.contains_key(&(file.root.as_str(), file.destination.as_str())));
            if changed_package || changed_file {
                return Some(UndoConflict {
                    operation_id: later.id.clone(),
                    kind: later.kind,
                    reason: if changed_package {
                        "changed the same package".to_owned()
                    } else {
                        "changed the same files".to_owned()
                    },
                });
            }
        }
    }
    None
}

/// One release that an undo installs again.
#[expect(missing_docs, reason = "Field names state their meaning")]
#[derive(Debug)]
#[non_exhaustive]
pub struct UndoRestore {
    pub archive: PathBuf,
    pub plan: InstallPlan,
    /// Recorded files that the restored release must reproduce.
    pub expected: Vec<HistoryFile>,
}

impl UndoRestore {
    /// Restore `archive` with `plan`, reproducing `expected` files.
    #[must_use]
    pub fn new(archive: PathBuf, plan: InstallPlan, expected: Vec<HistoryFile>) -> Self {
        Self {
            archive,
            plan,
            expected,
        }
    }
}

impl Store {
    /// Undo of an install: remove exactly the packages that it installed.
    ///
    /// All removals share one journal, one ownership commit and one history
    /// record. A failure restores every file withdrawn so far.
    pub fn uninstall_for_undo(
        &mut self,
        game: &Path,
        target: &HistoryOperation,
    ) -> Result<Vec<String>> {
        if target.kind != HistoryKind::Install {
            bail!("operation {} is not an install", target.id);
        }
        let _guard = lock_store(self)?;
        self.ensure_idle()?;
        let game = self.transaction_root(game)?;
        let installed = self.installed(&game)?;
        let mut members = Vec::new();
        for package in &target.packages {
            let id = package
                .package_id_after
                .as_deref()
                .context("history record has no installed package ID")?;
            let current = installed
                .iter()
                .find(|installed| installed.id == id)
                .with_context(|| format!("managed package {id} is no longer installed"))?;
            if current.pinned {
                return Err(coded(
                    ErrorCode::PackagePinned,
                    format!("managed package {id} is pinned; run `modn unpin {id}` first"),
                ));
            }
            members.push(id.to_owned());
        }
        let allowed: std::collections::BTreeSet<String> = members.iter().cloned().collect();
        self.control.checkpoint()?;
        self.begin_removal(&game, &target.id)?;
        let mut journals = Vec::new();
        for id in &members {
            match self.withdraw_member_locked(&game, id, &allowed) {
                Ok(journal) => journals.push((journal, id.clone())),
                Err(err) => {
                    if let Err(recovery) = self.recover_locked() {
                        return Err(anyhow::anyhow!(
                            "{err:#}; undo recovery incomplete: {recovery:#}; run 'modn recover'"
                        ));
                    }
                    return Err(err);
                }
            }
        }
        if let Err(err) = self.commit_uninstall_many(&journals) {
            if let Err(recovery) = self.recover_locked() {
                bail!("{err:#}; undo recovery incomplete: {recovery:#}; run 'modn recover'");
            }
            return Err(err);
        }
        self.recover_locked()
            .context("undo committed; cleanup incomplete, run 'modn recover'")?;
        Ok(members)
    }

    /// Undo of an uninstall: install every recorded release as one operation.
    ///
    /// All members install under one stack marker and one history record. A
    /// failure in any member, or a restored file whose digest differs from the
    /// record, removes the members already installed.
    pub fn restore_for_undo(
        &mut self,
        game: &Path,
        target: &HistoryOperation,
        restores: &[UndoRestore],
    ) -> Result<Vec<ManagedPackage>> {
        if target.kind != HistoryKind::Uninstall {
            bail!("operation {} is not an uninstall", target.id);
        }
        if restores.len() != target.packages.len() {
            bail!("undo needs one archive for each recorded package");
        }
        let _guard = lock_store(self)?;
        self.ensure_idle()?;
        let game = self.transaction_root(game)?;
        self.control.checkpoint()?;
        let stack_id = self.begin_stack(&game)?;
        let mut installed = Vec::new();
        for restore in restores {
            match self.install_locked(
                &game,
                &restore.archive,
                &restore.plan,
                None,
                &[],
                None,
                NativeInjectorPolicy::Refuse,
                Some(&stack_id),
            ) {
                Ok(package) => installed.push(package),
                Err(err) => {
                    return Err(match self.rollback_stack(&stack_id, &game) {
                        Ok(count) => {
                            err.context(format!("undo rolled back {count} restored package(s)"))
                        }
                        Err(rollback) => anyhow::anyhow!(
                            "{err:#}; undo rollback incomplete: {rollback:#}; run 'modn recover'"
                        ),
                    });
                }
            }
        }
        for (ordering, restore) in restores.iter().enumerate() {
            let expected = expected_digests(&restore.expected);
            let actual = restored_digests(&installed[ordering]);
            if expected != actual {
                let package = target.packages[ordering]
                    .package_id_before
                    .as_deref()
                    .unwrap_or("package");
                return Err(match self.rollback_stack(&stack_id, &game) {
                    Ok(_) => coded(
                        ErrorCode::UndoFailed,
                        format!(
                            "restored files of {package} do not match the recorded digests; no change kept"
                        ),
                    ),
                    Err(rollback) => anyhow::anyhow!(
                        "restored files of {package} do not match the recorded digests; rollback incomplete: {rollback:#}; run 'modn recover'"
                    ),
                });
            }
        }
        self.finish_undo_install(&stack_id, target, &installed)?;
        Ok(installed)
    }

    /// Refuse when the archive members that `plan` selects would restore
    /// different bytes than the recorded `digest_before` of `expected`.
    ///
    /// The check runs before an update starts and before any ownership
    /// change, so a module policy that selects another archive member for the
    /// same destination refuses with [`ErrorCode::UndoFailed`] and leaves
    /// game files, ownership and history untouched.
    pub(super) fn verify_restored_bytes(
        &self,
        archive: &Path,
        plan: &InstallPlan,
        expected: &[HistoryFile],
    ) -> Result<()> {
        let expected = expected_digests(expected);
        if expected.is_empty() {
            return Ok(());
        }
        let actual = selected_digests(archive, plan, &self.state_root)?;
        if actual != expected {
            return Err(coded(
                ErrorCode::UndoFailed,
                "the archive would restore different file bytes than the operation record; no change made",
            ));
        }
        Ok(())
    }

    fn finish_undo_install(
        &mut self,
        stack_id: &str,
        target: &HistoryOperation,
        installed: &[ManagedPackage],
    ) -> Result<()> {
        let tx = self.connection.transaction()?;
        let mut packages = Vec::new();
        for (ordering, package) in installed.iter().enumerate() {
            let recorded = &target.packages[ordering];
            let release = recorded
                .release_before
                .clone()
                .context("history record has no release to restore")?;
            record_release(&tx, &package.id, &recorded.mod_id, &release)?;
            let mut files: Vec<HistoryFile> = package
                .files
                .iter()
                .map(|file| HistoryFile {
                    root: file.root.clone(),
                    destination: file.destination.clone(),
                    digest_before: None,
                    digest_after: Some(file.sha256.clone()),
                })
                .collect();
            files.sort_by(|a, b| (&a.root, &a.destination).cmp(&(&b.root, &b.destination)));
            packages.push(crate::history::HistoryPackage {
                mod_id: recorded.mod_id.clone(),
                package_id_before: None,
                package_id_after: Some(package.id.clone()),
                release_before: None,
                release_after: Some(release),
                files,
            });
        }
        tx.execute(
            "DELETE FROM pending_stack_packages WHERE stack_id=?1",
            [stack_id],
        )?;
        tx.execute("DELETE FROM pending_stacks WHERE id=?1", [stack_id])?;
        let record = HistoryOperation {
            id: stack_id.to_owned(),
            kind: HistoryKind::Install,
            game_id: target.game_id.clone(),
            installation: target.installation.clone(),
            recorded_at: String::new(),
            packages,
            undoes: Some(target.id.clone()),
        };
        crate::history::insert(&tx, &record)?;
        tx.commit()?;
        Ok(())
    }
}

fn record_release(
    tx: &Transaction<'_>,
    package_id: &str,
    mod_id: &str,
    release: &HistoryRelease,
) -> Result<()> {
    let source = release
        .source
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    tx.execute(
        "INSERT OR REPLACE INTO managed_package_releases
             (package_id, mod_id, version, source, archive_blake3)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            package_id,
            mod_id,
            release.version,
            source,
            release.archive_blake3
        ],
    )?;
    Ok(())
}

fn expected_digests(files: &[HistoryFile]) -> BTreeMap<(String, String), String> {
    files
        .iter()
        .filter_map(|file| {
            file.digest_before.as_ref().map(|digest| {
                (
                    (file.root.clone(), file.destination.clone()),
                    digest.clone(),
                )
            })
        })
        .collect()
}

fn restored_digests(package: &ManagedPackage) -> BTreeMap<(String, String), String> {
    package
        .files
        .iter()
        .map(|file| {
            (
                (file.root.clone(), file.destination.clone()),
                file.sha256.clone(),
            )
        })
        .collect()
}

/// SHA-256 of every archive member that `plan` selects, keyed by root and
/// destination.
fn selected_digests(
    archive: &Path,
    plan: &InstallPlan,
    staging_root: &Path,
) -> Result<BTreeMap<(String, String), String>> {
    let inspection = crate::archive::inspect(archive)?;
    match inspection.format {
        ArchiveFormat::Zip => zip_digests(archive, &inspection, plan),
        ArchiveFormat::SevenZ => sevenz_digests(archive, &inspection, plan, staging_root),
        _ => bail!("archive format cannot be installed"),
    }
}

fn zip_digests(
    archive: &Path,
    inspection: &Inspection,
    plan: &InstallPlan,
) -> Result<BTreeMap<(String, String), String>> {
    let original: HashMap<&str, &str> = inspection
        .files
        .iter()
        .map(|file| {
            (
                file.entry.as_str(),
                file.original_entry.as_deref().unwrap_or(&file.entry),
            )
        })
        .collect();
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    let mut digests = BTreeMap::new();
    for operation in &plan.operations {
        let name = original
            .get(operation.source.as_str())
            .copied()
            .context("source is not present in inspected ZIP")?;
        let mut input = zip.by_name(name)?;
        if input.is_dir() {
            bail!("archive metadata changed for {}", operation.source);
        }
        let mut hasher = Sha256::new();
        let copied = io::copy(&mut input.by_ref().take(operation.bytes + 1), &mut hasher)?;
        if copied != operation.bytes {
            bail!("archive extraction size mismatch for {}", operation.source);
        }
        digests.insert(
            (operation.root.clone(), operation.destination.clone()),
            format!("{:x}", hasher.finalize()),
        );
    }
    Ok(digests)
}

fn sevenz_digests(
    archive: &Path,
    inspection: &Inspection,
    plan: &InstallPlan,
    staging_root: &Path,
) -> Result<BTreeMap<(String, String), String>> {
    let staging = tempfile::Builder::new()
        .prefix("undo-check-")
        .tempdir_in(staging_root)?;
    let control = crate::progress::OperationControl::default();
    let files = super::staging::stage_7z(archive, inspection, plan, staging.path(), &control)?;
    Ok(files
        .into_iter()
        .map(|file| {
            (
                (file.root.label().to_owned(), file.destination),
                file.sha256,
            )
        })
        .collect())
}
