//! Per-command development overrides and embedded first-party Lua policies.
use super::bundled::read_bundled;
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

pub(super) fn read_policy(game: &str) -> Result<(VerifiedModule, String)> {
    if !loader::valid_game_id(game) {
        bail!("invalid game module identifier");
    }
    // An explicit development source wins and never falls back; otherwise use
    // the reviewed module compiled into this executable.
    let source = MODULE_SOURCE.with(|slot| slot.borrow().clone());
    let directory = source.or_else(|| {
        std::env::var_os("MODN_GAME_MODULES_DIR").map(|base| PathBuf::from(base).join(game))
    });
    let (manifest, source) = match directory {
        Some(directory) => loader::read_verified_module(&directory)?,
        None => read_bundled(game)?,
    };
    if manifest.game_id != game {
        bail!("game module identity does not match requested game");
    }
    Ok((manifest, source))
}
