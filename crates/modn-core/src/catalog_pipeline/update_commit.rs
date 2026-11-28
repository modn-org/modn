//! Commit one catalog update plan through the update transaction entry point.
//!
//! The routine previews the selected release, checks the consent against a
//! fresh [`PlanDigest`](super::PlanDigest), and calls
//! `Store::update_catalog_release_planned` once. That entry point re-plans the
//! installed facts under the transaction lock, so a change between the review
//! and the commit is a [`StalePlan`](super::StalePlan) with no write.

use super::acquire::{archives_of, digests_of, sources_of};
use super::commit::{native_policy, staged_approvals, verify_digest, verify_reviewable};
use super::consent::Consent;
use super::outcome::CommitOutcome;
use super::plan::{CatalogPlan, UpdatePlan};
use super::preview::{approved_for, preview_catalog_update};
use super::update::refresh_update;
use super::AcquiredArchives;
use crate::history::HistoryContext;
use crate::manager_roots::ManagerRoots;
use crate::progress::OperationControl;
use crate::state::Store;
use anyhow::{Context, Result};

/// Commit one update plan through its re-planning entry point.
pub(super) fn commit_update(
    roots: &ManagerRoots,
    plan: &CatalogPlan,
    consent: &Consent,
    acquired: &AcquiredArchives,
    update: &UpdatePlan,
    control: &OperationControl,
    store: &mut Store,
) -> Result<CommitOutcome> {
    let archives = archives_of(plan, acquired);
    let digests = digests_of(plan, acquired);
    let sources = sources_of(plan, acquired);
    let planned = plan
        .order
        .last()
        .context("the update plan has no selected release")?;
    let reference = planned.package.reference();
    let key = reference.to_string();
    let archive = archives
        .get(&key)
        .with_context(|| format!("{key}: the plan has no archive to update from"))?;
    let approved = approved_for(&reference, &consent.executables);
    let preview = preview_catalog_update(Some(&*store), roots, plan, archive, &approved)?;
    verify_digest(&preview, digests.get(&key))?;
    verify_reviewable(&preview)?;
    consent.check(plan, std::slice::from_ref(&preview))?;
    let policy = native_policy(&[planned], consent);
    let source = sources
        .get(&key)
        .with_context(|| format!("{key}: no serving source was recorded"))?;
    let history = HistoryContext::new(plan.game_id.clone(), plan.installation.clone())
        .with_source(&reference, source.clone());
    store.set_control(control.clone());
    store.set_history_context(history);
    store.stage_approvals(staged_approvals(plan, &[planned]));
    let result = store.update_catalog_release_planned(
        &plan.installation,
        &update.installed.id,
        archive,
        &planned.release,
        policy,
        &approved,
        |archive, old, store| {
            let (refreshed, preview) =
                refresh_update(roots, plan, old, store, archive, &consent.executables)?;
            verify_digest(&preview, digests.get(&key))?;
            consent.check(&refreshed, std::slice::from_ref(&preview))?;
            verify_reviewable(&preview)?;
            Ok(preview.plan)
        },
    );
    store.clear_staged_approvals();
    let package = result?;
    let operation_id = store.committed_operation_id().map(str::to_owned);
    Ok(CommitOutcome {
        packages: vec![package],
        operation_id,
        attempts: acquired.attempts.clone(),
        sources,
        up_to_date: false,
    })
}
