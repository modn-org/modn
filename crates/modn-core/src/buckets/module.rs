use super::manifest::valid_id;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

const MAX_MANIFEST: u64 = 16 * 1024;
const MAX_SCRIPT: u64 = 256 * 1024;

/// Immutable Lua game-module release metadata; validation never executes the module.
#[expect(
    missing_docs,
    reason = "Serialized fields describe the bucket contract"
)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ModuleReleaseManifest {
    pub schema_version: u32,
    pub game_id: String,
    pub version: String,
    pub api_version: u32,
    pub entrypoint: String,
    /// Digest of the exact entrypoint bytes, not a publisher signature.
    pub content_sha256: String,
}

#[expect(missing_docs, reason = "Catalog fields are self-describing")]
#[derive(Clone, Debug, Serialize)]
#[non_exhaustive]
pub struct BucketModule {
    pub bucket: String,
    #[serde(flatten)]
    pub module: ModuleReleaseManifest,
}

pub(super) fn valid_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 48
        && version != "."
        && version != ".."
        && !version.ends_with('.')
        && crate::archive::validated_name(version).is_ok()
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))
}

fn valid_entrypoint(name: &str) -> bool {
    name.ends_with(".lua")
        && name != ".lua"
        && name.len() <= 96
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        && crate::archive::validated_name(name).is_ok()
}

fn bounded_file(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.len() > limit {
        bail!(
            "bucket module entry must be a bounded regular file: {}",
            path.display()
        );
    }
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        bail!("bucket module entry exceeds size limit");
    }
    Ok(bytes)
}

pub(super) fn validate_release(
    manifest_bytes: &[u8],
    script: &[u8],
    game: &str,
    version: &str,
) -> Result<ModuleReleaseManifest> {
    if manifest_bytes.len() as u64 > MAX_MANIFEST || script.len() as u64 > MAX_SCRIPT {
        bail!("bucket module entry exceeds size limit");
    }
    let release: ModuleReleaseManifest =
        serde_json::from_slice(manifest_bytes).context("invalid bucket module.json")?;
    if release.schema_version != 1
        || release.game_id != game
        || !valid_id(&release.game_id)
        || release.version != version
        || !valid_version(&release.version)
    {
        bail!("bucket module identity or schema disagrees with modules/<game>/<version> path");
    }
    if release.api_version != crate::game_modules::HOST_API_VERSION {
        bail!("unsupported bucket module host API");
    }
    if !valid_entrypoint(&release.entrypoint) {
        bail!("invalid bucket module entrypoint");
    }
    let source = std::str::from_utf8(script).context("bucket Lua entrypoint must be UTF-8")?;
    if source.contains('\0') {
        bail!("bucket Lua entrypoint cannot contain NUL bytes");
    }
    if release.content_sha256.len() != 64
        || !release
            .content_sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || format!("{:x}", Sha256::digest(script)) != release.content_sha256
    {
        bail!("bucket module content SHA-256 mismatch");
    }
    Ok(release)
}

/// Read exactly one manifest and Lua source; do not evaluate Lua or modify state.
pub(super) fn read_release(
    path: &Path,
    game: &str,
    version: &str,
) -> Result<ModuleReleaseManifest> {
    let manifest = bounded_file(&path.join("module.json"), MAX_MANIFEST)?;
    let header: ModuleReleaseManifest =
        serde_json::from_slice(&manifest).context("invalid bucket module.json")?;
    if !valid_entrypoint(&header.entrypoint) {
        bail!("invalid bucket module entrypoint");
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            bail!("bucket module release contains non-file entry");
        }
        names.push(entry.file_name());
    }
    if names.len() != 2
        || !names.iter().any(|name| name == "module.json")
        || !names.iter().any(|name| name == header.entrypoint.as_str())
    {
        bail!("bucket module release must contain only module.json and its entrypoint");
    }
    let script = bounded_file(&path.join(&header.entrypoint), MAX_SCRIPT)?;
    validate_release(&manifest, &script, game, version)
}
