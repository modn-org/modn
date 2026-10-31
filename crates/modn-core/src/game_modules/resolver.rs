//! Per-command development overrides and bundled first-party Lua policies.
use super::loader::{self, VerifiedModule};
use anyhow::{bail, Result};
use std::cell::RefCell;
use std::path::{Path, PathBuf};

thread_local! {
    static MODULE_SOURCE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Temporarily select one local source directory for policy operations.
///
/// Nested calls restore the previous directory, including during unwinding.
/// No override is persisted to manager state or propagated to other threads.
pub fn with_module_source<T>(source: Option<&Path>, action: impl FnOnce() -> T) -> T {
    struct Restore(Option<PathBuf>);
    impl Drop for Restore {
        fn drop(&mut self) {
            MODULE_SOURCE.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous = MODULE_SOURCE.with(|slot| slot.replace(source.map(Path::to_path_buf)));
    let _restore = Restore(previous);
    action()
}

fn bundled_directory(game: &str) -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let directory = parent.join("game-modules").join(game);
            // Presence (including a broken link) selects the bundle and fails closed.
            if std::fs::symlink_metadata(&directory).is_ok() {
                return directory;
            }
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("game-modules")
        .join(game)
}

pub(super) fn read_policy(game: &str) -> Result<(VerifiedModule, String)> {
    if !loader::valid_game_id(game) {
        bail!("invalid game module identifier");
    }
    let source = MODULE_SOURCE.with(|slot| slot.borrow().clone());
    let directory = if let Some(directory) = source {
        directory
    } else if let Some(base) = std::env::var_os("MODN_GAME_MODULES_DIR") {
        PathBuf::from(base).join(game)
    } else {
        bundled_directory(game)
    };
    let (manifest, source) = loader::read_verified_module(&directory)?;
    if manifest.game_id != game {
        bail!("game module identity does not match requested game");
    }
    Ok((manifest, source))
}

/// List game identities exposed by the selected first-party or developer source.
///
/// This reads directory names or an explicit development manifest without
/// executing Lua or changing manager state. Loading a selected game still
/// validates the full module and enforces game identity.
pub fn available_games(source: Option<&Path>) -> Result<Vec<String>> {
    if let Some(source) = source {
        let info = loader::check_module(source)?;
        return Ok(vec![info.game_id]);
    }
    let root = if let Some(base) = std::env::var_os("MODN_GAME_MODULES_DIR") {
        PathBuf::from(base)
    } else {
        let sibling = std::env::current_exe()?
            .parent()
            .ok_or_else(|| anyhow::anyhow!("invalid executable directory"))?
            .join("game-modules");
        if std::fs::symlink_metadata(&sibling).is_ok() {
            sibling
        } else {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("game-modules")
        }
    };
    let mut games = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if loader::valid_game_id(&name) {
            games.push(name);
        }
    }
    games.sort();
    games.dedup();
    Ok(games)
}
