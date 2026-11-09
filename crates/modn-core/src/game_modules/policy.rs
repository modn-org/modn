use super::capabilities::{AllowedRoots, ReadProfile};
use super::resolver::read_policy;
use super::runtime;
use crate::planning::{RequestedFile, RequestedPlan};
use crate::scan::{DetectedMod, Diagnostic, FrameworkSignal, ScanReport, ScanRoot};

use anyhow::{bail, Context, Result};
use mlua::{Function, Lua, Table};
use std::path::{Component, Path, PathBuf};

const MAX_ITEMS: usize = 20_000;

fn evaluate<T>(
    game_id: &str,
    roots: &[PathBuf],
    action: impl FnOnce(&Lua, Table) -> mlua::Result<T>,
) -> Result<T> {
    evaluate_hiding(game_id, roots, &[], action)
}

fn evaluate_hiding<T>(
    game_id: &str,
    roots: &[PathBuf],
    replaced: &[String],
    action: impl FnOnce(&Lua, Table) -> mlua::Result<T>,
) -> Result<T> {
    evaluate_with_profile(game_id, roots, ReadProfile::Standard, replaced, action)
}

fn evaluate_with_profile<T>(
    game_id: &str,
    roots: &[PathBuf],
    profile: ReadProfile,
    replaced: &[String],
    action: impl FnOnce(&Lua, Table) -> mlua::Result<T>,
) -> Result<T> {
    let (manifest, source) = read_policy(game_id)?;
    if manifest.game_id != game_id {
        bail!("game module identity does not match requested game");
    }
    let roots = AllowedRoots::validate(roots)?.hiding_replaced(replaced);
    runtime::evaluate(
        &source,
        &manifest.entrypoint,
        manifest.api_version,
        roots,
        profile,
        |lua, policy| {
            let id: String = policy.get("game_id")?;
            let version: u32 = policy.get("api_version")?;
            let module_version: Option<String> = policy.get("version")?;
            if id != game_id
                || version != manifest.api_version
                || module_version.is_some_and(|version| version != manifest.version)
            {
                return Err(mlua::Error::RuntimeError(
                    "game module identity or host API mismatch".into(),
                ));
            }
            action(lua, policy)
        },
    )
}

