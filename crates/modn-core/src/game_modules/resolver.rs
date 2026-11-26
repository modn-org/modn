use super::loader::VerifiedModule;
use super::registry::ModuleRegistry;
use super::selection::ModuleSelection;
use crate::buckets::BucketStore;
use crate::manager_roots::ManagerRoots;
use anyhow::Result;

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
    Ok(ModuleSelection::resolve(roots, game)?.into_parts())
}
