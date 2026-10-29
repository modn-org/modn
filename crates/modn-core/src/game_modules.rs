//! Local Luau game module validation. No game or manager-state writes are exposed.
use anyhow::{bail, Context, Result};
use mlua::{Lua, LuaOptions, StdLib, Table, VmState};
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// First host API revision. Future incompatible changes require a new revision.
pub const HOST_API_VERSION: u32 = 1;
const MAX_MANIFEST_BYTES: u64 = 16 * 1024;
const MAX_SCRIPT_BYTES: u64 = 256 * 1024;
const MAX_VM_BYTES: usize = 8 * 1024 * 1024;
const MAX_INTERRUPTS: u64 = 100_000;
const MAX_EXECUTION: Duration = Duration::from_secs(1);

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

fn execute_module(script: &[u8], manifest: &ModuleManifest) -> Result<()> {
    let execute = || -> mlua::Result<()> {
        // No IO, OS, process, package loader, native module, network, or filesystem capability.
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH,
            LuaOptions::default(),
        )?;
        lua.set_memory_limit(MAX_VM_BYTES)?;
        let globals = lua.globals();
        for restricted in [
            "io",
            "os",
            "package",
            "debug",
            "require",
            "loadstring",
            "dofile",
            "loadfile",
            "getfenv",
            "setfenv",
        ] {
            globals.set(restricted, mlua::Value::Nil)?;
        }

        let host = lua.create_table()?;
        host.set("api_version", HOST_API_VERSION)?;
        host.set_readonly(true);
        globals.set("mods_host", host)?;
        lua.sandbox(true)?;

        let started = Instant::now();
        let interrupts = Cell::new(0u64);
        lua.set_interrupt(move |_| {
            let next = interrupts.get().saturating_add(1);
            interrupts.set(next);
            if next > MAX_INTERRUPTS || started.elapsed() > MAX_EXECUTION {
                return Err(mlua::Error::RuntimeError(
                    "Luau module execution budget exceeded".into(),
                ));
            }
            Ok(VmState::Continue)
        });

        let output: Table = lua.load(script).set_name(&manifest.entrypoint).eval()?;
        let declared_api: u32 = output.get("api_version")?;
        let declared_game: String = output.get("game_id")?;
        if declared_api != manifest.api_version || declared_game != manifest.game_id {
            return Err(mlua::Error::RuntimeError(
                "Luau module identity or host API disagrees with manifest".into(),
            ));
        }
        Ok(())
    };
    execute().map_err(|error| anyhow::anyhow!("Luau module execution failed: {error}"))
}

/// Load a user-selected local module solely to validate its identity and host API.
///
/// The module executes in a new memory- and time-bounded Luau VM with no
/// host filesystem APIs. This is not approval, activation, installation, or
/// a formal security boundary against native VM vulnerabilities.
pub fn check_module(directory: &Path) -> Result<GameModuleInfo> {
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
    execute_module(&script, &manifest)
        .with_context(|| format!("module {} failed validation", manifest.game_id))?;
    Ok(GameModuleInfo {
        game_id: manifest.game_id,
        version: manifest.version,
        api_version: HOST_API_VERSION,
    })
}
