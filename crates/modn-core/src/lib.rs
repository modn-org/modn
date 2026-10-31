//! Game discovery, archive inspection, and guarded management of legacy archives.
#![warn(missing_docs)]
#![deny(
    clippy::exhaustive_enums,
    clippy::exhaustive_structs,
    clippy::must_use_candidate,
    clippy::return_self_not_must_use,
    clippy::missing_panics_doc
)]
pub mod archive;
/// Versioned bucket metadata, validation and local registration.
pub mod buckets;
/// Validate installations and discover registered game locations.
pub mod discovery;
/// Local, sandboxed Luau game module inspection without host filesystem capabilities.
pub mod game_modules;
pub mod inventory;
pub mod planning;
/// Inventory detected game files and produce diagnostic findings.
pub mod scan;
/// Persist game registrations and manager-owned file manifests.
pub mod state;
pub mod transactions;
