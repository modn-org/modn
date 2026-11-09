//! Report trusted catalog modules hidden by an official game module.

use super::loader;
use crate::buckets::BucketStore;
use crate::data_dir;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;

/// One trusted catalog module that the official main bucket overrides.
#[expect(
    clippy::exhaustive_structs,
    reason = "Reports are plain serialized records"
)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HiddenModule {
    /// Trusted bucket that provides the hidden module.
    pub bucket: String,
    /// Version declared by the hidden catalog module.
    pub version: String,
}

#[derive(Deserialize)]
struct ModuleVersion {
    version: String,
}

/// List trusted catalog modules for `game` that `main` hides by official priority.
///
/// Only the manifest version is parsed. The module's Lua is never executed.
pub fn hidden_modules(game: &str) -> Result<Vec<HiddenModule>> {
    if !loader::valid_game_id(game) {
        anyhow::bail!("invalid game module identifier");
    }
    let data = data_dir::profile_directory()?;
    let store = BucketStore::new(&data);
    if !store.official_games()?.iter().any(|id| id == game) {
        return Ok(Vec::new());
    }
    let mut hidden = Vec::new();
    for (bucket, catalog) in store.trusted_module_catalogs()? {
        let directory = catalog.join("games").join(game);
        let manifest = directory.join("module.json");
        match fs::symlink_metadata(&manifest) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
            Ok(metadata) if !metadata.file_type().is_file() => {
                anyhow::bail!(
                    "trusted catalog module manifest must be a regular file: {}",
                    manifest.display()
                );
            }
            Ok(_) => {}
        }
        let root = directory.canonicalize()?;
        let bytes = loader::bounded_file(&manifest, &root, loader::MAX_MANIFEST_BYTES)
            .with_context(|| {
                format!("cannot read hidden module manifest {}", manifest.display())
            })?;
        let version: ModuleVersion = serde_json::from_slice(&bytes)
            .with_context(|| format!("invalid hidden module manifest {}", manifest.display()))?;
        hidden.push(HiddenModule {
            bucket,
            version: version.version,
        });
    }
    hidden.sort_by(|left, right| left.bucket.cmp(&right.bucket));
    Ok(hidden)
}
