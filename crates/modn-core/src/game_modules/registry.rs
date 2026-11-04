use super::loader::{self, GameModuleInfo, VerifiedModule};
use crate::buckets::BucketStore;
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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

/// Identity and content digest of a retained local module snapshot.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct RegisteredModule {
    /// Validated manifest and Lua identity.
    pub module: GameModuleInfo,
    /// SHA-256 of the exact manifest bytes followed by the entrypoint bytes.
    pub sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    game_id: String,
    version: String,
    api_version: u32,
    sha256: String,
}

/// Separate trusted module storage under Modn's data directory.
///
/// Registration copies only the manifest and its single Lua entrypoint.
/// It requires a valid local main bucket to exclude official game identities.
/// This registry supplies no sandbox guarantee or game-write approval.
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

    fn root(&self) -> PathBuf {
        self.data.join("game-modules")
    }

    fn directory(&self, game: &str) -> Result<PathBuf> {
        if !loader::valid_game_id(game) {
            bail!("invalid game module identifier");
        }
        Ok(self.root().join(game))
    }

    fn lock(&self) -> Result<File> {
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

    /// Copy and validate a trusted source. Existing registrations are never replaced.
    ///
    /// Registration changes manager metadata only. It grants no install approval.
    pub fn register(&self, source: &Path, trust: ModuleTrust) -> Result<RegisteredModule> {
        if trust != ModuleTrust::Acknowledged {
            bail!("module registration requires explicit trust acknowledgement");
        }
        let (manifest, script) = loader::read_verified_module(source)?;
        if BucketStore::new(&self.data)
            .official_games()?
            .contains(&manifest.game_id)
        {
            bail!("cannot replace an official game module through trusted registration");
        }
        let _lock = self.lock()?;
        let target = self.directory(&manifest.game_id)?;
        if fs::symlink_metadata(&target).is_ok() {
            bail!("game module is already registered; remove it explicitly first");
        }
        let staging = tempfile::Builder::new()
            .prefix(".staging-")
            .tempdir_in(self.root())?;
        let root = source.canonicalize()?;
        let bytes =
            loader::bounded_file(&root.join("module.json"), &root, loader::MAX_MANIFEST_BYTES)?;
        fs::write(staging.path().join("module.json"), &bytes)?;
        fs::write(staging.path().join(&manifest.entrypoint), script.as_bytes())?;
        let checked = loader::check_module(staging.path())?;
        if checked.game_id != manifest.game_id
            || checked.version != manifest.version
            || checked.api_version != manifest.api_version
        {
            bail!("module source identity changed during registration");
        }
        let receipt = Receipt {
            game_id: checked.game_id.clone(),
            version: checked.version.clone(),
            api_version: checked.api_version,
            sha256: digest(&bytes, script.as_bytes()),
        };
        fs::write(
            staging.path().join("registration.json"),
            serde_json::to_vec(&receipt)?,
        )?;
        fs::rename(staging.path(), target)?;
        Ok(RegisteredModule {
            module: checked,
            sha256: receipt.sha256,
        })
    }

    /// Inspect retained snapshots. Missing storage yields an empty list without writes.
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
        registered.sort_by(|a, b| a.module.game_id.cmp(&b.module.game_id));
        Ok(registered)
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
        let path = self.directory(game)?;
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            result => {
                result?;
            }
        }
        self.validate_root()?;
        let (manifest, script) = loader::read_verified_module(&path)?;
        let root = path.canonicalize()?;
        let bytes =
            loader::bounded_file(&path.join("module.json"), &root, loader::MAX_MANIFEST_BYTES)?;
        let receipt: Receipt = serde_json::from_slice(&loader::bounded_file(
            &path.join("registration.json"),
            &root,
            loader::MAX_MANIFEST_BYTES,
        )?)?;
        let sha256 = digest(&bytes, script.as_bytes());
        if receipt.game_id != game
            || manifest.game_id != game
            || receipt.version != manifest.version
            || receipt.api_version != manifest.api_version
            || receipt.sha256 != sha256
        {
            bail!("registered module snapshot identity or digest changed");
        }
        let info = RegisteredModule {
            module: GameModuleInfo {
                game_id: manifest.game_id.clone(),
                version: manifest.version.clone(),
                api_version: manifest.api_version,
            },
            sha256,
        };
        Ok(Some((manifest, script, info)))
    }
}

fn digest(manifest: &[u8], script: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(manifest);
    hash.update(script);
    format!("{:x}", hash.finalize())
}
