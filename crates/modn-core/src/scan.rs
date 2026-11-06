use crate::content_root::{self, ContentRootHealth};
use crate::game_modules::policy;
use anyhow::Result;
use serde::Serialize;
use std::fmt;
use std::path::{Path, PathBuf};

const MAX_ENTRIES: usize = 20_000;

/// Installation root that contains a scan result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum ScanRoot {
    /// The selected game installation.
    Game,
    /// The module-declared content root outside the game installation.
    Content,
}

impl ScanRoot {
    /// Stable label used in text and JSON output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Game => "game",
            Self::Content => "content",
        }
    }

    /// Parse a module-provided root label.
    #[must_use]
    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "game" => Some(Self::Game),
            "content" => Some(Self::Content),
            _ => None,
        }
    }

    const fn is_game(&self) -> bool {
        matches!(self, Self::Game)
    }

    pub(crate) const fn order(self) -> u8 {
        match self {
            Self::Game => 0,
            Self::Content => 1,
        }
    }
}

impl fmt::Display for ScanRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Detected mod entry and optional ownership attribution.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Serialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct DetectedMod {
    pub kind: String,
    /// Root-relative file or directory path.
    pub relative_path: PathBuf,
    /// Installation root that contains this entry.
    #[serde(skip_serializing_if = "ScanRoot::is_game")]
    pub root: ScanRoot,
    /// Whether a recorded manager-owned file matched.
    pub managed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrity: Option<IntegrityStatus>,
}

/// Read-only inventory of a game installation.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct ScanReport {
    /// Whether some candidate mod content could not be inventoried.
    pub incomplete: bool,
    /// Bounded reasons for skipped, unsafe, or truncated directory entries.
    pub scan_issues: Vec<String>,
    pub game_id: String,
    pub installation: PathBuf,
    /// Declared content root, when the module has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_root: Option<PathBuf>,
    pub redmod_available: bool,
    pub mods: Vec<DetectedMod>,
    /// Owned-file checks, including absent files.
    pub owned_files: Vec<OwnedFileCheck>,
    /// Filesystem hints for possible framework installations.
    pub framework_signals: Vec<FrameworkSignal>,
}

/// Integrity of a manager-owned file, as observed on disk.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
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
    Missing,
    /// Linked, redirected or not the expected kind of filesystem entry.
    Unsafe,
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
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Serialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct OwnedFileCheck {
    pub package_id: String,
    /// Installation root that contains the recorded destination.
    #[serde(skip_serializing_if = "ScanRoot::is_game")]
    pub root: ScanRoot,
    /// Recorded root-relative destination.
    pub relative_path: String,
    pub status: IntegrityStatus,
    /// SHA-256 recorded during installation.
    pub expected_sha256: String,
    /// Size recorded during installation.
    pub expected_bytes: u64,
}

/// Presence hint for a possible mod framework, not a health check.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Serialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct FrameworkSignal {
    pub name: String,
    /// Root-relative path supporting the presence hint.
    pub relative_path: String,
    /// Installation root that contains this signal.
    #[serde(skip_serializing_if = "ScanRoot::is_game")]
    pub root: ScanRoot,
    /// Observed `file` or `directory` type.
    pub kind: String,
}

/// Non-mutating diagnostic finding.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct Diagnostic {
    pub code: String,
    /// Severity such as `info` or `warning`.
    pub severity: String,
    pub message: String,
    /// Recommended next safe action, when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

/// Read-only inventory and its diagnostic messages.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct DoctorReport {
    pub scan: ScanReport,
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
    scan_bounded_with_health(game_id, installation, None, limit).0
}

fn scan_bounded_with_health(
    game_id: &str,
    installation: &Path,
    appdata: Option<&Path>,
    limit: usize,
) -> (ScanReport, Option<ContentRootHealth>) {
    let health = crate::game_modules::content_root_spec(game_id)
        .ok()
        .flatten()
        .and_then(|spec| {
            let base = appdata
                .map(Path::to_path_buf)
                .or_else(|| content_root::appdata_directory().ok())?;
            Some(content_root::inspect(installation, &spec, &base, None))
        });
    let content = health.as_ref().and_then(|health| health.root.as_deref());
    let content_path = health.as_ref().map(|health| health.path.clone());
    let report =
        policy::scan(game_id, installation, content, limit).unwrap_or_else(|_| ScanReport {
            incomplete: true,
            scan_issues: vec!["SCAN_UNREADABLE_ENTRY".into()],
            game_id: game_id.into(),
            installation: installation.to_owned(),
            content_root: content_path.clone(),
            redmod_available: false,
            mods: Vec::new(),
            owned_files: Vec::new(),
            framework_signals: Vec::new(),
        });
    let mut report = report;
    if report.content_root.is_none() {
        report.content_root = content_path;
    }
    (report, health)
}

/// Diagnose a runtime-selected game's read-only inventory.
#[must_use]
pub fn doctor_for(game_id: &str, installation: &Path) -> DoctorReport {
    let (report, health) = scan_bounded_with_health(game_id, installation, None, MAX_ENTRIES);
    let mut doctor = diagnose(report);
    doctor
        .diagnostics
        .extend(content_root_diagnostics(health.as_ref()));
    doctor
}

/// Diagnostics for a content root that cannot be read safely.
pub(crate) fn content_root_diagnostics(health: Option<&ContentRootHealth>) -> Vec<Diagnostic> {
    health
        .and_then(ContentRootHealth::diagnostic)
        .map(|(code, message, action)| Diagnostic {
            code: code.into(),
            severity: "warning".into(),
            message,
            action: Some(action),
        })
        .into_iter()
        .collect()
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
                action: None,
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
        crate::test_support::seeded(|| {
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
        });
    }

    #[test]
    fn exact_entry_limit_is_not_truncation() {
        crate::test_support::seeded(|| {
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
        });
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
