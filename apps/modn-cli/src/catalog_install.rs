//! Catalog resolution and manifest-verified local mod installation.
use crate::selection::selected_installation;
use crate::state_access::validate_state_path;
use anyhow::{bail, Context, Result};
use modn_core::archive::inspect;
use modn_core::buckets::{ArchiveFormat, BucketPackage, BucketStore, PackageSource};
use modn_core::inventory::{original_package_payloads_intact, scan_with_ownership_for};
use modn_core::planning::preview_install_zip_with_permissions;
use modn_core::state::Store;
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

fn existing_store(state_path: &Path) -> Result<Option<Store>> {
    state_path
        .is_file()
        .then(|| Store::open_readonly(state_path))
        .transpose()
}

fn registered_roots(store: Option<&Store>) -> Result<Vec<PathBuf>> {
    Ok(store
        .map(Store::list)
        .transpose()?
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| entry.path.canonicalize().ok())
        .collect())
}

fn expected_format(format: &ArchiveFormat) -> &'static str {
    match format {
        ArchiveFormat::Zip => "zip",
        ArchiveFormat::SevenZ => "7z",
        _ => "unsupported",
    }
}

fn dependency_check(
    selected: &BucketPackage,
    catalog: &BucketStore,
    store: Option<&Store>,
    game: &Path,
) -> Result<()> {
    if selected.package.dependencies.is_empty() {
        return Ok(());
    }
    let catalog_packages = catalog.packages(Some(&selected.package.game))?;
    let owned = store
        .map(|db| db.installed(game))
        .transpose()?
        .unwrap_or_default();
    // A ledger entry is not proof that the prerequisite is still present.
    // Reuse the exact ownership/alias checks used for the normal inventory.
    let checked = store
        .map(|db| scan_with_ownership_for(db, &selected.package.game, game))
        .transpose()?;
    for id in &selected.package.dependencies {
        let dependency = catalog_packages
            .iter()
            .find(|candidate| candidate.bucket == selected.bucket && candidate.package.id == *id)
            .with_context(|| {
                format!(
                    "dependency unavailable in bucket: {}/{}/{}",
                    selected.bucket, selected.package.game, id
                )
            })?;
        let is_intact = owned.iter().any(|package| {
            if package.archive_sha256 != dependency.package.sha256 {
                return false;
            }
            let Some(report) = checked.as_ref() else {
                return false;
            };
            let mut checks = report
                .owned_files
                .iter()
                .filter(|entry| entry.package_id == package.id);
            let first = checks.next();
            first.is_some_and(|entry| matches!(entry.status.as_str(), "intact" | "present"))
                && checks.all(|entry| matches!(entry.status.as_str(), "intact" | "present"))
                // A folder's "present" status does not prove its original files.
                && original_package_payloads_intact(game, package)
        });
        if !is_intact {
            bail!(
                "dependency not installed or intact: {}/{}/{}; restore its managed content before adding {}",
                selected.bucket, selected.package.game, id, selected.package.id
            );
        }
    }
    Ok(())
}

fn install_mod(
    selected: BucketPackage,
    file: Option<&Path>,
    path: Option<&Path>,
    yes: bool,
    dry_run: bool,
    state_path: &Path,
    json: bool,
) -> Result<()> {
    let reference = format!(
        "{}/{}/{}",
        selected.bucket, selected.package.game, selected.package.id
    );
    let archive = match file {
        Some(path) => path,
        None => {
            let source = match &selected.package.source {
                PackageSource::Manual { filename } => format!("download {filename:?} manually"),
                PackageSource::Https { url } => format!("download from {url}"),
                _ => "unsupported source".to_owned(),
            };
            bail!("{reference}: {source}, then use modn install {reference} --file <archive> --dry-run or --yes (automatic downloads are not implemented)");
        }
    };
    if !yes && !dry_run {
        bail!("installation writes game files; choose --dry-run or explicitly approve with --yes");
    }
    if !std::fs::symlink_metadata(archive)?.file_type().is_file() {
        bail!("archive must be a regular file");
    }
    let inspected = inspect(archive)?;
    if inspected.format != expected_format(&selected.package.archive_format) {
        bail!("{reference}: archive format does not match catalog manifest");
    }
    if inspected.sha256 != selected.package.sha256 {
        bail!("{reference}: archive SHA-256 does not match catalog manifest");
    }
    let existing = existing_store(state_path)?;
    let (game_id, installation) = selected_installation(
        existing.as_ref(),
        path.map(Path::to_path_buf),
        Some(&selected.package.game),
    )?;
    dependency_check(
        &selected,
        &BucketStore::new(
            state_path
                .parent()
                .context("invalid manager state directory")?,
        ),
        existing.as_ref(),
        &installation,
    )?;
    let plan = preview_install_zip_with_permissions(&game_id, &installation, archive, &[])?;
    if plan.archive_sha256 != selected.package.sha256 {
        bail!("{reference}: source changed after catalog verification");
    }
    if dry_run {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "package": reference, "plan": plan,
                }))?
            );
        } else {
            println!("Dry run for {reference}: {}", plan.status);
            for operation in &plan.operations {
                println!("{} => {}", operation.source, operation.destination);
            }
            for issue in &plan.blockers {
                println!("BLOCKER {}: {}", issue.code, issue.message);
            }
        }
        return Ok(());
    }
    if plan.status != "reviewable" {
        bail!("{reference}: installation blocked; inspect with --dry-run first");
    }
    let roots = registered_roots(existing.as_ref())?;
    validate_state_path(state_path, Some(&game_id), Some(&installation), &roots)?;
    let mut store = Store::open(state_path)?;
    let package =
        store.install_operations_zip_with_permissions(&installation, archive, &plan, &[])?;
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
#[expect(
    clippy::too_many_arguments,
    reason = "Existing CLI passes explicit installation options without persisting them"
)]
pub(super) fn run(
    target: &str,
    path: Option<&Path>,
    file: Option<&Path>,
    yes: bool,
    dry_run: bool,
    state_path: &Path,
    game_filter: Option<&str>,
    json: bool,
) -> Result<()> {
    let root = state_path
        .parent()
        .context("invalid manager state directory")?;
    let catalog = BucketStore::new(root);
    let package = resolve(target, &catalog, game_filter, json)?;
    install_mod(package, file, path, yes, dry_run, state_path, json)
}
