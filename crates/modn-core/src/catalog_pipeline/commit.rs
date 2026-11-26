//! Commit one catalog plan through the transaction layer.
//!
//! `commit_plan` is the one routine that turns a planned catalog install into
//! game files, ownership rows and history. It needs a [`CatalogPlan`] and a
//! separate [`Consent`]; a plan alone is never authorization. Missing archives
//! are acquired before any transaction lock is taken. The transaction layer is
//! then called exactly once: a single release through the re-planning entry
//! point, several releases through `install_catalog_stack`. Each closure
//! re-runs the game-module preview under the lock and compares a fresh
//! [`PlanDigest`](super::PlanDigest) with the consent; a mismatch is a
//! [`StalePlan`](super::StalePlan) refusal with no write. Approvals are
//! recorded only after the transaction returns.
//!
//! The lock, the journal and the ownership writes stay in
//! `crate::transactions`; this module opens no second write path.

use super::acquire::{acquire_pending, archives_of, digests_of, sources_of};
use super::consent::Consent;
use super::digest::ReleasePreview;
use super::outcome::{blocking_refusal, CommitOutcome};
use super::plan::{CatalogPlan, PlannedRelease};
use super::preview::{approved_for, preview_catalog_install, preview_release};
use crate::acquisition::{
    ArchiveCache, CacheUse, ReleaseDigest, ReleaseResolver, SourcePolicy, SourceTransport,
};
use crate::approval::Capabilities;
use crate::error::{coded, ErrorCode};
use crate::history::HistoryContext;
use crate::manager_roots::ManagerRoots;
use crate::planning::InstallPlan;
use crate::progress::OperationControl;
use crate::state::Store;
use crate::transactions::{ManagedPackage, NativeInjectorPolicy, StackMember};
use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// Everything one catalog commit needs.
///
/// The caller opens and validates the writable [`Store`]; moving state-location
/// validation into core is out of scope. `roots` and `cache` locate the game
/// module and the verified archive cache. Every other field is the plan, the
/// consent the user gave for it, and the acquisition seams.
#[non_exhaustive]
pub struct CommitRequest<'a> {
    /// The manager-owned locations for the module registry and archive cache.
    pub roots: &'a ManagerRoots,
    /// The verified archive cache of the manager profile.
    pub cache: &'a ArchiveCache,
    /// The plan the user reviewed.
    pub plan: &'a CatalogPlan,
    /// The user's separate consent for that plan.
    pub consent: &'a Consent,
    /// Network transport for artifact sources.
    pub transport: &'a dyn SourceTransport,
    /// Resolver for Git, Nexus and provider metadata steps.
    pub resolver: &'a dyn ReleaseResolver,
    /// Timeout and retry policy of each source.
    pub source_policy: SourcePolicy,
    /// Whether a verified cache entry may serve a release.
    pub cache_use: CacheUse,
    /// Progress and cancellation of the whole operation.
    pub control: &'a OperationControl,
    /// The writable manager state, already opened and validated by the caller.
    pub store: &'a mut Store,
}

impl<'a> CommitRequest<'a> {
    /// Build one commit request.
    ///
    /// The request carries data only. It derives no consent from the plan and
    /// grants nothing by itself.
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "the request keeps the plan, consent, acquisition seams and open store explicit"
    )]
    pub fn new(
        roots: &'a ManagerRoots,
        cache: &'a ArchiveCache,
        plan: &'a CatalogPlan,
        consent: &'a Consent,
        transport: &'a dyn SourceTransport,
        resolver: &'a dyn ReleaseResolver,
        source_policy: SourcePolicy,
        cache_use: CacheUse,
        control: &'a OperationControl,
        store: &'a mut Store,
    ) -> Self {
        Self {
            roots,
            cache,
            plan,
            consent,
            transport,
            resolver,
            source_policy,
            cache_use,
            control,
            store,
        }
    }
}

