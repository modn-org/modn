use super::catalog::{CatalogInstall, CatalogRelease};
use super::journal::{lock_store, ManagedPackage};
use super::native_injector::NativeInjectorPolicy;
use crate::planning::InstallPlan;
use crate::state::Store;
use anyhow::Result;
use std::path::Path;

mod execute;
mod planned;

#[cfg(test)]
mod planned_tests;

#[derive(Clone, Copy)]
pub(super) enum Fault {
    FailAfter(usize),
    #[cfg_attr(not(test), expect(dead_code, reason = "only tests simulate a crash"))]
    CrashAfter(usize),
}

impl Fault {
    pub(super) fn after_files(self) -> usize {
        match self {
            Self::FailAfter(files) | Self::CrashAfter(files) => files,
        }
    }
}

impl Store {
    /// Execute a caller-approved proposal, independently checked against the current archive
    /// and filesystem. This is a write API, not exposed to untrusted Lua modules.
    /// ZIP and 7z file-copy operations share the same journal and recovery.
    pub fn install_operations_zip(
        &mut self,
        game: &Path,
        archive: &Path,
        proposal: &InstallPlan,
    ) -> Result<ManagedPackage> {
        self.install_operations_inner(game, archive, proposal, None)
    }

    /// Install with independent caller-authorized tool executable destinations.
    /// Approval is never read from the module's untrusted operation labels.
    pub fn install_operations_zip_with_permissions(
        &mut self,
        game: &Path,
        archive: &Path,
        proposal: &InstallPlan,
        approved: &[String],
    ) -> Result<ManagedPackage> {
        self.install_inner_with_permissions(
            game,
            archive,
            proposal,
            None,
            approved,
            None,
            NativeInjectorPolicy::Refuse,
        )
    }

    /// Install a catalog release, recording its provenance and dependency links.
    ///
    /// Dependency readiness is checked while the transaction lock is held, so a
    /// concurrent uninstall cannot remove a prerequisite between the check and
    /// the ownership commit. Recorded dependencies then block their uninstall.
    pub fn install_catalog_release(
        &mut self,
        game: &Path,
        archive: &Path,
        proposal: &InstallPlan,
        release: &CatalogRelease,
        policy: NativeInjectorPolicy,
    ) -> Result<ManagedPackage> {
        self.install_catalog_release_with_permissions(game, archive, proposal, release, policy, &[])
    }

    /// Install a catalog release with caller-authorized tool executable destinations.
    ///
    /// Approval comes from the user's invocation, never from the bucket
    /// manifest or the module's operation labels; the archive is revalidated
    /// against the same approvals while the transaction lock is held.
    pub fn install_catalog_release_with_permissions(
        &mut self,
        game: &Path,
        archive: &Path,
        proposal: &InstallPlan,
        release: &CatalogRelease,
        policy: NativeInjectorPolicy,
        approved: &[String],
    ) -> Result<ManagedPackage> {
        self.install_inner_with_permissions(
            game,
            archive,
            proposal,
            None,
            approved,
            Some(release),
            policy,
        )
    }

    pub(super) fn install_operations_inner(
        &mut self,
        game: &Path,
        archive: &Path,
        proposal: &InstallPlan,
        fail_after: Option<usize>,
    ) -> Result<ManagedPackage> {
        let fault = fail_after.map(Fault::FailAfter);
        self.install_inner_with_permissions(
            game,
            archive,
            proposal,
            fault,
            &[],
            None,
            NativeInjectorPolicy::Refuse,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the internal install path threads the native-injector policy through journaled steps"
    )]
    fn install_inner_with_permissions(
        &mut self,
        game: &Path,
        archive: &Path,
        proposal: &InstallPlan,
        fault: Option<Fault>,
        approved: &[String],
        release: Option<&CatalogRelease>,
        policy: NativeInjectorPolicy,
    ) -> Result<ManagedPackage> {
        if let Some(release) = release {
            release.ensure_native_injector_allowed(policy)?;
        }
        let _guard = lock_store(self)?;
        self.ensure_idle()?;
        let catalog = release.map(|release| CatalogInstall {
            release,
            replacing: None,
        });
        self.install_locked(
            game, archive, proposal, fault, approved, catalog, policy, None,
        )
    }
}
