//! The typed result of catalog install planning.

use super::consent::installs;
use crate::acquisition::{PlannedSource, ReleaseDigest};
use crate::approval::{ApprovalRefusal, Capabilities};
use crate::buckets::{BucketPackage, PackageSource};
use crate::planning::PlanIssue;
use crate::transactions::{
    CatalogReference, CatalogRelease, DependencyState, NativeInjectorPolicy,
};
use anyhow::{bail, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// One catalog install plan.
///
/// The plan is data and never authorization: it holds no consent value, no
/// approved-executables set and no method that grants or commits. A caller
/// needs a separate consent value before anything may write.
#[derive(Debug)]
#[non_exhaustive]
pub struct CatalogPlan {
    /// The operation this plan prepares.
    pub kind: PlanKind,
    /// The release the caller selected.
    pub selected: CatalogReference,
    /// Game identifier of the selected release.
    pub game_id: String,
    /// Canonical game installation root.
    pub installation: PathBuf,
    /// Dependencies first, the selected release last. One archive per digest.
    pub order: Vec<PlannedRelease>,
    /// Releases with no supplied or verified cached archive, in order order.
    pub pending: Vec<PendingDownload>,
    /// Planned releases that conflict with installed packages.
    pub conflicts: Vec<PlanConflict>,
    /// Allow only when every installing native injector is covered.
    pub native_injector: NativeInjectorPolicy,
    /// Why this plan cannot be committed as it stands. Empty or not, it is data.
    pub refusals: Vec<PlanRefusal>,
}

/// The operation one plan prepares.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum PlanKind {
    /// Install the selected release and any missing dependencies.
    Install,
    /// Replace one installed managed package with the selected release.
    Update(Box<UpdatePlan>),
}

/// What an update plan replaces and what it must keep working.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct UpdatePlan {
    /// The managed package the plan replaces.
    pub installed: InstalledPackage,
    /// Version of the installed release, when a bucket release pins its archive.
    pub installed_version: Option<String>,
    /// Game-relative files and exclusive folders that the policy plans as removed.
    pub replaced: Vec<String>,
    /// Whether the installed release already is the target release.
    pub up_to_date: bool,
    /// Managed packages that depend on the installed package.
    pub dependents: Vec<PlannedDependent>,
}

/// The installed managed package that one update plan replaces.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct InstalledPackage {
    /// Managed package ID.
    pub id: String,
    /// Whether the package is pinned against updates.
    pub pinned: bool,
    /// Archive digest recorded at installation.
    pub archive_blake3: String,
    /// Serving source recorded at installation, when recorded.
    pub source: Option<PackageSource>,
}

/// One managed package that depends on the package an update replaces.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PlannedDependent {
    /// Managed package ID of the dependent.
    pub package_id: String,
    /// Catalog identity of the dependent, or empty for a standalone package.
    pub package: String,
    /// Whether the dependent is pinned.
    pub pinned: bool,
    /// The dependent's recorded requirement on the replaced package.
    pub requirement: DependentRequirement,
    /// Whether the requirement accepts the target release.
    pub satisfied: bool,
}

/// A dependent's recorded requirement on the package an update replaces.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DependentRequirement {
    /// No usable requirement was recorded.
    Unknown,
    /// The dependent recorded no version requirement.
    Unrestricted,
    /// A version requirement, such as `>=1.31.0`.
    Requires(String),
    /// The dependent needs the installed archive under another catalog name.
    Alias(String),
}

/// One release of a plan's install order, with the evidence the plan read.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PlannedRelease {
    /// Published release, kept for version, sources, executables and format.
    pub package: BucketPackage,
    /// Pinned dependency and conflict evidence from the one snapshot.
    pub release: CatalogRelease,
    /// The digest Modn has for the release.
    pub digest: ReleaseDigest,
    /// Archive already available for this release, supplied or cached.
    pub served: Option<ServedArchive>,
    /// Installed state for a dependency; `None` for the selected release.
    pub state: Option<DependencyState>,
    /// Capabilities this release needs.
    pub needed: Capabilities,
    /// Stored approval for this reference and installation, when one exists.
    pub stored: Option<Capabilities>,
}

/// An archive that is already available for one release.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ServedArchive {
    /// Archive file on disk.
    pub path: PathBuf,
    /// Manifest source that serves it: a supplied flag, the cache, or the
    /// manifest's own primary source.
    pub source: PackageSource,
    /// Digest of the served archive.
    pub digest: ReleaseDigest,
    /// Where the archive came from.
    pub origin: ServedOrigin,
}

/// Where a served archive came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ServedOrigin {
    /// A `--file` archive that matched the release pin.
    Supplied,
    /// A verified entry of the manager profile archive cache.
    Cache,
}

/// One release whose archive the plan does not have yet.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PendingDownload {
    /// The release that needs an archive.
    pub reference: CatalogReference,
    /// Release version that the caller may have to download.
    pub version: String,
    /// The digest Modn has for the release.
    pub digest: ReleaseDigest,
    /// Every artifact source, in `plan_sources` order.
    pub sources: Vec<PlannedSource>,
    /// False when every source is manual: `--file` is the only route.
    pub downloadable: bool,
}

