//! Generic adapter for runtime-loaded, game-specific Luau policies.
use super::capabilities::{AllowedRoots, ReadProfile};
use super::loader::{self, HOST_API_VERSION};
use super::runtime;
use crate::scan::{DetectedMod, Diagnostic, FrameworkSignal, ScanReport};

use anyhow::{bail, Result};
use mlua::{Function, Lua, Table};
use std::path::{Component, Path, PathBuf};

const MAX_ITEMS: usize = 20_000;

fn module_directory(game_id: &str) -> Result<PathBuf> {
    if game_id.len() < 2
        || game_id.len() > 64
        || !game_id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
    {
        bail!("invalid game module identifier");
    }
    if let Some(base) = std::env::var_os("MODN_GAME_MODULES_DIR") {
        return Ok(PathBuf::from(base).join(game_id));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let directory = parent.join("game-modules").join(game_id);
            if directory.is_dir() {
                return Ok(directory);
            }
        }
    }
    // Development fixture fallback: packaging must ship game modules beside the executable.
    Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("game-modules")
        .join(game_id))
}

fn evaluate<T>(
    game_id: &str,
    roots: &[PathBuf],
    action: impl FnOnce(&Lua, Table) -> mlua::Result<T>,
) -> Result<T> {
    evaluate_with_profile(game_id, roots, ReadProfile::Standard, action)
}

fn evaluate_with_profile<T>(
    game_id: &str,
    roots: &[PathBuf],
    profile: ReadProfile,
    action: impl FnOnce(&Lua, Table) -> mlua::Result<T>,
) -> Result<T> {
    let directory = module_directory(game_id)?;
    let (manifest, source) = loader::read_verified_module(&directory)?;
    if manifest.game_id != game_id {
        bail!("game module identity does not match requested game");
    }
    let roots = AllowedRoots::validate(roots)?;
    runtime::evaluate(
        &source,
        &manifest.entrypoint,
        roots,
        profile,
        |lua, policy| {
            let id: String = policy.get("game_id")?;
            let version: u32 = policy.get("api_version")?;
            if id != game_id || version != HOST_API_VERSION {
                return Err(mlua::Error::RuntimeError(
                    "game module identity or host API mismatch".into(),
                ));
            }
            action(lua, policy)
        },
    )
}

/// Validate a selected installation by its game policy, not a hardcoded Rust executable path.
pub(crate) fn validate(game_id: &str, root: &Path) -> Result<(bool, String)> {
    evaluate(game_id, &[root.to_path_buf()], |_, policy| {
        let valid: bool = policy.get::<Function>("validate")?.call(())?;
        let marker: String = policy.get("marker")?;
        Ok((valid, marker))
    })
}

/// Determine the Steam installation folder from game-specific manifest policy.
pub(crate) fn steam_name(game_id: &str, root: &Path) -> Result<String> {
    evaluate(game_id, &[root.to_path_buf()], |_, policy| {
        policy.get::<Function>("steam_name")?.call(())
    })
}

/// Decide which Epic display name identifies the bundled game.
pub(crate) fn epic_match(game_id: &str, display_name: &str) -> Result<bool> {
    evaluate(game_id, &[], |_, policy| {
        policy.get::<Function>("is_epic")?.call(display_name)
    })
}

/// Game-specific common installation directory hints relative to system game roots.
pub(crate) fn common_paths(game_id: &str) -> Result<Vec<String>> {
    evaluate(game_id, &[], |_, policy| {
        let paths: Table = policy.get("common_paths")?;
        paths.sequence_values().collect()
    })
}

fn relative_path(value: &str) -> mlua::Result<PathBuf> {
    let path = Path::new(value);
    if value.len() > 256
        || value.is_empty()
        || value.contains(['\\', ':', '\0'])
        || !path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err(mlua::Error::RuntimeError("unsafe Luau scan path".into()));
    }
    Ok(path.to_owned())
}

fn decode_scan(root: &Path, game_id: &str, table: Table) -> mlua::Result<ScanReport> {
    let raw_mods: Table = table.get("mods")?;
    let mut mods = Vec::new();
    for entry in raw_mods.sequence_values::<Table>() {
        if mods.len() >= MAX_ITEMS {
            return Err(mlua::Error::RuntimeError(
                "Luau scan entry limit exceeded".into(),
            ));
        }
        let entry = entry?;
        mods.push(DetectedMod {
            kind: entry.get("kind")?,
            relative_path: relative_path(&entry.get::<String>("relative_path")?)?,
            managed: false,
            package_id: None,
            integrity: None,
        });
    }
    mods.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    let raw_signals: Table = table.get("framework_signals")?;
    let mut signals = Vec::new();
    for entry in raw_signals.sequence_values::<Table>() {
        let entry = entry?;
        signals.push(FrameworkSignal {
            name: entry.get("name")?,
            relative_path: relative_path(&entry.get::<String>("relative_path")?)?
                .to_string_lossy()
                .into_owned(),
            kind: entry.get("kind")?,
        });
    }
    signals.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then(a.relative_path.cmp(&b.relative_path))
    });
    Ok(ScanReport {
        incomplete: table.get("incomplete")?,
        game_id: game_id.into(),
        installation: root.to_owned(),
        redmod_available: table.get("redmod_available")?,
        mods,
        owned_files: Vec::new(),
        framework_signals: signals,
    })
}

/// Obtain game-specific inventory policy through a bounded read-only Luau VM.
pub(crate) fn scan(game_id: &str, root: &Path, limit: usize) -> Result<ScanReport> {
    let canonical = root.canonicalize()?;
    evaluate_with_profile(
        game_id,
        std::slice::from_ref(&canonical),
        ReadProfile::Scan,
        |_, policy| {
            let scan: Function = policy.get("scan")?;
            let report: Table = scan.call(limit.min(MAX_ITEMS))?;
            decode_scan(&canonical, game_id, report)
        },
    )
}

/// Game-specific diagnostic policy receives only a sanitized inventory summary.
pub(crate) fn diagnose(report: &ScanReport) -> Result<Vec<Diagnostic>> {
    evaluate(&report.game_id, &[], |lua, policy| {
        let input = lua.create_table()?;
        input.set("incomplete", report.incomplete)?;
        input.set("redmod_available", report.redmod_available)?;
        let mods = lua.create_table()?;
        for (index, item) in report.mods.iter().enumerate() {
            let value = lua.create_table()?;
            value.set("kind", item.kind.as_str())?;
            value.set("managed", item.managed)?;
            mods.set(index + 1, value)?;
        }
        input.set("mods", mods)?;
        let owned = lua.create_table()?;
        for (index, item) in report.owned_files.iter().enumerate() {
            let value = lua.create_table()?;
            value.set("status", item.status.as_str())?;
            owned.set(index + 1, value)?;
        }
        input.set("owned_files", owned)?;
        let doctor: Function = policy.get("doctor")?;
        let entries: Table = doctor.call(input)?;
        let mut messages = Vec::new();
        for entry in entries.sequence_values::<Table>() {
            let entry = entry?;
            messages.push(Diagnostic {
                code: entry.get("code")?,
                severity: entry.get("severity")?,
                message: entry.get("message")?,
            });
        }
        Ok(messages)
    })
}
