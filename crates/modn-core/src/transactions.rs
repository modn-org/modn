//! Exclusive journaled transactions with rollback and recovery guarantees.
#[cfg(test)]
mod generic_tests;
mod install;
mod journal;
#[cfg(test)]
mod tests;
mod uninstall;
mod validation;

pub use journal::{ManagedPackage, OwnedFile};
