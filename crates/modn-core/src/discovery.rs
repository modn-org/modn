//! Generic filesystem discovery plumbing. The bundled game module owns game policy.
use crate::game_modules::policy;
use crate::CYBERPUNK_ID;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Discovered installation and the source of its location.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_structs,
    reason = "CLI constructs explicit manual game candidates"
)]
pub struct Candidate {
    /// Canonical installation directory.
    pub path: PathBuf,
    /// Discovery origin: Steam, Epic, common or manual.
    pub source: String,
}

/// Confirm that the selected path satisfies the bundled game's Lua marker policy.
pub fn validate_installation(path: &Path) -> Result<PathBuf> {
    validate_installation_for(CYBERPUNK_ID, path)
}

/// Validate an installation against a runtime-selected game's Luau policy.
pub fn validate_installation_for(game_id: &str, path: &Path) -> Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("cannot access game directory {}", path.display()))?;
    let (valid, marker) = policy::validate(game_id, &canonical)?;
    if !valid {
        bail!("{} does not contain {}", canonical.display(), marker);
    }
    Ok(canonical)
}

fn valid_install_folder(name: &str) -> bool {
    let path = Path::new(name);
    !name.is_empty()
        && name.len() <= 128
        && !name.contains(':')
        && !name.contains('\\')
        && !name.chars().any(char::is_control)
        && path.components().count() == 1
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn add_candidate(candidates: &mut Vec<Candidate>, game_id: &str, path: &Path, source: &str) {
    if let Ok(path) = validate_installation_for(game_id, path) {
        candidates.push(Candidate {
            path,
            source: source.into(),
        });
    }
}

fn vdf_value(line: &str, key: &str) -> Option<String> {
    let parts: Vec<_> = line.split('"').collect();
    (parts.len() >= 4 && parts[1].eq_ignore_ascii_case(key)).then(|| parts[3].replace("\\\\", "\\"))
}

/// Collect shared Steam library locations; the app ID and install name belong to Luau.
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

/// Injectable platform locations for deterministic multi-installation fixture tests.
#[must_use]
pub fn discover_in(
    steam_roots: &[PathBuf],
    epic_manifest_dirs: &[PathBuf],
    common_game_dirs: &[PathBuf],
) -> Vec<Candidate> {
    discover_in_for(
        CYBERPUNK_ID,
        steam_roots,
        epic_manifest_dirs,
        common_game_dirs,
    )
}

/// Discover installations with a dynamically selected game's policy.
#[must_use]
pub fn discover_in_for(
    game_id: &str,
    steam_roots: &[PathBuf],
    epic_manifest_dirs: &[PathBuf],
    common_game_dirs: &[PathBuf],
) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    for root in steam_roots {
        for library in steam_libraries(root) {
            if let Ok(game_name) = policy::steam_name(game_id, &library) {
                if !valid_install_folder(&game_name) {
                    continue;
                }
                add_candidate(
                    &mut candidates,
                    game_id,
                    &library.join("steamapps/common").join(game_name),
                    "steam",
                );
            }
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
            let (Some(display_name), Some(location)) = (
                data["DisplayName"].as_str(),
                data["InstallLocation"].as_str(),
            ) else {
                continue;
            };
            if policy::epic_match(game_id, display_name).unwrap_or(false) {
                add_candidate(&mut candidates, game_id, Path::new(location), "epic");
            }
        }
    }
    for path in common_game_dirs {
        add_candidate(&mut candidates, game_id, path, "common");
    }
    let mut seen = HashSet::new();
    candidates.retain(|candidate| seen.insert(candidate.path.clone()));
    candidates.sort_by(|a, b| a.path.cmp(&b.path));
    candidates
}

/// Best-effort platform search. Explicit --path remains supported.
#[must_use]
pub fn discover() -> Vec<Candidate> {
    discover_for(CYBERPUNK_ID)
}

/// Search known platform locations using a runtime-selected game module.
#[must_use]
pub fn discover_for(game_id: &str) -> Vec<Candidate> {
    let mut steam_roots = Vec::new();
    let mut common = Vec::new();
    let hints = policy::common_paths(game_id).unwrap_or_default();
    for key in ["PROGRAMFILES(X86)", "PROGRAMFILES"] {
        if let Some(root) = std::env::var_os(key) {
            let root = PathBuf::from(root);
            steam_roots.push(root.join("Steam"));
            for hint in &hints {
                if !hint.is_empty()
                    && hint.len() <= 256
                    && !hint.contains(':')
                    && !hint.contains('\\')
                    && Path::new(hint)
                        .components()
                        .all(|component| matches!(component, Component::Normal(_)))
                {
                    common.push(root.join(hint));
                }
            }
        }
    }
    let epic_dirs = std::env::var_os("PROGRAMDATA")
        .map(|root| vec![PathBuf::from(root).join("Epic/EpicGamesLauncher/Data/Manifests")])
        .unwrap_or_default();
    discover_in_for(game_id, &steam_roots, &epic_dirs, &common)
}
