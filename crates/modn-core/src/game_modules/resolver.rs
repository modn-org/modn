//! Explicit override, selected user version, bundled policy, development fallback.
use super::loader::{self, VerifiedModule};
use super::store::ModuleStore;
use anyhow::{bail, Result};
use std::cell::RefCell;
use std::path::{Path, PathBuf};

thread_local! {
    static STATE_ROOT: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Scope synchronous policy operations to a manager state directory without creating it.
/// Nested calls restore the previous scope, including during unwinding. This does not
/// authorize game roots or propagate configuration to other threads.
pub fn with_module_store<T>(state_dir: &Path, action: impl FnOnce() -> T) -> T {
    struct Restore(Option<PathBuf>);
    impl Drop for Restore {
        fn drop(&mut self) {
            STATE_ROOT.with(|root| {
                root.replace(self.0.take());
            });
        }
    }
    let previous = STATE_ROOT.with(|root| root.replace(Some(state_dir.to_path_buf())));
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

pub(super) fn read_policy(game: &str) -> Result<(VerifiedModule, String, &'static str)> {
    if !loader::valid_game_id(game) {
        bail!("invalid game module identifier");
    }
    let (manifest, source, origin) = if let Some(base) = std::env::var_os("MODN_GAME_MODULES_DIR") {
        let (info, source) = loader::read_verified_module(&PathBuf::from(base).join(game))?;
        (info, source, "override")
    } else if let Some(package) = STATE_ROOT
        .with(|root| {
            root.borrow()
                .as_deref()
                .map(|root| ModuleStore::new(root).active(game))
                .transpose()
        })?
        .flatten()
    {
        let info = VerifiedModule {
            game_id: package.info.game_id,
            version: package.info.version,
            api_version: package.info.api_version,
            entrypoint: package.info.entrypoint,
        };
        (info, String::from_utf8(package.script)?, "user-installed")
    } else {
        let (info, source) = loader::read_verified_module(&bundled_directory(game))?;
        (info, source, "bundled")
    };
    if manifest.game_id != game {
        bail!("game module identity does not match requested game");
    }
    Ok((manifest, source, origin))
}

/// Report the effective resolution source, verifying selected stored bytes on each read.
pub fn module_origin(game: &str) -> Result<&'static str> {
    Ok(read_policy(game)?.2)
}
