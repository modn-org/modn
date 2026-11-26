use super::registry::{activation, ImportReport, ModuleRegistry, ModuleSource, ReleaseOutcome};
use crate::buckets::BucketStore;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

#[cfg(test)]
pub(crate) static FAIL_NEXT_CATALOG_COMMIT: std::sync::Mutex<BTreeSet<PathBuf>> =
    std::sync::Mutex::new(BTreeSet::new());

#[cfg(test)]
static STOP_NEXT_AFTER_MOVES: std::sync::Mutex<BTreeSet<PathBuf>> =
    std::sync::Mutex::new(BTreeSet::new());

#[cfg(test)]
fn injected_commit_failure(root: &Path) -> bool {
    let mut armed = FAIL_NEXT_CATALOG_COMMIT.lock().expect("seam lock");
    armed.remove(root)
}

#[cfg(test)]
fn injected_stop_after_moves(root: &Path) -> bool {
    let mut armed = STOP_NEXT_AFTER_MOVES.lock().expect("seam lock");
    armed.remove(root)
}

mod catalog;
mod staging;

pub(crate) use catalog::ignored_catalog_modules;

use catalog::module_directories;
use staging::{stage_one, StagedRelease};

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

/// Stage every module in one trusted catalog without changing active releases.
///
/// The catalog signature must already be verified by the caller. Each module is
/// copied and executed only inside private staging storage.
pub fn stage_catalog_modules(
    roots: &crate::manager_roots::ManagerRoots,
    bucket: &str,
    signer_key_id: &str,
    catalog: &Path,
) -> Result<StagedCatalogImport> {
    stage_catalog_modules_at(roots, bucket, signer_key_id, catalog)
}

pub(crate) fn stage_catalog_modules_at(
    roots: &crate::manager_roots::ManagerRoots,
    bucket: &str,
    signer_key_id: &str,
    catalog: &Path,
) -> Result<StagedCatalogImport> {
    if bucket.is_empty() || signer_key_id.is_empty() {
        bail!("catalog module source must name a bucket and signer");
    }
    let registry = ModuleRegistry::new(roots);
    let (lock, _repair) = registry.lock_and_repair()?;
    let staging = tempfile::Builder::new()
        .prefix(".staging-catalog-")
        .tempdir_in(registry.root())?;
    let candidates = module_directories(catalog)?;
    let source = ModuleSource {
        bucket: Some(bucket.to_owned()),
        signer_key_id: Some(signer_key_id.to_owned()),
    };
    let mut staged = Vec::new();
    let mut reports = Vec::new();
    let mut ignored = Vec::new();
    let mut seen = BTreeSet::new();
    let official = if candidates.is_empty() {
        Vec::new()
    } else {
        BucketStore::new(roots).official_games()?
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
            &source,
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
    /// in use. A failure during this call moves every version directory and
    /// restores every replaced `current` pointer before returning the error.
    pub fn commit(self) -> Result<Vec<ImportReport>> {
        let StagedCatalogImport {
            registry,
            _lock,
            staging,
            staged,
            mut reports,
            ignored: _,
        } = self;
        #[cfg(test)]
        if injected_commit_failure(&registry.root()) {
            drop(staging);
            bail!("injected catalog module commit failure");
        }
        let mut progress = activation::ActivationProgress::default();
        let result = apply(&registry, &staged, &mut progress, &mut reports);
        if let Err(error) = result {
            return match activation::undo_activation(&registry.root(), &progress) {
                Ok(()) => Err(error.context(
                    "catalog module commit failed; the module registry was restored",
                )),
                Err(restore_error) => Err(error.context(format!(
                    "catalog module commit failed and the module registry could not be fully restored: {restore_error:#}"
                ))),
            };
        }
        drop(staging);
        reports.sort_by(|left, right| left.game_id.cmp(&right.game_id));
        Ok(reports)
    }
}

fn apply(
    registry: &ModuleRegistry,
    staged: &[StagedRelease],
    progress: &mut activation::ActivationProgress,
    reports: &mut Vec<ImportReport>,
) -> Result<()> {
    let root = registry.root();
    let releases = staged
        .iter()
        .map(|release| {
            let staged = release.directory.strip_prefix(&root).with_context(|| {
                format!(
                    "staged game module {} is outside the registry",
                    release.game_id
                )
            })?;
            Ok(activation::ActivationRelease {
                game_id: release.game_id.clone(),
                version: release.version.clone(),
                blake3: release.blake3.clone(),
                previous: release.current.clone(),
                staged: staged.to_path_buf(),
                newer: release.newer,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    activation::write_intent(&root, &releases)?;
    activation::move_releases(&root, &releases, progress)?;
    #[cfg(test)]
    if injected_stop_after_moves(&root) {
        return Ok(());
    }
    activation::write_current_pointers(&root, &releases, progress)?;
    activation::finish_activation(&root)?;
    for release in staged {
        reports.push(ImportReport {
            game_id: release.game_id.clone(),
            version: release.version.clone(),
            blake3: release.blake3.clone(),
            outcome: if release.newer {
                ReleaseOutcome::Imported
            } else {
                ReleaseOutcome::NotNewer
            },
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
