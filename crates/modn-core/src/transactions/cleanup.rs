use super::install::Fault;
use super::journal::lock_store;
use crate::state::Store;
use anyhow::{bail, Result};
use std::path::Path;

/// Independent authorization to remove modified files of one managed package.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CleanupApproval {
    /// Preserve all files and ownership records.
    Denied,
    /// The caller explicitly approved removal of modified owned files.
    Approved,
}

#[derive(Clone, Copy)]
pub(super) enum RemovalMode {
    Strict,
    ModifiedOwned,
}

impl Store {
    /// Remove only recorded files of the selected managed package, including modifications.
    ///
    /// Refuses missing files, unsafe paths, changed exclusive-directory identities,
    /// and packages required by another package. Preserves unrecorded descendants.
    /// Backups contain current bytes; failure rolls back or retains a recovery journal.
    /// Exclusive directories remain on disk; this API never grants raw-path deletion.
    pub fn cleanup_owned(
        &mut self,
        game: &Path,
        package_id: &str,
        approval: CleanupApproval,
    ) -> Result<()> {
        self.cleanup_owned_inner(game, package_id, approval, None)
    }

    pub(super) fn cleanup_owned_inner(
        &mut self,
        game: &Path,
        package_id: &str,
        approval: CleanupApproval,
        fault: Option<Fault>,
    ) -> Result<()> {
        if approval != CleanupApproval::Approved {
            bail!("forced cleanup requires explicit approval");
        }
        let _guard = lock_store(self)?;
        self.ensure_idle()?;
        self.uninstall_with_mode_locked(game, package_id, RemovalMode::ModifiedOwned, fault)
    }
}
