pub(super) mod layout;
mod pin;
#[cfg(test)]
mod tests;

use self::layout::RetainedRelease;
pub use self::pin::ModulePin;
use super::loader::{self, GameModuleInfo, VerifiedModule};
use crate::buckets::BucketStore;
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

/// Explicit acknowledgement that a module contains executable third-party policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "Explicit two-state acknowledgement"
)]
pub enum ModuleTrust {
    /// No trust decision was supplied. Registration must fail.
    NotAcknowledged,
    /// The operator accepts execution of the selected module.
    Acknowledged,
}

/// Where one retained release came from.
#[expect(
    clippy::exhaustive_structs,
    reason = "Callers construct release origins as plain values"
)]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModuleSource {
    /// Trusted bucket that supplied the release, or `None` for a local registration.
    pub bucket: Option<String>,
    /// Pinned signer key identifier of the source bucket, when known.
    pub signer_key_id: Option<String>,
}

impl ModuleSource {
    fn describe(&self) -> String {
        match &self.bucket {
            Some(bucket) => match &self.signer_key_id {
                Some(signer) => format!("bucket {bucket} (signer {signer})"),
                None => format!("bucket {bucket}"),
            },
            None => "local registration".to_owned(),
        }
    }
}

/// Identity and content digest of one retained local module release.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct RegisteredModule {
    /// Validated manifest and Lua identity.
    pub module: GameModuleInfo,
    /// BLAKE3 digest of the exact manifest bytes followed by the entrypoint bytes.
    pub blake3: String,
    /// Source bucket and signer recorded for the release.
    pub source: ModuleSource,
    /// Whether this release is the game's current release.
    pub active: bool,
}

/// Result of importing one release into the trusted registry.
#[expect(clippy::exhaustive_enums, reason = "Release outcomes are a closed set")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseOutcome {
    /// A newer release was retained and became active.
    Imported,
    /// The same release is already retained and active state did not change.
    Unchanged,
    /// The release was retained but is not newer than the active release.
    NotNewer,
    /// A pin refused the import, so nothing was retained.
    Pinned,
}

/// Report returned by [`ModuleRegistry::import_release`].
#[expect(
    clippy::exhaustive_structs,
    reason = "Reports are plain serialized records"
)]
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ImportReport {
    /// Game identity of the inspected release.
    pub game_id: String,
    /// Version of the inspected release.
    pub version: String,
    /// BLAKE3 digest of the inspected manifest followed by the entrypoint.
    pub blake3: String,
    /// What the import did with the release.
    pub outcome: ReleaseOutcome,
}

/// Separate trusted module storage under Modn's data directory.
///
/// Each game keeps every retained release below `versions/<version>` and a
/// single `current` pointer. Registration and import copy only the manifest and
/// its single Lua entrypoint. Import requires a valid local main bucket to
/// exclude official game identities. This registry supplies no sandbox
/// guarantee or game-write approval.
#[derive(Debug)]
pub struct ModuleRegistry {
    data: PathBuf,
}

impl ModuleRegistry {
    /// Use the same data directory as `BucketStore` and normal policy resolution.
    #[must_use]
    pub fn new(data_directory: &Path) -> Self {
        Self {
            data: data_directory.to_owned(),
        }
    }

    pub(super) fn root(&self) -> PathBuf {
        self.data.join("game-modules")
    }

    pub(super) fn directory(&self, game: &str) -> Result<PathBuf> {
        if !loader::valid_game_id(game) {
            bail!("invalid game module identifier");
        }
        Ok(self.root().join(game))
    }

