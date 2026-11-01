//! Versioned, read-only mod-package metadata and local bucket registration.
//! A manifest never authorizes writes to a game.
mod dependencies;
mod manifest;
mod store;

pub use dependencies::install_order;
pub use manifest::{ArchiveFormat, PackageManifest, PackageSource};
pub use store::{BucketInfo, BucketPackage, BucketStore};