/// Commit one catalog plan.
///
/// The routine refuses a plan that carries a blocking refusal, acquires every
/// missing release before the lock, previews every release that it would
/// install, and checks the consent against a freshly derived
/// [`PlanDigest`](super::PlanDigest). It then calls the transaction layer once
/// and records the approvals after the call returns.
///
/// # Errors
///
/// Returns a coded refusal for a blocking plan, a [`StalePlan`](super::StalePlan)
/// when the plan changed, the acquisition error unchanged (including
/// `CANCELLED`), or the transaction error unchanged. Nothing is written when a
/// check or the transaction refuses.
pub fn commit_plan(request: CommitRequest<'_>) -> Result<CommitOutcome> {
    let CommitRequest {
        roots,
        cache,
        plan,
        consent,
        transport,
        resolver,
        source_policy,
        cache_use,
        control,
        store,
    } = request;
    if let Some(error) = blocking_refusal(plan) {
        return Err(error);
    }
    let acquired = acquire_pending(
        plan,
        cache,
        cache_use,
        transport,
        source_policy,
        resolver,
        control,
    )?;
    let archives = archives_of(plan, &acquired);
    let digests = digests_of(plan, &acquired);
    let sources = sources_of(plan, &acquired);
    let installing = plan.members()?;
    for planned in &installing {
        let key = planned.package.reference().to_string();
        if !archives.contains_key(&key) {
            return Err(coded(
                ErrorCode::ArchiveRequired,
                format!("{key}: the plan has no archive to install"),
            ));
        }
    }
    let previews = preview_catalog_install(roots, plan, &archives, &consent.executables)?;
    for preview in &previews {
        verify_digest(preview, digests.get(&preview.reference.to_string()))?;
    }
    if installing.len() == 1 {
        for preview in &previews {
            verify_reviewable(preview)?;
        }
    }
    consent.check(plan, &previews)?;
    let policy = native_policy(&installing, consent);
    let mut history = HistoryContext::new(plan.game_id.clone(), plan.installation.clone());
    for planned in &installing {
        let reference = planned.package.reference();
        let key = reference.to_string();
        let source = sources
            .get(&key)
            .with_context(|| format!("{key}: no serving source was recorded"))?;
        history = history.with_source(&reference, source.clone());
    }
    store.set_control(control.clone());
    store.set_history_context(history);
    let mut destinations: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let packages = if installing.len() == 1 {
        install_one(
            store,
            plan,
            consent,
            roots,
            &installing,
            &archives,
            &digests,
            policy,
            &mut destinations,
        )?
    } else {
        install_stack(
            store,
            plan,
            consent,
            roots,
            &installing,
            &archives,
            &digests,
            policy,
            &mut destinations,
        )?
    };
    let operation_id = store.committed_operation_id().map(str::to_owned);
    record_approvals(store, plan, &installing, &destinations)?;
    Ok(CommitOutcome {
        packages,
        operation_id,
        attempts: acquired.attempts,
        sources,
    })
}

/// Install the one release of `installing` through the re-planning entry point.
#[expect(
    clippy::too_many_arguments,
    reason = "the install step keeps the plan evidence and the commit bookkeeping explicit"
)]
fn install_one(
    store: &mut Store,
    plan: &CatalogPlan,
    consent: &Consent,
    roots: &ManagerRoots,
    installing: &[&PlannedRelease],
    archives: &BTreeMap<String, PathBuf>,
    digests: &BTreeMap<String, ReleaseDigest>,
    policy: NativeInjectorPolicy,
    destinations: &mut BTreeMap<String, BTreeSet<String>>,
) -> Result<Vec<ManagedPackage>> {
    let planned = installing[0];
    let reference = planned.package.reference();
    let key = reference.to_string();
    let archive = archives
        .get(&key)
        .with_context(|| format!("{key}: the plan has no archive to install"))?;
    let approved = approved_for(&reference, &consent.executables);
    let expected = digests.get(&key).cloned();
    let package = store.install_catalog_release_planned(
        &plan.installation,
        archive,
        &planned.release,
        policy,
        &approved,
        |archive_path| {
            let preview = ReleasePreview {
                reference: reference.clone(),
                plan: preview_release(
                    roots,
                    &plan.game_id,
                    &plan.installation,
                    archive_path,
                    &approved,
                )?,
            };
            verify_preview(&preview, expected.as_ref())?;
            consent.check(plan, std::slice::from_ref(&preview))?;
            destinations.insert(key.clone(), destinations_of(&preview.plan));
            Ok(preview.plan)
        },
    )?;
    Ok(vec![package])
}

