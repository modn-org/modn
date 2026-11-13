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
/// Durable per-mod approval of native-injector and executable capabilities.
pub mod approval;
pub mod archive;
/// Deterministic content digest for bucket update reporting.
pub mod bucket_digest;
/// Versioned bucket metadata, validation and local registration.
pub mod buckets;
/// Resolve module-declared content roots below a known folder.
pub mod content_root;
/// Resolve manager-owned data locations.
pub mod data_dir;
/// Find installed games by matching store libraries with module store identities.
pub mod detection;
/// Validate game installations with the selected game module.
pub mod discovery;
pub mod error;
/// Local, sandboxed Lua game module inspection without host filesystem capabilities.
pub mod game_modules;
mod git;
/// Durable history of committed install, update and uninstall operations.
pub mod history;
mod integrity;
pub mod inventory;
/// Prepare and perform a validated game launch.
pub mod launch;
pub mod planning;
pub mod progress;
/// Inventory detected game files and produce diagnostic findings.
pub mod scan;
/// Persist game registrations and manager-owned file manifests.
pub mod state;
pub mod stores;
pub mod transactions;
/// Reverse one committed operation from the operation history.
pub use transactions::undo;
#[cfg(test)]
extern crate self as modn_core;
#[cfg(test)]
#[path = "../tests/support/main_bucket.rs"]
mod test_support;
