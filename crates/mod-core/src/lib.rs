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
/// Validate installations and discover registered game locations.
pub mod discovery;
pub mod inventory;
pub mod planning;
/// Inventory detected game files and produce diagnostic findings.
pub mod scan;
/// Persist game registrations and manager-owned file manifests.
pub mod state;
pub mod transactions;

/// Stable identifier used for Cyberpunk 2077 registrations.
pub const CYBERPUNK_ID: &str = "cyberpunk-2077";
