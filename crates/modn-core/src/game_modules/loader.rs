//! Local Luau module validation with bounded, root-scoped reads; no mutation APIs.
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
const MAX_MANIFEST_BYTES: u64 = 16 * 1024;
const MAX_SCRIPT_BYTES: u64 = 256 * 1024;

/// Validated identity of one external game integration; no installation privileges implied.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct GameModuleInfo {
    /// Lowercase game identifier supplied by both manifest and script.
    pub game_id: String,
    /// Local module version for diagnostics; not a host compatibility guarantee.
    pub version: String,
    /// Host API revision negotiated with the module.
    pub api_version: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModuleManifest {
    game_id: String,
    version: String,
    api_version: u32,
    entrypoint: String,
}

fn bounded_file(path: &Path, root: &Path, limit: u64) -> Result<Vec<u8>> {
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

fn valid_game_id(value: &str) -> bool {
    value.len() >= 2
        && value.len() <= 64
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn valid_entrypoint(value: &str) -> bool {
    value.len() <= 96
        && value.ends_with(".luau")
        && value != ".luau"
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        && value != ".."
}

fn validate_manifest(manifest: &ModuleManifest) -> Result<()> {
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
        bail!("entrypoint must be a single safe .luau filename in the module directory");
    }
    Ok(())
}

fn execute_module(script: &[u8], manifest: &ModuleManifest, roots: AllowedRoots) -> Result<()> {
    let source = std::str::from_utf8(script).context("Luau entrypoint must be UTF-8 source")?;
    if source.contains('\0') {
        bail!("Luau entrypoint cannot contain NUL bytes or binary chunks");
    }
    runtime::evaluate(
        source,
        &manifest.entrypoint,
        roots,
        ReadProfile::Standard,
        |_, output: Table| {
            let declared_api: u32 = output.get("api_version")?;
            let declared_game: String = output.get("game_id")?;
            if declared_api != manifest.api_version || declared_game != manifest.game_id {
                return Err(mlua::Error::RuntimeError(
                    "Luau module identity or host API disagrees with manifest".into(),
                ));
            }
            Ok(())
        },
    )
}

/// Load a user-selected local module solely to validate its identity and host API.
///
/// The module executes in a new memory- and time-bounded Luau VM with no
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

/// Read and validate the same manifest/source boundary for executable game policies.
pub(super) fn read_verified_module(directory: &Path) -> Result<(VerifiedModule, String)> {
    let metadata = fs::symlink_metadata(directory)
        .with_context(|| format!("cannot inspect game module {}", directory.display()))?;
    if !metadata.file_type().is_dir() {
        bail!("module directory must be a real directory (not a link)");
    }
    let root: PathBuf = directory.canonicalize()?;
    let manifest_bytes = bounded_file(&root.join("module.json"), &root, MAX_MANIFEST_BYTES)?;
    let manifest: ModuleManifest =
        serde_json::from_slice(&manifest_bytes).context("invalid module.json manifest")?;
    validate_manifest(&manifest)?;
    let script = bounded_file(&root.join(&manifest.entrypoint), &root, MAX_SCRIPT_BYTES)?;
    let source = std::str::from_utf8(&script).context("Luau entrypoint must be UTF-8 source")?;
    if source.contains('\0') {
        bail!("Luau entrypoint cannot contain NUL bytes or binary chunks");
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
