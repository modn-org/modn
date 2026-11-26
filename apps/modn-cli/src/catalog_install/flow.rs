//! The argument, consent-prompt and output flow of one catalog install.

use super::{plan_data, reference_of, refuse_unpromptable};
use crate::catalog_approval;
use crate::commands::InstallArgs;
use crate::dependency_plan;
use crate::download;
use crate::install_consent;
use crate::install_report::{self, ArchivePreview};
use crate::progress::Progress;
use crate::selection::selected_installation;
use crate::state_access::{ManagerState, ProtectedTarget};
use anyhow::{bail, Result};
use modn_core::acquisition::{ArchiveCache, CacheUse, SourceAttempt, SourcePolicy};
use modn_core::approval::Capabilities;
use modn_core::buckets::{BucketPackage, BucketSnapshot};
use modn_core::catalog_pipeline::{
    acquire_pending, preview_catalog_install, AcquiredArchives, CatalogPlan, ExecutableApproval,
    PlanRefusal, PlannedRelease, ReleasePreview,
};
use modn_core::error::{coded, ErrorCode};
use modn_core::manager_roots::ManagerRoots;
use modn_core::state::Store;
use modn_core::transactions::CatalogReference;
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::Path;

/// Resolve, plan, confirm and commit one catalog install.
pub(super) fn install_mod(
    selected: BucketPackage,
    catalog: &BucketSnapshot,
    args: &InstallArgs,
    roots: &ManagerRoots,
    json: bool,
    progress: &Progress,
) -> Result<()> {
    if progress.enabled() && !args.yes && !args.dry_run {
        return Err(coded(
            ErrorCode::ApprovalRequired,
            "installation writes game files; pass --yes with --progress json",
        ));
    }
    let reference_key = reference_of(&selected);
    let reference = reference_key.to_string();
    let interactive = !args.yes && !args.dry_run && !progress.enabled();
    let mut order = catalog.install_order(&selected)?;
    order.pop();
    catalog_approval::check_native_approvals(&selected, &order, args, roots)?;
    let flags = install_consent::flags(args.allow_native_injector, &args.allow_executable);
    let cache = roots.archive_cache();
    let cache_use = if args.no_cache {
        CacheUse::Refresh
    } else {
        CacheUse::Reuse
    };
    let state = ManagerState::load_deferred(roots)?;
    let selection = selected_installation(
        roots,
        state.existing(),
        args.path.clone(),
        Some(&selected.package.game),
    );
    let (game_id, installation, mut plan) = match selection {
        Ok((game_id, installation)) => {
            let plan = plan_data::plan(
                &selected,
                catalog,
                state.existing(),
                &game_id,
                &installation,
                &args.file,
                &cache,
                cache_use,
                &flags,
            )?;
            (game_id, installation, plan)
        }
        Err(error) => {
            let probe = plan_data::plan(
                &selected,
                catalog,
                None,
                &selected.package.game,
                Path::new(""),
                &args.file,
                &cache,
                cache_use,
                &flags,
            )?;
            plan_data::archive_refusals(&probe, &reference_key, &reference, args.dry_run)?;
            refuse_unpromptable(interactive, json)?;
            return Err(error);
        }
    };
    plan_data::archive_refusals(&plan, &reference_key, &reference, args.dry_run)?;
    refuse_unpromptable(interactive, json)?;
    if !args.dry_run || args.download {
        state.validate_download(
            cache.root(),
            &ProtectedTarget {
                game: Some(game_id.clone()),
                root: Some(installation.clone()),
            },
        )?;
    }
    let linked = plan_data::installing(&plan);
    download::validate_links(&args.nxm_links, &linked)?;

    if args.dry_run {
        let mut attempts = Vec::new();
        if args.download {
            let acquired = acquire(&plan, &cache, cache_use, &args.nxm_links, json, progress)?;
            plan = plan_data::plan(
                &selected,
                catalog,
                state.existing(),
                &game_id,
                &installation,
                &args.file,
                &cache,
                // The batch just verified and cached these archives, so the
                // report reads them back even after `--no-cache`.
                CacheUse::Reuse,
                &flags,
            )?;
            acquired.apply_to(&mut plan);
            attempts = acquired.attempts;
        }
        return dry_run_report(
            roots,
            &plan,
            &reference_key,
            &flags,
            &args.allow_executable,
            &attempts,
            json,
        );
    }

    let conflicts = install_report::conflict_lines(&plan);
    if !conflicts.is_empty() {
        bail!(
            "installation blocked by declared conflicts; uninstall the installed packages first:\n  {}",
            conflicts.join("\n  ")
        );
    }
    let members = plan.members()?;
    let stack = members.len() > 1;
    if stack {
        missing_dependency_archives(&plan, &members)?;
    } else if let Some(PlanRefusal::DependencyNotReady { reference, .. }) = plan
        .refusals
        .iter()
        .find(|refusal| matches!(refusal, PlanRefusal::DependencyNotReady { .. }))
    {
        bail!("dependency not installed or intact: {reference}; install or restore it first");
    }

    if stack {
        super::commit::install_stack(
            &mut plan,
            &selected,
            catalog,
            args,
            roots,
            &cache,
            cache_use,
            state,
            &game_id,
            &installation,
            &flags,
            interactive,
            progress,
            json,
        )?;
    } else {
        super::commit::install_release(
            &mut plan,
            &selected,
            catalog,
            args,
            roots,
            &cache,
            cache_use,
            state,
            &game_id,
            &installation,
            &reference_key,
            &reference,
            &flags,
            interactive,
            progress,
            json,
        )?;
    }
    Ok(())
}

