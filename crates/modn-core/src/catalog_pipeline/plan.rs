//! The typed result of catalog install planning.

use crate::acquisition::{PlannedSource, ReleaseDigest};
use crate::approval::{ApprovalRefusal, Capabilities};
use crate::buckets::{BucketPackage, PackageSource};
use crate::planning::PlanIssue;
use crate::transactions::{
    CatalogReference, CatalogRelease, DependencyState, NativeInjectorPolicy,
};
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlanKind {
    /// Install the selected release and any missing dependencies.
    Install,
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
}
