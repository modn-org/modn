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
use modn_core::buckets::{
    ArchiveFormat, BucketPackage, BucketSnapshot, BucketStore, PackageSource,
};
use modn_core::planning::{preview_install_zip_with_permissions, InstallPlan, PlanStatus};
use modn_core::transactions::CatalogReference;
use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

#[cfg(test)]
#[path = "catalog_snapshot_tests.rs"]
mod tests;

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

pub(super) fn resolve(query: &str, catalog: &BucketSnapshot, json: bool) -> Result<BucketPackage> {
    let mut candidates = catalog.candidates(query)?;
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

pub(super) fn expected_format(format: &ArchiveFormat) -> Option<archive::ArchiveFormat> {
    match format {
        ArchiveFormat::Zip => Some(archive::ArchiveFormat::Zip),
        ArchiveFormat::SevenZ => Some(archive::ArchiveFormat::SevenZ),
        _ => None,
    }
}

pub(super) fn reference_of(package: &BucketPackage) -> CatalogReference {
    package.reference()
}

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
    catalog: &BucketSnapshot,
    args: &InstallArgs,
    state_path: &Path,
    json: bool,
) -> Result<()> {
    let reference = reference_of(&selected).to_string();
    let mut order = catalog.install_order(&selected)?;
    order.pop();
    let interactive = !args.yes && !args.dry_run;
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
    let release = catalog.release(&selected)?;
    if !args.dry_run || args.download {
        state.validate_download(
            cache.root(),
            &ProtectedTarget {
                game: Some(game_id.clone()),
                root: Some(installation.clone()),
            },
        )?;
    }
    let mut dependencies =
        dependency_plan::plan(&order, &archives, state.existing(), &installation)?;
    let installing: Vec<&BucketPackage> = order
        .iter()
        .zip(&dependencies)
        .filter(|(_, dependency)| dependency.missing())
        .map(|(release, _)| release)
        .chain([&selected])
        .collect();
    if args.download {
        download::download(&cache, cache_use, &installing, &mut archives, json)?;
        dependencies = dependency_plan::plan(&order, &archives, state.existing(), &installation)?;
    }
    let conflicts =
        dependency_plan::conflicts(&installing, catalog, state.existing(), &installation)?;
    if !args.dry_run && !conflicts.is_empty() {
        bail!(
            "installation blocked by declared conflicts; uninstall the installed packages first:\n  {}",
            conflicts.join("\n  ")
        );
    }
    let stackable = dependencies
        .iter()
        .all(|dependency| dependency.ready() || dependency.missing());
    if !args.dry_run && stackable && !dependencies.iter().all(PlannedDependency::ready) {
        let stack = stack_install::Stack {
            selected: &selected,
            order: &order,
            dependencies: &dependencies,
            archives,
            catalog,
            cache: &cache,
            cache_use,
            game_id,
            installation,
        };
        return stack_install::run(stack, state, args, json);
    }
    if !args.dry_run {
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

pub(super) fn run(
    args: &InstallArgs,
    state_path: &Path,
    game_filter: Option<&str>,
    json: bool,
) -> Result<()> {
    let root = state_path
        .parent()
        .context("invalid manager state directory")?;
    let catalog = BucketStore::new(root).snapshot(game_filter)?;
    let package = resolve(&args.target, &catalog, json)?;
    install_mod(package, &catalog, args, state_path, json)
}
