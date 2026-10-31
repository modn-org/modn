//! Unified game registration and manifest-verified local mod installation.
use crate::selection::{choose_candidate, selected_installation};
use crate::state_access::validate_state_path;
use anyhow::{bail, Context, Result};
use modn_core::archive::inspect;
use modn_core::buckets::{ArchiveFormat, BucketPackage, BucketStore, PackageSource};
use modn_core::game_modules::available_games;
use modn_core::inventory::scan_with_ownership_for;
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

enum Target {
    Game(String),
    Mod(Box<BucketPackage>),
}

impl Target {
    fn description(&self) -> String {
        match self {
            Self::Game(game) => format!("game:{game}"),
            Self::Mod(package) => format!(
                "{}/{}/{}",
                package.bucket, package.package.game, package.package.id
            ),
        }
    }
}

fn resolve(
    query: &str,
    catalog: &BucketStore,
    game_filter: Option<&str>,
    module_source: Option<&Path>,
    json: bool,
) -> Result<Target> {
    if query.is_empty() || query.split('/').any(|part| part.is_empty()) {
        bail!("invalid add target; expected game, mod, game/mod or bucket/game/mod");
    }
    // The game: prefix is also used in ambiguity candidates, so every
    // suggested choice can be passed back to a noninteractive invocation.
    if let Some(game) = query.strip_prefix("game:") {
        if !game.is_empty()
            && !game.contains('/')
            && game_filter.is_none_or(|filter| filter == game)
            && available_games(module_source)?.iter().any(|id| id == game)
        {
            return Ok(Target::Game(game.into()));
        }
        bail!("unknown supported game: {game}");
    }
    let parts: Vec<&str> = query.split('/').collect();
    if parts.len() > 3 {
        bail!("invalid add target; expected game, mod, game/mod or bucket/game/mod");
    }
    let packages = catalog.packages(game_filter)?;
    let mut candidates = Vec::new();
    if parts.len() == 1 {
        for game in available_games(module_source)? {
            if game == query && game_filter.is_none_or(|filter| filter == game) {
                candidates.push(Target::Game(game));
            }
        }
    }
    for package in packages {
        let matches = match parts.as_slice() {
            [id] => package.package.id == *id,
            [game, id] => package.package.game == *game && package.package.id == *id,
            [bucket, game, id] => {
                package.bucket == *bucket
                    && package.package.game == *game
                    && package.package.id == *id
            }
            _ => false,
        };
        if matches {
            candidates.push(Target::Mod(Box::new(package)));
        }
    }
    match candidates.len() {
        0 => bail!("unknown supported game or catalog mod: {query}"),
        1 => Ok(candidates.remove(0)),
        _ if json || !io::stdin().is_terminal() || !io::stderr().is_terminal() => {
            Err(AmbiguousTarget {
                query: query.into(),
                candidates: candidates.iter().map(Target::description).collect(),
                json,
            }
            .into())
        }
        _ => {
            eprintln!("Multiple matches for {query:?}. Which one do you mean?");
            for (index, candidate) in candidates.iter().enumerate() {
                eprintln!("  {}. {}", index + 1, candidate.description());
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

fn add_game(game: &str, path: Option<&Path>, state_path: &Path, json: bool) -> Result<()> {
    let candidate = choose_candidate(game, path.map(Path::to_path_buf))?;
    let existing = existing_store(state_path)?;
    let roots = registered_roots(existing.as_ref())?;
    validate_state_path(state_path, Some(game), Some(&candidate.path), &roots)?;
    let store = Store::open(state_path)?;
    let entry = store.add_game(game, &candidate.path, &candidate.source)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&entry)?);
    } else {
        println!("Registered {} ({})", entry.path.display(), entry.source);
    }
    Ok(())
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

fn add_mod(
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
            bail!("{reference}: {source}, then use modn add {reference} --file <archive> --dry-run or --yes (automatic downloads are not implemented)");
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

#[expect(
    clippy::too_many_arguments,
    reason = "Pass the explicit CLI options without introducing persistent configuration"
)]
pub(super) fn run(
    target: &str,
    path: Option<&Path>,
    file: Option<&Path>,
    yes: bool,
    dry_run: bool,
    state_path: &Path,
    game_filter: Option<&str>,
    module_source: Option<&Path>,
    json: bool,
) -> Result<()> {
    let root = state_path
        .parent()
        .context("invalid manager state directory")?;
    let catalog = BucketStore::new(root);
    match resolve(target, &catalog, game_filter, module_source, json)? {
        Target::Game(game) => {
            if file.is_some() || yes || dry_run {
                bail!("game registration does not accept --file, --yes, or --dry-run");
            }
            add_game(&game, path, state_path, json)
        }
        Target::Mod(package) => {
            if module_source.is_some() {
                bail!("--src is only supported for game registration");
            }
            add_mod(*package, file, path, yes, dry_run, state_path, json)
        }
    }
}
