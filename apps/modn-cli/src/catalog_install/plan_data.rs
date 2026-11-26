//! CLI views over one catalog install plan.
//!
//! The plan is data from `modn_core`; these helpers project it into the values
//! the existing download, stack and report steps already use. They never
//! re-plan: every fact comes from the `CatalogPlan` the caller built.

use crate::dependency_plan;
use anyhow::{bail, Result};
use modn_core::acquisition::{ArchiveCache, CacheUse};
use modn_core::approval::Capabilities;
use modn_core::buckets::{BucketPackage, BucketSnapshot};
use modn_core::catalog_pipeline::{
    plan_install, CatalogPlan, ConsentInputs, InstallInput, InstallTarget, PlanRefusal,
    PlannedRelease,
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

/// The verified archive of every release that already has one, by reference.
pub(super) fn archives<'a>(
    releases: impl IntoIterator<Item = &'a PlannedRelease>,
) -> BTreeMap<String, PathBuf> {
    releases
        .into_iter()
        .filter_map(|release| {
            release
                .served
                .as_ref()
                .map(|served| (release.package.reference().to_string(), served.path.clone()))
        })
        .collect()
}
