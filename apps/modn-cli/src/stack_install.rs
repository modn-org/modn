use crate::catalog_install::reference_of;
use crate::commands::InstallArgs;
use crate::dependency_plan::{self, PlannedDependency};
use crate::download;
use crate::install_consent::{self, StackRequest};
use crate::modules::content_root_appdata;
use crate::state_access::{ManagerState, ProtectedTarget};
use anyhow::{bail, Context, Result};
use modn_core::acquisition::{ArchiveCache, CacheUse};
use modn_core::buckets::{BucketPackage, BucketSnapshot};
use modn_core::planning::{
    preview_install_zip_with_appdata, preview_install_zip_with_permissions, PlanStatus,
};
use modn_core::transactions::{NativeInjectorPolicy, StackMember};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::PathBuf;

pub(super) struct Stack<'a> {
    pub(super) selected: &'a BucketPackage,
    pub(super) order: &'a [BucketPackage],
    pub(super) dependencies: &'a [PlannedDependency],
    pub(super) archives: BTreeMap<String, PathBuf>,
    pub(super) digests: BTreeMap<String, String>,
    pub(super) catalog: &'a BucketSnapshot,
    pub(super) cache: &'a ArchiveCache,
    pub(super) cache_use: CacheUse,
    pub(super) game_id: String,
    pub(super) installation: PathBuf,
}

fn releases<'a>(stack: &Stack<'a>) -> Result<Vec<&'a BucketPackage>> {
    let mut by_digest: BTreeMap<&str, &BucketPackage> = BTreeMap::new();
    let mut needed: Vec<&BucketPackage> = Vec::new();
    let missing = stack
        .order
        .iter()
        .zip(stack.dependencies)
        .filter(|(_, dependency)| dependency.missing())
        .map(|(release, _)| release);
    for release in missing {
        let Some(digest) = download::digest(release, &stack.digests) else {
            needed.push(release);
            continue;
        };
        let Some(kept) = by_digest.get(digest) else {
            by_digest.insert(digest, release);
            needed.push(release);
            continue;
        };
        let declared = |release: &BucketPackage| -> (BTreeSet<String>, BTreeSet<String>) {
            let set = |ids: &[String]| ids.iter().cloned().collect();
            (
                set(&release.package.dependencies),
                set(&release.package.conflicts),
            )
        };
        if declared(kept) != declared(release) {
            bail!(
                "{} and {} share one archive but declare different dependencies or conflicts; install them separately",
                reference_of(kept),
                reference_of(release)
            );
        }
    }
    if stack
        .selected
        .package
        .sha256
        .as_deref()
        .is_some_and(|digest| by_digest.contains_key(digest))
    {
        bail!(
            "{}: its archive is also the archive of a missing dependency; install that dependency first",
            reference_of(stack.selected)
        );
    }
    needed.push(stack.selected);
    let absent: Vec<String> = needed
        .iter()
        .filter(|release| {
            !download::downloadable(release)
                && !stack
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
            "archives missing for this installation (manual download required):\n{}\nPass --file once for each archive.",
            absent.join("\n")
        );
    }
    Ok(needed)
}

pub(super) fn run(
    mut stack: Stack<'_>,
    state: ManagerState,
    args: &InstallArgs,
    json: bool,
) -> Result<()> {
    let releases = releases(&stack)?;
    let interactive = !args.yes;
    let policy = if args.allow_native_injector {
        NativeInjectorPolicy::Allow
    } else {
        NativeInjectorPolicy::Refuse
    };
    let explicit = &args.allow_executable;
    let unapproved: Vec<String> = releases
        .iter()
        .flat_map(|release| &release.package.executables)
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
        .map(|release| {
            let reference = reference_of(release).to_string();
            let download = if stack.archives.contains_key(&reference) {
                ""
            } else {
                " (downloads its archive)"
            };
            format!("{reference} {}{download}", release.package.version)
        })
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
    download::download(
        stack.cache,
        stack.cache_use,
        &releases,
        &mut stack.archives,
        json,
    )?;
    let mut digests = BTreeMap::new();
    let members = releases
        .iter()
        .map(|release| {
            let archive = stack
                .archives
                .get(&reference_of(release).to_string())
                .context("every stack member has a supplied, cached or downloaded archive")?;
            let mut approved = explicit.clone();
            if interactive {
                approved.extend(release.package.executables.iter().cloned());
            }
            digests.insert(
                reference_of(release).to_string(),
                release.package.sha256.as_deref(),
            );
            Ok(StackMember::new(
                archive.clone(),
                stack.catalog.release(release)?,
                approved,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut store = state.open_writable(&ProtectedTarget {
        game: Some(stack.game_id.clone()),
        root: Some(stack.installation.clone()),
    })?;
    let installed =
        store.install_catalog_stack(&stack.installation, &members, policy, |member| {
            let reference = member.release.reference.to_string();
            let plan = match content_root_appdata(&stack.game_id)? {
                Some(appdata) => preview_install_zip_with_appdata(
                    &stack.game_id,
                    &stack.installation,
                    &member.archive,
                    &appdata,
                )?,
                None => preview_install_zip_with_permissions(
                    &stack.game_id,
                    &stack.installation,
                    &member.archive,
                    &member.approved_executables,
                )?,
            };
            if digests.get(&reference).copied().flatten() != Some(plan.archive_sha256.as_str()) {
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
