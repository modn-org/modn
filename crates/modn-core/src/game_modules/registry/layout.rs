use super::super::loader::{self, GameModuleInfo, VerifiedModule};
use super::{ModuleSource, RegisteredModule};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub(in crate::game_modules) const CURRENT: &str = "current";
pub(in crate::game_modules) const PIN: &str = "pin.json";
pub(in crate::game_modules) const RECEIPT: &str = "registration.json";
pub(in crate::game_modules) const VERSIONS: &str = "versions";
const MAX_CURRENT_BYTES: u64 = 64;

/// Versioned receipt stored beside one retained release.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::game_modules) struct Receipt {
    pub(in crate::game_modules) game_id: String,
    pub(in crate::game_modules) version: String,
    pub(in crate::game_modules) api_version: u32,
    pub(in crate::game_modules) sha256: String,
    pub(in crate::game_modules) bucket: Option<String>,
    pub(in crate::game_modules) signer_key_id: Option<String>,
}

/// One verified release under `versions/<version>/`.
pub(in crate::game_modules) struct RetainedRelease {
    pub(in crate::game_modules) manifest: VerifiedModule,
    pub(in crate::game_modules) script: String,
    pub(in crate::game_modules) info: GameModuleInfo,
    pub(in crate::game_modules) sha256: String,
    pub(in crate::game_modules) source: ModuleSource,
}

impl RetainedRelease {
    pub(in crate::game_modules) fn registered(&self, active: bool) -> RegisteredModule {
        RegisteredModule {
            module: self.info.clone(),
            sha256: self.sha256.clone(),
            source: self.source.clone(),
            active,
        }
    }
}

/// Validate the versioned shape of an existing game entry and return its current version.
///
/// Missing storage yields `None`. An old single-snapshot entry is refused with the
/// re-registration instruction required by the no-backward-compatibility rule.
pub(in crate::game_modules) fn open_game(directory: &Path, game: &str) -> Result<Option<String>> {
    match fs::symlink_metadata(directory) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        result => {
            result?;
        }
    }
    if !fs::symlink_metadata(directory)?.file_type().is_dir() {
        bail!("registered game module {game} must be a real directory");
    }
    for legacy in ["module.json", RECEIPT] {
        if fs::symlink_metadata(directory.join(legacy)).is_ok() {
            bail!(
                "legacy unversioned game module registration for {game}; remove and register the module again"
            );
        }
    }
    let current_path = directory.join(CURRENT);
    match fs::symlink_metadata(&current_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if fs::symlink_metadata(directory.join(VERSIONS)).is_ok() {
                bail!(
                    "module registry entry for {game} has no current release; remove and register the module again"
                );
            }
            bail!(
                "module registry entry for {game} is incomplete; remove and register the module again"
            );
        }
        result => {
            let metadata = result?;
            if !metadata.file_type().is_file() {
                bail!("module registry current pointer for {game} must be a regular file");
            }
        }
    }
    let versions = directory.join(VERSIONS);
    match fs::symlink_metadata(&versions) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        _ => {
            bail!("corrupt module registry layout for {game}; remove and register the module again")
        }
    }
    let current = read_current(directory)?;
    match fs::symlink_metadata(release_directory(directory, &current)) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        _ => bail!("corrupt module registry current pointer for {game}"),
    }
    Ok(Some(current))
}

pub(in crate::game_modules) fn read_current(directory: &Path) -> Result<String> {
    let root = directory.canonicalize()?;
    let bytes = loader::bounded_file(&directory.join(CURRENT), &root, MAX_CURRENT_BYTES)?;
    let value =
        std::str::from_utf8(&bytes).context("module registry current pointer must be UTF-8")?;
    let value = value.trim_end_matches(['\r', '\n']);
    if !loader::valid_version(value) {
        bail!("corrupt module registry current pointer");
    }
    Ok(value.to_owned())
}

pub(in crate::game_modules) fn write_current(directory: &Path, version: &str) -> Result<()> {
    if !loader::valid_version(version) {
        bail!("invalid module version");
    }
    let path = directory.join(CURRENT);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            bail!("module registry current pointer must be a regular file");
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
        Ok(_) | Err(_) => {}
    }
    let mut temp = tempfile::NamedTempFile::new_in(directory)?;
    temp.write_all(version.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist(&path).map_err(|error| error.error)?;
    Ok(())
}

pub(in crate::game_modules) fn release_directory(directory: &Path, version: &str) -> PathBuf {
    directory.join(VERSIONS).join(version)
}

pub(in crate::game_modules) fn list_versions(directory: &Path) -> Result<Vec<String>> {
    let mut versions = Vec::new();
    for entry in fs::read_dir(directory.join(VERSIONS))? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_str().context("invalid module release name")?;
        if !loader::valid_version(name) {
            bail!("invalid retained module release {name:?}");
        }
        if !entry.file_type()?.is_dir() {
            bail!("retained module release must be a real directory");
        }
        versions.push(name.to_owned());
    }
    versions.sort_by(|left, right| compare_versions(left, right));
    Ok(versions)
}

pub(in crate::game_modules) fn read_release(
    directory: &Path,
    game: &str,
    version: &str,
) -> Result<RetainedRelease> {
    if !loader::valid_version(version) {
        bail!("invalid module version");
    }
    let version_directory = release_directory(directory, version);
    let metadata = fs::symlink_metadata(&version_directory)
        .with_context(|| format!("module release {version} is not retained for {game}"))?;
    if !metadata.file_type().is_dir() {
        bail!("module release {version} for {game} must be a real directory");
    }
    let (manifest, script) = loader::read_verified_module(&version_directory)?;
    let root = version_directory.canonicalize()?;
    let bytes = loader::bounded_file(
        &version_directory.join("module.json"),
        &root,
        loader::MAX_MANIFEST_BYTES,
    )?;
    let receipt: Receipt = serde_json::from_slice(&loader::bounded_file(
        &version_directory.join(RECEIPT),
        &root,
        loader::MAX_MANIFEST_BYTES,
    )?)?;
    let sha256 = digest(&bytes, script.as_bytes());
    if receipt.game_id != game
        || manifest.game_id != game
        || receipt.version != version
        || manifest.version != version
        || receipt.api_version != manifest.api_version
        || receipt.sha256 != sha256
    {
        bail!("registered module release identity or digest changed");
    }
    let info = GameModuleInfo {
        game_id: manifest.game_id.clone(),
        version: manifest.version.clone(),
        api_version: manifest.api_version,
    };
    Ok(RetainedRelease {
        manifest,
        script,
        info,
        sha256,
        source: ModuleSource {
            bucket: receipt.bucket,
            signer_key_id: receipt.signer_key_id,
        },
    })
}

/// Order semantic versions by precedence when both parse, else compare bytes.
pub(in crate::game_modules) fn compare_versions(left: &str, right: &str) -> Ordering {
    match (semver::Version::parse(left), semver::Version::parse(right)) {
        (Ok(left), Ok(right)) => left.cmp(&right),
        _ => left.as_bytes().cmp(right.as_bytes()),
    }
}

pub(in crate::game_modules) fn digest(manifest: &[u8], script: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(manifest);
    hash.update(script);
    format!("{:x}", hash.finalize())
}
