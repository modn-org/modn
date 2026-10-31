//! Versioned, read-only package and game-module metadata and local bucket registration.
//! A manifest never authorizes writes to a game.
mod manifest;
mod module;
mod store;

pub use manifest::{ArchiveFormat, PackageManifest, PackageSource};
pub use module::{BucketModule, ModuleReleaseManifest};
pub use store::{BucketInfo, BucketPackage, BucketStore};
