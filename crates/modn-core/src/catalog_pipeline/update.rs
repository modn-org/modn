//! Plan one catalog update and refresh its installed facts under the lock.
//!
//! The plan reads one catalog snapshot, the read-only manager store and the
//! archive cache. It never prompts, downloads, runs a game module or writes.
//! It records as data what the update command decides before it commits:
//! the installed package, the release it replaces, its dependents and every
//! refusal that gates a commit.

use super::order::OrderedRelease;
use super::plan::{
    CatalogPlan, DependentRequirement, InstalledPackage, PendingDownload, PlanKind, PlanRefusal,
    PlannedDependent, PlannedRelease, UpdatePlan,
};
use super::requirements;
use super::{archive_required, archives, conflicts};
use crate::acquisition::{ArchiveCache, CacheUse};
use crate::buckets::{BucketPackage, BucketSnapshot, PackageManifest, PackageSource, VersionReq};
use crate::manager_roots::ManagerRoots;
use crate::state::Store;
use crate::transactions::{CatalogReference, DependencyState, ManagedPackage, RecordedDependency};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// Everything one catalog update needs, except prompts and output.
///
/// Every field is data or a durable read. The value grants nothing: the core
/// never prompts and never derives a consent value from it.
#[derive(Clone)]
#[non_exhaustive]
pub struct UpdateInput<'a> {
    /// The target release the caller selected after ambiguity resolution.
    pub selected: &'a BucketPackage,
    /// The one snapshot this command uses for pins, versions and dependents.
    pub catalog: &'a BucketSnapshot,
    /// Read-only manager state.
    pub store: &'a Store,
    /// The game and installation that the plan targets.
    pub target: super::InstallTarget<'a>,
    /// The command's `--file` values, not yet matched or verified.
    pub supplied: &'a [PathBuf],
    /// The manager profile archive cache.
    pub cache: &'a ArchiveCache,
    /// Whether a verified cache entry may serve a release.
    pub cache_use: CacheUse,
    /// The command-line grants, as data.
    pub consent: super::ConsentInputs,
}

impl<'a> UpdateInput<'a> {
    /// Build one update plan input.
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "the plan input keeps its target release, catalog, state, target, archives and consent explicit"
    )]
    pub fn new(
        selected: &'a BucketPackage,
        catalog: &'a BucketSnapshot,
        store: &'a Store,
        target: super::InstallTarget<'a>,
        supplied: &'a [PathBuf],
        cache: &'a ArchiveCache,
        cache_use: CacheUse,
        consent: super::ConsentInputs,
    ) -> Self {
        Self {
            selected,
            catalog,
            store,
            target,
            supplied,
            cache,
            cache_use,
            consent,
        }
    }
}

/// Plan one catalog update of `input.installed` to `input.selected`.
///
/// The plan holds the installed package, the release it replaces, the
/// replaced paths, whether the target is already current, every dependent
/// with its recorded requirement, and the refusals that gate a commit. It
/// never prompts, downloads, runs a game module or writes.
///
/// # Errors
///
/// Returns an error when no single managed package comes from the target
/// reference, when the target does not resolve to a catalog release, or when
/// manager state cannot be read.
pub fn plan_update(input: &UpdateInput<'_>) -> Result<CatalogPlan> {
    let installation = input.target.installation;
    let reference = input.selected.reference();
    let old = installed_package(input.store.installed(installation)?, &reference)?;
    let releases = input.catalog.releases(
        &input.selected.bucket,
        &input.selected.package.game,
        &input.selected.package.id,
    )?;
    let installed_version = releases
        .iter()
        .find(|release| release.package.pin() == Some(old.archive_blake3.as_str()))
        .map(|release| release.package.version.clone())
        .or_else(|| old.version.clone());
    let up_to_date = same_release(
        installed_identity(&old),
        target_identity(&input.selected.package, &old),
    );

    let mut refusals = Vec::new();
    if old.pinned {
        refusals.push(PlanRefusal::PackagePinned {
            package_id: old.id.clone(),
        });
    }
    if let Err(error) = input.catalog.install_order(input.selected) {
        refusals.push(PlanRefusal::UpdateOrder {
            reason: format!("{error:#}"),
        });
    }
    let release = input.catalog.release(input.selected)?;
    let order = [OrderedRelease {
        package: input.selected.clone(),
        release,
    }];
    let resolved = archives::resolve_update(&order, input.supplied, input.cache, input.cache_use)?;
    for path in &resolved.unmatched {
        refusals.push(PlanRefusal::ArchiveUnmatched { path: path.clone() });
    }
    for (dependency, state) in order[0].release.dependencies.iter().zip(
        input
            .store
            .dependency_states(installation, &order[0].release.dependencies)?,
    ) {
        if !matches!(state, DependencyState::Ready { .. }) {
            refusals.push(PlanRefusal::DependencyNotReady {
                reference: dependency.reference.clone(),
                state,
            });
        }
    }
    let dependents = dependent_checks(
        input.store,
        installation,
        &old,
        Some(input.catalog),
        input.selected,
        &[],
    )?;
    for dependent in dependents.iter().filter(|check| !check.satisfied) {
        refusals.push(PlanRefusal::DependentBlocks {
            dependent: dependent.clone(),
        });
    }
    let conflicts = conflicts::resolve(&order, &resolved, &[0], Some(input.store), installation)?;
    refusals.extend(
        conflicts
            .iter()
            .cloned()
            .map(|conflict| PlanRefusal::Conflict { conflict }),
    );
    let requirements = requirements::resolve(
        &order,
        &[0],
        Some(input.store),
        installation,
        &input.consent.flags,
    )?;
    let pending = if up_to_date {
        Vec::new()
    } else {
        let pending: Vec<PendingDownload> = resolved.pending.clone();
        if let Some(refusal) = archive_required(&pending, &order[0]) {
            refusals.push(refusal);
        }
        pending
    };
    if !up_to_date {
        refusals.extend(requirements.native_refusals);
        refusals.extend(requirements.approval_refusals);
    }
    Ok(CatalogPlan {
        kind: PlanKind::Update(Box::new(UpdatePlan {
            installed: installed_package_of(&old),
            installed_version,
            replaced: replaced_paths(&old),
            up_to_date,
            dependents,
        })),
        selected: reference,
        game_id: input.target.game_id.to_owned(),
        installation: installation.to_path_buf(),
        order: vec![PlannedRelease {
            package: input.selected.clone(),
            release: order[0].release.clone(),
            digest: resolved.releases[0].digest.clone(),
            served: resolved.releases[0].served.clone(),
            state: None,
            needed: requirements.needed[0].clone(),
            stored: requirements.stored[0].clone(),
        }],
        pending,
        conflicts,
        native_injector: requirements.native_injector,
        refusals,
    })
}

