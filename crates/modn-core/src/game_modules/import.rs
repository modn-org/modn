//! Stage game modules from a trusted Git catalog before publishing it.
//!
//! Staging validates every module in a private copy and keeps the registry's
//! active releases untouched. The caller publishes the catalog and then calls
//! [`StagedCatalogImport::commit`] to switch the retained releases.

use super::loader::{self, VerifiedModule};
use super::registry::layout;
use super::registry::{ImportReport, ModulePin, ModuleRegistry, ReleaseOutcome};
use crate::buckets::BucketStore;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Why one catalog module was not imported.
#[expect(clippy::exhaustive_enums, reason = "Ignore reasons are a closed set")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleIgnoreReason {
    /// The catalog is not a trusted module source.
    UntrustedSource,
    /// The official main bucket already provides the game.
    OfficialPriority,
}

/// One catalog module that sync did not import.
#[expect(
    clippy::exhaustive_structs,
    reason = "Reports are plain serialized records"
)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct IgnoredModule {
    /// Game identity declared by the catalog directory.
    pub game_id: String,
    /// Why the module was ignored.
    pub reason: ModuleIgnoreReason,
}

/// Module outcomes reported by one Git catalog sync.
#[expect(
    clippy::exhaustive_structs,
    reason = "Reports are plain serialized records"
)]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CatalogModuleReport {
    /// Releases inspected or imported during the sync.
    pub imported: Vec<ImportReport>,
    /// Modules that were skipped without being imported.
    pub ignored: Vec<IgnoredModule>,
}

/// Validated catalog modules waiting for the catalog publication to succeed.
pub struct StagedCatalogImport {
    registry: ModuleRegistry,
    _lock: fs::File,
    staging: tempfile::TempDir,
    staged: Vec<StagedRelease>,
    reports: Vec<ImportReport>,
    ignored: Vec<IgnoredModule>,
}

struct StagedRelease {
    game_id: String,
    version: String,
    sha256: String,
    directory: PathBuf,
    newer: bool,
}

/// Stage every module in one trusted catalog without changing active releases.
///
/// The catalog signature must already be verified by the caller. Each module is
/// copied and executed only inside private staging storage.
pub fn stage_catalog_modules(
    bucket: &str,
    signer_key_id: &str,
    catalog: &Path,
) -> Result<StagedCatalogImport> {
    stage_catalog_modules_at(
        &crate::data_dir::data_directory()?,
        bucket,
        signer_key_id,
        catalog,
    )
}

pub(crate) fn stage_catalog_modules_at(
    data: &Path,
    bucket: &str,
    signer_key_id: &str,
    catalog: &Path,
) -> Result<StagedCatalogImport> {
    if bucket.is_empty() || signer_key_id.is_empty() {
        bail!("catalog module source must name a bucket and signer");
    }
    let registry = ModuleRegistry::new(data);
    let lock = registry.lock()?;
    let staging = tempfile::Builder::new()
        .prefix(".staging-catalog-")
        .tempdir_in(registry.root())?;
    let candidates = module_directories(catalog)?;
    let mut staged = Vec::new();
    let mut reports = Vec::new();
    let mut ignored = Vec::new();
    let mut seen = BTreeSet::new();
    let official = if candidates.is_empty() {
        Vec::new()
    } else {
        BucketStore::new(data).official_games()?
    };
    for (game, directory) in candidates {
        if official.contains(&game) {
            ignored.push(IgnoredModule {
                game_id: game,
                reason: ModuleIgnoreReason::OfficialPriority,
            });
            continue;
        }
        if !seen.insert(game.clone()) {
            bail!("catalog declares game module {game} more than once");
        }
        stage_one(
            &registry,
            staging.path(),
            bucket,
            signer_key_id,
            &game,
            &directory,
            &mut staged,
            &mut reports,
        )
        .with_context(|| format!("cannot import game module {game} from bucket {bucket}"))?;
    }
    Ok(StagedCatalogImport {
        registry,
        _lock: lock,
        staging,
        staged,
        reports,
        ignored,
    })
}

impl StagedCatalogImport {
    /// Modules skipped because the official main bucket takes priority.
    #[must_use]
    pub fn ignored(&self) -> &[IgnoredModule] {
        &self.ignored
    }

    /// Switch every staged release into the registry.
    ///
    /// All version directories are moved first, then the `current` pointers are
    /// replaced. A failure before this call leaves every previous active release
    /// in use.
    pub fn commit(self) -> Result<Vec<ImportReport>> {
        let StagedCatalogImport {
            registry,
            _lock,
            staging,
            staged,
            mut reports,
            ignored: _,
        } = self;
        for release in &staged {
            let directory = registry.directory(&release.game_id)?;
            fs::create_dir_all(directory.join(layout::VERSIONS))?;
            let target = layout::release_directory(&directory, &release.version);
            fs::rename(&release.directory, &target).with_context(|| {
                format!(
                    "cannot activate game module {} version {}",
                    release.game_id, release.version
                )
            })?;
        }
        for release in &staged {
            if release.newer {
                let directory = registry.directory(&release.game_id)?;
                layout::write_current(&directory, &release.version)?;
            }
            reports.push(ImportReport {
                game_id: release.game_id.clone(),
                version: release.version.clone(),
                sha256: release.sha256.clone(),
                outcome: if release.newer {
                    ReleaseOutcome::Imported
                } else {
                    ReleaseOutcome::NotNewer
                },
            });
        }
        drop(staging);
        reports.sort_by(|left, right| left.game_id.cmp(&right.game_id));
        Ok(reports)
    }
}