/// Install every release of `installing` as one stack.
///
/// The first closure call re-previews every installing release before the stack
/// writes any member, and compares the whole digest with the consent. Later
/// calls preview their member again, as a stack does today, so game policy
/// sees the files of earlier members. A stale first call refuses before any
/// member is written; a later failure keeps today's stack rollback.
#[expect(
    clippy::too_many_arguments,
    reason = "the stack step keeps the plan evidence and the commit bookkeeping explicit"
)]
fn install_stack(
    store: &mut Store,
    plan: &CatalogPlan,
    consent: &Consent,
    roots: &ManagerRoots,
    installing: &[&PlannedRelease],
    archives: &BTreeMap<String, PathBuf>,
    digests: &BTreeMap<String, ReleaseDigest>,
    policy: NativeInjectorPolicy,
    destinations: &mut BTreeMap<String, BTreeSet<String>>,
) -> Result<Vec<ManagedPackage>> {
    let members = installing
        .iter()
        .map(|planned| {
            let reference = planned.package.reference();
            let key = reference.to_string();
            let archive = archives
                .get(&key)
                .with_context(|| format!("{key}: the plan has no archive to install"))?;
            Ok(StackMember::new(
                archive.clone(),
                planned.release.clone(),
                approved_for(&reference, &consent.executables),
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut first = true;
    let installed =
        store.install_catalog_stack(&plan.installation, &members, policy, |member| {
            let reference = member.release.reference.clone();
            let key = reference.to_string();
            let expected = digests.get(&key).cloned();
            let preview = if first {
                first = false;
                let locked = preview_catalog_install(roots, plan, archives, &consent.executables)?;
                for preview in &locked {
                    verify_digest(preview, digests.get(&preview.reference.to_string()))?;
                }
                consent.check(plan, &locked)?;
                let current = locked
                    .into_iter()
                    .find(|preview| preview.reference == reference)
                    .with_context(|| format!("{key}: the re-plan returned no preview"))?;
                verify_reviewable(&current)?;
                current
            } else {
                ReleasePreview {
                    reference: reference.clone(),
                    plan: preview_release(
                        roots,
                        &plan.game_id,
                        &plan.installation,
                        &member.archive,
                        &member.approved_executables,
                    )?,
                }
            };
            verify_preview(&preview, expected.as_ref())?;
            destinations.insert(key, destinations_of(&preview.plan));
            Ok(preview.plan)
        })?;
    Ok(installed)
}

/// Record the capabilities of each installed release after the commit.
fn record_approvals(
    store: &mut Store,
    plan: &CatalogPlan,
    installing: &[&PlannedRelease],
    destinations: &BTreeMap<String, BTreeSet<String>>,
) -> Result<()> {
    for planned in installing {
        let reference = planned.package.reference();
        let key = reference.to_string();
        let present = destinations.get(&key).cloned().unwrap_or_default();
        let needed = Capabilities::new(
            planned.needed.native_injector,
            planned
                .needed
                .executables
                .iter()
                .filter(|path| present.contains(path.as_str()))
                .cloned(),
        );
        store
            .record_approval(
                &plan.installation,
                &reference.bucket,
                &reference.mod_id,
                &needed,
            )
            .with_context(|| {
                format!("{reference} is installed, but its approval could not be stored")
            })?;
    }
    Ok(())
}

/// Whether every installing native injector is covered by the consent.
fn native_policy(installing: &[&PlannedRelease], consent: &Consent) -> NativeInjectorPolicy {
    let places_native = installing
        .iter()
        .any(|planned| planned.needed.native_injector);
    let every_native_covered = installing
        .iter()
        .all(|planned| !planned.needed.native_injector || consent.native_injector);
    if places_native && every_native_covered {
        NativeInjectorPolicy::Allow
    } else {
        NativeInjectorPolicy::Refuse
    }
}

/// Check one fresh preview against the digest the plan expects.
fn verify_preview(preview: &ReleasePreview, expected: Option<&ReleaseDigest>) -> Result<()> {
    verify_digest(preview, expected)?;
    verify_reviewable(preview)
}

/// Check that the archive a preview read is the release that the plan pins.
fn verify_digest(preview: &ReleasePreview, expected: Option<&ReleaseDigest>) -> Result<()> {
    if let Some(expected) = expected {
        if expected.as_str() != Some(preview.plan.archive_blake3.as_str()) {
            return Err(coded(
                ErrorCode::DigestMismatch,
                format!(
                    "{}: source changed after catalog verification",
                    preview.reference
                ),
            ));
        }
    }
    Ok(())
}

/// Check that the game module allows one release as previewed.
fn verify_reviewable(preview: &ReleasePreview) -> Result<()> {
    if !preview.reviewable() {
        return Err(coded(
            ErrorCode::PlanBlocked,
            format!(
                "{}: installation blocked; inspect with --dry-run first",
                preview.reference
            ),
        ));
    }
    Ok(())
}

/// The destinations of one preview, for the capability record.
fn destinations_of(plan: &InstallPlan) -> BTreeSet<String> {
    plan.operations
        .iter()
        .map(|operation| operation.destination.clone())
        .collect()
}
