//! Repair of an interrupted activation set when the registry opens.
//!
//! An activation intent lists the releases that a commit was making active.
//! Repair verifies every listed release against the recorded digest, completes
//! the set when all bytes match, and otherwise restores the recorded previous
//! `current` pointers. It never writes or replaces module bytes. A rollback that
//! cannot finish keeps the intent, so the next open retries it.

use super::super::loader;
use super::activation::{remove_intent, ActivationIntent, IntentRelease, ACTIVATION};
use super::{layout, ModuleRegistry};
use crate::manager_roots::ManagerRoots;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Component, Path};

const MAX_INTENT_BYTES: u64 = 1024 * 1024;

/// What opening a registry did with a pending activation set.
#[expect(clippy::exhaustive_enums, reason = "Repair outcomes are a closed set")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActivationRepair {
    /// Every listed release verified and became active.
    Completed {
        /// Games whose listed release became active.
        games: Vec<String>,
    },
    /// Verification failed, so every listed game kept or regained its previous release.
    ///
    /// An incomplete rollback keeps the intent, so the next open retries it.
    RolledBack {
        /// Games whose `current` pointer was restored.
        games: Vec<String>,
        /// Why the set was rolled back.
        reason: String,
    },
    /// The intent could not be read or validated, so nothing was changed.
    Unreadable {
        /// Why the intent was not applied.
        reason: String,
    },
}

/// A registry opened after any pending activation was repaired.
#[expect(
    clippy::exhaustive_structs,
    reason = "Plain result record shared with callers"
)]
#[derive(Debug)]
pub struct RegistryOpen {
    /// The opened registry.
    pub registry: ModuleRegistry,
    /// What repair did, if an activation intent existed.
    pub repair: Option<ActivationRepair>,
}

impl ModuleRegistry {
    /// Open the registry and complete or roll back a pending activation.
    pub fn open(roots: &ManagerRoots) -> Result<RegistryOpen> {
        let registry = Self::new(roots);
        let repair = registry.repair()?;
        Ok(RegistryOpen { registry, repair })
    }

    /// Repair a pending activation, if the intent exists.
    pub(in crate::game_modules) fn repair(&self) -> Result<Option<ActivationRepair>> {
        if !self.root().join(ACTIVATION).try_exists()? {
            return Ok(None);
        }
        let _lock = self.lock()?;
        repair_locked(&self.root())
    }

    /// Lock the registry and repair a pending activation before a change.
    pub(in crate::game_modules) fn lock_and_repair(
        &self,
    ) -> Result<(fs::File, Option<ActivationRepair>)> {
        let lock = self.lock()?;
        let repair = repair_locked(&self.root())?;
        Ok((lock, repair))
    }
}

fn repair_locked(root: &Path) -> Result<Option<ActivationRepair>> {
    let intent = match read_intent(root) {
        Ok(Some(intent)) => intent,
        Ok(None) => return Ok(None),
        Err(error) => {
            return Ok(Some(ActivationRepair::Unreadable {
                reason: format!("{error:#}"),
            }));
        }
    };
    let games = intent
        .releases
        .iter()
        .map(|release| release.game_id.clone())
        .collect::<Vec<_>>();
    let mut failures = Vec::new();
    for release in &intent.releases {
        if let Err(error) = make_release_available(root, release) {
            failures.push(format!(
                "cannot verify game module {} version {}: {error:#}",
                release.game_id, release.version
            ));
        }
    }
    if failures.is_empty() {
        for release in &intent.releases {
            let directory = root.join(&release.game_id);
            if let Err(error) = layout::write_current(&directory, &release.version) {
                failures.push(format!(
                    "cannot activate game module {} version {}: {error:#}",
                    release.game_id, release.version
                ));
            }
        }
    }
    if failures.is_empty() {
        remove_intent(root)?;
        return Ok(Some(ActivationRepair::Completed { games }));
    }
    let mut rollback_failures = Vec::new();
    for release in &intent.releases {
        match &release.previous {
            Some(previous) => {
                let directory = root.join(&release.game_id);
                if let Err(error) = layout::write_current(&directory, previous) {
                    rollback_failures.push(format!(
                        "cannot restore game module {} version {previous}: {error:#}",
                        release.game_id
                    ));
                }
            }
            None => {
                if let Err(error) = remove_created_entry(root, release) {
                    rollback_failures.push(format!(
                        "cannot remove the new game module entry {}: {error:#}",
                        release.game_id
                    ));
                }
            }
        }
    }
    if rollback_failures.is_empty() {
        remove_intent(root)?;
    }
    let mut reason = failures.join("; ");
    if !rollback_failures.is_empty() {
        reason.push_str("; rollback incomplete: ");
        reason.push_str(&rollback_failures.join("; "));
    }
    Ok(Some(ActivationRepair::RolledBack { games, reason }))
}

