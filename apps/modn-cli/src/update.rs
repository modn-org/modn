use crate::catalog_install::{self, expected_format, reference_of};
use crate::commands::UpdateArgs;
use crate::dependency_plan::acquisition;
use crate::download;
use crate::install_consent::{self, UpdateRequest};
use crate::modules::content_root_appdata;
use crate::progress::Progress;
use crate::selection::selected_installation;
use crate::state_access::{ManagerState, ProtectedTarget};
use crate::update_report::{self, DependentCheck, FileChanges, Preview};
use anyhow::{bail, Context, Result};
use identity::{installed_identity, same_release, target_identity};
use modn_core::acquisition::CacheUse;
use modn_core::approval::{ApprovalRefusal, Capabilities, Uncovered};
use modn_core::archive::{inspect, ArchiveFormat};
use modn_core::buckets::{BucketPackage, BucketStore, PackageSource};
use modn_core::error::{coded, ErrorCode};
use modn_core::history::HistoryContext;
use modn_core::manager_roots::ManagerRoots;
use modn_core::planning::{
    preview_update_with_appdata, preview_update_with_permissions, InstallPlan, PlanStatus,
};
use modn_core::transactions::ManagedPackage;
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

#[path = "update_identity.rs"]
mod identity;

fn planned_source(release: &BucketPackage) -> &PackageSource {
    crate::dependency_plan::fetchable_source(&release.package.sources)
        .unwrap_or(release.package.primary_source())
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

fn supplied_format(target: &BucketPackage) -> Option<ArchiveFormat> {
    expected_format(target.package.archive_format.as_ref()).or_else(|| {
        matches!(target.package.primary_source(), PackageSource::Git { .. })
            .then_some(ArchiveFormat::Zip)
    })
}

fn supplied_archive(
    file: &Path,
    target: &BucketPackage,
    digests: &BTreeMap<String, String>,
) -> Result<download::SuppliedArchive> {
    if !std::fs::symlink_metadata(file)?.file_type().is_file() {
        bail!("archive must be a regular file: {}", file.display());
    }
    let inspected = inspect(file)?;
    let reference = reference_of(target);
    let expected = download::digest(target, digests).with_context(|| {
        format!(
            "{reference}: Modn knows no archive digest to verify {} against; run modn update {reference} without --file so it can acquire the archive from the source",
            file.display()
        )
    })?;
    if expected != inspected.blake3 {
        bail!(
            "{}: archive BLAKE3 does not match {reference} {}",
            file.display(),
            target.package.version
        );
    }
    if Some(inspected.format) != supplied_format(target) {
        bail!("{reference}: archive format does not match catalog manifest");
    }
    Ok(download::SuppliedArchive {
        path: file.to_path_buf(),
        blake3: inspected.blake3,
    })
}

fn installed_package(
    packages: Vec<ManagedPackage>,
    reference: &modn_core::transactions::CatalogReference,
) -> Result<ManagedPackage> {
    let mut matches: Vec<ManagedPackage> = packages
        .into_iter()
        .filter(|package| package.catalog.as_ref() == Some(reference))
        .collect();
    match matches.len() {
        0 => bail!("{reference} is not a managed package here; install it with modn install"),
        1 => Ok(matches.remove(0)),
        _ => bail!(
            "several managed packages come from {reference}: {}; uninstall the extra ones first",
            matches
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn replaced_paths(old: &ManagedPackage) -> Vec<String> {
    old.files
        .iter()
        .map(|file| file.destination.clone())
        .chain(old.directories.iter().cloned())
        .collect()
}

struct Candidate {
    plan: InstallPlan,
    pending: Vec<String>,
    present: std::collections::BTreeSet<String>,
}

fn plan_update(
    game_id: &str,
    game: &Path,
    archive: &Path,
    target: &BucketPackage,
    replaced: &[String],
    explicit: &[String],
    stored: Option<&Capabilities>,
) -> Result<Candidate> {
    let appdata = content_root_appdata(game_id)?;
    let preview = |approved: &[String]| -> Result<InstallPlan> {
        match &appdata {
            Some(appdata) => {
                preview_update_with_appdata(game_id, game, archive, approved, replaced, appdata)
            }
            None => preview_update_with_permissions(game_id, game, archive, approved, replaced),
        }
    };
    let declared = &target.package.executables;
    let mut base_approved = explicit.to_vec();
    if let Some(stored) = stored {
        let additions: Vec<String> = stored
            .executables
            .iter()
            .filter(|path| !base_approved.contains(path))
            .cloned()
            .collect();
        base_approved.extend(additions);
    }
    let mut approved = base_approved.clone();
    let additions: Vec<String> = declared
        .iter()
        .filter(|path| !approved.contains(path))
        .cloned()
        .collect();
    approved.extend(additions);
    let plan = preview(&approved)?;
    let destinations: Vec<&str> = plan
        .operations
        .iter()
        .map(|operation| operation.destination.as_str())
        .collect();
    let pending = install_consent::pending_executables(declared, &base_approved, &destinations);
    let present = install_consent::present_executables(declared, &destinations);
    Ok(Candidate {
        plan,
        pending,
        present,
    })
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
    let Some(target) = args.target.as_deref() else {
        return crate::update_refresh::run(args, roots, requested, json);
    };
    if progress.enabled() && !args.yes && !args.dry_run {
        return Err(coded(
            ErrorCode::ApprovalRequired,
            "an update writes game files; pass --yes with --progress json",
        ));
    }
    let state_path = roots.state_database();
    let catalog = BucketStore::new(roots).snapshot(requested)?;
    let current = catalog_install::resolve(target, &catalog, json)?;
    let reference = reference_of(&current);
    let label = reference.to_string();
    let releases = catalog.releases(&current.bucket, &current.package.game, &current.package.id)?;
    let installed_versions: Vec<(String, String)> = releases
        .iter()
        .filter_map(|r| {
            r.package
                .pin()
                .map(|pin| (pin.to_owned(), r.package.version.clone()))
        })
        .collect();
    let target = choose_release(releases, args.version.as_deref(), &label)?;
    download::validate_links(&args.nxm_links, &[&target])?;
    let interactive = !args.yes && !args.dry_run && !progress.enabled();
    let state = ManagerState::load_deferred(&state_path)?;
    let store = state
        .existing()
        .context("no manager state database; nothing is installed")?;
    let (game_id, game) =
        selected_installation(Some(store), args.path.clone(), Some(&current.package.game))?;
    let old = installed_package(store.installed(&game)?, &reference)?;
    if old.pinned {
        println!(
            "Skipped {}: managed package is pinned; run `modn unpin {}` to update it.",
            old.id, old.id
        );
        return Ok(());
    }
    let flags = install_consent::flags(args.allow_native_injector, &args.allow_executable);
    let stored = install_consent::stored_approval(Some(store), &game, &reference)?;
    let native_covered = args.allow_native_injector
        || stored
            .as_ref()
            .is_some_and(|approval| approval.native_injector);
    if interactive && !install_consent::can_prompt(json) {
        bail!("an update writes game files; choose --dry-run, confirm in a terminal, or explicitly approve with --yes");
    }
    let installed_version = installed_versions
        .iter()
        .find(|(archive_blake3, _)| *archive_blake3 == old.archive_blake3)
        .map(|(_, version)| version.as_str())
        .or(old.version.as_deref());
    let mut blockers = Vec::new();
    if let Err(error) = catalog.install_order(&target) {
        blockers.push(format!("{error:#}"));
    }
    let release = catalog.release(&target)?;
    let dependents: Vec<DependentCheck> =
        update_report::dependent_checks(store, &game, &old, &catalog, &target)?;
    blockers.extend(update_report::dependency_blockers(
        &release,
        &store.dependency_states(&game, &release.dependencies)?,
    ));
    blockers.extend(update_report::dependent_blockers(
        &dependents,
        &current.package.id,
    ));
    let up_to_date = same_release(
        installed_identity(&old),
        target_identity(&target.package, &old),
    );
    if target.package.native_injector && !native_covered && !up_to_date && !args.dry_run {
        return Err(
            ApprovalRefusal::new(reference.clone(), Uncovered::native_injector()).into_error(),
        );
    }
    let cache = roots.archive_cache();
    let mut archives = BTreeMap::new();
    let mut digests = BTreeMap::new();
    let mut served = BTreeMap::new();
    let mut download_attempts = Vec::new();
    let cache_use = if args.no_cache {
        CacheUse::Refresh
    } else {
        download::add_cached(&cache, [&target], &mut archives, &mut digests, &mut served)?;
        CacheUse::Reuse
    };
    let mut supplied: BTreeMap<String, download::SuppliedArchive> = BTreeMap::new();
    if let Some(file) = &args.file {
        let verified = supplied_archive(file, &target, &digests)?;
        archives.insert(label.clone(), verified.path.clone());
        supplied.insert(label.clone(), verified);
    }
    let protected = ProtectedTarget {
        game: Some(game_id.clone()),
        root: Some(game.clone()),
    };
    let fetch = !up_to_date && !archives.contains_key(&label) && (args.download || !args.dry_run);
    if fetch && blockers.is_empty() {
        let planned = planned_source(&target);
        if !download::downloadable(&target) {
            let source = acquisition(planned);
            bail!("{label}: {source}, then use modn update {label} --file <archive>");
        }
        state.validate_download(cache.root(), &protected)?;
        if interactive {
            let download = format!(
                "{label} {}; {}",
                target.package.version,
                acquisition(planned)
            );
            if !install_consent::confirm_download(
                &[download],
                &mut io::stdin().lock(),
                &mut io::stderr(),
            )? {
                bail!("{label}: download cancelled");
            }
        }
        download_attempts.extend(download::download(
            &cache,
            cache_use,
            &[&target],
            &mut archives,
            &mut digests,
            &mut served,
            &args.nxm_links,
            json,
            progress,
        )?);
    }
    let archive = archives.get(&label).cloned();
    let candidate = match (&archive, up_to_date) {
        (Some(archive), false) => Some(plan_update(
            &game_id,
            &game,
            archive,
            &target,
            &replaced_paths(&old),
            &args.allow_executable,
            stored.as_ref(),
        )?),
        _ => None,
    };
    if let Some(candidate) = &candidate {
        let expected = download::digest(&target, &digests).with_context(|| {
            format!(
                "{label}: Modn knows no archive digest to verify {} against",
                target.package.version
            )
        })?;
        if candidate.plan.archive_blake3 != expected {
            bail!("{label}: source changed after catalog verification");
        }
    }
    let check = candidate.as_ref().map(|candidate| {
        let needed = Capabilities::new(
            target.package.native_injector,
            candidate.present.iter().cloned(),
        );
        install_consent::ApprovalCheck::new(reference.clone(), needed, stored.clone(), &flags)
    });
    let preview = Preview {
        reference: &label,
        old: &old,
        installed_version,
        target: &target,
        up_to_date,
        files: candidate
            .as_ref()
            .map(|candidate| FileChanges::new(&old, &candidate.plan)),
        plan: candidate.as_ref().map(|candidate| &candidate.plan),
        archive_needed: (archive.is_none() && !up_to_date)
            .then(|| acquisition(planned_source(&target))),
        dependents: &dependents,
        blockers: &blockers,
        downloads: &download_attempts,
        unapproved_declared_executables: candidate
            .as_ref()
            .map_or(&[], |candidate| candidate.pending.as_slice()),
    };
    if args.dry_run || up_to_date {
        return preview.print(json);
    }
    if !blockers.is_empty() {
        bail!("{label}: update blocked:\n  {}", blockers.join("\n  "));
    }
    let (Some(archive), Some(candidate)) = (archive.as_deref(), candidate.as_ref()) else {
        bail!("{label}: no archive for {}", target.package.version);
    };
    let check = check.as_ref().context("update plan has approval state")?;
    if !check.uncovered.is_empty() && (!interactive || check.uncovered.native_injector) {
        return check.ensure_approved();
    }
    let mut approved = check.approved_executables();
    if interactive {
        approved.extend(check.uncovered.executables.iter().cloned());
    }
    if candidate.plan.status != PlanStatus::Reviewable {
        bail!("{label}: update blocked; inspect it with --dry-run first");
    }
    if interactive {
        preview.print(false)?;
        let dependents: Vec<String> = dependents
            .iter()
            .map(|check| check.package_id_label())
            .collect();
        let request = UpdateRequest {
            reference: &label,
            game: &game,
            from: installed_version.unwrap_or("an unlisted release"),
            to: &target.package.version,
            dependents: &dependents,
            executables: &candidate.pending,
        };
        if !install_consent::confirm_update(&request, &mut io::stdin().lock(), &mut io::stderr())? {
            bail!("{label}: update cancelled");
        }
    }
    let mut store = state.open_writable(&protected)?;
    store.set_control(progress.control().clone());
    store.set_history_context(
        HistoryContext::new(game_id.clone(), game.clone()).with_source(
            &reference,
            download::serving_source(&label, &supplied, &served, &target),
        ),
    );
    let package = store.update_catalog_release(
        &game,
        &old.id,
        archive,
        &candidate.plan,
        &release,
        check.policy(),
        &approved,
    )?;
    let operation_id = store.committed_operation_id().map(str::to_owned);
    install_consent::record(&mut store, &game, &reference, &check.needed)?;
    if json {
        let mut result = serde_json::json!({
            "package": label, "replaced": old.id, "installed": package,
        });
        if let Some(operation_id) = operation_id {
            result["operation_id"] = serde_json::Value::String(operation_id);
        }
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!(
            "Updated {label} to {} ({} owned files)",
            target.package.version,
            package.files.len()
        );
    }
    Ok(())
}
