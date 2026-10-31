//! Exclusive journaled transactions with rollback and recovery guarantees.
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

pub use journal::{ManagedPackage, OwnedFile};