fn make_release_available(root: &Path, release: &IntentRelease) -> Result<()> {
    let directory = root
        .join(&release.game_id)
        .join(layout::VERSIONS)
        .join(&release.version);
    match fs::symlink_metadata(&directory) {
        Ok(metadata) => {
            if !metadata.file_type().is_dir() {
                bail!("retained release must be a real directory");
            }
            verify_release(&directory, release)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let staged = root.join(&release.staged);
            match fs::symlink_metadata(&staged) {
                Ok(metadata) => {
                    if !metadata.file_type().is_dir() {
                        bail!("staged release must be a real directory");
                    }
                    verify_release(&staged, release)?;
                    let versions = directory
                        .parent()
                        .context("release directory has no parent")?;
                    fs::create_dir_all(versions)?;
                    fs::rename(&staged, &directory).with_context(|| {
                        format!(
                            "cannot move the staged release into {}",
                            directory.display()
                        )
                    })?;
                    Ok(())
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    bail!("release directory and private staged copy are both missing")
                }
                Err(error) => Err(error.into()),
            }
        }
        Err(error) => Err(error.into()),
    }
}

fn verify_release(directory: &Path, release: &IntentRelease) -> Result<()> {
    let (manifest, script) = loader::read_verified_module(directory)?;
    if manifest.game_id != release.game_id || manifest.version != release.version {
        bail!("release identity does not match the activation intent");
    }
    let root = directory.canonicalize()?;
    let manifest_bytes = loader::bounded_file(
        &directory.join("module.json"),
        &root,
        loader::MAX_MANIFEST_BYTES,
    )?;
    if layout::digest(&manifest_bytes, script.as_bytes()) != release.blake3 {
        bail!("release bytes do not match the activation digest");
    }
    Ok(())
}

fn remove_created_entry(root: &Path, release: &IntentRelease) -> Result<()> {
    match fs::remove_dir_all(root.join(&release.game_id)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn read_intent(root: &Path) -> Result<Option<ActivationIntent>> {
    let path = root.join(ACTIVATION);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.file_type().is_file() {
        bail!("activation intent must be a regular file");
    }
    let canonical_root = root.canonicalize()?;
    let bytes = loader::bounded_file(&path, &canonical_root, MAX_INTENT_BYTES)
        .context("cannot read the activation intent")?;
    let intent: ActivationIntent =
        serde_json::from_slice(&bytes).context("invalid activation intent")?;
    if intent.schema_version != 1 {
        bail!("unsupported activation intent schema");
    }
    for release in &intent.releases {
        validate_release(release)?;
    }
    Ok(Some(intent))
}

fn validate_release(release: &IntentRelease) -> Result<()> {
    if !loader::valid_game_id(&release.game_id) || !loader::valid_version(&release.version) {
        bail!("activation intent names an unsafe release");
    }
    if release.blake3.len() != 64 || !release.blake3.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("activation intent records an invalid digest");
    }
    if release
        .previous
        .as_deref()
        .is_some_and(|version| !loader::valid_version(version))
    {
        bail!("activation intent records an invalid previous release");
    }
    validate_staged(&release.staged, &release.game_id)
}

fn validate_staged(staged: &Path, game: &str) -> Result<()> {
    let mut components = staged.components();
    let Some(Component::Normal(private)) = components.next() else {
        bail!("activation intent stages a release outside private storage");
    };
    let private = private
        .to_str()
        .context("activation intent staging path must be UTF-8")?;
    if !private.starts_with(".staging-") || private == ".staging-" {
        bail!("activation intent stages a release outside private storage");
    }
    let Some(Component::Normal(name)) = components.next() else {
        bail!("activation intent staging path must name a game directory");
    };
    if name.to_str() != Some(game) || components.next().is_some() {
        bail!("activation intent staging path does not match its game");
    }
    Ok(())
}
