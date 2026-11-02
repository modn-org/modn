use crate::catalog_install::{self, catalog_release, expected_format, reference_of};
use crate::commands::UpdateArgs;
use crate::dependency_plan::acquisition;
use crate::download;
use crate::install_consent::{self, UpdateRequest};
use crate::selection::selected_installation;
use crate::state_access::{ManagerState, ProtectedTarget};
use crate::update_report::{self, DependentCheck, FileChanges, Preview};
use anyhow::{bail, Context, Result};
use modn_core::acquisition::CacheUse;
use modn_core::archive::inspect;
use modn_core::buckets::{install_order, BucketPackage, BucketStore};
use modn_core::planning::{preview_update_with_permissions, InstallPlan, PlanStatus};
use modn_core::transactions::ManagedPackage;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

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

fn supplied_archive(file: &Path, target: &BucketPackage) -> Result<PathBuf> {
    if !std::fs::symlink_metadata(file)?.file_type().is_file() {
        bail!("archive must be a regular file: {}", file.display());
    }
    let inspected = inspect(file)?;
    let reference = reference_of(target);
    if inspected.sha256 != target.package.sha256 {
        bail!(
            "{}: archive SHA-256 does not match {reference} {}",
            file.display(),
            target.package.version
        );
    }
    if Some(inspected.format) != expected_format(&target.package.archive_format) {
        bail!("{reference}: archive format does not match catalog manifest");
    }
    Ok(file.to_path_buf())
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
    approved: Vec<String>,
    pending: Vec<String>,
}

fn plan_update(
    game_id: &str,
    game: &Path,
    archive: &Path,
    target: &BucketPackage,
    replaced: &[String],
    explicit: &[String],
) -> Result<Candidate> {
    let preview = |approved: &[String]| -> Result<InstallPlan> {
        let plan = preview_update_with_permissions(game_id, game, archive, approved, replaced)?;
        if plan.archive_sha256 != target.package.sha256 {
            bail!(
                "{}: source changed after catalog verification",
                reference_of(target)
            );
        }
        Ok(plan)
    };
    let declared = &target.package.executables;
    let mut approved = explicit.to_vec();
    approved.extend(declared.iter().filter(|p| !explicit.contains(p)).cloned());
    let plan = preview(&approved)?;
    let destinations: Vec<&str> = plan
        .operations
        .iter()
        .map(|operation| operation.destination.as_str())
        .collect();
    let pending = install_consent::pending_executables(declared, explicit, &destinations);
    Ok(Candidate {
        plan,
        approved,
        pending,
    })
}

pub(super) fn run(
    args: &UpdateArgs,
    state_path: &Path,
    requested: Option<&str>,
    json: bool,
) -> Result<()> {
    if args.download && !args.dry_run {
        bail!("--download only prepares a --dry-run; an update downloads what it needs");
    }
    let catalog = BucketStore::new(
        state_path
            .parent()
            .context("invalid manager state directory")?,
    );
    let current = catalog_install::resolve(&args.target, &catalog, requested, json)?;
    let reference = reference_of(&current);
    let label = reference.to_string();
    let releases = catalog.releases(&current.bucket, &current.package.game, &current.package.id)?;
    let installed_versions: Vec<(String, String)> = releases
        .iter()
        .map(|r| (r.package.sha256.clone(), r.package.version.clone()))
        .collect();
    let target = choose_release(releases, args.version.as_deref(), &label)?;
    let interactive = !args.yes && !args.dry_run;
    if interactive && !install_consent::can_prompt(json) {
        bail!("an update writes game files; choose --dry-run, confirm in a terminal, or explicitly approve with --yes");
    }
    let state = ManagerState::load_deferred(state_path)?;
    let store = state
        .existing()
        .context("no manager state database; nothing is installed")?;
    let (game_id, game) =
        selected_installation(Some(store), args.path.clone(), Some(&current.package.game))?;
    let old = installed_package(store.installed(&game)?, &reference)?;
    let installed_version = installed_versions
        .iter()
        .find(|(sha256, _)| *sha256 == old.archive_sha256)
        .map(|(_, version)| version.as_str());
    let mut blockers = Vec::new();
    if let Err(error) = install_order(&target, &catalog.packages(Some(&target.package.game))?) {
        blockers.push(format!("{error:#}"));
    }
    let release = catalog_release(&target, &catalog)?;
    let dependents: Vec<DependentCheck> =
        update_report::dependent_checks(store, &game, &old.id, &catalog, &target)?;
    blockers.extend(update_report::dependency_blockers(
        &release,
        &store.dependency_states(&game, &release.dependencies)?,
    ));
    blockers.extend(update_report::dependent_blockers(
        &dependents,
        &current.package.id,
    ));
    let up_to_date = old.archive_sha256 == target.package.sha256;
    let mut archives = BTreeMap::new();
    if let Some(file) = &args.file {
        archives.insert(label.clone(), supplied_archive(file, &target)?);
    }
    let cache = download::cache_for(state_path)?;
    let cache_use = if args.no_cache {
        CacheUse::Refresh
    } else {
        download::add_cached(&cache, [&target], &mut archives)?;
        CacheUse::Reuse
    };
    let protected = ProtectedTarget {
        game: Some(game_id.clone()),
        root: Some(game.clone()),
    };
    let fetch = !up_to_date && !archives.contains_key(&label) && (args.download || !args.dry_run);
    if fetch && blockers.is_empty() {
        if !download::downloadable(&target) {
            let source = acquisition(&target.package.source);
            bail!("{label}: {source}, then use modn update {label} --file <archive>");
        }
        state.validate_download(cache.root(), &protected)?;
        if interactive {
            let download = format!("{label} {}", target.package.version);
            if !install_consent::confirm_download(
                &[download],
                &mut io::stdin().lock(),
                &mut io::stderr(),
            )? {
                bail!("{label}: download cancelled");
            }
        }
        download::download(&cache, cache_use, &[&target], &mut archives, json)?;
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
        )?),
        _ => None,
    };
    let preview = Preview {
        reference: &label,
        old: &old,
        installed_version,
        target: &target,
        files: candidate
            .as_ref()
            .map(|candidate| FileChanges::new(&old, &candidate.plan)),
        plan: candidate.as_ref().map(|candidate| &candidate.plan),
        archive_needed: (archive.is_none() && !up_to_date)
            .then(|| acquisition(&target.package.source)),
        dependents: &dependents,
        blockers: &blockers,
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
    let approved = if interactive {
        candidate.approved.clone()
    } else {
        args.allow_executable.clone()
    };
    if !interactive && !candidate.pending.is_empty() {
        bail!(
            "{label}: its manifest declares {}. Confirm in a terminal without --yes, or pass --allow-executable for each path",
            candidate.pending.join(", ")
        );
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
    let package = store.update_catalog_release(
        &game,
        &old.id,
        archive,
        &candidate.plan,
        &release,
        &approved,
    )?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "package": label, "replaced": old.id, "installed": package,
            }))?
        );
    } else {
        println!(
            "Updated {label} to {} ({} owned files)",
            target.package.version,
            package.files.len()
        );
    }
    Ok(())
}
