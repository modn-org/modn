//! Discover the module directories that a catalog offers and reports.

use super::super::loader;
use super::{IgnoredModule, ModuleIgnoreReason};
use anyhow::{bail, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Modules an untrusted catalog offers but cannot import.
///
/// The `games/` folder is reported, never validated, parsed or executed for an
/// untrusted source. Only directory names and the presence of a module file
/// are read.
pub(crate) fn ignored_catalog_modules(catalog: &Path) -> Result<Vec<IgnoredModule>> {
    let games = catalog.join("games");
    match fs::symlink_metadata(&games) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Ok(Vec::new())
        }
        Ok(_) => {}
    }
    let mut ignored = Vec::new();
    for entry in fs::read_dir(&games)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let game = entry.file_name().to_string_lossy().into_owned();
        if loader::valid_game_id(&game)
            && (fs::symlink_metadata(entry.path().join("module.json")).is_ok()
                || fs::symlink_metadata(entry.path().join("module.lua")).is_ok())
        {
            ignored.push(IgnoredModule {
                game_id: game,
                reason: ModuleIgnoreReason::UntrustedSource,
            });
        }
    }
    ignored.sort_by(|left, right| left.game_id.cmp(&right.game_id));
    Ok(ignored)
}

pub(super) fn module_directories(catalog: &Path) -> Result<Vec<(String, PathBuf)>> {
    let games = catalog.join("games");
    match fs::symlink_metadata(&games) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        result => {
            let metadata = result?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                bail!("catalog games directory must be a real directory");
            }
        }
    }
    let mut candidates = Vec::new();
    for entry in fs::read_dir(&games)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            bail!("catalog games entry must be a real directory");
        }
        let game = entry.file_name().to_string_lossy().into_owned();
        let directory = entry.path();
        let has_module = fs::symlink_metadata(directory.join("module.json")).is_ok();
        let has_script = fs::symlink_metadata(directory.join("module.lua")).is_ok();
        if !has_module && !has_script {
            continue;
        }
        if !loader::valid_game_id(&game) {
            bail!("invalid catalog game module identifier: {game}");
        }
        candidates.push((game, directory));
    }
    candidates.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(candidates)
}
