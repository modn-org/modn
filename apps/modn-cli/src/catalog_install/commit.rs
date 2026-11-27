//! The confirmation prompts and commit step of one catalog install.

use super::{flow, plan_data};
use crate::commands::InstallArgs;
use crate::dependency_plan;
use crate::download;
use crate::install_consent::{self, Request, StackRequest};
use crate::progress::Progress;
use crate::state_access::{ManagerState, ProtectedTarget};
use anyhow::{bail, Context, Result};
use modn_core::acquisition::{ArchiveCache, CacheUse, SourceAttempt, SourcePolicy};
use modn_core::approval::Capabilities;
use modn_core::buckets::{BucketPackage, BucketSnapshot};
use modn_core::catalog_pipeline::{
    commit_plan, plan_digest, preview_catalog_install, CatalogPlan, CommitRequest, Consent,
    ExecutableApproval, PlannedRelease,
};
use modn_core::manager_roots::ManagerRoots;
use modn_core::transactions::{CatalogReference, ManagedPackage, NativeInjectorPolicy};
use std::collections::BTreeSet;
use std::io;
use std::path::Path;

/// Confirm and commit one planned release.
#[expect(
    clippy::too_many_arguments,
    reason = "the release step keeps the command, plan and consent evidence explicit"
)]
pub(super) fn install_release(
    plan: &mut CatalogPlan,
    selected: &BucketPackage,
    catalog: &BucketSnapshot,
    args: &InstallArgs,
    roots: &ManagerRoots,
    cache: &ArchiveCache,
    cache_use: CacheUse,
    state: ManagerState,
    game_id: &str,
    installation: &Path,
    reference_key: &CatalogReference,
    reference: &str,
    flags: &Capabilities,
    interactive: bool,
    progress: &Progress,
    json: bool,
) -> Result<()> {
    let mut attempts = Vec::new();
    if plan_data::selected(plan).served.is_none() {
        if interactive {
            let planned = plan_data::selected(plan);
            let download = dependency_plan::download_line(
                reference,
                &planned.package.package.version,
                &planned.package.package.sources,
                planned.package.package.primary_source(),
            );
            if !install_consent::confirm_download(
                &[download],
                &mut io::stdin().lock(),
                &mut io::stderr(),
            )? {
                bail!("{reference}: download cancelled");
            }
        }
        attempts.extend(flow::acquire_and_replan(
            plan,
            selected,
            catalog,
            args,
            cache,
            cache_use,
            state.existing(),
            game_id,
            installation,
            flags,
            progress,
            json,
        )?);
    }
    let planned = plan_data::selected(plan);
    let mut candidate_approvals = args.allow_executable.clone();
    for path in &planned.package.package.executables {
        if !candidate_approvals.contains(path) {
            candidate_approvals.push(path.clone());
        }
    }
    let candidate = if candidate_approvals.is_empty() {
        None
    } else {
        flow::preview_selected(roots, plan, &candidate_approvals)?
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
    let needed = Capabilities::new(selected.package.native_injector, present);
    let check = install_consent::ApprovalCheck::new(
        reference_key.clone(),
        needed,
        planned.stored.clone(),
        flags,
    );
    let pending = check.uncovered.executables.clone();
    let mut approved = check.approved_executables();
    let mut preview = flow::preview_selected(roots, plan, &approved)?
        .with_context(|| format!("{reference}: the plan has no archive to install"))?;
    if !check.uncovered.is_empty() && (!interactive || check.uncovered.native_injector) {
        return check.ensure_approved();
    }
    if interactive && !pending.is_empty() {
        approved.extend(pending.iter().cloned());
        preview = flow::preview_selected(roots, plan, &approved)?
            .with_context(|| format!("{reference}: the plan has no archive to install"))?;
    }
    if !preview.reviewable() {
        bail!("{reference}: installation blocked; inspect with --dry-run first");
    }
    if interactive {
        let request = Request {
            reference,
            game: installation,
            files: preview.plan.operations.len(),
            executables: &pending,
        };
        if !install_consent::confirm(&request, &mut io::stdin().lock(), &mut io::stderr())? {
            bail!("{reference}: installation cancelled");
        }
    }
    let approvals = vec![ExecutableApproval::new(reference_key.clone(), approved)];
    let previews = vec![preview];
    let digest = plan_digest(plan, &approvals, &previews);
    let consent = Consent::new(
        plan.native_injector == NativeInjectorPolicy::Allow,
        approvals,
        digest,
    );
    let mut store = state.open_writable(&ProtectedTarget {
        game: Some(game_id.to_owned()),
        root: Some(installation.to_path_buf()),
    })?;
    let mut report = io::stderr();
    let resolver = download::resolver(&args.nxm_links, !json && !progress.enabled(), &mut report);
    let outcome = commit_plan(CommitRequest::new(
        roots,
        cache,
        plan,
        &consent,
        download::transport(),
        &resolver,
        SourcePolicy::default(),
        CacheUse::Reuse,
        progress.control(),
        &mut store,
    ))?;
    attempts.extend(outcome.attempts.iter().cloned());
    let package = outcome
        .packages
        .first()
        .context("the commit installed no package")?;
    print_release(
        reference,
        package,
        outcome.operation_id.as_deref(),
        &attempts,
        json,
    )?;
    Ok(())
}

/// Confirm and commit every member of one stack plan.
#[expect(
    clippy::too_many_arguments,
    reason = "the stack step keeps the command, plan and consent evidence explicit"
)]
pub(super) fn install_stack(
    plan: &mut CatalogPlan,
    selected: &BucketPackage,
    catalog: &BucketSnapshot,
    args: &InstallArgs,
    roots: &ManagerRoots,
    cache: &ArchiveCache,
    cache_use: CacheUse,
    state: ManagerState,
    game_id: &str,
    installation: &Path,
    flags: &Capabilities,
    interactive: bool,
    progress: &Progress,
    json: bool,
) -> Result<()> {
    let members = plan.members()?;
    let checks = stack_checks(&members, flags);
    if let Some(check) = checks.iter().find(|check| check.uncovered.native_injector) {
        return check.ensure_approved();
    }
    let unapproved: Vec<String> = checks
        .iter()
        .flat_map(|check| check.uncovered.executables.iter().cloned())
        .collect();
    if !interactive {
        if let Some(check) = checks
            .iter()
            .find(|check| !check.uncovered.executables.is_empty())
        {
            return check.ensure_approved();
        }
    }
    if interactive {
        let packages = stack_packages(&members);
        let request = StackRequest {
            game: installation,
            packages: &packages,
            executables: &unapproved,
        };
        if !install_consent::confirm_stack(&request, &mut io::stdin().lock(), &mut io::stderr())? {
            bail!("installation cancelled");
        }
    }
    let mut attempts = flow::acquire_and_replan(
        plan,
        selected,
        catalog,
        args,
        cache,
        cache_use,
        state.existing(),
        game_id,
        installation,
        flags,
        progress,
        json,
    )?;
    let members = plan.members()?;
    let approvals = stack_approvals(&members, &checks, interactive);
    let archives = plan_data::archives(members.iter().copied());
    let previews = preview_catalog_install(roots, plan, &archives, &approvals)?;
    let digest = plan_digest(plan, &approvals, &previews);
    let consent = Consent::new(
        plan.native_injector == NativeInjectorPolicy::Allow,
        approvals,
        digest,
    );
    let mut store = state.open_writable(&ProtectedTarget {
        game: Some(game_id.to_owned()),
        root: Some(installation.to_path_buf()),
    })?;
    let mut report = io::stderr();
    let resolver = download::resolver(&args.nxm_links, !json && !progress.enabled(), &mut report);
    let outcome = commit_plan(CommitRequest::new(
        roots,
        cache,
        plan,
        &consent,
        download::transport(),
        &resolver,
        SourcePolicy::default(),
        CacheUse::Reuse,
        progress.control(),
        &mut store,
    ))?;
    attempts.extend(outcome.attempts.iter().cloned());
    print_stack(&plan.selected, &members, &outcome, &attempts, json)?;
    Ok(())
}

