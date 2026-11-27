use crate::catalog_install::{self, reference_of};
use crate::commands::UpdateArgs;
use crate::dependency_plan;
use crate::download;
use crate::install_consent::{self, UpdateRequest};
use crate::nxm::NxmLink;
use crate::progress::Progress;
use crate::selection::selected_installation;
use crate::state_access::{ManagerState, ProtectedTarget};
use crate::update_report::{self, FileChanges, Preview};
use anyhow::{bail, Context, Result};
use modn_core::acquisition::{ArchiveCache, CacheUse, SourcePolicy};
use modn_core::approval::{ApprovalRefusal, Capabilities, Uncovered};
use modn_core::buckets::{BucketPackage, BucketSnapshot, BucketStore, PackageSource};
use modn_core::catalog_pipeline::{
    acquire_pending, commit_plan, plan_digest, plan_update, preview_catalog_update,
    AcquiredArchives, CatalogPlan, CommitRequest, Consent, ConsentInputs, ExecutableApproval,
    InstallTarget, PlanKind, PlanRefusal, PlannedRelease, ReleasePreview, UpdateInput, UpdatePlan,
};
use modn_core::error::{coded, ErrorCode};
use modn_core::manager_roots::ManagerRoots;
use modn_core::state::Store;
use modn_core::transactions::NativeInjectorPolicy;
use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

fn planned_source(release: &BucketPackage) -> &PackageSource {
    crate::dependency_plan::fetchable_source(&release.package.sources)
        .unwrap_or_else(|| release.package.primary_source())
}

fn choose_release(
    releases: Vec<BucketPackage>,
    version: Option<&str>,
    reference: &str,
) -> Result<BucketPackage> {
    let Some(version) = version else {
        return releases
            .into_iter()
            .next()
            .with_context(|| format!("{reference} has no release"));
    };
    let available: Vec<String> = releases.iter().map(|r| r.package.version.clone()).collect();
    releases
        .into_iter()
        .find(|release| release.package.version == version)
        .with_context(|| {
            format!(
                "{reference} has no release {version}; available: {}",
                available.join(", ")
            )
        })
}

#[expect(
    clippy::too_many_arguments,
    reason = "the plan call keeps the selected release, catalog, state, target, supplied files, cache and consent explicit"
)]
fn build_plan(
    target: &BucketPackage,
    catalog: &BucketSnapshot,
    store: &Store,
    game_id: &str,
    game: &Path,
    supplied: &[PathBuf],
    cache: &ArchiveCache,
    cache_use: CacheUse,
    flags: &Capabilities,
) -> Result<CatalogPlan> {
    plan_update(&UpdateInput::new(
        target,
        catalog,
        store,
        InstallTarget::new(game_id, game),
        supplied,
        cache,
        cache_use,
        ConsentInputs::new(flags.clone()),
    ))
}

fn update_facts(plan: &CatalogPlan) -> Result<&UpdatePlan> {
    match &plan.kind {
        PlanKind::Update(update) => Ok(update),
        _ => bail!("the plan is not an update"),
    }
}

fn selected(plan: &CatalogPlan) -> Result<&PlannedRelease> {
    plan.order
        .last()
        .context("the update plan has no selected release")
}

fn pinned_package(plan: &CatalogPlan) -> Option<&str> {
    plan.refusals.iter().find_map(|refusal| match refusal {
        PlanRefusal::PackagePinned { package_id } => Some(package_id.as_str()),
        _ => None,
    })
}

fn native_refused(plan: &CatalogPlan) -> bool {
    plan.refusals
        .iter()
        .any(|refusal| matches!(refusal, PlanRefusal::NativeInjector { .. }))
}

fn archive_required_source(plan: &CatalogPlan) -> Option<&PackageSource> {
    plan.refusals.iter().find_map(|refusal| match refusal {
        PlanRefusal::ArchiveRequired { reference, source } if *reference == plan.selected => {
            Some(source)
        }
        _ => None,
    })
}

fn unmatched_supplied(plan: &CatalogPlan) -> Option<&Path> {
    plan.refusals.iter().find_map(|refusal| match refusal {
        PlanRefusal::ArchiveUnmatched { path } => Some(path.as_path()),
        _ => None,
    })
}

