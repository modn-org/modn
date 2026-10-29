//! Exclusive journaled transactions with rollback and recovery guarantees.
mod install;
mod journal;
#[cfg(test)]
mod tests;
mod uninstall;

pub use install::preview_install_zip;
pub use journal::{ManagedPackage, OwnedFile};
