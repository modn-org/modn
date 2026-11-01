//! Catalog resolution and manifest-verified local mod installation.
use crate::commands::InstallArgs;
use crate::dependency_plan::{self, PlannedDependency};
use crate::download;
use crate::install_consent::{self, Request};
use crate::selection::selected_installation;
use crate::stack_install;
use crate::state_access::{ManagerState, ProtectedTarget};
use anyhow::{bail, Context, Result};
use modn_core::acquisition::CacheUse;
use modn_core::archive::{self, inspect};
use modn_core::buckets::{install_order, ArchiveFormat, BucketPackage, BucketStore, PackageSource};
use modn_core::planning::{preview_install_zip_with_permissions, InstallPlan, PlanStatus};
use modn_core::transactions::{CatalogDependency, CatalogReference, CatalogRelease};
use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(super) struct AmbiguousTarget {
    query: String,
    candidates: Vec<String>,
    pub(super) json: bool,
}

impl AmbiguousTarget {
    pub(super) fn as_json(&self) -> serde_json::Value {
        serde_json::json!({
            "error": "ambiguous_target",
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

fn resolve(
    query: &str,
    catalog: &BucketStore,
    game_filter: Option<&str>,
    json: bool,
) -> Result<BucketPackage> {
    let parts: Vec<&str> = query.split('/').collect();
    if !(1..=3).contains(&parts.len()) || parts.iter().any(|part| part.is_empty()) {
        bail!("invalid mod reference; expected mod, game/mod or bucket/game/mod");
    }
    let mut candidates: Vec<_> = catalog
        .packages(game_filter)?
        .into_iter()
        .filter(|package| match parts.as_slice() {
            [id] => package.package.id == *id,
            [game, id] => package.package.game == *game && package.package.id == *id,
            [bucket, game, id] => {
                package.bucket == *bucket
                    && package.package.game == *game
                    && package.package.id == *id
            }
            _ => false,
        })
        .collect();
    match candidates.len() {
        0 => bail!("catalog mod not found: {query}"),
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

// Bucket manifests name only installable formats; unknown ones never match.
fn expected_format(format: &ArchiveFormat) -> Option<archive::ArchiveFormat> {
    match format {
        ArchiveFormat::Zip => Some(archive::ArchiveFormat::Zip),
        ArchiveFormat::SevenZ => Some(archive::ArchiveFormat::SevenZ),
        _ => None,
    }
}

pub(super) fn reference_of(package: &BucketPackage) -> CatalogReference {
    CatalogReference::new(
        package.bucket.clone(),
        package.package.game.clone(),
        package.package.id.clone(),
    )
}

// Resolve declared dependencies to manifest-pinned digests in the same bucket.
// Readiness itself is decided by the Rust core, under the install lock.
pub(super) fn catalog_release(
    selected: &BucketPackage,
    catalog: &BucketStore,
) -> Result<CatalogRelease> {
    let reference = reference_of(selected);
    let catalog_packages = if selected.package.dependencies.is_empty() {
        Vec::new()
    } else {
        catalog.packages(Some(&selected.package.game))?
    };
    let dependencies = selected
        .package
        .dependencies
        .iter()
        .map(|id| {
            let dependency = catalog_packages
                .iter()
                .find(|candidate| {
                    candidate.bucket == selected.bucket && candidate.package.id == *id
                })
                .with_context(|| {
                    format!(
                        "dependency unavailable in bucket: {}/{}/{}",
                        selected.bucket, selected.package.game, id
                    )
                })?;
            Ok(CatalogDependency::new(
                reference_of(dependency),
                dependency.package.sha256.clone(),
            ))
        })
        .collect::<Result<_>>()?;
    let conflicts = selected
        .package
        .conflicts
        .iter()
        .map(|id| {
            CatalogReference::new(
                selected.bucket.clone(),
                selected.package.game.clone(),
                id.clone(),
            )
        })
        .collect();
    Ok(CatalogRelease::new(reference, dependencies).with_conflicts(conflicts))
}

/// Match each supplied archive to the selected release or one of its
/// dependencies by SHA-256, verifying its format against that manifest.
fn match_archives(
    files: &[PathBuf],
    selected: &BucketPackage,
    dependencies: &[BucketPackage],
) -> Result<BTreeMap<String, PathBuf>> {
    let reference = reference_of(selected);
    let mut archives = BTreeMap::new();
    for file in files {
        if !std::fs::symlink_metadata(file)?.file_type().is_file() {
            bail!("archive must be a regular file: {}", file.display());
        }
        let inspected = inspect(file)?;
        // One archive is the verified artifact of every release pinning its digest.
        let matches: Vec<&BucketPackage> = dependencies
            .iter()
            .chain([selected])
            .filter(|release| release.package.sha256 == inspected.sha256)
            .collect();
        if matches.is_empty() {
            bail!(
                "{}: archive SHA-256 does not match catalog manifest of {reference} or its dependencies",
                file.display()
            );
        }
        for release in matches {
            let matched = reference_of(release);
            if Some(inspected.format) != expected_format(&release.package.archive_format) {
                bail!("{matched}: archive format does not match catalog manifest");
            }
            archives.insert(matched.to_string(), file.clone());
        }
    }
    Ok(archives)
}

fn install_mod(
    selected: BucketPackage,
    args: &InstallArgs,
    state_path: &Path,
    json: bool,
) -> Result<()> {
    let reference = reference_of(&selected).to_string();
    let catalog = BucketStore::new(
        state_path
            .parent()
            .context("invalid manager state directory")?,
    );
    let mut order = install_order(&selected, &catalog.packages(Some(&selected.package.game))?)?;
    order.pop(); // The selected release is last; the rest are its dependencies.
    let interactive = !args.yes && !args.dry_run;
    // Archives match releases by digest before any manager state is read.
    let mut archives = match_archives(&args.file, &selected, &order)?;
    let cache = download::cache_for(state_path)?;
    let cache_use = if args.no_cache {
        CacheUse::Refresh
    } else {
        download::add_cached(&cache, order.iter().chain([&selected]), &mut archives)?;
        CacheUse::Reuse
    };
    if !archives.contains_key(&reference) && !args.dry_run && !download::downloadable(&selected) {
        let source = dependency_plan::acquisition(&selected.package.source);
        bail!("{reference}: {source}, then use modn install {reference} --file <archive>");
    }
    if interactive && !install_consent::can_prompt(json) {
        bail!("installation writes game files; choose --dry-run, confirm in a terminal, or explicitly approve with --yes");
    }
    let state = ManagerState::load_deferred(state_path)?;
    let (game_id, installation) = selected_installation(
        state.existing(),
        args.path.clone(),
        Some(&selected.package.game),
    )?;
    let release = catalog_release(&selected, &catalog)?;
    if !args.dry_run {
        // Downloads write the cache before the install opens writable state.
        state.validate_download(
            cache.root(),
            &ProtectedTarget {
                game: Some(game_id.clone()),
                root: Some(installation.clone()),
            },
        )?;
    }
    let dependencies = dependency_plan::plan(&order, &archives, state.existing(), &installation)?;
    // Check declared conflicts before any download or write; the Rust core
    // repeats the check for each package under the transaction lock.
    let installing: Vec<&BucketPackage> = order
        .iter()
        .zip(&dependencies)
        .filter(|(_, dependency)| dependency.missing())
        .map(|(release, _)| release)
        .chain([&selected])
        .collect();
    let conflicts =
        dependency_plan::conflicts(&installing, &catalog, state.existing(), &installation)?;
    if !args.dry_run && !conflicts.is_empty() {
        bail!(
            "installation blocked by declared conflicts; uninstall the installed packages first:\n  {}",
            conflicts.join("\n  ")
        );
    }
    // Missing dependencies install with the mod as one stack. A damaged
    // dependency or another installed release keeps the single-install path,
    // which fails closed: Modn never replaces or repairs a framework implicitly.
    let stackable = dependencies
        .iter()
        .all(|dependency| dependency.ready() || dependency.missing());
    if !args.dry_run && stackable && !dependencies.iter().all(PlannedDependency::ready) {
        let stack = stack_install::Stack {
            selected: &selected,
            order: &order,
            dependencies: &dependencies,
            archives,
            catalog: &catalog,
            cache: &cache,
            cache_use,
            game_id,
            installation,
        };
        return stack_install::run(stack, state, args, json);
    }
    if !args.dry_run {
        // Preview readiness early; `install_catalog_release` repeats it under the lock.
        match state.existing() {
            Some(store) => {
                store.ready_dependencies(&installation, &release.dependencies)?;
            }
            None => {
                if let Some(dependency) = release.dependencies.first() {
                    bail!(
                        "dependency not installed or intact: {}; install or restore it first",
                        dependency.reference
                    );
                }
            }
        }
    }
    if !args.dry_run && !archives.contains_key(&reference) {
        // The install prompt needs the archive's plan, so a download asks first.
        if interactive {
            let url = match &selected.package.source {
                PackageSource::Https { url } => url.as_str(),
                _ => "",
            };
            let download = format!("{reference} {} from {url}", selected.package.version);
            if !install_consent::confirm_download(
                &[download],
                &mut io::stdin().lock(),
                &mut io::stderr(),
            )? {
                bail!("{reference}: download cancelled");
            }
        }
        download::download(&cache, cache_use, &[&selected], &mut archives, json)?;
    }
    let Some(archive) = archives.get(&reference).map(PathBuf::as_path) else {
        return dependency_plan::report(
            &reference,
            &selected,
            None,
            &dependencies,
            &conflicts,
            json,
        );
    };
    let preview = |approved: &[String]| -> Result<InstallPlan> {
        let plan =
            preview_install_zip_with_permissions(&game_id, &installation, archive, approved)?;
        if plan.archive_sha256 != selected.package.sha256 {
            bail!("{reference}: source changed after catalog verification");
        }
        Ok(plan)
    };
    let explicit = &args.allow_executable;
    let declared = &selected.package.executables;
    let mut plan = preview(explicit)?;
    // An unapproved executable is absent from a blocked plan, so find the
    // declared ones the archive installs from a candidate that includes them.
    // The candidate is only installed after interactive confirmation.
    let mut candidate_approvals = explicit.clone();
    candidate_approvals.extend(
        declared
            .iter()
            .filter(|path| !explicit.contains(path))
            .cloned(),
    );
    let candidate = if candidate_approvals.len() > explicit.len() {
        Some(preview(&candidate_approvals)?)
    } else {
        None
    };
    let pending = candidate.as_ref().map_or_else(Vec::new, |candidate| {
        let destinations: Vec<&str> = candidate
            .operations
            .iter()
            .map(|operation| operation.destination.as_str())
            .collect();
        install_consent::pending_executables(declared, explicit, &destinations)
    });
    if args.dry_run {
        let preview = dependency_plan::ArchivePreview {
            plan: &plan,
            unapproved_declared_executables: &pending,
        };
        return dependency_plan::report(
            &reference,
            &selected,
            Some(preview),
            &dependencies,
            &conflicts,
            json,
        );
    }
    let mut approved = explicit.clone();
    if interactive && !pending.is_empty() {
        // Declared executables join the approval only after the operator says yes.
        approved.extend(pending.iter().cloned());
        plan = preview(&approved)?;
    }
    if plan.status != PlanStatus::Reviewable {
        if !pending.is_empty() && !interactive {
            bail!(
                "{reference}: installation blocked; its manifest declares {}. Confirm in a terminal without --yes, or pass --allow-executable for each path",
                pending.join(", ")
            );
        }
        bail!("{reference}: installation blocked; inspect with --dry-run first");
    }
    if interactive {
        let request = Request {
            reference: &reference,
            game: &installation,
            files: plan.operations.len(),
            executables: &pending,
        };
        if !install_consent::confirm(&request, &mut io::stdin().lock(), &mut io::stderr())? {
            bail!("{reference}: installation cancelled");
        }
    }
    let mut store = state.open_writable(&ProtectedTarget {
        game: Some(game_id),
        root: Some(installation.clone()),
    })?;
    let package = store.install_catalog_release_with_permissions(
        &installation,
        archive,
        &plan,
        &release,
        &approved,
    )?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "package": reference, "installed": package,
            }))?
        );
    } else {
        println!(
            "Installed {reference} ({} owned files)",
            package.files.len()
        );
    }
    Ok(())
}

/// Install one bucket-resolved release using an explicit, verified local archive.
pub(super) fn run(
    args: &InstallArgs,
    state_path: &Path,
    game_filter: Option<&str>,
    json: bool,
) -> Result<()> {
    let root = state_path
        .parent()
        .context("invalid manager state directory")?;
    let catalog = BucketStore::new(root);
    let package = resolve(&args.target, &catalog, game_filter, json)?;
    install_mod(package, args, state_path, json)
}