/// One planned release that conflicts with an installed package.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PlanConflict {
    /// The planned release that declares the conflict.
    pub reference: CatalogReference,
    /// Managed `package-id (bucket/game/mod)` identities from the store query.
    pub installed: Vec<String>,
}

/// One reason a plan cannot be committed as it stands.
///
/// A refusal is data. The caller decides which one to report and whether a dry
/// run may still succeed.
#[derive(Debug)]
#[non_exhaustive]
pub enum PlanRefusal {
    /// An installing native injector lacks a flag or stored approval.
    NativeInjector {
        /// The release that needs the approval.
        reference: CatalogReference,
    },
    /// An executable path is not covered by a flag or stored approval.
    ApprovalRequired {
        /// The formatted approval refusal.
        refusal: ApprovalRefusal,
    },
    /// The install order conflicts with a managed package.
    Conflict {
        /// The declared conflict and the installed packages it names.
        conflict: PlanConflict,
    },
    /// A dependency is damaged or installed as another release.
    DependencyNotReady {
        /// The dependency that is not ready.
        reference: CatalogReference,
        /// Its installed state.
        state: DependencyState,
    },
    /// A release has no archive and no downloadable source.
    ArchiveRequired {
        /// The release that needs an archive.
        reference: CatalogReference,
        /// The first artifact source the caller must fetch manually.
        source: PackageSource,
    },
    /// A supplied archive matches no release in the install order.
    ArchiveUnmatched {
        /// The unmatched `--file` path.
        path: PathBuf,
    },
    /// A supplied or cached archive changed after catalog verification.
    SourceChanged {
        /// The release whose archive changed.
        reference: CatalogReference,
    },
    /// The module preview returned `PlanStatus::Blocked`.
    PreviewBlocked {
        /// The release whose preview is blocked.
        reference: CatalogReference,
        /// The blockers that the preview reported.
        blockers: Vec<PlanIssue>,
    },
    /// The installed package is pinned against updates.
    PackagePinned {
        /// Managed package ID of the pinned package.
        package_id: String,
    },
    /// The target release does not resolve to an install order.
    UpdateOrder {
        /// The resolution error, as reported today.
        reason: String,
    },
    /// A managed dependent does not accept the target release.
    DependentBlocks {
        /// The dependent that would stop accepting the replaced package.
        dependent: PlannedDependent,
    },
}

impl CatalogPlan {
    /// Copy this plan's identity facts without its refusals.
    ///
    /// Re-planning uses it to compare a fresh digest under the transaction
    /// lock. Acquisition state is dropped: it never reaches the digest.
    pub(crate) fn clone_without_refusals(&self) -> Self {
        Self {
            kind: self.kind.clone(),
            selected: self.selected.clone(),
            game_id: self.game_id.clone(),
            installation: self.installation.clone(),
            order: self.order.clone(),
            pending: Vec::new(),
            conflicts: self.conflicts.clone(),
            native_injector: self.native_injector,
            refusals: Vec::new(),
        }
    }

    /// The releases this plan would place, dependencies first, selected last.
    ///
    /// A dependency that is ready, damaged or installed as another release is
    /// not placed. Releases that share one archive digest are placed once when
    /// they declare the same dependencies and conflicts; two that differ, and
    /// a selected release that shares its digest with a dependency, are
    /// refused. A release without a digest is never combined with another.
    ///
    /// # Errors
    ///
    /// Returns the refusal of a shared archive that cannot be merged.
    pub fn members(&self) -> Result<Vec<&PlannedRelease>> {
        let Some((selected, dependencies)) = self.order.split_last() else {
            bail!("the plan has no selected release");
        };
        let mut by_digest: BTreeMap<&str, &PlannedRelease> = BTreeMap::new();
        let mut members = Vec::new();
        for planned in dependencies.iter().filter(|planned| installs(planned)) {
            let Some(digest) = planned.digest.as_str() else {
                members.push(planned);
                continue;
            };
            let Some(first) = by_digest.get(digest) else {
                by_digest.insert(digest, planned);
                members.push(planned);
                continue;
            };
            if declared(&first.package) != declared(&planned.package) {
                bail!(
                    "{} and {} share one archive but declare different dependencies or conflicts; install them separately",
                    first.package.reference(),
                    planned.package.reference()
                );
            }
        }
        if selected
            .package
            .package
            .pin()
            .is_some_and(|pin| by_digest.contains_key(pin))
        {
            bail!(
                "{}: its archive is also the archive of a missing dependency; install that dependency first",
                selected.package.reference()
            );
        }
        members.push(selected);
        Ok(members)
    }
}

/// The dependency and conflict IDs that a package declares.
fn declared(package: &BucketPackage) -> (BTreeSet<&str>, BTreeSet<&str>) {
    (
        package
            .package
            .dependencies
            .iter()
            .map(String::as_str)
            .collect(),
        package
            .package
            .conflicts
            .iter()
            .map(String::as_str)
            .collect(),
    )
}
