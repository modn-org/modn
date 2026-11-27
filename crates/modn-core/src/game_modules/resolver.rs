use super::loader::{self, VerifiedModule};
use super::official::read_official;
use super::registry::ModuleRegistry;
use crate::buckets::BucketStore;
use crate::manager_roots::ManagerRoots;
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

/// Game IDs of every module that ordinary commands can select: the official
/// `main` modules and the registered third-party modules, without duplicates.
///
/// A registered module whose release cannot be read stays listed, so detection
/// reports it in its `skipped` list instead of failing every game.
/// Development overrides are not listed, because they name no game until a
/// command selects one.
pub fn available_games(roots: &ManagerRoots) -> Result<Vec<String>> {
    let mut games = BucketStore::new(roots).official_games()?;
    games.extend(ModuleRegistry::new(roots).game_ids()?);
    games.sort();
    games.dedup();
    Ok(games)
}

pub(crate) fn read_policy(roots: &ManagerRoots, game: &str) -> Result<(VerifiedModule, String)> {
    if !loader::valid_game_id(game) {
        bail!("invalid game module identifier");
    }
    let directory = roots
        .development_modules()
        .map(Path::to_path_buf)
        .or_else(|| {
            std::env::var_os("MODN_GAME_MODULES_DIR").map(|base| PathBuf::from(base).join(game))
        });
    let (manifest, source) = match directory {
        Some(directory) => loader::read_verified_module(&directory)?,
        None => {
            let store = BucketStore::new(roots);
            if store.official_games()?.iter().any(|id| id == game) {
                read_official(roots, game)?
            } else if let Some((manifest, script, registered)) =
                ModuleRegistry::new(roots).read(game)?
            {
                if let Some(bucket) = registered.source.bucket.as_deref() {
                    let decision = store.module_source_trust(&registered.source)?;
                    if !decision.is_trusted() {
                        let remedy = decision
                            .remedy()
                            .expect("a refused module source always names a remedy");
                        bail!(
                            "game module {game} comes from bucket {bucket}, which is not a trusted module source; run `{}` to trust it again, or `{}` to remove the module",
                            remedy.trust_modules_command(),
                            remedy.remove_module_command(game)
                        );
                    }
                }
                (manifest, script)
            } else {
                read_official(roots, game)?
            }
        }
    };
    if manifest.game_id != game {
        bail!("game module identity does not match requested game");
    }
    Ok((manifest, source))
}