pub(crate) fn validate(game_id: &str, root: &Path) -> Result<(bool, String)> {
    evaluate(game_id, &[root.to_path_buf()], |_, policy| {
        let valid: bool = policy.get::<Function>("validate")?.call(())?;
        let marker: String = policy.get("marker")?;
        Ok((valid, marker))
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
        return Err(mlua::Error::RuntimeError("unsafe Lua scan path".into()));
    }
    Ok(path.to_owned())
}

fn scan_root(entry: &Table, content_authorized: bool) -> mlua::Result<ScanRoot> {
    match entry.get::<Option<String>>("root")? {
        None => Ok(ScanRoot::Game),
        Some(label) if label == "game" => Ok(ScanRoot::Game),
        Some(label) if label == "content" && content_authorized => Ok(ScanRoot::Content),
        Some(label) if label == "content" => Err(mlua::Error::RuntimeError(
            "Lua scan entry uses a content root that the module did not declare".into(),
        )),
        Some(label) => Err(mlua::Error::RuntimeError(format!(
            "unknown Lua scan entry root {label:?}"
        ))),
    }
}

fn decode_scan(
    root: &Path,
    content_root: Option<&Path>,
    game_id: &str,
    table: Table,
) -> mlua::Result<ScanReport> {
    let raw_mods: Table = table.get("mods")?;
    let mut mods = Vec::new();
    for entry in raw_mods.sequence_values::<Table>() {
        if mods.len() >= MAX_ITEMS {
            return Err(mlua::Error::RuntimeError(
                "Lua scan entry limit exceeded".into(),
            ));
        }
        let entry = entry?;
        mods.push(DetectedMod {
            kind: entry.get("kind")?,
            relative_path: relative_path(&entry.get::<String>("relative_path")?)?,
            root: scan_root(&entry, content_root.is_some())?,
            managed: false,
            package_id: None,
            integrity: None,
        });
    }
    mods.sort_by(|a, b| {
        a.root
            .order()
            .cmp(&b.root.order())
            .then(a.relative_path.cmp(&b.relative_path))
    });
    let raw_signals: Table = table.get("framework_signals")?;
    let mut signals = Vec::new();
    for entry in raw_signals.sequence_values::<Table>() {
        let entry = entry?;
        signals.push(FrameworkSignal {
            name: entry.get("name")?,
            relative_path: relative_path(&entry.get::<String>("relative_path")?)?
                .to_string_lossy()
                .into_owned(),
            root: scan_root(&entry, content_root.is_some())?,
            kind: entry.get("kind")?,
        });
    }
    signals.sort_by(|a, b| {
        a.root
            .order()
            .cmp(&b.root.order())
            .then(a.name.cmp(&b.name))
            .then(a.relative_path.cmp(&b.relative_path))
    });
    let scan_issues = match table.get::<Option<Table>>("scan_issues")? {
        Some(issues) if issues.raw_len() <= 4 => issues
            .sequence_values::<String>()
            .collect::<mlua::Result<Vec<_>>>()?,
        Some(_) => return Err(mlua::Error::RuntimeError("too many scan issues".into())),
        None => Vec::new(),
    };
    if scan_issues.iter().any(|code| {
        !matches!(
            code.as_str(),
            "SCAN_INCOMPLETE"
                | "SCAN_UNSAFE_ENTRY"
                | "SCAN_UNREADABLE_ENTRY"
                | "SCAN_IGNORED_GENERATED_LOG"
        )
    }) {
        return Err(mlua::Error::RuntimeError("unknown scan issue".into()));
    }
    Ok(ScanReport {
        incomplete: table.get("incomplete")?,
        scan_issues,
        game_id: game_id.into(),
        installation: root.to_owned(),
        content_root: content_root.map(Path::to_path_buf),
        redmod_available: table.get("redmod_available")?,
        mods,
        owned_files: Vec::new(),
        framework_signals: signals,
    })
}

pub(crate) fn scan(
    game_id: &str,
    root: &Path,
    content_root: Option<&Path>,
    limit: usize,
) -> Result<ScanReport> {
    let canonical = root.canonicalize()?;
    let mut roots = vec![canonical.clone()];
    if let Some(content_root) = content_root {
        roots.push(content_root.to_path_buf());
    }
    evaluate_with_profile(game_id, &roots, ReadProfile::Scan, &[], |_, policy| {
        let scan: Function = policy.get("scan")?;
        let report: Table = scan.call(limit.min(MAX_ITEMS))?;
        decode_scan(&canonical, content_root, game_id, report)
    })
}

pub(crate) fn diagnose(report: &ScanReport) -> Result<Vec<Diagnostic>> {
    evaluate(&report.game_id, &[], |lua, policy| {
        let input = lua.create_table()?;
        input.set("incomplete", report.incomplete)?;
        let issues = lua.create_table()?;
        for (index, code) in report.scan_issues.iter().enumerate() {
            issues.set(index + 1, code.as_str())?;
        }
        input.set("scan_issues", issues)?;
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
            messages.push(diagnostic(&entry)?);
        }
        Ok(messages)
    })
}

fn diagnostic(entry: &Table) -> mlua::Result<Diagnostic> {
    let action = match entry.get::<mlua::Value>("action")? {
        mlua::Value::Nil => None,
        mlua::Value::String(value) => Some(value.to_str()?.to_owned()),
        _ => {
            return Err(mlua::Error::RuntimeError(
                "diagnostic action must be a string".into(),
            ))
        }
    };
    Ok(Diagnostic {
        code: entry.get("code")?,
        severity: entry.get("severity")?,
        message: entry.get("message")?,
        action,
    })
}

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

