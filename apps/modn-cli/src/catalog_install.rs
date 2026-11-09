use crate::commands::InstallArgs;
use crate::dependency_plan::{self, PlannedDependency};
use crate::download;
use crate::install_consent::{self, Request};
use crate::modules::content_root_appdata;
use crate::progress::Progress;
use crate::selection::selected_installation;
use crate::stack_install;
use crate::state_access::{ManagerState, ProtectedTarget};
use anyhow::{bail, Context, Result};
use modn_core::acquisition::CacheUse;
use modn_core::approval::{ApprovalRefusal, Capabilities, Uncovered};
use modn_core::archive::{self, inspect};
use modn_core::buckets::{ArchiveFormat, BucketPackage, BucketSnapshot, BucketStore};
use modn_core::error::{coded, ErrorCode};
use modn_core::planning::{
    preview_install_zip_with_appdata, preview_install_zip_with_permissions, InstallPlan, PlanStatus,
};
use modn_core::transactions::{CatalogReference, NativeInjectorPolicy};
use std::collections::{BTreeMap, BTreeSet};
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
            .filter(|release| release.package.sha256.as_deref() == Some(inspected.sha256.as_str()))
            .collect();
        if matches.is_empty() {
            bail!(
                "{}: archive SHA-256 does not match catalog manifest of {reference} or its dependencies",
                file.display()
            );
        }
        for release in matches {
            let matched = reference_of(release);
            if Some(inspected.format) != expected_format(release.package.archive_format.as_ref()) {
                bail!("{matched}: archive format does not match catalog manifest");
            }
            archives.insert(matched.to_string(), file.clone());
        }
    }
    Ok(archives)
}

fn check_native_approvals(
    selected: &BucketPackage,
    order: &[BucketPackage],
    args: &InstallArgs,
    state_path: &Path,
) -> Result<()> {
    if args.allow_native_injector || args.dry_run {
        return Ok(());
    }
    let state = ManagerState::load_deferred(state_path)?;
    let game = selected_installation(
        state.existing(),
        args.path.clone(),
        Some(&selected.package.game),
    )
    .ok()
    .map(|(_, installation)| installation);
    let installing: Vec<&BucketPackage> = match &game {
        Some(installation) => {
            let planned = dependency_plan::plan(
                order,
                &BTreeMap::new(),
                &BTreeMap::new(),
                state.existing(),
                installation,
            )?;
            order
                .iter()
                .zip(&planned)
                .filter(|(_, dependency)| dependency.missing())
                .map(|(release, _)| release)
                .chain([selected])
                .collect()
        }
        // Game selection can fail for ordinary reasons, such as no registration
        // or a missing --path. Refuse native injectors conservatively so the
        // approval error still precedes those generic errors.
        None => order.iter().chain([selected]).collect(),
    };
    for package in installing {
        if !package.package.native_injector {
            continue;
        }
        let reference = reference_of(package);
        let stored = match &game {
            Some(game) => install_consent::stored_approval(state.existing(), game, &reference)?,
            None => None,
        };
        if !stored.is_some_and(|approval| approval.native_injector) {
            return Err(ApprovalRefusal::new(reference, Uncovered::native_injector()).into_error());
        }
    }
    Ok(())
}

