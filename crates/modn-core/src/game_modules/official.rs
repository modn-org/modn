use crate::buckets::BucketStore;
use crate::manager_roots::ManagerRoots;
use anyhow::Result;
use std::path::PathBuf;

/// The directory of the local first-party `main` module for `game`.
pub(super) fn official_directory(roots: &ManagerRoots, game: &str) -> Result<PathBuf> {
    BucketStore::new(roots).official_module(game)
}
