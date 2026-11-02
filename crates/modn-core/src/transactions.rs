//! Exclusive journaled transactions with rollback and recovery guarantees.
mod catalog;
#[cfg(test)]
mod catalog_tests;
mod conflicts;
#[cfg(test)]
mod conflicts_tests;
mod created_dirs;
#[cfg(test)]
mod created_dirs_tests;
mod directory;
pub(crate) use directory::case_key as directory_candidate_key;
mod directory_ledger;
#[cfg(test)]
mod generic_tests;
mod install;
mod journal;
mod recovery;
mod stack;
mod staging;
#[cfg(test)]
mod tests;
mod uninstall;
mod uninstall_plan;
#[cfg(test)]
mod uninstall_plan_tests;
mod update;
#[cfg(test)]
mod update_tests;
mod validation;

pub use catalog::{
    CatalogConflict, CatalogDependency, CatalogReference, CatalogRelease, DependencyState,
    RecordedDependency,
};
pub(crate) use journal::lock_store;
pub use journal::{ManagedPackage, OwnedFile};
pub use stack::StackMember;
pub use uninstall_plan::{PlannedRemoval, UninstallScope};
