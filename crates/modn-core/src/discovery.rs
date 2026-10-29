use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const EXECUTABLE: &str = "bin/x64/Cyberpunk2077.exe";

/// Discovered game installation and its discovery source.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_structs,
    reason = "CLI consumers construct candidates for explicit manual game paths"
)]
pub struct Candidate {
    /// Canonical game installation directory.
    pub path: PathBuf,
    /// Discovery source, such as Steam or Epic.
    pub source: String,
}

/// Canonicalize a directory and verify it contains the expected game executable.
pub fn validate_installation(path: &Path) -> Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("cannot access game directory {}", path.display()))?;
    if !canonical.join(EXECUTABLE).is_file() {
        bail!("{} does not contain {}", canonical.display(), EXECUTABLE);
    }
    Ok(canonical)
}

fn add_candidate(candidates: &mut Vec<Candidate>, path: &Path, source: &str) {
    if let Ok(path) = validate_installation(path) {
        candidates.push(Candidate {
            path,
            source: source.to_owned(),
        });
    }
}

fn vdf_value(line: &str, key: &str) -> Option<String> {
    let parts: Vec<_> = line.split('"').collect();
    (parts.len() >= 4 && parts[1].eq_ignore_ascii_case(key)).then(|| parts[3].replace("\\\\", "\\"))
}

/// Read Steam libraryfolders.vdf without assuming library indices or drive letters.
fn steam_libraries(root: &Path) -> Vec<PathBuf> {
    let mut libraries = vec![root.to_owned()];
    if let Ok(vdf) = fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
        for line in vdf.lines() {
            if let Some(path) = vdf_value(line.trim(), "path") {
                libraries.push(PathBuf::from(path));
            }
        }
    }
    libraries
}

fn steam_install_dir(steamapps: &Path) -> String {
    let manifest = steamapps.join("appmanifest_1091500.acf");
    fs::read_to_string(manifest)
        .ok()
        .and_then(|content| {
            content
                .lines()
                .find_map(|line| vdf_value(line.trim(), "installdir"))
        })
        .filter(|value| {
            !value.is_empty()
                && value != "."
                && value != ".."
                && !value.contains('/')
                && !value.contains('\\')
        })
        .unwrap_or_else(|| "Cyberpunk 2077".to_owned())
}

/// Injectable roots make discovery deterministic and testable without a live game.
#[must_use]
pub fn discover_in(
    steam_roots: &[PathBuf],
    epic_manifest_dirs: &[PathBuf],
    common_game_dirs: &[PathBuf],
) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    for root in steam_roots {
        for library in steam_libraries(root) {
            let apps = library.join("steamapps");
            let game_name = steam_install_dir(&apps);
            add_candidate(
                &mut candidates,
                &apps.join("common").join(game_name),
                "steam",
            );
        }
    }

    for directory in epic_manifest_dirs {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|ext| ext.to_str()) != Some("item") {
                continue;
            }
            let Ok(contents) = fs::read_to_string(entry.path()) else {
                continue;
            };
            let Ok(data) = serde_json::from_str::<serde_json::Value>(&contents) else {
                continue;
            };
            if data["DisplayName"].as_str() == Some("Cyberpunk 2077") {
                if let Some(location) = data["InstallLocation"].as_str() {
                    add_candidate(&mut candidates, Path::new(location), "epic");
                }
            }
        }
    }

    for path in common_game_dirs {
        add_candidate(&mut candidates, path, "common");
    }

    let mut seen = HashSet::new();
    candidates.retain(|candidate| seen.insert(candidate.path.clone()));
    candidates.sort_by(|a, b| a.path.cmp(&b.path));
    candidates
}

/// Best-effort Windows discovery. Custom installations remain supported via --path.
#[must_use]
pub fn discover() -> Vec<Candidate> {
    let mut steam_roots = Vec::new();
    let mut common = Vec::new();
    for key in ["PROGRAMFILES(X86)", "PROGRAMFILES"] {
        if let Some(root) = std::env::var_os(key) {
            let root = PathBuf::from(root);
            steam_roots.push(root.join("Steam"));
            common.push(root.join("GOG Galaxy/Games/Cyberpunk 2077"));
            common.push(root.join("Epic Games/Cyberpunk2077"));
            common.push(root.join("Epic Games/Cyberpunk 2077"));
        }
    }
    let epic_dirs = std::env::var_os("PROGRAMDATA")
        .map(|path| vec![PathBuf::from(path).join("Epic/EpicGamesLauncher/Data/Manifests")])
        .unwrap_or_default();
    discover_in(&steam_roots, &epic_dirs, &common)
}