/// Acquire the pending archives of `plan`, reporting through `progress`.
pub(super) fn acquire(
    plan: &CatalogPlan,
    cache: &ArchiveCache,
    cache_use: CacheUse,
    links: &[crate::nxm::NxmLink],
    json: bool,
    progress: &Progress,
) -> Result<AcquiredArchives> {
    let text = !json && !progress.enabled();
    let mut report = io::stderr();
    let resolver = download::resolver(links, text, &mut report);
    acquire_pending(
        plan,
        cache,
        cache_use,
        download::transport(),
        SourcePolicy::default(),
        &resolver,
        progress.control(),
    )
}

/// Acquire the pending archives of `plan` and plan again from the cache.
#[expect(
    clippy::too_many_arguments,
    reason = "the re-plan keeps the selected release, catalog, state, target, supplied files, cache and consent explicit"
)]
pub(super) fn acquire_and_replan(
    plan: &mut CatalogPlan,
    selected: &BucketPackage,
    catalog: &BucketSnapshot,
    args: &InstallArgs,
    cache: &ArchiveCache,
    cache_use: CacheUse,
    store: Option<&Store>,
    game_id: &str,
    installation: &Path,
    flags: &Capabilities,
    progress: &Progress,
    json: bool,
) -> Result<Vec<SourceAttempt>> {
    let acquired = acquire(plan, cache, cache_use, &args.nxm_links, json, progress)?;
    *plan = plan_data::plan(
        selected,
        catalog,
        store,
        game_id,
        installation,
        &args.file,
        cache,
        // The batch just verified and cached these archives, so the install
        // reads them back even after `--no-cache`.
        CacheUse::Reuse,
        flags,
    )?;
    acquired.apply_to(plan);
    Ok(acquired.attempts)
}

/// Preview the selected release when its archive is already available.
pub(super) fn preview_selected(
    roots: &ManagerRoots,
    plan: &CatalogPlan,
    approved: &[String],
) -> Result<Option<ReleasePreview>> {
    let Some(served) = &plan_data::selected(plan).served else {
        return Ok(None);
    };
    let reference = plan.selected.clone();
    let archives = BTreeMap::from([(reference.to_string(), served.path.clone())]);
    let approvals = vec![ExecutableApproval::new(reference, approved.iter().cloned())];
    let previews = preview_catalog_install(roots, plan, &archives, &approvals)?;
    Ok(previews.into_iter().next())
}

/// The member that shares a refreshable archive is also the archive of a
/// missing dependency; refuse it before any prompt or download.
fn missing_dependency_archives(plan: &CatalogPlan, members: &[&PlannedRelease]) -> Result<()> {
    let mut absent = Vec::new();
    for refusal in &plan.refusals {
        let PlanRefusal::ArchiveRequired { reference, source } = refusal else {
            continue;
        };
        if *reference == plan.selected
            || !members
                .iter()
                .any(|member| member.package.reference() == *reference)
        {
            continue;
        }
        let Some(planned) = plan
            .order
            .iter()
            .find(|planned| planned.package.reference() == *reference)
        else {
            continue;
        };
        absent.push(format!(
            "  dependency not installed: {} {}; {}",
            reference,
            planned.package.package.version,
            dependency_plan::acquisition(source)
        ));
    }
    if absent.is_empty() {
        return Ok(());
    }
    bail!(
        "archives missing for this installation (manual download required):\n{}\nPass --file once for each archive.",
        absent.join("\n")
    )
}

/// Print the dry-run report of `plan`.
fn dry_run_report(
    roots: &ManagerRoots,
    plan: &CatalogPlan,
    reference_key: &CatalogReference,
    flags: &Capabilities,
    allow_executable: &[String],
    attempts: &[SourceAttempt],
    json: bool,
) -> Result<()> {
    let planned = plan_data::selected(plan);
    let mut candidate_approvals = allow_executable.to_vec();
    for path in &planned.package.package.executables {
        if !candidate_approvals.contains(path) {
            candidate_approvals.push(path.clone());
        }
    }
    let candidate = if candidate_approvals.is_empty() {
        None
    } else {
        preview_selected(roots, plan, &candidate_approvals)?
    };
    let present = candidate.as_ref().map_or_else(BTreeSet::new, |preview| {
        let destinations: Vec<&str> = preview
            .plan
            .operations
            .iter()
            .map(|operation| operation.destination.as_str())
            .collect();
        install_consent::present_executables(&planned.package.package.executables, &destinations)
    });
    let needed = Capabilities::new(planned.package.package.native_injector, present);
    let check = install_consent::ApprovalCheck::new(
        reference_key.clone(),
        needed,
        planned.stored.clone(),
        flags,
    );
    let pending = check.uncovered.executables.clone();
    let approved = check.approved_executables();
    let preview = preview_selected(roots, plan, &approved)?;
    let archive = preview.as_ref().map(|preview| ArchivePreview {
        preview,
        unapproved_declared_executables: &pending,
    });
    install_report::print(plan, archive, Some(&check), attempts, json)
}
