//! Deterministic, metadata-only plans. No game writes or extraction.
use crate::archive::Inspection;
use serde::Serialize;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// Proposed metadata-only file operation.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct PlannedFile {
    /// Entry path inside the archive.
    pub source: String,
    /// Game-relative destination path.
    pub destination: String,
    pub bytes: u64,
    /// Recognized mod layout category.
    pub kind: String,
    /// Exclusive, newly created subdirectory owned as a unit, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owned_directory: Option<String>,
}

/// Blocker that prevents a proposed installation.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct PlanIssue {
    /// Stable machine-readable issue code.
    pub code: String,
    pub message: String,
}

/// Whether a proposal may be submitted for approved installation.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum PlanStatus {
    /// No blockers; still requires explicit approval.
    Reviewable,
    Blocked,
}

impl fmt::Display for PlanStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Reviewable => "reviewable",
            Self::Blocked => "blocked",
        })
    }
}

/// Read-only installation proposal; not write authorization.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct InstallPlan {
    pub archive_sha256: String,
    pub game_installation: PathBuf,
    pub status: PlanStatus,
    /// True: file contents have not been verified.
    pub metadata_only: bool,
    pub operations: Vec<PlannedFile>,
    pub blockers: Vec<PlanIssue>,
    /// Generated outputs intentionally omitted from installation and future ownership.
    pub skipped_generated_files: Vec<String>,
}

pub(crate) fn prohibited_payload(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [
        ".exe", ".com", ".scr", ".hta", ".pif", ".cpl", ".msi", ".bat", ".cmd", ".ps1", ".vbs",
        ".lnk", ".url",
    ]
    .iter()
    .any(|suffix| lower.ends_with(suffix))
}
pub(crate) fn approved_tool_executable(
    source: &str,
    destination: &str,
    approved: &[String],
) -> bool {
    destination.ends_with(".exe")
        && destination.split('/').any(|part| part == "tools")
        && approved.iter().any(|path| path == destination)
        && (source == destination || source.ends_with(&format!("/{destination}")))
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
                if metadata.file_type().is_symlink() {
                    return Some("LINKED_DESTINATION");
                }
                let Ok(resolved) = current.canonicalize() else {
                    return Some("LINKED_DESTINATION");
                };
                if !resolved.starts_with(&canonical_game) {
                    return Some("LINKED_DESTINATION");
                }
                // On Windows, a short (8.3) name can alias a long directory name
                // without matching its SQLite ownership key. Never publish a
                // new file below an ancestor addressed through a different name.
                #[cfg(windows)]
                {
                    let key = component.to_lowercase();
                    if !resolved
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|actual| actual.to_lowercase() == key)
                    {
                        return Some("ALIASED_DESTINATION");
                    }
                    if component.contains('~')
                        && !current.parent().is_some_and(|parent| {
                            fs::read_dir(parent).is_ok_and(|entries| {
                                entries.flatten().any(|entry| {
                                    entry
                                        .file_name()
                                        .to_str()
                                        .is_some_and(|actual| actual.to_lowercase() == key)
                                })
                            })
                        })
                    {
                        return Some("ALIASED_DESTINATION");
                    }
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

pub(crate) struct RequestedFile {
    pub(crate) source: String,
    pub(crate) destination: String,
    pub(crate) kind: String,
    pub(crate) package_root: String,
    pub(crate) owned_directory: Option<String>,
}
pub(crate) struct RequestedPlan {
    pub(crate) operations: Vec<RequestedFile>,
    pub(crate) blockers: Vec<(String, String)>,
    pub(crate) skipped_generated_files: Vec<String>,
}
fn issue(issues: &mut Vec<PlanIssue>, code: &str, value: impl Into<String>) {
    issues.push(PlanIssue {
        code: code.into(),
        message: value.into(),
    });
}
/// Resolve untrusted module mappings into a read-only preview. Tool
/// executable permissions must come from a trusted caller, never from Lua.
pub fn resolve(game_id: &str, inspection: &Inspection, game: &Path) -> anyhow::Result<InstallPlan> {
    resolve_with_permissions(game_id, inspection, game, &[])
}

fn resolve_with_permissions(
    game_id: &str,
    inspection: &Inspection,
    game: &Path,
    approved: &[String],
) -> anyhow::Result<InstallPlan> {
    use anyhow::bail;
    if !fs::symlink_metadata(game)?.file_type().is_dir() {
        bail!("installation root must be a real directory, not a link");
    }
    let game = game.canonicalize()?;
    let proposed = crate::game_modules::policy::plan(game_id, &game, inspection)?;
    let mut blockers = Vec::new();
    let mut sources = HashMap::new();
    let approved_sources: HashSet<_> = proposed
        .operations
        .iter()
        .filter(|operation| {
            approved_tool_executable(&operation.source, &operation.destination, approved)
        })
        .map(|operation| operation.source.as_str())
        .collect();
    for entry in &inspection.files {
        sources.insert(entry.entry.as_str(), entry.bytes);
        if crate::archive::validated_name(&entry.entry).is_err() {
            issue(
                &mut blockers,
                "UNSAFE_SOURCE",
                format!("Unsafe archive entry: {}", entry.entry),
            );
        }
        if prohibited_payload(&entry.entry) && !approved_sources.contains(entry.entry.as_str()) {
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
    let mut skipped_generated_files = Vec::new();
    let mut skipped = HashSet::new();
    for source in proposed.skipped_generated_files {
        if !sources.contains_key(source.as_str()) || !skipped.insert(source.clone()) {
            issue(
                &mut blockers,
                "INVALID_SKIPPED_SOURCE",
                "Invalid generated output omission",
            );
        } else {
            skipped_generated_files.push(source);
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
        if prohibited_payload(&request.destination)
            && !approved_tool_executable(&request.source, &request.destination, approved)
        {
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
        if skipped.contains(&request.source) {
            issue(
                &mut blockers,
                "INVALID_SKIPPED_SOURCE",
                "Source both selected and omitted",
            );
            continue;
        }
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
            owned_directory: request.owned_directory,
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
            PlanStatus::Reviewable
        } else {
            PlanStatus::Blocked
        },
        metadata_only: true,
        operations,
        blockers,
        skipped_generated_files,
    })
}

/// Produce a read-only ZIP installation preview through a selected game module.
/// A reviewable proposal is not an approval to write game files.
pub fn preview_install_zip(
    game_id: &str,
    game: &Path,
    archive: &Path,
) -> anyhow::Result<InstallPlan> {
    preview_install_zip_with_permissions(game_id, game, archive, &[])
}

/// Preview with explicit, trusted per-destination tool-executable permissions.
pub fn preview_install_zip_with_permissions(
    game_id: &str,
    game: &Path,
    archive: &Path,
    approved: &[String],
) -> anyhow::Result<InstallPlan> {
    let inspection = crate::archive::inspect(archive)?;
    let mut proposal = resolve_with_permissions(game_id, &inspection, game, approved)?;
    if !inspection.format.is_installable() {
        proposal.status = PlanStatus::Blocked;
        proposal.blockers.push(PlanIssue {
            code: "NOT_INSTALLABLE".into(),
            message: "Only ZIP and 7z archives can be installed".into(),
        });
    }
    Ok(proposal)
}