/// The stored and flag evidence of every stack member.
fn stack_checks(
    members: &[&PlannedRelease],
    flags: &Capabilities,
) -> Vec<install_consent::ApprovalCheck> {
    members
        .iter()
        .map(|planned| {
            install_consent::ApprovalCheck::new(
                planned.package.reference(),
                planned.needed.clone(),
                planned.stored.clone(),
                flags,
            )
        })
        .collect()
}

/// The prompt lines of one stack, in install order.
fn stack_packages(members: &[&PlannedRelease]) -> Vec<String> {
    members
        .iter()
        .map(|planned| {
            let reference = planned.package.reference().to_string();
            let download = if planned.served.is_some() {
                String::new()
            } else {
                let source = dependency_plan::fetchable_source(&planned.package.package.sources)
                    .unwrap_or(planned.package.package.primary_source());
                match download::source_label(source) {
                    Some(source) => format!(" (downloads its archive from {source})"),
                    None => " (downloads its archive)".to_owned(),
                }
            };
            format!("{reference} {}{download}", planned.package.package.version)
        })
        .collect()
}

/// The executable paths that one stack consent grants for every member.
fn stack_approvals(
    members: &[&PlannedRelease],
    checks: &[install_consent::ApprovalCheck],
    interactive: bool,
) -> Vec<ExecutableApproval> {
    members
        .iter()
        .zip(checks)
        .map(|(planned, check)| {
            let mut approved = check.approved_executables();
            if interactive {
                approved.extend(check.uncovered.executables.iter().cloned());
            }
            ExecutableApproval::new(planned.package.reference(), approved)
        })
        .collect()
}

/// Print one stack result, members in install order.
fn print_stack(
    selected: &CatalogReference,
    members: &[&PlannedRelease],
    outcome: &modn_core::catalog_pipeline::CommitOutcome,
    attempts: &[SourceAttempt],
    json: bool,
) -> Result<()> {
    let report: Vec<(String, &ManagedPackage)> = members
        .iter()
        .zip(&outcome.packages)
        .map(|(member, package)| (member.package.reference().to_string(), package))
        .collect();
    if json {
        let packages: Vec<_> = report
            .iter()
            .map(|(reference, package)| {
                serde_json::json!({"package": reference, "installed": package})
            })
            .collect();
        let mut result = serde_json::json!({
            "package": selected.to_string(),
            "stack": packages,
            "downloads": download::downloads_json(attempts),
        });
        if let Some(operation_id) = &outcome.operation_id {
            result["operation_id"] = serde_json::Value::String(operation_id.clone());
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&result)
                .context("cannot serialize installation report")?
        );
    } else {
        for (reference, package) in &report {
            println!(
                "Installed {reference} ({} owned files)",
                package.files.len()
            );
        }
    }
    Ok(())
}

/// Print one release result.
fn print_release(
    reference: &str,
    package: &ManagedPackage,
    operation_id: Option<&str>,
    attempts: &[SourceAttempt],
    json: bool,
) -> Result<()> {
    if json {
        let mut result = serde_json::json!({
            "package": reference,
            "installed": package,
            "downloads": download::downloads_json(attempts),
        });
        if let Some(operation_id) = operation_id {
            result["operation_id"] = serde_json::Value::String(operation_id.to_owned());
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
