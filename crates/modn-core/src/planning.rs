//! Deterministic, metadata-only plans. No game writes or extraction.
use crate::archive::Inspection;
use serde::Serialize;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Proposed metadata-only file operation.
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct PlannedFile {
    /// Entry path inside the archive.
    pub source: String,
    /// Game-relative destination path.
    pub destination: String,
    /// Declared uncompressed size.
    pub bytes: u64,
    /// Recognized mod layout category.
    pub kind: String,
}

/// Blocker that prevents a proposed installation.
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct PlanIssue {
    /// Stable machine-readable issue code.
    pub code: String,
    /// Human-readable reason for the blocker.
    pub message: String,
}

/// Read-only installation proposal; not write authorization.
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct InstallPlan {
    /// Archive hash observed during inspection.
    pub archive_sha256: String,
    /// Target game directory.
    pub game_installation: PathBuf,
    /// `reviewable` or `blocked`.
    pub status: String,
    /// True: file contents have not been verified.
    pub metadata_only: bool,
    /// File operations proposed for review.
    pub operations: Vec<PlannedFile>,
    /// Issues requiring resolution before installation.
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
    let (stem, extension) = name.rsplit_once('.').unwrap_or((&name, ""));
    name == "license"
        || name == "copying"
        || name == "readme"
        || name == "changelog"
        || (matches!(extension, "md" | "txt" | "rst")
            && (stem.starts_with("readme")
                || stem.starts_with("license")
                || stem.starts_with("changelog")
                || name == "description.txt"))
}

pub(crate) fn prohibited_payload(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [
        ".exe", ".msi", ".bat", ".cmd", ".ps1", ".vbs", ".lnk", ".url",
    ]
    .iter()
    .any(|suffix| lower.ends_with(suffix))
}
pub(crate) fn existing_destination_issue(game: &Path, target: &str) -> Option<&'static str> {
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
pub(crate) fn insert_destination(
    targets: &mut BTreeSet<String>,
    destination: &str,
) -> Option<&'static str> {
    let key = destination.to_lowercase();
    let mut ancestor = key.as_str();
    while let Some((parent, _)) = ancestor.rsplit_once('/') {
        if targets.contains(parent) {
            return Some("FILE_DIRECTORY_COLLISION");
        }
        ancestor = parent;
    }
    let prefix = format!("{key}/");
    if targets
        .range(prefix.clone()..)
        .next()
        .is_some_and(|v| v.starts_with(&prefix))
    {
        return Some("FILE_DIRECTORY_COLLISION");
    }
    if !targets.insert(key) {
        return Some("DUPLICATE_DESTINATION");
    }
    None
}

fn blocker(issues: &mut Vec<PlanIssue>, code: &str, message: impl Into<String>) {
    issues.push(PlanIssue {
        code: code.into(),
        message: message.into(),
    });
}