/// Refresh the installed facts of an update plan under the transaction lock.
///
/// The returned plan carries the same order, conflicts and capabilities as
/// the reviewed plan, with the installed package, replaced paths,
/// up-to-date flag and dependents re-read from current state. It holds no
/// refusals; the caller checks its digest with the consent.
///
/// # Errors
///
/// Returns an error for a plan that is not an update, a missing selected
/// release, or unreadable manager state.
pub(super) fn refresh_update(
    roots: &ManagerRoots,
    plan: &CatalogPlan,
    old: &ManagedPackage,
    store: &Store,
    archive: &Path,
    approvals: &[super::ExecutableApproval],
) -> Result<(CatalogPlan, super::ReleasePreview)> {
    let PlanKind::Update(update) = &plan.kind else {
        bail!("the plan is not an update");
    };
    let target = plan
        .order
        .last()
        .context("the update plan has no selected release")?;
    let dependents = dependent_checks(
        store,
        &plan.installation,
        old,
        None,
        &target.package,
        &update.dependents,
    )?;
    let mut refreshed = plan.clone_without_refusals();
    refreshed.kind = PlanKind::Update(Box::new(UpdatePlan {
        installed: installed_package_of(old),
        installed_version: if old.archive_blake3 == update.installed.archive_blake3 {
            update.installed_version.clone()
        } else {
            old.version.clone()
        },
        replaced: replaced_paths(old),
        up_to_date: same_release(
            installed_identity(old),
            target_identity(&target.package.package, old),
        ),
        dependents,
    }));
    let approved = super::preview::approved_for(&plan.selected, approvals);
    let preview =
        super::preview_catalog_update(Some(store), roots, &refreshed, archive, &approved)?;
    Ok((refreshed, preview))
}

/// The one managed package installed from `reference`.
fn installed_package(
    packages: Vec<ManagedPackage>,
    reference: &CatalogReference,
) -> Result<ManagedPackage> {
    let mut matches: Vec<ManagedPackage> = packages
        .into_iter()
        .filter(|package| package.catalog.as_ref() == Some(reference))
        .collect();
    match matches.len() {
        0 => bail!("{reference} is not a managed package here; install it with modn install"),
        1 => Ok(matches.remove(0)),
        _ => bail!(
            "several managed packages come from {reference}: {}; uninstall the extra ones first",
            matches
                .iter()
                .map(|package| package.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn installed_package_of(old: &ManagedPackage) -> InstalledPackage {
    InstalledPackage {
        id: old.id.clone(),
        pinned: old.pinned,
        archive_blake3: old.archive_blake3.clone(),
        source: old.source.clone(),
        files: old
            .files
            .iter()
            .map(|file| file.destination.clone())
            .collect(),
        directories: old.directories.clone(),
    }
}

fn replaced_paths(old: &ManagedPackage) -> Vec<String> {
    old.files
        .iter()
        .map(|file| file.destination.clone())
        .chain(old.directories.iter().cloned())
        .collect()
}

#[derive(Clone, Copy)]
enum Identity<'a> {
    Archive(&'a str),
    Commit(&'a str),
    Unknown,
}

fn source_identity(manifest: &PackageManifest) -> Identity<'_> {
    match manifest.primary_source() {
        PackageSource::Git { commit, .. } => Identity::Commit(commit),
        _ => manifest.pin().map_or(Identity::Unknown, Identity::Archive),
    }
}

fn listed_git_commit<'a>(
    manifest: &'a PackageManifest,
    recorded: &PackageSource,
) -> Option<&'a str> {
    let PackageSource::Git { repository, .. } = recorded else {
        return None;
    };
    manifest.sources.iter().find_map(|source| match source {
        PackageSource::Git {
            repository: listed,
            commit,
        } if listed == repository => Some(commit.as_str()),
        _ => None,
    })
}

fn target_identity<'a>(manifest: &'a PackageManifest, installed: &ManagedPackage) -> Identity<'a> {
    installed
        .source
        .as_ref()
        .and_then(|source| listed_git_commit(manifest, source))
        .map_or_else(|| source_identity(manifest), Identity::Commit)
}