fn acquire(
    plan: &CatalogPlan,
    cache: &ArchiveCache,
    cache_use: CacheUse,
    links: &[NxmLink],
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

struct Candidate {
    preview: ReleasePreview,
    pending: Vec<String>,
    present: BTreeSet<String>,
}

pub(super) fn run(
    args: &UpdateArgs,
    roots: &ManagerRoots,
    requested: Option<&str>,
    json: bool,
    progress: &Progress,
) -> Result<()> {
    progress.begin()?;
    if args.download && !args.dry_run {
        bail!("--download only prepares a --dry-run; an update downloads what it needs");
    }
    let Some(query) = args.target.as_deref() else {
        return crate::update_refresh::run(args, roots, requested, json);
    };
    if progress.enabled() && !args.yes && !args.dry_run {
        return Err(coded(
            ErrorCode::ApprovalRequired,
            "an update writes game files; pass --yes with --progress json",
        ));
    }
    let catalog = BucketStore::new(roots).snapshot(requested)?;
    let current = catalog_install::resolve(query, &catalog, json)?;
    let reference = reference_of(&current);
    let label = reference.to_string();
    let releases = catalog.releases(&current.bucket, &current.package.game, &current.package.id)?;
    let target = choose_release(releases, args.version.as_deref(), &label)?;
    download::validate_links(&args.nxm_links, &[&target])?;
    let interactive = !args.yes && !args.dry_run && !progress.enabled();
    let state = ManagerState::load_deferred(roots)?;
    let store = state
        .existing()
        .context("no manager state database; nothing is installed")?;
    let (game_id, game) = selected_installation(
        roots,
        Some(store),
        args.path.clone(),
        Some(&current.package.game),
    )?;
    let flags = install_consent::flags(args.allow_native_injector, &args.allow_executable);
    let cache = roots.archive_cache();
    let cache_use = if args.no_cache {
        CacheUse::Refresh
    } else {
        CacheUse::Reuse
    };
    let protected = ProtectedTarget {
        game: Some(game_id.clone()),
        root: Some(game.clone()),
    };
    let mut plan = build_plan(
        &target,
        &catalog,
        store,
        &game_id,
        &game,
        args.file.as_slice(),
        &cache,
        cache_use,
        &flags,
    )?;
    let update = update_facts(&plan)?;
    if let Some(package_id) = pinned_package(&plan) {
        println!(
            "Skipped {package_id}: managed package is pinned; run `modn unpin {package_id}` to update it."
        );
        return Ok(());
    }
    if interactive && !install_consent::can_prompt(json) {
        bail!("an update writes game files; choose --dry-run, confirm in a terminal, or explicitly approve with --yes");
    }
    let blockers = update_report::blockers(&plan);
    if !update.up_to_date && !args.dry_run && native_refused(&plan) {
        return Err(
            ApprovalRefusal::new(reference.clone(), Uncovered::native_injector()).into_error(),
        );
    }
    if let Some(path) = unmatched_supplied(&plan) {
        let planned = selected(&plan)?;
        if planned.served.is_some() || planned.digest.as_str().is_some() {
            bail!(
                "{}: archive BLAKE3 does not match {label} {}",
                path.display(),
                planned.package.package.version
            );
        }
        bail!(
            "{label}: Modn knows no archive digest to verify {} against; run modn update {label} without --file so it can acquire the archive from the source",
            path.display()
        );
    }
    let mut download_attempts = Vec::new();
    let served = selected(&plan)?.served.is_some();
    let fetch =
        !update.up_to_date && !served && (args.download || !args.dry_run) && blockers.is_empty();
    if fetch {
        if let Some(source) = archive_required_source(&plan) {
            bail!(
                "{label}: {}, then use modn update {label} --file <archive>",
                dependency_plan::acquisition(source)
            );
        }
        state.validate_download(cache.root(), &protected)?;
        if interactive {
            let download = format!(
                "{label} {}; {}",
                target.package.version,
                dependency_plan::acquisition(planned_source(&target))
            );
            if !install_consent::confirm_download(
                &[download],
                &mut io::stdin().lock(),
                &mut io::stderr(),
            )? {
                bail!("{label}: download cancelled");
            }
        }
        let acquired = acquire(&plan, &cache, cache_use, &args.nxm_links, json, progress)?;
        plan = build_plan(
            &target,
            &catalog,
            store,
            &game_id,
            &game,
            args.file.as_slice(),
            &cache,
            CacheUse::Reuse,
            &flags,
        )?;
        acquired.apply_to(&mut plan);
        download_attempts = acquired.attempts;
    }
    let update = update_facts(&plan)?;
    let planned = selected(&plan)?;
    let archive = planned.served.as_ref().map(|served| served.path.clone());
    let mut candidate_approvals = args.allow_executable.clone();
    if let Some(stored) = &planned.stored {
        for path in &stored.executables {
            if !candidate_approvals.contains(path) {
                candidate_approvals.push(path.clone());
            }
        }
    }
    let base_approved = candidate_approvals.clone();
    for path in &planned.package.package.executables {
        if !candidate_approvals.contains(path) {
            candidate_approvals.push(path.clone());
        }
    }
    let candidate = match (&archive, update.up_to_date) {
        (Some(archive), false) => {
            let preview =
                preview_catalog_update(Some(store), roots, &plan, archive, &candidate_approvals)?;
            if let Some(expected) = planned.digest.as_str() {
                if preview.plan.archive_blake3 != expected {
                    bail!("{label}: source changed after catalog verification");
                }
            }
            let destinations: Vec<&str> = preview
                .plan
                .operations
                .iter()
                .map(|operation| operation.destination.as_str())
                .collect();
            let present = install_consent::present_executables(
                &planned.package.package.executables,
                &destinations,
            );
            let pending = install_consent::pending_executables(
                &planned.package.package.executables,
                &base_approved,
                &destinations,
            );
            Some(Candidate {
                preview,
                pending,
                present,
            })
        }
        _ => None,
    };
    let check = candidate.as_ref().map(|candidate| {
        let needed = Capabilities::new(
            planned.package.package.native_injector,
            candidate.present.iter().cloned(),
        );
        install_consent::ApprovalCheck::new(
            reference.clone(),
            needed,
            planned.stored.clone(),
            &flags,
        )
    });
    let preview = Preview {
        reference: &label,
        installed: &update.installed,
        installed_version: update.installed_version.as_deref(),
        target: &planned.package,
        up_to_date: update.up_to_date,
        files: candidate
            .as_ref()
            .map(|candidate| FileChanges::new(&update.installed.files, &candidate.preview.plan)),
        plan: candidate.as_ref().map(|candidate| &candidate.preview.plan),
        archive_needed: (archive.is_none() && !update.up_to_date)
            .then(|| dependency_plan::acquisition(planned_source(&target))),
        dependents: &update.dependents,
        blockers: &blockers,
        downloads: &download_attempts,
        unapproved_declared_executables: candidate
            .as_ref()
            .map_or(&[], |candidate| candidate.pending.as_slice()),
    };
    if args.dry_run || update.up_to_date {
        return preview.print(json);
    }
    if !blockers.is_empty() {
        bail!("{label}: update blocked:\n  {}", blockers.join("\n  "));
    }
    let Some(candidate) = candidate.as_ref() else {
        bail!(
            "{label}: no archive for {}",
            planned.package.package.version
        );
    };
    let check = check.as_ref().context("update plan has approval state")?;
    if !check.uncovered.is_empty() && (!interactive || check.uncovered.native_injector) {
        return check.ensure_approved();
    }
    if !candidate.preview.reviewable() {
        bail!("{label}: update blocked; inspect it with --dry-run first");
    }
    if interactive {
        preview.print(false)?;
        let dependents: Vec<String> = update
            .dependents
            .iter()
            .map(update_report::dependent_label)
            .collect();
        let request = UpdateRequest {
            reference: &label,
            game: &game,
            from: update
                .installed_version
                .as_deref()
                .unwrap_or("an unlisted release"),
            to: &planned.package.package.version,
            dependents: &dependents,
            executables: &candidate.pending,
        };
        if !install_consent::confirm_update(&request, &mut io::stdin().lock(), &mut io::stderr())? {
            bail!("{label}: update cancelled");
        }
    }
    let approvals = vec![ExecutableApproval::new(reference, candidate_approvals)];
    let digest = plan_digest(&plan, &approvals, std::slice::from_ref(&candidate.preview));
    let consent = Consent::new(
        plan.native_injector == NativeInjectorPolicy::Allow,
        approvals,
        digest,
    );
    let mut store = state.open_writable(&protected)?;
    let mut report = io::stderr();
    let resolver = download::resolver(&args.nxm_links, !json && !progress.enabled(), &mut report);
    let outcome = commit_plan(CommitRequest::new(
        roots,
        &cache,
        &plan,
        &consent,
        download::transport(),
        &resolver,
        SourcePolicy::default(),
        CacheUse::Reuse,
        progress.control(),
        &mut store,
    ))?;
    let package = outcome
        .packages
        .first()
        .context("the update committed no package")?;
    if json {
        let mut result = serde_json::json!({
            "package": label, "replaced": update.installed.id, "installed": package,
        });
        if let Some(operation_id) = &outcome.operation_id {
            result["operation_id"] = serde_json::Value::String(operation_id.clone());
        }
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!(
            "Updated {label} to {} ({} owned files)",
            planned.package.package.version,
            package.files.len()
        );
    }
    Ok(())
}
