//! Exclusive journaled transactions with rollback and recovery guarantees.
mod directory;
pub(crate) use directory::case_key as directory_candidate_key;
mod directory_ledger;
#[cfg(test)]
mod generic_tests;
mod install;
mod journal;
#[cfg(test)]
mod tests;
mod uninstall;
mod validation;

pub(crate) use journal::lock_store;
pub(crate) use validation::{
    validate_ownership_paths, validate_registered_adoption_roots, validate_transaction_root,
};

pub use journal::{ManagedPackage, OwnedFile};
