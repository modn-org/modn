//! Deterministic, metadata-only plans. No game writes or extraction.
use crate::archive::Inspection;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize)]
pub struct PlannedFile {
    pub source: String,
    pub destination: String,
    pub bytes: u64,
    pub kind: String,
}

#[derive(Debug, Serialize)]
pub struct PlanIssue {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct InstallPlan {
    pub archive_sha256: String,
    pub game_installation: PathBuf,
    pub status: String,
    pub metadata_only: bool,
    pub operations: Vec<PlannedFile>,
    pub blockers: Vec<PlanIssue>,
}

fn classification(path: &str) -> Option<&'static str> {
    let segments: Vec<_> = path.split('/').collect();
    let lower: Vec<_> = segments.iter().map(|s| s.to_ascii_lowercase()).collect();
    match lower.as_slice() {
        [a, b, c, file]
            if a == "archive" && b == "pc" && c == "mod" && file.ends_with(".archive") =>
        {
            Some("legacy-archive")
        }
        [first, mod_name, ..] if first == "mods" && !mod_name.is_empty() && segments.len() >= 3 => {
            Some("redmod")
        }
        [a, b, ..] if a == "r6" && b == "scripts" && segments.len() >= 3 => Some("redscript"),
        [a, b, ..] if a == "r6" && b == "tweaks" && segments.len() >= 3 => Some("tweak"),
        [a, b, c, d, e, mod_name, ..]
            if a == "bin"
                && b == "x64"
                && c == "plugins"
                && d == "cyber_engine_tweaks"
                && e == "mods"
                && !mod_name.is_empty()
                && segments.len() >= 7 =>
        {
            Some("cet")
        }
        [a, b, mod_name, ..]
            if a == "red4ext" && b == "plugins" && !mod_name.is_empty() && segments.len() >= 4 =>
        {
            Some("red4ext")
        }
        _ => None,
    }
}

fn is_documentation(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or("").to_ascii_lowercase();
    name.starts_with("readme")
        || name.starts_with("license")
        || name.starts_with("changelog")
        || name == "description.txt"
}

fn prohibited_payload(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [
        ".exe", ".msi", ".bat", ".cmd", ".ps1", ".vbs", ".lnk", ".url",
    ]
    .iter()
    .any(|suffix| lower.ends_with(suffix))
}

fn existing_destination_issue(game: &Path, target: &str) -> Option<&'static str> {
    let mut current = game.to_path_buf();
    let canonical_game = game.canonicalize().ok()?;
    let components: Vec<_> = target.split('/').collect();
    for (index, component) in components.iter().enumerate() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if index + 1 < components.len() && !metadata.is_dir() {
                    return Some("NON_DIRECTORY_ANCESTOR");
                }
                if metadata.file_type().is_symlink()
                    || !current
                        .canonicalize()
                        .is_ok_and(|resolved| resolved.starts_with(&canonical_game))
                {
                    return Some("LINKED_DESTINATION");
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => return Some("DESTINATION_UNREADABLE"),
        }
    }
    if fs::symlink_metadata(&current).is_ok() || game.join(target).exists() {
        Some("EXISTING_FILE")
    } else {
        None
    }
}

fn blocker(issues: &mut Vec<PlanIssue>, code: &str, message: impl Into<String>) {
    issues.push(PlanIssue {
        code: code.into(),
        message: message.into(),
    });
}

pub fn plan(inspection: &Inspection, game: &Path) -> InstallPlan {
    let mut operations = Vec::new();
    let mut blockers = Vec::new();
    let mut destinations = HashSet::new();
    let mut redmods: HashMap<String, bool> = HashMap::new();
    let mut wrappers = HashSet::new();

    for file in &inspection.files {
        if prohibited_payload(&file.entry) {
            blocker(
                &mut blockers,
                "EXECUTABLE_PAYLOAD",
                format!(
                    "Executable installer or shortcut is not allowed: {}",
                    file.entry
                ),
            );
            continue;
        }
        if is_documentation(&file.entry) {
            continue;
        }
        let mut mapped = None;
        let segments: Vec<_> = file.entry.split('/').collect();
        for start in 0..segments.len() {
            let suffix = segments[start..].join("/");
            if let Some(kind) = classification(&suffix) {
                mapped = Some((segments[..start].join("/"), suffix, kind));
                break;
            }
        }
        let Some((wrapper, destination, kind)) = mapped else {
            blocker(
                &mut blockers,
                "UNRECOGNIZED_LAYOUT",
                format!(
                    "Cannot map {} to a known Cyberpunk mod directory",
                    file.entry
                ),
            );
            continue;
        };
        wrappers.insert(wrapper);
        if prohibited_payload(&destination) {
            blocker(
                &mut blockers,
                "EXECUTABLE_PAYLOAD",
                format!(
                    "Executable installer or shortcut is not allowed: {}",
                    file.entry
                ),
            );
            continue;
        }
        if !destinations.insert(destination.to_lowercase()) {
            blocker(
                &mut blockers,
                "DUPLICATE_DESTINATION",
                format!("Two archive entries target {destination}"),
            );
            continue;
        }
        if let Some(code) = existing_destination_issue(game, &destination) {
            blocker(
                &mut blockers,
                code,
                format!("Destination is occupied or unsafe: {destination}"),
            );
        }
        if kind == "redmod" {
            let mod_name = destination.split('/').nth(1).unwrap_or("");
            let info = destination.ends_with("/info.json");
            redmods
                .entry(mod_name.to_ascii_lowercase())
                .and_modify(|v| *v |= info)
                .or_insert(info);
        }
        operations.push(PlannedFile {
            source: file.entry.clone(),
            destination,
            bytes: file.bytes,
            kind: kind.into(),
        });
    }
    if wrappers.len() > 1 {
        blocker(
            &mut blockers,
            "MULTIPLE_PACKAGE_ROOTS",
            "Multiple install roots or variants found; select a single package variant",
        );
    }
    if operations.is_empty() {
        blocker(
            &mut blockers,
            "NO_INSTALLABLE_FILES",
            "No recognized Cyberpunk mod payloads were found",
        );
    }
    for (name, has_info) in &redmods {
        if !has_info {
            blocker(
                &mut blockers,
                "REDMOD_INFO_MISSING",
                format!("REDmod package {name} has no info.json"),
            );
        }
    }
    if !redmods.is_empty() && !game.join("tools/redmod/bin/redMod.exe").is_file() {
        blocker(
            &mut blockers,
            "REDMOD_NOT_INSTALLED",
            "REDmod deployment tools are not installed",
        );
    }

    InstallPlan {
        archive_sha256: inspection.sha256.clone(),
        game_installation: game.to_path_buf(),
        status: if blockers.is_empty() {
            "reviewable"
        } else {
            "blocked"
        }
        .into(),
        metadata_only: true,
        operations,
        blockers,
    }
}
