use crate::game_modules::policy;
use anyhow::Result;
use serde::Serialize;
use std::fmt;
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
    pub integrity: Option<IntegrityStatus>,
}

/// Read-only inventory of a game installation.
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct ScanReport {
    /// Whether some candidate mod content could not be inventoried.
    pub incomplete: bool,
    /// Bounded reasons for skipped, unsafe, or truncated directory entries.
    pub scan_issues: Vec<String>,
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

/// Integrity of a manager-owned file, as observed on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum IntegrityStatus {
    /// Size and SHA-256 match the installation record.
    Intact,
    /// Covered by a verified exclusive directory; contents are not hashed.
    Present,
    /// Present but with a different size or digest.
    Modified,
    /// Not present at its recorded path.
    Missing,
    /// Linked, redirected or not the expected kind of filesystem entry.
    Unsafe,
    /// Could not be read to verify.
    Unreadable,
}

impl IntegrityStatus {
    /// Serialized name, also passed to Lua doctor policies.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Intact => "intact",
            Self::Present => "present",
            Self::Modified => "modified",
            Self::Missing => "missing",
            Self::Unsafe => "unsafe",
            Self::Unreadable => "unreadable",
        }
    }

    /// Whether the recorded content is still in place: `intact` files or
    /// files inside a verified exclusive directory.
    #[must_use]
    pub fn is_in_place(self) -> bool {
        matches!(self, Self::Intact | Self::Present)
    }
}

impl fmt::Display for IntegrityStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Integrity result for a manager-owned file.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct OwnedFileCheck {
    /// Package that owns the file.
    pub package_id: String,
    /// Recorded game-relative destination.
    pub relative_path: String,
    /// Observed integrity of the recorded file.
    pub status: IntegrityStatus,
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
    /// Folders Modn created for its packages, removed when empty. Empty
    /// when the report was made without manager state.
    pub created_directories: Vec<String>,
}

/// Scan using a runtime-selected game's Lua module.
#[must_use]
pub fn scan_for(game_id: &str, installation: &Path) -> ScanReport {
    scan_bounded_for(game_id, installation, MAX_ENTRIES)
}

fn scan_bounded_for(game_id: &str, installation: &Path, limit: usize) -> ScanReport {
    policy::scan(game_id, installation, limit).unwrap_or_else(|_| ScanReport {
        incomplete: true,
        scan_issues: vec!["SCAN_UNREADABLE_ENTRY".into()],
        game_id: game_id.into(),
        installation: installation.to_owned(),
        redmod_available: false,
        mods: Vec::new(),
        owned_files: Vec::new(),
        framework_signals: Vec::new(),
    })
}

/// Diagnose a runtime-selected game's read-only inventory.
#[must_use]
pub fn doctor_for(game_id: &str, installation: &Path) -> DoctorReport {
    diagnose(scan_for(game_id, installation))
}

pub(crate) fn diagnose_checked(report: ScanReport) -> Result<DoctorReport> {
    let diagnostics = policy::diagnose(&report)?;
    Ok(DoctorReport {
        scan: report,
        diagnostics,
        created_directories: Vec::new(),
    })
}

pub(crate) fn diagnose(report: ScanReport) -> DoctorReport {
    match policy::diagnose(&report) {
        Ok(diagnostics) => DoctorReport {
            scan: report,
            diagnostics,
            created_directories: Vec::new(),
        },
        Err(_) => DoctorReport {
            scan: ScanReport {
                incomplete: true,
                ..report
            },
            diagnostics: vec![Diagnostic {
                code: "MODULE_POLICY_ERROR".into(),
                severity: "warning".into(),
                message: "Game diagnostic policy failed; scan results are incomplete.".into(),
            }],
            created_directories: Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn exhausted_budget_is_reported() {
        let game = tempdir().unwrap();
        let dir = game.path().join("archive/pc/mod");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("unrelated.txt"), "fixture").unwrap();
        fs::create_dir_all(game.path().join("mods/ActualRedmod")).unwrap();
        let report = scan_bounded_for("cyberpunk-2077", game.path(), 1);
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
        let report = scan_bounded_for("cyberpunk-2077", game.path(), 1);
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
        assert!(scan_for("cyberpunk-2077", game.path()).mods.is_empty());
    }
}