fn installed_identity(package: &ManagedPackage) -> Identity<'_> {
    match &package.source {
        Some(PackageSource::Git { commit, .. }) => Identity::Commit(commit),
        _ => Identity::Archive(&package.archive_blake3),
    }
}

fn same_release(installed: Identity<'_>, target: Identity<'_>) -> bool {
    match (installed, target) {
        (Identity::Archive(installed), Identity::Archive(target))
        | (Identity::Commit(installed), Identity::Commit(target)) => installed == target,
        _ => false,
    }
}

/// The dependents of `old`, with their recorded requirement and acceptance.
///
/// `catalog` resolves a legacy dependent's requirement from the release that
/// is installed. Without it, a legacy dependent keeps the requirement that
/// `previous` recorded for it.
fn dependent_checks(
    store: &Store,
    game: &Path,
    old: &ManagedPackage,
    catalog: Option<&BucketSnapshot>,
    target: &BucketPackage,
    previous: &[PlannedDependent],
) -> Result<Vec<PlannedDependent>> {
    store
        .dependents_of(game, &old.id)?
        .into_iter()
        .map(|dependent| {
            let requirement = match store.recorded_dependencies(&dependent.id)? {
                Some(recorded) => recorded_requirement(recorded, old, target),
                None => previous
                    .iter()
                    .find(|check| check.package_id == dependent.id)
                    .map_or_else(
                        || {
                            catalog.map_or(DependentRequirement::Unknown, |catalog| {
                                legacy_requirement(catalog, &dependent, target)
                            })
                        },
                        |check| check.requirement.clone(),
                    ),
            };
            let satisfied = match &requirement {
                DependentRequirement::Requires(text) => VersionReq::parse(text)?
                    .matches(&target.package.version)
                    .unwrap_or(false),
                DependentRequirement::Unrestricted => true,
                DependentRequirement::Unknown | DependentRequirement::Alias(_) => false,
            };
            Ok(PlannedDependent {
                package: dependent
                    .catalog
                    .as_ref()
                    .map_or_else(String::new, ToString::to_string),
                package_id: dependent.id,
                pinned: dependent.pinned,
                requirement,
                satisfied,
            })
        })
        .collect()
}

fn recorded_requirement(
    recorded: Vec<RecordedDependency>,
    old: &ManagedPackage,
    target: &BucketPackage,
) -> DependentRequirement {
    let updated = target.reference();
    if let Some(alias) = recorded
        .iter()
        .find(|row| row.reference != updated && row.archive_blake3 == old.archive_blake3)
    {
        return DependentRequirement::Alias(alias.reference.to_string());
    }
    recorded
        .into_iter()
        .find(|row| row.reference == updated)
        .map_or(DependentRequirement::Unknown, |row| {
            row.requirement.map_or(
                DependentRequirement::Unrestricted,
                DependentRequirement::Requires,
            )
        })
}

fn legacy_requirement(
    catalog: &BucketSnapshot,
    dependent: &ManagedPackage,
    target: &BucketPackage,
) -> DependentRequirement {
    let Some(reference) = &dependent.catalog else {
        return DependentRequirement::Unknown;
    };
    let installed = catalog
        .releases(&reference.bucket, &reference.game_id, &reference.mod_id)
        .ok()
        .and_then(|releases| {
            releases
                .into_iter()
                .find(|release| release.package.pin() == Some(dependent.archive_blake3.as_str()))
        });
    match installed {
        Some(release)
            if reference.bucket == target.bucket
                && release.package.dependencies.contains(&target.package.id) =>
        {
            release
                .package
                .dependency_versions
                .get(&target.package.id)
                .cloned()
                .map_or(
                    DependentRequirement::Unrestricted,
                    DependentRequirement::Requires,
                )
        }
        _ => DependentRequirement::Unknown,
    }
}