/// Classify archive entries and produce a metadata-only plan for review.
#[must_use]
pub fn plan(inspection: &Inspection, game: &Path) -> InstallPlan {
    let mut operations = Vec::new();
    let mut blockers = Vec::new();
    let mut destinations: BTreeSet<String> = BTreeSet::new();
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
            if is_documentation(&file.entry) {
                continue;
            }
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
        if let Some(code) = insert_destination(&mut destinations, &destination) {
            blocker(
                &mut blockers,
                code,
                format!("Duplicate or overlapping target: {destination}"),
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
            let parts: Vec<_> = destination.split('/').collect();
            let info = parts.len() == 3 && parts[2].eq_ignore_ascii_case("info.json");
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
// Untrusted, versioned Luau file mapping. It carries no installation authority.
pub(crate) struct RequestedFile {
    pub(crate) source: String,
    pub(crate) destination: String,
    pub(crate) kind: String,
    pub(crate) package_root: String,
}
pub(crate) struct RequestedPlan {
    pub(crate) operations: Vec<RequestedFile>,
    pub(crate) blockers: Vec<(String, String)>,
}
fn issue(issues: &mut Vec<PlanIssue>, code: &str, value: impl Into<String>) {
    issues.push(PlanIssue {
        code: code.into(),
        message: value.into(),
    });
}
/// Resolve untrusted module mappings into a read-only preview. This result
/// never authorizes execution: installs require fresh validation and approval.
pub fn resolve(game_id: &str, inspection: &Inspection, game: &Path) -> anyhow::Result<InstallPlan> {
    use anyhow::bail;
    if !fs::symlink_metadata(game)?.file_type().is_dir() {
        bail!("installation root must be a real directory, not a link");
    }
    let game = game.canonicalize()?;
    let proposed = crate::game_modules::policy::plan(game_id, &game, inspection)?;
    let mut blockers = Vec::new();
    let mut sources = HashMap::new();
    for entry in &inspection.files {
        sources.insert(entry.entry.as_str(), entry.bytes);
        if crate::archive::validated_name(&entry.entry).is_err() {
            issue(
                &mut blockers,
                "UNSAFE_SOURCE",
                format!("Unsafe archive entry: {}", entry.entry),
            );
        }
        if prohibited_payload(&entry.entry) {
            issue(
                &mut blockers,
                "EXECUTABLE_PAYLOAD",
                format!("Executable payload: {}", entry.entry),
            );
        }
    }
    for (code, message) in proposed.blockers {
        if code.len() > 64
            || code.is_empty()
            || message.len() > 512
            || !code.bytes().all(|b| b.is_ascii_uppercase() || b == b'_')
        {
            issue(
                &mut blockers,
                "INVALID_POLICY_ISSUE",
                "Invalid module issue",
            );
        } else {
            issue(&mut blockers, &code, message);
        }
    }
    let mut operations = Vec::new();
    let mut selected_sources = HashSet::new();
    let mut destinations = BTreeSet::new();
    let mut root_key: Option<String> = None;
    let mut total = 0u64;
    for request in proposed.operations {
        if request.package_root.len() > 256
            || (!request.package_root.is_empty()
                && crate::archive::validated_name(&request.package_root).is_err())
        {
            issue(
                &mut blockers,
                "INVALID_PACKAGE_ROOT",
                "Invalid source package root",
            );
            continue;
        }
        if let Some(root) = &root_key {
            if root != &request.package_root {
                issue(
                    &mut blockers,
                    "MULTIPLE_PACKAGE_ROOTS",
                    "Multiple package variants",
                );
            }
        } else {
            root_key = Some(request.package_root.clone());
        }
        if !request.package_root.is_empty()
            && !request
                .source
                .starts_with(&format!("{}/", request.package_root))
        {
            issue(
                &mut blockers,
                "INVALID_PACKAGE_ROOT",
                "Source outside package root",
            );
            continue;
        }
        if request.destination.len() > 1024
            || !crate::archive::validated_name(&request.destination)
                .is_ok_and(|normalized| normalized == request.destination)
            || !request.destination.contains('/')
        {
            issue(
                &mut blockers,
                "INVALID_DESTINATION",
                format!("Unsafe target: {}", request.destination),
            );
            continue;
        }
        if prohibited_payload(&request.destination) {
            issue(
                &mut blockers,
                "EXECUTABLE_PAYLOAD",
                format!("Executable target: {}", request.destination),
            );
            continue;
        }
        if request.kind.is_empty()
            || request.kind.len() > 64
            || !request
                .kind
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            issue(&mut blockers, "INVALID_KIND", "Invalid operation kind");
            continue;
        }
        let Some(&bytes) = sources.get(request.source.as_str()) else {
            issue(
                &mut blockers,
                "UNKNOWN_SOURCE",
                format!("Unknown source: {}", request.source),
            );
            continue;
        };
        if !selected_sources.insert(request.source.clone()) {
            issue(
                &mut blockers,
                "DUPLICATE_SOURCE",
                format!("Repeated source: {}", request.source),
            );
            continue;
        }
        if bytes > 4 * 1024 * 1024 * 1024 {
            issue(&mut blockers, "FILE_TOO_LARGE", "File exceeds 4 GiB");
        }
        total = total.saturating_add(bytes);
        if total > 8 * 1024 * 1024 * 1024 {
            issue(&mut blockers, "TOTAL_TOO_LARGE", "Total exceeds 8 GiB");
        }
        if let Some(code) = insert_destination(&mut destinations, &request.destination) {
            issue(
                &mut blockers,
                code,
                format!("Duplicate or overlapping target: {}", request.destination),
            );
            continue;
        }
        if let Some(code) = existing_destination_issue(&game, &request.destination) {
            issue(
                &mut blockers,
                code,
                format!("Occupied or unsafe target: {}", request.destination),
            );
        }
        operations.push(PlannedFile {
            source: request.source,
            destination: request.destination,
            bytes,
            kind: request.kind,
        });
    }
    if operations.is_empty() {
        issue(&mut blockers, "NO_INSTALLABLE_FILES", "No files selected");
    }
    if crate::archive::inspect(&inspection.archive)
        .map_or(true, |current| current.sha256 != inspection.sha256)
    {
        issue(
            &mut blockers,
            "SOURCE_CHANGED",
            "Source archive changed during resolution",
        );
    }
    Ok(InstallPlan {
        archive_sha256: inspection.sha256.clone(),
        game_installation: game,
        status: if blockers.is_empty() {
            "reviewable"
        } else {
            "blocked"
        }
        .into(),
        metadata_only: true,
        operations,
        blockers,
    })
}