    pub(super) fn lock(&self) -> Result<File> {
        fs::create_dir_all(self.root())?;
        self.validate_root()?;
        let path = self.root().join("registry.lock");
        match fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.file_type().is_file() => {
                bail!("module registry lock must be a regular file");
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            Ok(_) | Err(_) => {}
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        file.lock_exclusive()?;
        Ok(file)
    }

    fn validate_root(&self) -> Result<()> {
        if !fs::symlink_metadata(self.root())?.file_type().is_dir() {
            bail!("module registry must be a real directory");
        }
        Ok(())
    }

    /// Copy, validate and import a trusted local source as a release.
    ///
    /// Registration changes manager metadata only. It grants no install approval.
    pub fn register(&self, source: &Path, trust: ModuleTrust) -> Result<RegisteredModule> {
        if trust != ModuleTrust::Acknowledged {
            bail!("module registration requires explicit trust acknowledgement");
        }
        let report = self.import_release(source, ModuleSource::default())?;
        if report.outcome == ReleaseOutcome::Pinned {
            bail!(
                "game module {} is pinned; unpin it before registering another release",
                report.game_id
            );
        }
        self.registered(&report.game_id, &report.version)
    }

    /// Import one trusted release into the versioned registry.
    ///
    /// A newer release becomes active atomically. A same-version release with a
    /// different digest is refused, a pin refuses every different release, and
    /// a lower release is retained without becoming active.
    pub fn import_release(&self, source: &Path, origin: ModuleSource) -> Result<ImportReport> {
        validate_source(&origin)?;
        let (manifest, script) = loader::read_verified_module(source)?;
        if BucketStore::new(&self.data)
            .official_games()?
            .contains(&manifest.game_id)
        {
            bail!("cannot replace an official game module through trusted registration");
        }
        let root = source.canonicalize()?;
        let manifest_bytes =
            loader::bounded_file(&root.join("module.json"), &root, loader::MAX_MANIFEST_BYTES)?;
        let blake3 = layout::digest(&manifest_bytes, script.as_bytes());
        let _lock = self.lock()?;
        self.import_verified(&manifest, &script, &manifest_bytes, blake3, origin)
    }

    fn import_verified(
        &self,
        manifest: &VerifiedModule,
        script: &str,
        manifest_bytes: &[u8],
        blake3: String,
        origin: ModuleSource,
    ) -> Result<ImportReport> {
        let game = manifest.game_id.as_str();
        let directory = self.directory(game)?;
        let current = layout::open_game(&directory, game)?;
        if let Some(active) = current.as_deref() {
            layout::read_release(&directory, game, active)?;
        }
        let version_directory = layout::release_directory(&directory, &manifest.version);
        match fs::symlink_metadata(&version_directory) {
            Ok(_) => {
                let existing = layout::read_release(&directory, game, &manifest.version)?;
                if existing.blake3 == blake3 {
                    return Ok(report(game, manifest, blake3, ReleaseOutcome::Unchanged));
                }
                bail!(
                    "module {game} version {} is already retained with a different digest (existing source: {}; incoming source: {}); remove and register the module again",
                    manifest.version,
                    existing.source.describe(),
                    origin.describe()
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if ModulePin::read(&directory, game)?.is_some() {
            return Ok(report(game, manifest, blake3, ReleaseOutcome::Pinned));
        }
        let staging = tempfile::Builder::new()
            .prefix(".staging-")
            .tempdir_in(self.root())?;
        fs::write(staging.path().join("module.json"), manifest_bytes)?;
        fs::write(staging.path().join(&manifest.entrypoint), script.as_bytes())?;
        let checked = loader::check_module(staging.path())?;
        if checked.game_id != manifest.game_id
            || checked.version != manifest.version
            || checked.api_version != manifest.api_version
        {
            bail!("module source identity changed during import");
        }
        let receipt = layout::Receipt {
            game_id: checked.game_id.clone(),
            version: checked.version.clone(),
            api_version: checked.api_version,
            blake3: blake3.clone(),
            bucket: origin.bucket,
            signer_key_id: origin.signer_key_id,
        };
        fs::write(
            staging.path().join(layout::RECEIPT),
            serde_json::to_vec(&receipt)?,
        )?;
        let newer = current
            .as_deref()
            .is_none_or(|active| layout::compare_versions(&manifest.version, active).is_gt());
        let versions = directory.join(layout::VERSIONS);
        fs::create_dir_all(&versions)?;
        fs::rename(staging.path(), &version_directory)?;
        if newer {
            layout::write_current(&directory, &manifest.version)?;
        }
        let outcome = if newer {
            ReleaseOutcome::Imported
        } else {
            ReleaseOutcome::NotNewer
        };
        Ok(report(game, manifest, blake3, outcome))
    }

    /// Inspect the active release of one game without executing its Lua.
    pub fn active(&self, game: &str) -> Result<Option<RegisteredModule>> {
        let directory = self.directory(game)?;
        let Some(current) = layout::open_game(&directory, game)? else {
            return Ok(None);
        };
        self.validate_root()?;
        let release = layout::read_release(&directory, game, &current)?;
        Ok(Some(release.registered(true)))
    }

    /// List every retained release of one game, oldest first.
    pub fn releases(&self, game: &str) -> Result<Vec<RegisteredModule>> {
        let directory = self.directory(game)?;
        let Some(current) = layout::open_game(&directory, game)? else {
            return Ok(Vec::new());
        };
        self.validate_root()?;
        let versions = layout::list_versions(&directory)?;
        if !versions.iter().any(|version| version == &current) {
            bail!("corrupt module registry current pointer for {game}");
        }
        let mut releases = Vec::with_capacity(versions.len());
        for version in versions {
            let release = layout::read_release(&directory, game, &version)?;
            releases.push(release.registered(version == current));
        }
        Ok(releases)
    }

    /// Pin the active release. Automatic imports and rollback then refuse to move it.
    pub fn pin(&self, game: &str) -> Result<ModulePin> {
        let _lock = self.lock()?;
        let directory = self.directory(game)?;
        let Some(current) = layout::open_game(&directory, game)? else {
            bail!("no registered game module for {game}");
        };
        let release = layout::read_release(&directory, game, &current)?;
        let pin = ModulePin::new(
            &release.source,
            &release.info.game_id,
            &release.info.version,
            &release.blake3,
        );
        ModulePin::write(&directory, &pin)?;
        Ok(pin)
    }

    /// Remove the pin, if any. Retained releases are unchanged.
    pub fn unpin(&self, game: &str) -> Result<()> {
        let _lock = self.lock()?;
        let directory = self.directory(game)?;
        if layout::open_game(&directory, game)?.is_none() {
            bail!("no registered game module for {game}");
        }
        ModulePin::remove(&directory)
    }

    /// Read the current pin without changing storage.
    pub fn pin_status(&self, game: &str) -> Result<Option<ModulePin>> {
        let directory = self.directory(game)?;
        if layout::open_game(&directory, game)?.is_none() {
            return Ok(None);
        }
        ModulePin::read(&directory, game)
    }

    /// Switch the `current` pointer to an older retained release or an explicit version.
    ///
    /// Rollback changes only manager metadata, so it can affect future plans but
    /// never installed files.
    pub fn rollback(&self, game: &str, to: Option<&str>) -> Result<RegisteredModule> {
        let _lock = self.lock()?;
        let directory = self.directory(game)?;
        let Some(current) = layout::open_game(&directory, game)? else {
            bail!("no registered game module for {game}");
        };
        if ModulePin::read(&directory, game)?.is_some() {
            bail!("game module {game} is pinned; unpin it before rollback");
        }
        layout::read_release(&directory, game, &current)?;
        let target = match to {
            Some(version) => {
                if !loader::valid_version(version) {
                    bail!("invalid module version");
                }
                if version == current {
                    bail!("module release {version} is already active for {game}");
                }
                match fs::symlink_metadata(layout::release_directory(&directory, version)) {
                    Ok(_) => version.to_owned(),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        bail!("module release {version} is not retained for {game}");
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            None => layout::list_versions(&directory)?
                .into_iter()
                .filter(|version| layout::compare_versions(version, &current).is_lt())
                .max_by(|left, right| layout::compare_versions(left, right))
                .with_context(|| format!("no older retained release to roll back to for {game}"))?,
        };
        let release = layout::read_release(&directory, game, &target)?;
        layout::write_current(&directory, &target)?;
        Ok(release.registered(true))
    }

    /// Inspect the active release of every registered game.
    pub fn list(&self) -> Result<Vec<RegisteredModule>> {
        match fs::symlink_metadata(self.root()) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            result => {
                result?;
            }
        }
        self.validate_root()?;
        let mut registered = Vec::new();
        for entry in fs::read_dir(self.root())? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().context("invalid module registry name")?;
            if name == "registry.lock" || name.starts_with(".staging-") {
                continue;
            }
            let (_, _, info) = self
                .read(name)?
                .context("module registration disappeared")?;
            registered.push(info);
        }
        registered.sort_by(|left, right| left.module.game_id.cmp(&right.module.game_id));
        Ok(registered)
    }

    /// Game IDs of every registration without reading its releases.
    ///
    /// A caller that needs the manifests uses [`Self::read`] and can report one
    /// unreadable registration without failing the others.
    pub(super) fn game_ids(&self) -> Result<Vec<String>> {
        match fs::symlink_metadata(self.root()) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            result => {
                result?;
            }
        }
        self.validate_root()?;
        let mut games = Vec::new();
        for entry in fs::read_dir(self.root())? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name == "registry.lock" || name.starts_with(".staging-") {
                continue;
            }
            if loader::valid_game_id(name) {
                games.push(name.to_owned());
            }
        }
        games.sort();
        Ok(games)
    }

    /// Remove only the copied registration. Installed packages and game files remain intact.
    /// A corrupt snapshot can be removed without executing its Lua source.
    pub fn unregister(&self, game: &str) -> Result<()> {
        let path = self.directory(game)?;
        let _lock = self.lock()?;
        if !fs::symlink_metadata(&path)?.file_type().is_dir() {
            bail!("registered module must be a real directory");
        }
        fs::remove_dir_all(path)?;
        Ok(())
    }

    pub(super) fn read(
        &self,
        game: &str,
    ) -> Result<Option<(VerifiedModule, String, RegisteredModule)>> {
        let directory = self.directory(game)?;
        let Some(current) = layout::open_game(&directory, game)? else {
            return Ok(None);
        };
        self.validate_root()?;
        let release = layout::read_release(&directory, game, &current)?;
        let registered = release.registered(true);
        Ok(Some((release.manifest, release.script, registered)))
    }

    fn registered(&self, game: &str, version: &str) -> Result<RegisteredModule> {
        let directory = self.directory(game)?;
        let Some(current) = layout::open_game(&directory, game)? else {
            bail!("game module {game} is no longer registered");
        };
        let release: RetainedRelease = layout::read_release(&directory, game, version)?;
        Ok(release.registered(version == current))
    }
}

fn validate_source(origin: &ModuleSource) -> Result<()> {
    if origin.bucket.as_deref() == Some("") {
        bail!("module source bucket must not be empty");
    }
    Ok(())
}

fn report(
    game_id: &str,
    manifest: &VerifiedModule,
    blake3: String,
    outcome: ReleaseOutcome,
) -> ImportReport {
    ImportReport {
        game_id: game_id.to_owned(),
        version: manifest.version.clone(),
        blake3,
        outcome,
    }
}
