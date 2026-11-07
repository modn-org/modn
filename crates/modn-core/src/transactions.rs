//! Exclusive journaled transactions with rollback and recovery guarantees.
mod adoption;
mod catalog;
#[cfg(test)]
mod catalog_tests;
mod conflicts;
#[cfg(test)]
mod conflicts_tests;
mod content;
mod created_dirs;
#[cfg(test)]
mod created_dirs_tests;
mod directory;
pub(crate) use directory::case_key as directory_candidate_key;
pub(crate) use directory::ownership_key;
mod cleanup;
mod directory_ledger;
#[cfg(test)]
mod generic_tests;
mod install;
mod journal;
#[cfg(test)]
mod lifecycle_tests;
mod native_injector;
mod recovery;
#[cfg(test)]
mod roots_tests;
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

pub use adoption::AdoptionApproval;
pub use catalog::{
    CatalogConflict, CatalogDependency, CatalogReference, CatalogRelease, DependencyState,
    RecordedDependency,
};
pub use cleanup::CleanupApproval;
pub(crate) use journal::lock_store;
pub use journal::{ManagedPackage, OwnedFile};
pub use native_injector::{
    NativeInjectorNotAllowed, NativeInjectorPolicy, NATIVE_INJECTOR_FLAG,
    NATIVE_INJECTOR_NOT_ALLOWED,
};
pub use stack::StackMember;
pub use uninstall_plan::{PlannedRemoval, UninstallScope};
pub(crate) use validation::validate_transaction_root;
