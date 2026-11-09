use super::super::loader;
use super::layout;
use super::ModuleSource;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// A retained release that automatic imports and rollback must not replace.
#[expect(
    clippy::exhaustive_structs,
    reason = "Pins are plain serialized records"
)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModulePin {
    /// Trusted bucket that supplied the release, or `None` for a local registration.
    pub bucket: Option<String>,
    /// Game identity the pin belongs to.
    pub game_id: String,
    /// Pinned release version.
    pub version: String,
    /// SHA-256 of the pinned manifest followed by the entrypoint.
    pub sha256: String,
}

impl ModulePin {
    pub(super) fn new(source: &ModuleSource, game: &str, version: &str, sha256: &str) -> Self {
        Self {
            bucket: source.bucket.clone(),
            game_id: game.to_owned(),
            version: version.to_owned(),
            sha256: sha256.to_owned(),
        }
    }

    pub(in crate::game_modules) fn read(directory: &Path, game: &str) -> Result<Option<Self>> {
        let path = directory.join(layout::PIN);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            result => {
                result?;
            }
        }
        let root = directory.canonicalize()?;
        let pin: Self = serde_json::from_slice(&loader::bounded_file(
            &path,
            &root,
            loader::MAX_MANIFEST_BYTES,
        )?)?;
        if pin.game_id != game
            || !loader::valid_version(&pin.version)
            || pin.sha256.len() != 64
            || !pin
                .sha256
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
            || pin.bucket.as_deref() == Some("")
        {
            bail!("module pin for {game} is corrupt");
        }
        Ok(Some(pin))
    }

    pub(super) fn write(directory: &Path, pin: &Self) -> Result<()> {
        let path = directory.join(layout::PIN);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.file_type().is_file() => {
                bail!("module pin must be a regular file");
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(error.into());
            }
            Ok(_) | Err(_) => {}
        }
        let mut temp = tempfile::NamedTempFile::new_in(directory)?;
        serde_json::to_writer(&mut temp, pin)?;
        temp.as_file().sync_all()?;
        temp.persist(&path).map_err(|error| error.error)?;
        Ok(())
    }

    pub(super) fn remove(directory: &Path) -> Result<()> {
        let path = directory.join(layout::PIN);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Ok(metadata) if !metadata.file_type().is_file() => {
                bail!("module pin must be a regular file")
            }
            Ok(_) => fs::remove_file(path).context("cannot remove module pin"),
            Err(error) => Err(error.into()),
        }
    }
}
