//! Typed catalog install plans.
//!
//! A plan records what one catalog installation would place, in which order,
//! with which archives, and what still blocks it. Every value here is data: the
//! plan holds no consent, grants no approval, writes nothing and can never
//! authorize a commit. Nothing in this module prompts, touches the network or
//! runs a game module.
//!
//! The planning steps are pure. Order resolution reads one catalog snapshot;
//! archive resolution reads supplied archives, the archive cache and each
//! release's planned sources; the remaining steps read the manager store
//! through its read-only methods.

use crate::buckets::PackageSource;
use crate::transactions::DependencyState;
use anyhow::Result;

mod acquire;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the commit step of #224 places one archive per digest"
    )
)]
mod archives;
mod commit;
mod conflicts;
mod consent;
mod dependencies;
mod digest;
mod input;
mod order;
mod outcome;
mod plan;
mod preview;
mod requirements;
mod update;
mod update_commit;
mod update_digest;

pub use acquire::{acquire_pending, AcquiredArchive, AcquiredArchives};
pub use commit::{commit_plan, CommitRequest};
pub use consent::{Consent, ExecutableApproval, StalePlan};
pub use digest::{plan_digest, PlanDigest, ReleasePreview};
pub use input::{ConsentInputs, InstallInput, InstallTarget};
pub use outcome::CommitOutcome;
pub use plan::{
    CatalogPlan, DependentRequirement, InstalledPackage, PendingDownload, PlanConflict, PlanKind,
    PlanRefusal, PlannedDependent, PlannedRelease, ServedArchive, ServedOrigin, UpdatePlan,
};
pub use preview::{preview_catalog_install, preview_catalog_release, preview_catalog_update};
pub use update::{plan_update, UpdateInput};

use archives::ResolvedArchives;
use order::OrderedRelease;

/// Plan one catalog installation.
///
/// Reads one catalog snapshot, the supplied `--file` archives, the verified
/// archive cache and the manager store's read-only methods. It never prompts,
/// downloads, runs a game module or writes. Refusals are returned in
/// [`CatalogPlan::refusals`] in the order today's install command checks them;
/// `Err` means a real error such as broken state or an unreadable file.
pub fn plan_install(input: &InstallInput<'_>) -> Result<CatalogPlan> {
    let order = order::resolve(input.catalog, input.selected)?;
    let archives = archives::resolve(&order, input.supplied, input.cache, input.cache_use)?;
    let dependencies =
        dependencies::resolve(&order, &archives, input.store, input.target.installation)?;
    let conflicts = conflicts::resolve(
        &order,
        &archives,
        &dependencies.installing,
        input.store,
        input.target.installation,
    )?;
    let requirements = requirements::resolve(
        &order,
        &dependencies.installing,
        input.store,
        input.target.installation,
        &input.consent.flags,
    )?;

    let selected_index = order.len() - 1;
    let pending = pending_downloads(&order, &archives, &dependencies.installing);
    let mut refusals = Vec::new();
    refusals.extend(requirements.native_refusals);
    for path in &archives.unmatched {
        refusals.push(PlanRefusal::ArchiveUnmatched { path: path.clone() });
    }
    if let Some(refusal) = archive_required(&pending, &order[selected_index]) {
        refusals.push(refusal);
    }
    refusals.extend(
        conflicts
            .iter()
            .cloned()
            .map(|conflict| PlanRefusal::Conflict { conflict }),
    );
    for (index, state) in dependencies.states.iter().enumerate().take(selected_index) {
        let Some(state) = state else {
            continue;
        };
        if matches!(
            state,
            DependencyState::Damaged { .. } | DependencyState::OtherRelease { .. }
        ) {
            refusals.push(PlanRefusal::DependencyNotReady {
                reference: order[index].package.reference(),
                state: state.clone(),
            });
        }
    }
    for &index in &dependencies.installing {
        if index == selected_index {
            continue;
        }
        if let Some(refusal) = archive_required(&pending, &order[index]) {
            refusals.push(refusal);
        }
    }
    refusals.extend(requirements.approval_refusals);

    let releases = order
        .into_iter()
        .enumerate()
        .map(|(index, ordered)| PlannedRelease {
            package: ordered.package,
            release: ordered.release,
            digest: archives.releases[index].digest.clone(),
            served: archives.releases[index].served.clone(),
            state: dependencies.states[index].clone(),
            needed: requirements.needed[index].clone(),
            stored: requirements.stored[index].clone(),
        })
        .collect();
    Ok(CatalogPlan {
        kind: PlanKind::Install,
        selected: input.selected.reference(),
        game_id: input.target.game_id.to_owned(),
        installation: input.target.installation.to_path_buf(),
        order: releases,
        pending,
        conflicts,
        native_injector: requirements.native_injector,
        refusals,
    })
}

/// The pending downloads of the releases this installation would place.
fn pending_downloads(
    order: &[OrderedRelease],
    archives: &ResolvedArchives,
    installing: &[usize],
) -> Vec<PendingDownload> {
    archives
        .pending
        .iter()
        .filter(|pending| {
            installing
                .iter()
                .any(|&index| order[index].package.reference() == pending.reference)
        })
        .cloned()
        .collect()
}

/// Refuse a release that has no archive and no source the caller can fetch.
fn archive_required(pending: &[PendingDownload], ordered: &OrderedRelease) -> Option<PlanRefusal> {
    let download = pending
        .iter()
        .find(|pending| pending.reference == ordered.package.reference())?;
    if download.downloadable {
        return None;
    }
    let source = download
        .sources
        .iter()
        .find(|planned| matches!(planned.source, PackageSource::Manual { .. }))
        .or_else(|| download.sources.first())
        .map_or_else(
            || ordered.package.package.primary_source().clone(),
            |planned| planned.source.clone(),
        );
    Some(PlanRefusal::ArchiveRequired {
        reference: download.reference.clone(),
        source,
    })
}

#[cfg(test)]
mod tests;
