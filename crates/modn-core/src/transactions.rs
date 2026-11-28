//! Exclusive journaled transactions with rollback and recovery guarantees.
mod adoption;
mod catalog;
#[cfg(test)]
mod catalog_tests;
pub mod claim;
#[cfg(test)]
mod claim_tests;
mod conflicts;
#[cfg(test)]
mod conflicts_tests;
mod content;
mod controlled_io;
mod created_dirs;
#[cfg(test)]
mod created_dirs_tests;
mod directory;
pub(crate) use directory::case_key as directory_candidate_key;
pub(crate) use directory::ownership_key;
mod cleanup;
mod commit;
mod directory_ledger;
#[cfg(test)]
mod generic_tests;
#[cfg(test)]
mod hash_tests;
#[cfg(test)]
mod history_release_tests;
#[cfg(test)]
mod history_tests;
mod install;
mod journal;
#[cfg(test)]
mod lifecycle_tests;
mod native_injector;
mod override_recovery;
#[cfg(test)]
mod override_recovery_tests;
#[cfg(test)]
mod override_tests;
mod profile_apply;
#[cfg(test)]
mod profile_apply_crash_tests;
mod profile_apply_plan;
#[cfg(test)]
mod profile_apply_tests;
mod profile_switch;
mod profile_toggle;
mod profile_toggle_apply;
#[cfg(test)]
mod profile_toggle_crash_tests;
mod profile_toggle_plan;
#[cfg(test)]
mod profile_toggle_tests;
#[cfg(test)]
mod progress_tests;
mod recovery;
mod restore;
#[cfg(test)]
mod restore_crash_tests;
mod restore_recovery;
#[cfg(test)]
mod restore_sevenz_tests;
#[cfg(test)]
mod restore_tests;
#[cfg(test)]
mod roots_tests;
mod stack;
mod staging;
#[cfg(test)]
mod tests;
mod undo_digests;
#[cfg(test)]
mod undo_digests_tests;
mod uninstall;
mod uninstall_plan;
#[cfg(test)]
mod uninstall_plan_tests;
mod update;
#[cfg(test)]
mod update_tests;
mod validation;

pub use adoption::AdoptionApproval;
pub use catalog::{
    CatalogConflict, CatalogDependency, CatalogReference, CatalogRelease, DependencyState,
    RecordedDependency, ReleaseDeclarations,
};
pub use cleanup::CleanupApproval;
pub(crate) use journal::lock_store;
pub use journal::{ManagedPackage, OwnedFile};
pub use native_injector::{
    NativeInjectorNotAllowed, NativeInjectorPolicy, NATIVE_INJECTOR_FLAG,
    NATIVE_INJECTOR_NOT_ALLOWED,
};
pub use profile_apply::{PriorityReport, ProfileApply};
pub use profile_switch::{
    plan_profile_switch, profile_switch, PathAction, PlannedPath, ProfileSwitchOptions,
    ProfileSwitchOutcome, ProfileSwitchPlan, UnchangedPath,
};
pub use profile_toggle::{ProfileToggle, ToggleAction, ToggleChange};
pub use stack::StackMember;
pub use uninstall_plan::{PlannedRemoval, PlannedRestore, UninstallScope};
pub(crate) use validation::validate_transaction_root;

pub mod undo;
pub use undo::{conflict, inverse, UndoConflict, UndoRestore};
