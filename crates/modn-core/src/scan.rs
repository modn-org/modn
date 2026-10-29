use crate::CYBERPUNK_ID;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_ENTRIES: usize = 20_000;

/// Detected mod entry and optional ownership attribution.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct DetectedMod {
    /// Detected layout category.
    pub kind: String,
    /// Game-relative file or directory path.
    pub relative_path: PathBuf,
    /// Whether a recorded manager-owned file matched.
    pub managed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Owning package ID, if managed.
    pub package_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Recorded owned-file integrity status, if available.
    pub integrity: Option<String>,
}

/// Read-only inventory of a game installation.
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct ScanReport {
    /// Whether an entry limit truncated the scan.
    pub incomplete: bool,
    /// Identifier of the scanned game.
    pub game_id: String,
    /// Scanned game installation path.
    pub installation: PathBuf,
    /// Whether REDmod tooling was found.
    pub redmod_available: bool,
    /// Detected mod content and ownership attribution.
    pub mods: Vec<DetectedMod>,
    /// Owned-file checks, including absent files.
    pub owned_files: Vec<OwnedFileCheck>,
    /// Filesystem hints for possible framework installations.
    pub framework_signals: Vec<FrameworkSignal>,
}

/// Integrity result for a manager-owned file.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct OwnedFileCheck {
    /// Package that owns the file.
    pub package_id: String,
    /// Recorded game-relative destination.
    pub relative_path: String,
    /// `intact`, `modified`, `missing`, `unsafe`, or `unreadable`.
    pub status: String,
    /// SHA-256 recorded during installation.
    pub expected_sha256: String,
    /// Size recorded during installation.
    pub expected_bytes: u64,
}

/// Presence hint for a possible mod framework, not a health check.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct FrameworkSignal {
    /// Framework name.
    pub name: String,
    /// Path supporting the presence hint.
    pub relative_path: String,
    /// Observed `file` or `directory` type.
    pub kind: String,
}

/// Non-mutating diagnostic finding.
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct Diagnostic {
    /// Machine-readable diagnostic identifier.
    pub code: String,
    /// Severity such as `info` or `warning`.
    pub severity: String,
    /// Description of the finding.
    pub message: String,
}

/// Read-only inventory and its diagnostic messages.
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct DoctorReport {
    /// Underlying inventory report.
    pub scan: ScanReport,
    /// Warnings and informational findings.
    pub diagnostics: Vec<Diagnostic>,
}

fn push_mod(root: &Path, path: &Path, kind: &str, mods: &mut Vec<DetectedMod>) {
    if let Ok(relative_path) = path.strip_prefix(root) {
        mods.push(DetectedMod {
            kind: kind.into(),
            relative_path: relative_path.to_owned(),
            managed: false,
            package_id: None,
            integrity: None,
        });
    }
}

fn is_real_directory(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_dir() && !metadata.file_type().is_symlink())
        .unwrap_or(false)
}

fn verified_folder(root: &Path, relative: &str) -> Option<PathBuf> {
    let canonical_root = root.canonicalize().ok()?;
    let mut directory = root.to_path_buf();
    for part in Path::new(relative).components() {
        let std::path::Component::Normal(segment) = part else {
            return None;
        };
        directory.push(segment);
        if !is_real_directory(&directory)
            || !directory.canonicalize().ok()?.starts_with(&canonical_root)
        {
            return None;
        }
    }
    Some(directory)
}

fn collect_files(
    root: &Path,
    folder: &str,
    extensions: &[&str],
    kind: &str,
    mods: &mut Vec<DetectedMod>,
    budget: &mut usize,
    incomplete: &mut bool,
) {
    let Some(initial) = verified_folder(root, folder) else {
        return;
    };
    let canonical_root = root
        .canonicalize()
        .expect("verified folder has canonical root");
    let mut pending = vec![initial];
    while let Some(directory) = pending.pop() {
        if !is_real_directory(&directory) {
            continue;
        }
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            if *budget == 0 {
                *incomplete = true;
                return;
            }
            *budget -= 1;
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                let path = entry.path();
                if is_real_directory(&path)
                    && path
                        .canonicalize()
                        .is_ok_and(|p| p.starts_with(&canonical_root))
                {
                    pending.push(path);
                }
            } else if file_type.is_file() {
                let path = entry.path();
                if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
                    if extensions
                        .iter()
                        .any(|candidate| extension.eq_ignore_ascii_case(candidate))
                    {
                        push_mod(root, &path, kind, mods);
                    }
                }
            }
        }
    }
}

fn collect_directories(
    root: &Path,
    folder: &str,
    kind: &str,
    mods: &mut Vec<DetectedMod>,
    budget: &mut usize,
    incomplete: &mut bool,
) {
    let Some(folder) = verified_folder(root, folder) else {
        return;
    };
    let Ok(canonical_root) = root.canonicalize() else {
        return;
    };
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        if *budget == 0 {
            *incomplete = true;
            return;
        }
        *budget -= 1;
        let path = entry.path();
        if entry.file_type().is_ok_and(|type_| type_.is_dir())
            && is_real_directory(&path)
            && path
                .canonicalize()
                .is_ok_and(|p| p.starts_with(&canonical_root))
        {
            push_mod(root, &path, kind, mods);
        }
    }
}