fn module_directories(catalog: &Path) -> Result<Vec<(String, PathBuf)>> {
    let games = catalog.join("games");
    match fs::symlink_metadata(&games) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        result => {
            let metadata = result?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                bail!("catalog games directory must be a real directory");
            }
        }
    }
    let mut candidates = Vec::new();
    for entry in fs::read_dir(&games)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            bail!("catalog games entry must be a real directory");
        }
        let game = entry.file_name().to_string_lossy().into_owned();
        let directory = entry.path();
        let has_module = fs::symlink_metadata(directory.join("module.json")).is_ok();
        let has_script = fs::symlink_metadata(directory.join("module.lua")).is_ok();
        if !has_module && !has_script {
            continue;
        }
        if !loader::valid_game_id(&game) {
            bail!("invalid catalog game module identifier: {game}");
        }
        candidates.push((game, directory));
    }
    candidates.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(candidates)
}

#[expect(
    clippy::too_many_arguments,
    reason = "Staging keeps one explicit state"
)]
fn stage_one(
    registry: &ModuleRegistry,
    staging: &Path,
    bucket: &str,
    signer_key_id: &str,
    game: &str,
    source: &Path,
    staged: &mut Vec<StagedRelease>,
    reports: &mut Vec<ImportReport>,
) -> Result<()> {
    if !source.join("module.json").is_file() || !source.join("module.lua").is_file() {
        bail!("game module {game} must contain module.json and module.lua");
    }
    let (manifest, script) = loader::read_verified_module(source)?;
    if manifest.game_id != game {
        bail!(
            "catalog directory {game} declares game_id {}",
            manifest.game_id
        );
    }
    let root = source.canonicalize()?;
    let manifest_bytes = loader::bounded_file(
        &source.join("module.json"),
        &root,
        loader::MAX_MANIFEST_BYTES,
    )?;
    let sha256 = layout::digest(&manifest_bytes, script.as_bytes());
    let directory = registry.directory(game)?;
    let current = layout::open_game(&directory, game)?;
    if let Some(active) = current.as_deref() {
        layout::read_release(&directory, game, active)?;
    }
    let version_directory = layout::release_directory(&directory, &manifest.version);
    match fs::symlink_metadata(&version_directory) {
        Ok(_) => {
            let existing = layout::read_release(&directory, game, &manifest.version)?;
            if existing.sha256 == sha256 {
                reports.push(report(&manifest, sha256, ReleaseOutcome::Unchanged));
                return Ok(());
            }
            bail!(
                "module {game} version {} is already retained with a different digest; remove and register the module again",
                manifest.version
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if ModulePin::read(&directory, game)?.is_some() {
        reports.push(report(&manifest, sha256, ReleaseOutcome::Pinned));
        return Ok(());
    }
    let staged_directory = staging.join(game);
    fs::create_dir(&staged_directory)?;
    fs::write(staged_directory.join("module.json"), &manifest_bytes)?;
    fs::write(
        staged_directory.join(&manifest.entrypoint),
        script.as_bytes(),
    )?;
    let checked = loader::check_module(&staged_directory).with_context(|| {
        format!(
            "game module {game} version {} failed staging validation",
            manifest.version
        )
    })?;
    if checked.game_id != manifest.game_id
        || checked.version != manifest.version
        || checked.api_version != manifest.api_version
    {
        bail!("game module {game} identity changed during staging");
    }
    let receipt = layout::Receipt {
        game_id: checked.game_id,
        version: checked.version,
        api_version: checked.api_version,
        sha256: sha256.clone(),
        bucket: Some(bucket.to_owned()),
        signer_key_id: Some(signer_key_id.to_owned()),
    };
    fs::write(
        staged_directory.join(layout::RECEIPT),
        serde_json::to_vec(&receipt)?,
    )?;
    let newer = current
        .as_deref()
        .is_none_or(|active| layout::compare_versions(&manifest.version, active).is_gt());
    staged.push(StagedRelease {
        game_id: game.to_owned(),
        version: manifest.version.clone(),
        sha256,
        directory: staged_directory,
        newer,
    });
    Ok(())
}

fn report(manifest: &VerifiedModule, sha256: String, outcome: ReleaseOutcome) -> ImportReport {
    ImportReport {
        game_id: manifest.game_id.clone(),
        version: manifest.version.clone(),
        sha256,
        outcome,
    }
}
