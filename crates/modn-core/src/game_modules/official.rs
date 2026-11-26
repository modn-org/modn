use super::loader::{self, VerifiedModule};
use crate::buckets::BucketStore;
use crate::manager_roots::ManagerRoots;
use anyhow::Result;

pub(super) fn read_official(roots: &ManagerRoots, game: &str) -> Result<(VerifiedModule, String)> {
    let store = BucketStore::new(roots);
    let directory = store.official_module(game)?;
    loader::read_verified_module(&directory)
}
