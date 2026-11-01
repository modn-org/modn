//! Exclusive journaled transactions with rollback and recovery guarantees.
mod catalog;
#[cfg(test)]
mod catalog_tests;
mod directory;
pub(crate) use directory::case_key as directory_candidate_key;
mod directory_ledger;
#[cfg(test)]
mod generic_tests;
mod install;
mod journal;
mod staging;
#[cfg(test)]
mod tests;
mod uninstall;
mod validation;

pub use catalog::{CatalogDependency, CatalogReference, CatalogRelease};
pub(crate) use journal::lock_store;
pub use journal::{ManagedPackage, OwnedFile};
