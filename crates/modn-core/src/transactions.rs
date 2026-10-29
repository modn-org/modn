//! Exclusive journaled transactions with rollback and recovery guarantees.
#[cfg(test)]
mod generic_tests;
mod install;
mod journal;
mod legacy;
#[cfg(test)]
mod tests;
mod uninstall;
mod validation;

pub use journal::{ManagedPackage, OwnedFile};
pub use legacy::preview_install_zip;
