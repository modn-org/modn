//! Install a catalog mod together with its missing dependencies, as one stack.
//!
//! The Rust core installs the stack under one transaction lock and uninstalls
//! every package it installed if any package fails. Each package keeps its own
//! ownership record, provenance and dependency links.
use crate::catalog_install::{catalog_release, reference_of};
use crate::commands::InstallArgs;
use crate::dependency_plan::{self, PlannedDependency};
use crate::install_consent::{self, StackRequest};
use crate::state_access::{ManagerState, ProtectedTarget};
use anyhow::{bail, Context, Result};
use modn_core::buckets::{BucketPackage, BucketStore};
use modn_core::planning::{preview_install_zip_with_permissions, PlanStatus};
use modn_core::transactions::StackMember;
use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;

/// A selected mod whose dependencies are not all ready.
pub(super) struct Stack<'a> {
    pub(super) selected: &'a BucketPackage,
    /// Dependencies of `selected`, in install order.
    pub(super) order: &'a [BucketPackage],
    /// The state of each release in `order`, at the same index.
    pub(super) dependencies: &'a [PlannedDependency],
    /// Supplied archives by catalog reference.
    pub(super) archives: &'a BTreeMap<String, PathBuf>,
    pub(super) catalog: &'a BucketStore,
    pub(super) game_id: String,
    pub(super) installation: PathBuf,
}

/// Releases to install, in order: every missing dependency, then the mod.
fn releases<'a>(stack: &Stack<'a>) -> Result<Vec<(&'a BucketPackage, PathBuf)>> {
    let needed: Vec<&BucketPackage> = stack
        .order
        .iter()
        .zip(stack.dependencies)
        .filter(|(_, dependency)| dependency.missing())
        .map(|(release, _)| release)
        .chain([stack.selected])
        .collect();
    let absent: Vec<String> = needed
        .iter()
        .filter(|release| {
            !stack
                .archives
                .contains_key(&reference_of(release).to_string())
        })
        .map(|release| {
            format!(
                "  dependency not installed: {} {}; {}",
                reference_of(release),
                release.package.version,
                dependency_plan::acquisition(&release.package.source)
            )
        })
        .collect();
    if !absent.is_empty() {
        bail!(
            "archives missing for this installation (automatic downloads are not implemented):\n{}\nPass --file once for each archive.",
            absent.join("\n")
        );
    }
    Ok(needed
        .into_iter()
        .map(|release| {
            let archive = stack.archives[&reference_of(release).to_string()].clone();
            (release, archive)
        })
        .collect())
}

/// Install the missing dependencies and the selected mod after one approval.
pub(super) fn run(
    stack: &Stack<'_>,
    state: ManagerState,
    args: &InstallArgs,
    json: bool,
) -> Result<()> {
    let releases = releases(stack)?;
    let interactive = !args.yes;
    let explicit = &args.allow_executable;
    // Manifests declare executables; only the operator approves them.
    let unapproved: Vec<String> = releases
        .iter()
        .flat_map(|(release, _)| &release.package.executables)
        .filter(|path| !explicit.contains(path))
        .cloned()
        .collect();
    if !interactive && !unapproved.is_empty() {
        bail!(
            "installation blocked; catalog manifests declare {}. Confirm in a terminal without --yes, or pass --allow-executable for each path",
            unapproved.join(", ")
        );
    }
    let packages: Vec<String> = releases
        .iter()
        .map(|(release, _)| format!("{} {}", reference_of(release), release.package.version))
        .collect();
    if interactive {
        let request = StackRequest {
            game: &stack.installation,
            packages: &packages,
            executables: &unapproved,
        };
        if !install_consent::confirm_stack(&request, &mut io::stdin().lock(), &mut io::stderr())? {
            bail!("installation cancelled");
        }
    }
    let mut digests = BTreeMap::new();
    let members = releases
        .iter()
        .map(|(release, archive)| {
            let mut approved = explicit.clone();
            if interactive {
                approved.extend(release.package.executables.iter().cloned());
            }
            digests.insert(reference_of(release).to_string(), &release.package.sha256);
            Ok(StackMember::new(
                archive.clone(),
                catalog_release(release, stack.catalog)?,
                approved,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut store = state.open_writable(&ProtectedTarget {
        game: Some(stack.game_id.clone()),
        root: Some(stack.installation.clone()),
    })?;
    let installed = store.install_catalog_stack(&stack.installation, &members, |member| {
        let reference = member.release.reference.to_string();
        let plan = preview_install_zip_with_permissions(
            &stack.game_id,
            &stack.installation,
            &member.archive,
            &member.approved_executables,
        )?;
        if Some(&&plan.archive_sha256) != digests.get(&reference) {
            bail!("{reference}: source changed after catalog verification");
        }
        if plan.status != PlanStatus::Reviewable {
            let blockers: Vec<String> = plan
                .blockers
                .iter()
                .map(|issue| format!("{}: {}", issue.code, issue.message))
                .collect();
            bail!("{reference}: installation blocked: {}", blockers.join("; "));
        }
        Ok(plan)
    })?;
    let report: Vec<_> = members
        .iter()
        .zip(&installed)
        .map(|(member, package)| (member.release.reference.to_string(), package))
        .collect();
    if json {
        let packages: Vec<_> = report
            .iter()
            .map(|(reference, package)| serde_json::json!({"package": reference, "installed": package}))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "package": reference_of(stack.selected).to_string(),
                "stack": packages,
            }))
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
