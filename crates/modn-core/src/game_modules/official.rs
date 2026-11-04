use super::loader::{self, VerifiedModule};
use crate::buckets::BucketStore;
use crate::data_dir::data_directory;
use anyhow::Result;

pub(super) fn read_official(game: &str) -> Result<(VerifiedModule, String)> {
    let store = BucketStore::new(&data_directory()?);
    let directory = store.official_module(game)?;
    loader::read_verified_module(&directory)
}
