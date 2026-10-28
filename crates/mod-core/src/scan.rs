use crate::CYBERPUNK_ID;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_ENTRIES: usize = 20_000;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct DetectedMod {
    pub kind: String,
    pub relative_path: PathBuf,
    pub managed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrity: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ScanReport {
    pub incomplete: bool,
    pub game_id: String,
    pub installation: PathBuf,
    pub redmod_available: bool,
    pub mods: Vec<DetectedMod>,
    pub owned_files: Vec<OwnedFileCheck>,
    pub framework_signals: Vec<FrameworkSignal>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct OwnedFileCheck {
    pub package_id: String,
    pub relative_path: String,
    pub status: String,
    pub expected_sha256: String,
    pub expected_bytes: u64,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct FrameworkSignal {
    pub name: String,
    pub relative_path: String,
    pub kind: String,
}

#[derive(Debug, Serialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub scan: ScanReport,
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
                .is_ok_and(|p| p.starts_with(root.canonicalize().unwrap()))
        {
            push_mod(root, &path, kind, mods);
        }
    }
}

/// Inventory only. Never assumes ownership of detected content or modifies game files.
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
