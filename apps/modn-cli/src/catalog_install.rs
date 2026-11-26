use crate::catalog_approval;
use crate::commands::InstallArgs;
use crate::dependency_plan::{self, PlannedDependency};
use crate::download;
use crate::install_consent::{self, Request};
use crate::install_report::{self, ArchivePreview};
use crate::modules::content_root_appdata;
use crate::progress::Progress;
use crate::selection::selected_installation;
use crate::stack_install;
use crate::state_access::{ManagerState, ProtectedTarget};
use anyhow::{bail, Context, Result};
use modn_core::acquisition::CacheUse;
use modn_core::approval::Capabilities;
use modn_core::archive;
use modn_core::buckets::{ArchiveFormat, BucketPackage, BucketSnapshot, BucketStore};
use modn_core::catalog_pipeline::PlanRefusal;
use modn_core::error::{coded, ErrorCode};
use modn_core::history::HistoryContext;
use modn_core::manager_roots::ManagerRoots;
use modn_core::planning::{
    preview_install_zip_with_appdata_permissions, preview_install_zip_with_permissions,
    InstallPlan, PlanStatus,
};
use modn_core::transactions::CatalogReference;
use std::collections::BTreeSet;
use std::fmt;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
#[cfg(test)]
#[path = "catalog_snapshot_tests.rs"]
mod tests;

mod plan_data;

#[derive(Debug)]
pub(super) struct AmbiguousTarget {
    query: String,
    candidates: Vec<String>,
    pub(super) json: bool,
}

impl AmbiguousTarget {
    pub(super) fn as_json(&self) -> serde_json::Value {
        serde_json::json!({
            "error": ErrorCode::AmbiguousTarget.as_str(),
            "query": self.query,
            "candidates": self.candidates,
        })
    }
}

impl fmt::Display for AmbiguousTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ambiguous target {:?}; choose one of: {}",
            self.query,
            self.candidates.join(", ")
        )
    }
}

impl std::error::Error for AmbiguousTarget {}

pub(super) fn resolve(query: &str, catalog: &BucketSnapshot, json: bool) -> Result<BucketPackage> {
    let mut candidates = catalog.candidates(query)?;
    match candidates.len() {
        0 => Err(coded(
            ErrorCode::ModNotFound,
            format!("catalog mod not found: {query}"),
        )),
        1 => Ok(candidates.remove(0)),
        _ if json || !io::stdin().is_terminal() || !io::stderr().is_terminal() => {
            Err(AmbiguousTarget {
                query: query.into(),
                candidates: candidates
                    .iter()
                    .map(|p| format!("{}/{}/{}", p.bucket, p.package.game, p.package.id))
                    .collect(),
                json,
            }
            .into())
        }
        _ => {
            eprintln!("Multiple matches for {query:?}. Which one do you mean?");
            for (index, candidate) in candidates.iter().enumerate() {
                eprintln!(
                    "  {}. {}/{}/{}",
                    index + 1,
                    candidate.bucket,
                    candidate.package.game,
                    candidate.package.id
                );
            }
            eprint!("Select [1-{}]: ", candidates.len());
            io::stderr().flush()?;
            let mut answer = String::new();
            io::stdin().read_line(&mut answer)?;
            let index: usize = answer.trim().parse().context("invalid selection")?;
            if !(1..=candidates.len()).contains(&index) {
                bail!("selection out of range");
            }
            Ok(candidates.remove(index - 1))
        }
    }
}

pub(super) fn expected_format(format: Option<&ArchiveFormat>) -> Option<archive::ArchiveFormat> {
    match format {
        Some(ArchiveFormat::Zip) => Some(archive::ArchiveFormat::Zip),
        Some(ArchiveFormat::SevenZ) => Some(archive::ArchiveFormat::SevenZ),
        _ => None,
    }
}

pub(super) fn reference_of(package: &BucketPackage) -> CatalogReference {
    package.reference()
}

