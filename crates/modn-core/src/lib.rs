//! Game discovery, archive inspection, and guarded management of legacy archives.
#![warn(missing_docs)]
#![deny(
    clippy::exhaustive_enums,
    clippy::exhaustive_structs,
    clippy::must_use_candidate,
    clippy::return_self_not_must_use,
    clippy::missing_panics_doc
)]
pub mod acquisition;
pub mod archive;
/// Deterministic content digest for bucket update reporting.
pub mod bucket_digest;
/// Versioned bucket metadata, validation and local registration.
pub mod buckets;
/// Resolve module-declared content roots below a known folder.
pub mod content_root;
/// Resolve manager-owned data locations.
pub mod data_dir;
/// Validate installations and discover registered game locations.
pub mod discovery;
/// Local, sandboxed Lua game module inspection without host filesystem capabilities.
pub mod game_modules;
mod integrity;
pub mod inventory;
pub mod planning;
/// Inventory detected game files and produce diagnostic findings.
pub mod scan;
/// Persist game registrations and manager-owned file manifests.
pub mod state;
pub mod transactions;
#[cfg(test)]
extern crate self as modn_core;
#[cfg(test)]
#[path = "../tests/support/main_bucket.rs"]
mod test_support;