/// Inventory only. Never assumes ownership of detected content or modifies game files.
#[must_use]
pub fn scan(installation: &Path) -> ScanReport {
    scan_bounded(installation, MAX_ENTRIES)
}

fn scan_bounded(installation: &Path, limit: usize) -> ScanReport {
    let mut mods = Vec::new();
    let mut budget = limit;
    let mut incomplete = false;
    collect_files(
        installation,
        "archive/pc/mod",
        &["archive"],
        "legacy-archive",
        &mut mods,
        &mut budget,
        &mut incomplete,
    );
    collect_files(
        installation,
        "r6/scripts",
        &["reds"],
        "redscript",
        &mut mods,
        &mut budget,
        &mut incomplete,
    );
    collect_files(
        installation,
        "r6/tweaks",
        &["yaml", "yml"],
        "tweak",
        &mut mods,
        &mut budget,
        &mut incomplete,
    );
    collect_directories(
        installation,
        "mods",
        "redmod-directory",
        &mut mods,
        &mut budget,
        &mut incomplete,
    );
    collect_directories(
        installation,
        "red4ext/plugins",
        "red4ext-plugin",
        &mut mods,
        &mut budget,
        &mut incomplete,
    );
    collect_directories(
        installation,
        "bin/x64/plugins/cyber_engine_tweaks/mods",
        "cet-mod",
        &mut mods,
        &mut budget,
        &mut incomplete,
    );
    mods.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    ScanReport {
        incomplete,
        game_id: CYBERPUNK_ID.into(),
        installation: installation.to_owned(),
        redmod_available: installation.join("tools/redmod/bin/redMod.exe").is_file(),
        mods,
        owned_files: Vec::new(),
        framework_signals: Vec::new(),
    }
}

/// Produce read-only diagnostics for a scanned game installation.
#[must_use]
pub fn doctor(installation: &Path) -> DoctorReport {
    diagnose(scan(installation))
}

pub(crate) fn diagnose(report: ScanReport) -> DoctorReport {
    let mut diagnostics = Vec::new();
    if report.incomplete {
        diagnostics.push(Diagnostic {
            code: "SCAN_INCOMPLETE".into(),
            severity: "warning".into(),
            message: "Inventory is incomplete because the scan entry limit was reached.".into(),
        });
    }
    if !report.redmod_available {
        diagnostics.push(Diagnostic {
            code: "REDMOD_NOT_FOUND".into(),
            severity: "info".into(),
            message: "REDmod tools were not detected. REDmod deployment is unavailable.".into(),
        });
    }
    if !report.redmod_available
        && report
            .mods
            .iter()
            .any(|entry| entry.kind == "redmod-directory")
    {
        diagnostics.push(Diagnostic {
            code: "REDMOD_CONTENT_WITHOUT_TOOLS".into(),
            severity: "warning".into(),
            message: "REDmod-style directories exist, but redMod.exe was not found.".into(),
        });
    }
    if report.mods.iter().any(|entry| !entry.managed) {
        diagnostics.push(Diagnostic {
            code: "UNMANAGED_CONTENT".into(),
            severity: "warning".into(),
            message: "Some detected mods are not owned by this manager; no changes will be made."
                .into(),
        });
    }
    for (status, code) in [
        ("missing", "OWNED_FILE_MISSING"),
        ("modified", "OWNED_FILE_MODIFIED"),
        ("unsafe", "OWNED_FILE_UNSAFE"),
        ("unreadable", "OWNED_FILE_UNREADABLE"),
    ] {
        let count = report
            .owned_files
            .iter()
            .filter(|file| file.status == status)
            .count();
        if count > 0 {
            diagnostics.push(Diagnostic {
                code: code.into(),
                severity: "warning".into(),
                message: format!("{count} manager-owned file(s) are {status}; inspect scan ownership details before uninstalling."),
            });
        }
    }
    DoctorReport {
        scan: report,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn exhausted_budget_is_reported() {
        let game = tempdir().unwrap();
        let dir = game.path().join("archive/pc/mod");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("unrelated.txt"), "fixture").unwrap();
        fs::create_dir_all(game.path().join("mods/ActualRedmod")).unwrap();
        let report = scan_bounded(game.path(), 1);
        assert!(report.incomplete);
        assert!(report.mods.is_empty());
        assert!(diagnose(report)
            .diagnostics
            .iter()
            .any(|d| d.code == "SCAN_INCOMPLETE"));
    }

    #[test]
    fn exact_entry_limit_is_not_truncation() {
        let game = tempdir().unwrap();
        let dir = game.path().join("archive/pc/mod");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("one.archive"), "fixture").unwrap();
        let report = scan_bounded(game.path(), 1);
        assert!(!report.incomplete);
        assert_eq!(report.mods.len(), 1);
        assert!(!diagnose(report)
            .diagnostics
            .iter()
            .any(|d| d.code == "SCAN_INCOMPLETE"));
    }

    #[test]
    fn linked_ancestor_is_excluded() {
        let game = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::create_dir_all(outside.path().join("pc/mod")).unwrap();
        fs::write(outside.path().join("pc/mod/external.archive"), "fixture").unwrap();
        #[cfg(windows)]
        {
            let link = game.path().join("archive");
            assert!(std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(outside.path())
                .status()
                .unwrap()
                .success());
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), game.path().join("archive")).unwrap();
        assert!(scan(game.path()).mods.is_empty());
    }
}
