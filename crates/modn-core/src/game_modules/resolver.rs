use super::loader::{self, VerifiedModule};
use super::official::read_official;
use super::registry::{ModuleRegistry, ModuleSource};
use crate::buckets::BucketStore;
use crate::data_dir;
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

pub(crate) fn read_policy(game: &str) -> Result<(VerifiedModule, String)> {
    if !loader::valid_game_id(game) {
        bail!("invalid game module identifier");
    }
    let source = MODULE_SOURCE.with(|slot| slot.borrow().clone());
    let directory = source.or_else(|| {
        std::env::var_os("MODN_GAME_MODULES_DIR").map(|base| PathBuf::from(base).join(game))
    });
    let (manifest, source) = match directory {
        Some(directory) => loader::read_verified_module(&directory)?,
        None => {
            let data = data_dir::profile_directory()?;
            let store = BucketStore::new(&data);
            if store.official_games()?.iter().any(|id| id == game) {
                read_official(game)?
            } else if let Some((manifest, script, registered)) =
                ModuleRegistry::new(&data).read(game)?
            {
                ensure_source_trusted(&store, game, &registered.source)?;
                (manifest, script)
            } else {
                read_official(game)?
            }
        }
    };
    if manifest.game_id != game {
        bail!("game module identity does not match requested game");
    }
    Ok((manifest, source))
}

fn ensure_source_trusted(store: &BucketStore, game: &str, source: &ModuleSource) -> Result<()> {
    let Some(bucket) = source.bucket.as_deref() else {
        return Ok(());
    };
    if bucket == "main" {
        return Ok(());
    }
    let trusted = store.trusted_module_sources()?.into_iter().any(|trusted| {
        trusted.bucket == bucket && Some(&trusted.signer_key_id) == source.signer_key_id.as_ref()
    });
    if !trusted {
        bail!(
            "game module {game} comes from bucket {bucket}, which is not a trusted module source; run `modn bucket trust-modules {bucket}` to trust it again, or `modn module remove {game}` to remove the module"
        );
    }
    Ok(())
}
