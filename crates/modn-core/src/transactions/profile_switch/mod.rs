//! Planning and executing a profile switch.
//!
//! A plan names the managed files of the active profile that the target
//! profile does not claim, the target winners that differ from the bytes on
//! disk, and the packages that enter or leave the active state. It refuses
//! before any write when the switch is unsafe. Execution applies one plan as
//! a journaled transaction: it stages every written member, quarantines the
//! bytes it removes, and commits the active profile, winner rows, history and
//! journal in one SQLite transaction. Recovery restores a consistent state
//! after a crash at any point.

#[cfg(test)]
mod commit_tests;
#[cfg(test)]
mod crash_tests;
mod execute;
#[cfg(test)]
mod execute_tests;
mod plan;
#[cfg(test)]
mod plan_tests;
mod recover;
#[cfg(test)]
mod refusal_tests;
mod refuse;

pub use execute::{profile_switch, ProfileSwitchOptions, ProfileSwitchOutcome};
pub(crate) use plan::{PreparedSwitch, Remove, Write};
pub(crate) use recover::recover_switch;

use crate::state::Store;
use anyhow::Result;
use serde::Serialize;
use std::path::Path;

/// What a switch does to one managed path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum PathAction {
    /// The path has no claimant in the target profile.
    Remove,
    /// The target winner writes a path with no managed row.
    Write,
    /// The target winner writes over another package's managed bytes.
    Replace,
}

/// One managed path that a switch removes, writes, or replaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct PlannedPath {
    /// Installation root that contains the destination: `game` or `content`.
    pub root: String,
    /// Root-relative destination path.
    pub path: String,
    /// What the switch does to this path.
    pub action: PathAction,
    /// Package whose recorded bytes the plan names.
    pub package: Option<String>,
    /// Current owner whose bytes a replace overwrites.
    pub replaces: Option<String>,
    /// BLAKE3 digest of the bytes written or removed.
    pub blake3: Option<String>,
    /// Exact size of those bytes.
    pub bytes: Option<u64>,
}

/// One managed path whose bytes already match the target winner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct UnchangedPath {
    /// Installation root that contains the destination.
    pub root: String,
    /// Root-relative destination path.
    pub path: String,
}

/// The file diff and package movement of one profile switch.
///
/// The document is data and never authorization. It names the target winner
/// of every changed path, so a later execution can re-check the same facts
/// before it writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct ProfileSwitchPlan {
    /// Game identifier of the installation, empty when it is not registered.
    pub game: String,
    /// Installation path the plan applies to.
    pub path: String,
    /// Name of the active profile.
    pub from: String,
    /// Name of the target profile.
    pub to: String,
    /// Managed paths the switch removes, in ownership key order.
    pub remove: Vec<PlannedPath>,
    /// Managed paths the switch writes, in ownership key order.
    pub write: Vec<PlannedPath>,
    /// Number of managed paths whose bytes already match the target winner.
    pub unchanged: usize,
    /// The paths counted by [`Self::unchanged`].
    pub unchanged_paths: Vec<UnchangedPath>,
    /// Packages enabled in the target profile and not enabled in the active one.
    pub entering: Vec<String>,
    /// Packages enabled in the active profile and not enabled in the target one.
    pub leaving: Vec<String>,
    /// Sum of the bytes in [`Self::write`].
    pub bytes_to_write: u64,
}

/// Plan the file diff of switching `game` from its active profile to `target`.
///
/// The routine reads manager state and the game tree and writes neither. It
/// reads `profiles`, `profile_packages`, `file_claims` and `managed_files`,
/// never a catalog, so it works offline. Every list is sorted
/// deterministically.
///
/// # Errors
///
/// Refuses, in this order and before any write, when a transaction or an
/// installation stack is pending, when `target` is unknown or already active,
/// when a managed file of the active profile drifted, when a written path has
/// no verified archive, or when an unmanaged file occupies a written path.
pub fn plan_profile_switch(store: &Store, game: &Path, target: &str) -> Result<ProfileSwitchPlan> {
    plan::plan(store, game, target)
}
