use super::capabilities::{AllowedRoots, ReadProfile};
use super::runtime;
use anyhow::{bail, Context, Result};
use mlua::Table;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

/// First host API revision. Future incompatible changes require a new revision.
pub const HOST_API_VERSION: u32 = 1;
pub(super) const MAX_MANIFEST_BYTES: u64 = 16 * 1024;
pub(super) const MAX_SCRIPT_BYTES: u64 = 256 * 1024;

/// Validated identity of one Lua game integration; no installation privileges implied.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct GameModuleInfo {
    /// Lowercase game identifier supplied by both manifest and script.
    pub game_id: String,
    /// Module version for diagnostics; not a host compatibility guarantee.
    pub version: String,
    pub api_version: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ModuleManifest {
    pub(super) game_id: String,
    pub(super) version: String,
    pub(super) api_version: u32,
    pub(super) entrypoint: String,
}

pub(super) fn bounded_file(path: &Path, root: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("cannot inspect module file {}", path.display()))?;
    if !metadata.file_type().is_file() || metadata.len() > limit {
        bail!(
            "module file must be regular, unlinked and no larger than {limit} bytes: {}",
            path.display()
        );
    }
    let resolved = path.canonicalize()?;
    if !resolved.starts_with(root) {
        bail!("module file escapes its selected directory");
    }
    let mut bytes = Vec::new();
    File::open(path)
        .with_context(|| format!("cannot open module file {}", path.display()))?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        bail!("module file exceeds {limit}-byte limit");
    }
    Ok(bytes)
}

pub(super) fn valid_game_id(value: &str) -> bool {
    value.len() >= 2
        && value.len() <= 64
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub(super) fn valid_entrypoint(value: &str) -> bool {
    value.len() <= 96
        && value.ends_with(".lua")
        && value != ".lua"
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        && value != ".."
        && crate::archive::validated_name(value).is_ok()
}

pub(super) fn validate_manifest(manifest: &ModuleManifest) -> Result<()> {
    if manifest.api_version != HOST_API_VERSION {
        bail!(
            "incompatible host API: module requires {}, host supports {}",
            manifest.api_version,
            HOST_API_VERSION
        );
    }
    if !valid_game_id(&manifest.game_id) {
        bail!("invalid module game_id; use lowercase ASCII letters, digits and hyphens");
    }
    if manifest.version.is_empty()
        || manifest.version.len() > 48
        || !manifest
            .version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))
    {
        bail!("invalid module version");
    }
    if !valid_entrypoint(&manifest.entrypoint) {
        bail!("entrypoint must be a single safe .lua filename in the module directory");
    }
    Ok(())
}

fn execute_module(script: &[u8], manifest: &ModuleManifest, roots: AllowedRoots) -> Result<()> {
    let source = std::str::from_utf8(script).context("Lua entrypoint must be UTF-8 source")?;
    if source.contains('\0') {
        bail!("Lua entrypoint cannot contain NUL bytes or binary chunks");
    }
    runtime::evaluate(
        source,
        &manifest.entrypoint,
        roots,
        ReadProfile::Standard,
        |_, output: Table| {
            let declared_api: u32 = output.get("api_version")?;
            let declared_game: String = output.get("game_id")?;
            let declared_version: Option<String> = output.get("version")?;
            if declared_api != manifest.api_version
                || declared_game != manifest.game_id
                || declared_version.is_some_and(|version| version != manifest.version)
            {
                return Err(mlua::Error::RuntimeError(
                    "Lua module identity or host API disagrees with manifest".into(),
                ));
            }
            Ok(())
        },
    )
}

/// Load a user-selected local module solely to validate its identity and host API.
///
/// The module executes in a new memory- and time-bounded Lua VM with no
/// host filesystem APIs. This is not approval, activation, installation, or
/// a formal security boundary against native VM vulnerabilities.
pub fn check_module(directory: &Path) -> Result<GameModuleInfo> {
    check_module_with_roots(directory, &[])
}

/// Check a local module with explicit, operator-approved read-only roots.
///
/// Root identifiers are opaque Lua indexes, not host path handles. This routine
/// does not create state, write to games, or authorize installation. A directory
/// granted here is only a candidate: game identity is determined by the module.
/// File access checks reject path traversal and linked components. Concurrent
/// external filesystem mutation is not protected by a snapshot.
pub fn check_module_with_roots(
    directory: &Path,
    allowed_roots: &[PathBuf],
) -> Result<GameModuleInfo> {
    let roots = AllowedRoots::validate(allowed_roots)?;
    let (info, script) = read_verified_module(directory)?;
    let manifest = ModuleManifest {
        game_id: info.game_id.clone(),
        version: info.version.clone(),
        api_version: info.api_version,
        entrypoint: info.entrypoint.clone(),
    };
    execute_module(script.as_bytes(), &manifest, roots)
        .with_context(|| format!("module {} failed validation", manifest.game_id))?;
    Ok(info.into())
}

#[derive(Debug)]
pub(super) struct VerifiedModule {
    pub(super) game_id: String,
    pub(super) version: String,
    pub(super) api_version: u32,
    pub(super) entrypoint: String,
}

impl From<VerifiedModule> for GameModuleInfo {
    fn from(value: VerifiedModule) -> Self {
        Self {
            game_id: value.game_id,
            version: value.version,
            api_version: value.api_version,
        }
    }
}

pub(super) fn read_verified_module(directory: &Path) -> Result<(VerifiedModule, String)> {
    let metadata = fs::symlink_metadata(directory)
        .with_context(|| format!("cannot inspect game module {}", directory.display()))?;
    if !metadata.file_type().is_dir() {
        bail!("module directory must be a real directory (not a link)");
    }
    let root: PathBuf = directory.canonicalize()?;
    let manifest_bytes = bounded_file(&root.join("module.json"), &root, MAX_MANIFEST_BYTES)?;
    let manifest = parse_manifest(&manifest_bytes)?;
    let script = bounded_file(&root.join(&manifest.entrypoint), &root, MAX_SCRIPT_BYTES)?;
    verified(manifest, &script)
}

pub(super) fn verify_embedded(
    manifest_bytes: &[u8],
    entrypoint: &str,
    script: &[u8],
) -> Result<(VerifiedModule, String)> {
    if manifest_bytes.len() as u64 > MAX_MANIFEST_BYTES || script.len() as u64 > MAX_SCRIPT_BYTES {
        bail!("embedded game module exceeds its size limit");
    }
    let manifest = parse_manifest(manifest_bytes)?;
    if manifest.entrypoint != entrypoint {
        bail!("embedded game module entrypoint does not match its manifest");
    }
    verified(manifest, script)
}

fn parse_manifest(bytes: &[u8]) -> Result<ModuleManifest> {
    let manifest: ModuleManifest =
        serde_json::from_slice(bytes).context("invalid module.json manifest")?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn verified(manifest: ModuleManifest, script: &[u8]) -> Result<(VerifiedModule, String)> {
    let source = std::str::from_utf8(script).context("Lua entrypoint must be UTF-8 source")?;
    if source.contains('\0') {
        bail!("Lua entrypoint cannot contain NUL bytes or binary chunks");
    }
    Ok((
        VerifiedModule {
            game_id: manifest.game_id,
            version: manifest.version,
            api_version: manifest.api_version,
            entrypoint: manifest.entrypoint,
        },
        source.to_owned(),
    ))
}
