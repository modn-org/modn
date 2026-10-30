//! Generic adapter for runtime-loaded, game-specific Luau policies.
use super::capabilities::{AllowedRoots, ReadProfile};
use super::loader::{self, HOST_API_VERSION};
use super::runtime;
use crate::planning::{RequestedFile, RequestedPlan};
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
        let signals = lua.create_table()?;
        for (index, signal) in report.framework_signals.iter().enumerate() {
            let value = lua.create_table()?;
            value.set("relative_path", signal.relative_path.as_str())?;
            signals.set(index + 1, value)?;
        }
        input.set("framework_signals", signals)?;
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

/// Untrusted, bounded Luau grouping of scanned existing files.
pub(crate) struct AdoptionGroup {
    pub(crate) selector: String,
    pub(crate) kind: String,
    pub(crate) files: Vec<String>,
}

/// Propose complete group boundaries. Game-specific grouping never acquires state access.
pub(crate) fn adoption_groups(
    game_id: &str,
    root: &Path,
    detected: &[DetectedMod],
) -> Result<Vec<AdoptionGroup>> {
    const LIMIT: usize = 2_048;
    if detected.len() > LIMIT {
        bail!("too many files for adoption planning");
    }
    evaluate(game_id, &[root.to_path_buf()], |lua, policy| {
        let input = lua.create_table()?;
        for (index, item) in detected.iter().enumerate() {
            let entry = lua.create_table()?;
            entry.set(
                "relative_path",
                item.relative_path.to_string_lossy().as_ref(),
            )?;
            entry.set("kind", item.kind.as_str())?;
            entry.set_readonly(true);
            input.set(index + 1, entry)?;
        }
        input.set_readonly(true);
        let output: Table = policy.get::<Function>("adoption")?.call(input)?;
        if output.get::<u32>("schema_version")? != 1 {
            return Err(mlua::Error::RuntimeError(
                "unsupported adoption plan schema version".into(),
            ));
        }
        let groups: Table = output.get("groups")?;
        if groups.raw_len() > LIMIT {
            return Err(mlua::Error::RuntimeError(
                "adoption group limit exceeded".into(),
            ));
        }
        let mut budget = 8 * 1024 * 1024;
        groups
            .sequence_values::<Table>()
            .map(|entry| {
                let entry = entry?;
                let files: Table = entry.get("files")?;
                if files.raw_len() > LIMIT {
                    return Err(mlua::Error::RuntimeError(
                        "adoption file limit exceeded".into(),
                    ));
                }
                let files = files
                    .sequence_values::<mlua::LuaString>()
                    .map(|value| {
                        let value = value?;
                        if value.as_bytes().len() > 256 || value.as_bytes().len() > budget {
                            return Err(mlua::Error::RuntimeError(
                                "adoption result too large".into(),
                            ));
                        }
                        budget -= value.as_bytes().len();
                        Ok(value.to_str()?.to_owned())
                    })
                    .collect::<mlua::Result<Vec<_>>>()?;
                Ok(AdoptionGroup {
                    selector: plan_field(&entry, "selector", 256, &mut budget)?,
                    kind: plan_field(&entry, "kind", 64, &mut budget)?,
                    files,
                })
            })
            .collect()
    })
}

// Check Luau string lengths before copying into Rust's memory (outside the VM limit).
fn plan_field(
    table: &Table,
    field: &str,
    limit: usize,
    budget: &mut usize,
) -> mlua::Result<String> {
    let value: mlua::LuaString = table.get(field)?;
    let bytes = value.as_bytes().len();
    if bytes > limit {
        return Err(mlua::Error::RuntimeError(format!(
            "installation field {field} exceeds {limit} bytes"
        )));
    }
    if bytes > *budget {
        return Err(mlua::Error::RuntimeError(
            "installation plan result exceeds 8 MiB byte budget".into(),
        ));
    }
    *budget -= bytes;
    Ok(value.to_str()?.to_string())
}

/// Decode a game module's versioned, metadata-only proposal. No host mutation is exposed.
pub(crate) fn plan(
    game_id: &str,
    root: &Path,
    inspection: &crate::archive::Inspection,
) -> Result<RequestedPlan> {
    const MAX_PROPOSALS: usize = 2_048;
    let root = root.canonicalize()?;
    if inspection.files.len() > MAX_PROPOSALS {
        return Ok(RequestedPlan {
            operations: Vec::new(),
            blockers: vec![(
                "TOO_MANY_FILES".into(),
                format!("Read-only preview supports at most {MAX_PROPOSALS} files"),
            )],
        });
    }
    evaluate(game_id, &[root], |lua, policy| {
        let input = lua.create_table()?;
        for (index, file) in inspection.files.iter().enumerate() {
            let entry = lua.create_table()?;
            entry.set("source", file.entry.as_str())?;
            entry.set("bytes", file.bytes)?;
            entry.set_readonly(true);
            input.set(index + 1, entry)?;
        }
        input.set_readonly(true);
        let output: Table = policy.get::<Function>("plan")?.call(input)?;
        let version: u32 = output.get("schema_version")?;
        if version != 1 {
            return Err(mlua::Error::RuntimeError(
                "unsupported installation plan schema version".into(),
            ));
        }
        let entries: Table = output.get("operations")?;
        let issues: Table = output.get("blockers")?;
        if entries.raw_len() > MAX_PROPOSALS || issues.raw_len() > MAX_PROPOSALS {
            return Err(mlua::Error::RuntimeError(
                "installation plan exceeds result limit".into(),
            ));
        }
        let mut result_budget = 8 * 1024 * 1024;
        let operations = entries
            .sequence_values::<Table>()
            .map(|value| {
                let value = value?;
                Ok(RequestedFile {
                    source: plan_field(&value, "source", 4_096, &mut result_budget)?,
                    destination: plan_field(&value, "destination", 1_024, &mut result_budget)?,
                    kind: plan_field(&value, "kind", 64, &mut result_budget)?,
                    package_root: plan_field(&value, "package_root", 256, &mut result_budget)?,
                })
            })
            .collect::<mlua::Result<Vec<_>>>()?;
        let blockers = issues
            .sequence_values::<Table>()
            .map(|value| {
                let value = value?;
                Ok((
                    plan_field(&value, "code", 64, &mut result_budget)?,
                    plan_field(&value, "message", 512, &mut result_budget)?,
                ))
            })
            .collect::<mlua::Result<Vec<_>>>()?;
        Ok(RequestedPlan {
            operations,
            blockers,
        })
    })
}
