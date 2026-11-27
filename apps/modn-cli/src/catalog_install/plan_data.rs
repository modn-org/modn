//! CLI views over one catalog install plan.
//!
//! The plan is data from `modn_core`; these helpers project it into the values
//! the existing download, stack and report steps already use. They never
//! re-plan: every fact comes from the `CatalogPlan` the caller built.

use crate::dependency_plan;
use crate::download;
use anyhow::{bail, Result};
use modn_core::acquisition::{ArchiveCache, CacheUse};
use modn_core::approval::Capabilities;
use modn_core::buckets::{BucketPackage, BucketSnapshot, PackageSource};
use modn_core::catalog_pipeline::{
    plan_install, CatalogPlan, ConsentInputs, InstallInput, InstallTarget, PlanRefusal,
    PlannedRelease, ServedOrigin,
};
use modn_core::state::Store;
use modn_core::transactions::{CatalogReference, DependencyState};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Plan one catalog install from the values the CLI already resolved.
#[expect(
    clippy::too_many_arguments,
    reason = "the plan input keeps the selected release, catalog, state, target, supplied files, cache and consent explicit"
)]
pub(super) fn plan<'a>(
    selected: &'a BucketPackage,
    catalog: &'a BucketSnapshot,
    store: Option<&'a Store>,
    game_id: &'a str,
    installation: &'a Path,
    supplied: &'a [PathBuf],
    cache: &'a ArchiveCache,
    cache_use: CacheUse,
    flags: &Capabilities,
) -> Result<CatalogPlan> {
    plan_install(&InstallInput::new(
        selected,
        catalog,
        store,
        InstallTarget::new(game_id, installation),
        supplied,
        cache,
        cache_use,
        ConsentInputs::new(flags.clone()),
    ))
}

/// Refuse the archive findings that today's command reports before the
/// installation is selected, in the plan's check order.
pub(super) fn archive_refusals(
    plan: &CatalogPlan,
    selected: &CatalogReference,
    reference: &str,
    dry_run: bool,
) -> Result<()> {
    for refusal in &plan.refusals {
        match refusal {
            PlanRefusal::ArchiveUnmatched { path } => {
                bail!(
                    "{}: archive BLAKE3 does not match catalog manifest of {reference} or its dependencies",
                    path.display()
                );
            }
            PlanRefusal::ArchiveRequired {
                reference: release,
                source,
            } if release == selected && !dry_run => {
                bail!(
                    "{reference}: {}, then use modn install {reference} --file <archive>",
                    dependency_plan::acquisition(source)
                );
            }
            _ => {}
        }
    }
    Ok(())
}

/// The selected release, which the plan keeps last.
pub(super) fn selected(plan: &CatalogPlan) -> &PlannedRelease {
    plan.order
        .last()
        .expect("catalog plan ends with the selected release")
}

/// The releases this installation would place, selected last.
pub(super) fn installing(plan: &CatalogPlan) -> Vec<&BucketPackage> {
    plan.order
        .iter()
        .filter(|release| {
            release
                .state
                .as_ref()
                .is_none_or(|state| matches!(state, DependencyState::Missing))
        })
        .map(|release| &release.package)
        .collect()
}

/// The archive maps that acquisition, the stack step and history already take.
#[derive(Default)]
pub(super) struct PlanArchives {
    pub(super) archives: BTreeMap<String, PathBuf>,
    pub(super) digests: BTreeMap<String, String>,
    pub(super) served: BTreeMap<String, PackageSource>,
    pub(super) supplied: BTreeMap<String, download::SuppliedArchive>,
}

impl PlanArchives {
    /// Project every served archive of `plan` into the CLI maps.
    pub(super) fn of(plan: &CatalogPlan) -> Self {
        let mut archives = Self::default();
        for release in &plan.order {
            let Some(served) = &release.served else {
                continue;
            };
            let reference = release.package.reference().to_string();
            archives
                .archives
                .insert(reference.clone(), served.path.clone());
            if let Some(digest) = served.digest.as_str() {
                archives
                    .digests
                    .insert(reference.clone(), digest.to_owned());
            }
            archives
                .served
                .insert(reference.clone(), served.source.clone());
            if served.origin == ServedOrigin::Supplied {
                archives.supplied.insert(
                    reference,
                    download::SuppliedArchive {
                        path: served.path.clone(),
                        blake3: served.digest.as_str().unwrap_or_default().to_owned(),
                    },
                );
            }
        }
        archives
    }
}
