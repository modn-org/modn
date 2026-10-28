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
}

#[derive(Debug, Serialize)]
pub struct ScanReport {
    pub game_id: String,
    pub installation: PathBuf,
    pub redmod_available: bool,
    pub mods: Vec<DetectedMod>,
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
        });
    }
}

fn is_real_directory(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_dir() && !metadata.file_type().is_symlink())
        .unwrap_or(false)
}

fn collect_files(
    root: &Path,
    folder: &str,
    extensions: &[&str],
    kind: &str,
    mods: &mut Vec<DetectedMod>,
    budget: &mut usize,
) {
    let mut pending = vec![root.join(folder)];
    while let Some(directory) = pending.pop() {
        if !is_real_directory(&directory) {
            continue;
        }
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            if *budget == 0 {
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
                pending.push(entry.path());
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
) {
    let folder = root.join(folder);
    if !is_real_directory(&folder) {
        return;
    }
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            push_mod(root, &entry.path(), kind, mods);
        }
    }
}

/// Inventory only. Never assumes ownership of detected content or modifies game files.
pub fn scan(installation: &Path) -> ScanReport {
    let mut mods = Vec::new();
    let mut budget = MAX_ENTRIES;
    collect_files(
        installation,
        "archive/pc/mod",
        &["archive"],
        "legacy-archive",
        &mut mods,
        &mut budget,
    );
    collect_files(
        installation,
        "r6/scripts",
        &["reds"],
        "redscript",
        &mut mods,
        &mut budget,
    );
    collect_files(
        installation,
        "r6/tweaks",
        &["yaml", "yml"],
        "tweak",
        &mut mods,
        &mut budget,
    );
    collect_directories(
        installation,
        "mods",
        "redmod-directory",
        &mut mods,
        &mut budget,
    );
    collect_directories(
        installation,
        "red4ext/plugins",
        "red4ext-plugin",
        &mut mods,
        &mut budget,
    );
    collect_directories(
        installation,
        "bin/x64/plugins/cyber_engine_tweaks/mods",
        "cet-mod",
        &mut mods,
        &mut budget,
    );
    mods.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    ScanReport {
        game_id: CYBERPUNK_ID.into(),
        installation: installation.to_owned(),
        redmod_available: installation.join("tools/redmod/bin/redMod.exe").is_file(),
        mods,
    }
}

pub fn doctor(installation: &Path) -> DoctorReport {
    let report = scan(installation);
    let mut diagnostics = Vec::new();
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
    if !report.mods.is_empty() {
        diagnostics.push(Diagnostic {
            code: "UNMANAGED_CONTENT".into(),
            severity: "warning".into(),
            message: "Detected mods are not owned by this manager; no changes will be made.".into(),
        });
    }
    DoctorReport {
        scan: report,
        diagnostics,
    }
}