pub(crate) fn plan(
    game_id: &str,
    root: &Path,
    inspection: &crate::archive::Inspection,
    replaced: &[String],
    appdata: Option<&Path>,
) -> Result<RequestedPlan> {
    const MAX_PROPOSALS: usize = 2_048;
    let root = root.canonicalize()?;
    let (manifest, _) = read_policy(game_id)?;
    let content_root = match &manifest.content_root {
        Some(spec) => {
            let base = appdata.context(
                "module declares content_root but no appdata base was supplied to the planner",
            )?;
            Some(crate::content_root::resolve(&root, spec, base)?)
        }
        None => None,
    };
    let declared_content = content_root.is_some();
    if inspection.files.len() > MAX_PROPOSALS {
        return Ok(RequestedPlan {
            operations: Vec::new(),
            skipped_generated_files: Vec::new(),
            blockers: vec![(
                "TOO_MANY_FILES".into(),
                format!("Read-only preview supports at most {MAX_PROPOSALS} files"),
            )],
            content_root,
        });
    }
    evaluate_hiding(game_id, &[root], replaced, |lua, policy| {
        let input = lua.create_table()?;
        for (index, file) in inspection.files.iter().enumerate() {
            let entry = lua.create_table()?;
            entry.set("source", file.entry.as_str())?;
            entry.set("bytes", file.bytes)?;
            input.set(index + 1, entry)?;
        }
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
                    root: match value.get::<Option<String>>("root")? {
                        None => crate::state::RootKind::Game,
                        Some(root) if root == "game" => crate::state::RootKind::Game,
                        Some(root) if root == "content" => crate::state::RootKind::Content,
                        Some(root) => {
                            return Err(mlua::Error::RuntimeError(format!(
                                "unknown operation root {root:?}"
                            )))
                        }
                    },
                    owned_directory: match value.get::<Option<String>>("owned_directory")? {
                        Some(root) if root.len() <= 1024 => Some(root),
                        Some(_) => {
                            return Err(mlua::Error::RuntimeError(
                                "directory claim exceeds 1024 bytes".into(),
                            ))
                        }
                        None => None,
                    },
                })
            })
            .collect::<mlua::Result<Vec<_>>>()?;
        let mut blockers = issues
            .sequence_values::<Table>()
            .map(|value| {
                let value = value?;
                Ok((
                    plan_field(&value, "code", 64, &mut result_budget)?,
                    plan_field(&value, "message", 512, &mut result_budget)?,
                ))
            })
            .collect::<mlua::Result<Vec<_>>>()?;
        if operations
            .iter()
            .any(|operation| operation.root == crate::state::RootKind::Content)
            && !declared_content
        {
            blockers.push((
                "CONTENT_ROOT_UNDECLARED".into(),
                "module did not declare content_root for content destinations".into(),
            ));
        }
        let skipped_generated_files = match output
            .get::<Option<Table>>("skipped_generated_files")?
        {
            Some(items) if items.raw_len() <= MAX_PROPOSALS => items
                .sequence_values::<mlua::LuaString>()
                .map(|item| {
                    let value = item?;
                    if value.as_bytes().len() > 4_096 || value.as_bytes().len() > result_budget {
                        return Err(mlua::Error::RuntimeError(
                            "generated path exceeds plan budget".into(),
                        ));
                    }
                    result_budget -= value.as_bytes().len();
                    Ok(value.to_str()?.to_string())
                })
                .collect::<mlua::Result<Vec<_>>>()?,
            Some(_) => {
                return Err(mlua::Error::RuntimeError(
                    "too many generated output paths".into(),
                ))
            }
            None => Vec::new(),
        };
        Ok(RequestedPlan {
            operations,
            blockers,
            skipped_generated_files,
            content_root,
        })
    })
}

#[cfg(test)]
mod diagnostic_tests {
    use super::diagnostic;

    #[test]
    fn rejects_non_string_action() {
        let lua = mlua::Lua::new();
        for action in ["{}", "42", "true"] {
            let entry: mlua::Table = lua
                .load(format!("return {{code = 'TEST', severity = 'warning', message = 'test', action = {action}}}"))
                .eval()
                .expect("valid Lua fixture");
            assert!(diagnostic(&entry).is_err());
        }
    }
}