fn install_mod(
    selected: BucketPackage,
    catalog: &BucketSnapshot,
    args: &InstallArgs,
    state_path: &Path,
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
    let mut order = catalog.install_order(&selected)?;
    order.pop();
    let interactive = !args.yes && !args.dry_run && !progress.enabled();
    check_native_approvals(&selected, &order, args, state_path)?;
    let mut archives = match_archives(&args.file, &selected, &order)?;
    let mut digests = BTreeMap::new();
    let cache = download::cache_for(state_path)?;
    let cache_use = if args.no_cache {
        CacheUse::Refresh
    } else {
        download::add_cached(
            &cache,
            order.iter().chain([&selected]),
            &mut archives,
            &mut digests,
        )?;
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
    let flags = install_consent::flags(args.allow_native_injector, &args.allow_executable);
    let stored = install_consent::stored_approval(state.existing(), &installation, &reference_key)?;
    let native_covered = args.allow_native_injector
        || stored
            .as_ref()
            .is_some_and(|approval| approval.native_injector);
    let policy = if selected.package.native_injector && native_covered {
        NativeInjectorPolicy::Allow
    } else {
        NativeInjectorPolicy::Refuse
    };
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
        dependency_plan::plan(&order, &archives, &digests, state.existing(), &installation)?;
    let installing: Vec<&BucketPackage> = order
        .iter()
        .zip(&dependencies)
        .filter(|(_, dependency)| dependency.missing())
        .map(|(release, _)| release)
        .chain([&selected])
        .collect();
    download::validate_links(&args.nxm_links, &installing)?;
    if args.download {
        download::download(
            &cache,
            cache_use,
            &installing,
            &mut archives,
            &mut digests,
            &args.nxm_links,
            json,
            progress,
        )?;
        dependencies =
            dependency_plan::plan(&order, &archives, &digests, state.existing(), &installation)?;
    }
    let conflicts = dependency_plan::conflicts(
        &installing,
        catalog,
        &digests,
        state.existing(),
        &installation,
    )?;
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
            digests,
            catalog,
            cache: &cache,
            cache_use,
            game_id,
            installation,
        };
        return stack_install::run(stack, state, args, json, progress);
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
            let source = dependency_plan::acquisition(&selected.package.source);
            let download = format!("{reference} {}; {source}", selected.package.version);
            if !install_consent::confirm_download(
                &[download],
                &mut io::stdin().lock(),
                &mut io::stderr(),
            )? {
                bail!("{reference}: download cancelled");
            }
        }
        download::download(
            &cache,
            cache_use,
            &[&selected],
            &mut archives,
            &mut digests,
            &args.nxm_links,
            json,
            progress,
        )?;
    }
    let Some(archive) = archives.get(&reference).map(PathBuf::as_path) else {
        let needed = Capabilities::new(
            selected.package.native_injector,
            selected.package.executables.iter().cloned(),
        );
        let check =
            install_consent::ApprovalCheck::new(reference_key.clone(), needed, stored, &flags);
        return dependency_plan::report(
            &reference,
            &selected,
            None,
            &dependencies,
            &conflicts,
            Some(&check),
            json,
        );
    };
    let preview = |approved: &[String]| -> Result<InstallPlan> {
        let plan = match content_root_appdata(&game_id)? {
            Some(appdata) => {
                preview_install_zip_with_appdata(&game_id, &installation, archive, &appdata)?
            }
            None => {
                preview_install_zip_with_permissions(&game_id, &installation, archive, approved)?
            }
        };
        if Some(plan.archive_sha256.as_str()) != download::digest(&selected, &digests) {
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
    let check =
        install_consent::ApprovalCheck::new(reference_key.clone(), needed.clone(), stored, &flags);
    let pending = check.uncovered.executables.clone();
    let mut approved = check.approved_executables();
    let mut plan = preview(&approved)?;
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
            Some(&check),
            json,
        );
    }
    if !check.uncovered.is_empty() && (!interactive || check.uncovered.native_injector) {
        return check.ensure_approved();
    }
    if interactive && !pending.is_empty() {
        approved.extend(pending.iter().cloned());
        plan = preview(&approved)?;
    }
    if plan.status != PlanStatus::Reviewable {
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
    store.set_control(progress.control().clone());
    let package = store.install_catalog_release_with_permissions(
        &installation,
        archive,
        &plan,
        &release,
        policy,
        &approved,
    )?;
    install_consent::record(&mut store, &installation, &reference_key, &needed)?;
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
    progress: &Progress,
) -> Result<()> {
    let root = state_path
        .parent()
        .context("invalid manager state directory")?;
    let catalog = BucketStore::new(root).snapshot(game_filter)?;
    let package = resolve(&args.target, &catalog, json)?;
    install_mod(package, &catalog, args, state_path, json, progress)
}
