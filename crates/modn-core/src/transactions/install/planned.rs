//! Catalog installs whose operations are re-planned under the transaction lock.

use super::super::catalog::{CatalogInstall, CatalogRelease};
use super::super::journal::{lock_store, ManagedPackage};
use super::super::native_injector::NativeInjectorPolicy;
use crate::archive::inspect;
use crate::planning::InstallPlan;
use crate::state::Store;
use anyhow::Result;
use std::path::Path;

impl Store {
    /// Install a catalog release after re-planning its operations under the
    /// transaction lock.
    ///
    /// The archive is inspected before `plan` runs, so a missing or malformed
    /// archive is refused without executing game policy. The plan that `plan`
    /// returns is the one installed; its error propagates unchanged before any
    /// journal entry or game file is written. `plan` reads the locked store, so
    /// it can plan against the files the active profile already owns. Every
    /// other check is the one
    /// [`Store::install_catalog_release_with_permissions`] performs, including
    /// the native-injector policy and the archive, ownership, conflict and
    /// dependency checks under the lock.
    pub fn install_catalog_release_planned(
        &mut self,
        game: &Path,
        archive: &Path,
        release: &CatalogRelease,
        policy: NativeInjectorPolicy,
        approved: &[String],
        plan: impl FnOnce(&Store, &Path) -> Result<InstallPlan>,
    ) -> Result<ManagedPackage> {
        release.ensure_native_injector_allowed(policy)?;
        let _guard = lock_store(self)?;
        self.ensure_idle()?;
        let _ = inspect(archive)?;
        let proposal = plan(&*self, archive)?;
        self.install_locked(
            game,
            archive,
            &proposal,
            None,
            approved,
            Some(CatalogInstall {
                release,
                replacing: None,
            }),
            policy,
            None,
        )
    }
}