fn refuse_unpromptable(interactive: bool, json: bool) -> Result<()> {
    if interactive && !install_consent::can_prompt(json) {
        bail!("installation writes game files; choose --dry-run, confirm in a terminal, or explicitly approve with --yes");
    }
    Ok(())
}

fn install_mod(
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
    let mut artifacts = plan_data::PlanArchives::of(&plan);
    {
        let installing = plan_data::installing(&plan);
        download::validate_links(&args.nxm_links, &installing)?;
    }
    let mut download_attempts = Vec::new();
    if args.download {
        let installing = plan_data::installing(&plan);
        download_attempts.extend(download::download(
            &cache,
            cache_use,
            &installing,
            &mut artifacts.archives,
            &mut artifacts.digests,
            &mut artifacts.served,
            &args.nxm_links,
            json,
            progress,
        )?);
        plan = plan_data::plan(
            &selected,
            catalog,
            state.existing(),
            &game_id,
            &installation,
            &args.file,
            &cache,
            // The batch just verified and cached these archives, so the report
            // reads them back even after `--no-cache`.
            CacheUse::Reuse,
            &flags,
        )?;
        artifacts = plan_data::PlanArchives::of(&plan);
    }
    if !args.dry_run {
        let conflicts = install_report::conflict_lines(&plan);
        if !conflicts.is_empty() {
            bail!(
                "installation blocked by declared conflicts; uninstall the installed packages first:\n  {}",
                conflicts.join("\n  ")
            );
        }
    }
    let dependencies = dependency_plan::dependencies(&plan);
    let stackable = dependencies
        .iter()
        .all(|dependency| dependency.ready() || dependency.missing());
    if !args.dry_run && stackable && !dependencies.iter().all(PlannedDependency::ready) {
        let stack_order: Vec<BucketPackage> = plan.order[..plan.order.len() - 1]
            .iter()
            .map(|release| release.package.clone())
            .collect();
        let stack = stack_install::Stack {
            selected: &selected,
            order: &stack_order,
            dependencies: &dependencies,
            archives: artifacts.archives,
            digests: artifacts.digests,
            served: artifacts.served,
            supplied: artifacts.supplied,
            catalog,
            cache: &cache,
            cache_use,
            game_id,
            installation,
        };
        return stack_install::run(stack, state, args, json, progress, download_attempts);
    }
    if !args.dry_run {
        if let Some(PlanRefusal::DependencyNotReady { reference, .. }) = plan
            .refusals
            .iter()
            .find(|refusal| matches!(refusal, PlanRefusal::DependencyNotReady { .. }))
        {
            bail!("dependency not installed or intact: {reference}; install or restore it first");
        }
    }
    if !args.dry_run && !artifacts.archives.contains_key(&reference) {
        if interactive {
            let download = dependency_plan::download_line(
                &reference,
                &selected.package.version,
                &selected.package.sources,
                selected.package.primary_source(),
            );
            if !install_consent::confirm_download(
                &[download],
                &mut io::stdin().lock(),
                &mut io::stderr(),
            )? {
                bail!("{reference}: download cancelled");
            }
        }
        download_attempts.extend(download::download(
            &cache,
            cache_use,
            &[&selected],
            &mut artifacts.archives,
            &mut artifacts.digests,
            &mut artifacts.served,
            &args.nxm_links,
            json,
            progress,
        )?);
    }
    let planned_selected = plan_data::selected(&plan);
    let Some(archive) = artifacts.archives.get(&reference).map(PathBuf::as_path) else {
        let check = install_consent::ApprovalCheck::new(
            reference_key.clone(),
            planned_selected.needed.clone(),
            planned_selected.stored.clone(),
            &flags,
        );
        return install_report::print(&plan, None, Some(&check), &download_attempts, json);
    };
    let preview = |approved: &[String]| -> Result<InstallPlan> {
        let plan = match content_root_appdata(roots, &game_id)? {
            Some(appdata) => preview_install_zip_with_appdata_permissions(
                roots,
                &game_id,
                &installation,
                archive,
                approved,
                &appdata,
            )?,
            None => preview_install_zip_with_permissions(
                roots,
                &game_id,
                &installation,
                archive,
                approved,
            )?,
        };
        let digest = download::digest(&selected, &artifacts.digests)?;
        if Some(plan.archive_blake3.as_str()) != digest.as_str() {
            bail!("{reference}: source changed after catalog verification");
        }
        Ok(plan)
    };
    let declared = &selected.package.executables;
    let mut candidate_approvals = args.allow_executable.clone();
    let additions: Vec<String> = declared
        .iter()
        .filter(|path| !candidate_approvals.contains(path))
        .cloned()
        .collect();
    candidate_approvals.extend(additions);
    let candidate = if candidate_approvals.is_empty() {
        None
    } else {
        Some(preview(&candidate_approvals)?)
    };
    let present = candidate.as_ref().map_or_else(BTreeSet::new, |candidate| {
        let destinations: Vec<&str> = candidate
            .operations
            .iter()
            .map(|operation| operation.destination.as_str())
            .collect();
        install_consent::present_executables(declared, &destinations)
    });
    let needed = Capabilities::new(selected.package.native_injector, present);
    let check = install_consent::ApprovalCheck::new(
        reference_key.clone(),
        needed.clone(),
        planned_selected.stored.clone(),
        &flags,
    );
    let pending = check.uncovered.executables.clone();
    let mut approved = check.approved_executables();
    let mut plan_preview = preview(&approved)?;
    if args.dry_run {
        let archive = ArchivePreview {
            plan: &plan_preview,
            unapproved_declared_executables: &pending,
        };
        return install_report::print(&plan, Some(archive), Some(&check), &download_attempts, json);
    }
    if !check.uncovered.is_empty() && (!interactive || check.uncovered.native_injector) {
        return check.ensure_approved();
    }
    if interactive && !pending.is_empty() {
        approved.extend(pending.iter().cloned());
        plan_preview = preview(&approved)?;
    }
    if plan_preview.status != PlanStatus::Reviewable {
        bail!("{reference}: installation blocked; inspect with --dry-run first");
    }
    if interactive {
        let request = Request {
            reference: &reference,
            game: &installation,
            files: plan_preview.operations.len(),
            executables: &pending,
        };
        if !install_consent::confirm(&request, &mut io::stdin().lock(), &mut io::stderr())? {
            bail!("{reference}: installation cancelled");
        }
    }
    let mut store = state.open_writable(&ProtectedTarget {
        game: Some(game_id.clone()),
        root: Some(installation.clone()),
    })?;
    store.set_control(progress.control().clone());
    store.set_history_context(
        HistoryContext::new(game_id.clone(), installation.clone()).with_source(
            &reference_of(&selected),
            download::serving_source(
                &reference,
                &artifacts.supplied,
                &artifacts.served,
                &selected,
            ),
        ),
    );
    let package = store.install_catalog_release_with_permissions(
        &installation,
        archive,
        &plan_preview,
        &planned_selected.release,
        plan.native_injector,
        &approved,
    )?;
    let operation_id = store.committed_operation_id().map(str::to_owned);
    install_consent::record(&mut store, &installation, &reference_key, &needed)?;
    if json {
        let downloads = download::downloads_json(&download_attempts);
        let mut result = serde_json::json!({
            "package": reference, "installed": package, "downloads": downloads,
        });
        if let Some(operation_id) = operation_id {
            result["operation_id"] = serde_json::Value::String(operation_id);
        }
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!(
            "Installed {reference} ({} owned files)",
            package.files.len()
        );
    }
    Ok(())
}

pub(super) fn run(
    args: &InstallArgs,
    roots: &ManagerRoots,
    game_filter: Option<&str>,
    json: bool,
    progress: &Progress,
) -> Result<()> {
    let catalog = BucketStore::new(roots).snapshot(game_filter)?;
    let package = resolve(&args.target, &catalog, json)?;
    install_mod(package, &catalog, args, roots, json, progress)
}
