//! Stage game modules from a trusted Git catalog before publishing it.
//!
//! Staging validates every module in a private copy and keeps the registry's
//! active releases untouched. The caller publishes the catalog and then calls
//! [`StagedCatalogImport::commit`] to switch the retained releases.

use super::registry::layout;
use super::registry::{ImportReport, ModuleRegistry, ReleaseOutcome};
use crate::buckets::BucketStore;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

mod catalog;
mod staging;

pub(crate) use catalog::ignored_catalog_modules;

use catalog::module_directories;
use staging::stage_one;

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
    blake3: String,
    directory: PathBuf,
    newer: bool,
    current: Option<String>,
}

/// One version directory moved out of staging, and how to undo the move.
struct MovedRelease {
    staged: PathBuf,
    target: PathBuf,
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
        &crate::data_dir::profile_directory()?,
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
        let mut moved = Vec::with_capacity(staged.len());
        let mut switched = Vec::new();
        let mut created = Vec::new();
        let result = apply(
            &registry,
            &staged,
            &mut moved,
            &mut switched,
            &mut created,
            &mut reports,
        );
        if let Err(error) = result {
            return match restore(&moved, &switched, &created) {
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
    moved: &mut Vec<MovedRelease>,
    switched: &mut Vec<(PathBuf, String)>,
    created: &mut Vec<PathBuf>,
    reports: &mut Vec<ImportReport>,
) -> Result<()> {
    for release in staged {
        let directory = registry.directory(&release.game_id)?;
        if release.current.is_none() {
            created.push(directory.clone());
        }
        fs::create_dir_all(directory.join(layout::VERSIONS))?;
        let target = layout::release_directory(&directory, &release.version);
        fs::rename(&release.directory, &target).with_context(|| {
            format!(
                "cannot activate game module {} version {}",
                release.game_id, release.version
            )
        })?;
        moved.push(MovedRelease {
            staged: release.directory.clone(),
            target,
        });
    }
    for release in staged {
        if release.newer {
            let directory = registry.directory(&release.game_id)?;
            layout::write_current(&directory, &release.version).with_context(|| {
                format!(
                    "cannot switch game module {} to version {}",
                    release.game_id, release.version
                )
            })?;
            if let Some(previous) = &release.current {
                switched.push((directory, previous.clone()));
            }
        }
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

/// Undo one partial commit: restore active pointers first, then move version
/// directories back to their staging area. A game entry created by this commit
/// had no directory before it, so restoring the entry means removing it.
/// Returns every restoration failure.
fn restore(
    moved: &[MovedRelease],
    switched: &[(PathBuf, String)],
    created: &[PathBuf],
) -> Result<()> {
    let mut failures = Vec::new();
    for (directory, previous) in switched.iter().rev() {
        if let Err(error) = layout::write_current(directory, previous) {
            failures.push(format!(
                "cannot restore the active release in {}: {error:#}",
                directory.display()
            ));
        }
    }
    for release in moved.iter().rev() {
        if let Err(error) = fs::rename(&release.target, &release.staged)
            .or_else(|_| fs::remove_dir_all(&release.target))
        {
            failures.push(format!(
                "cannot remove the partial release {}: {error:#}",
                release.target.display()
            ));
        }
    }
    for directory in created.iter().rev() {
        match fs::remove_dir_all(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => failures.push(format!(
                "cannot remove the new module entry {}: {error:#}",
                directory.display()
            )),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        bail!("{}", failures.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moved_release(root: &Path, game: &str, version: &str) -> MovedRelease {
        let staged = root.join("staging").join(game);
        fs::create_dir_all(&staged).expect("staging directory");
        fs::write(staged.join("module.json"), b"{}").expect("staged manifest");
        let game_directory = root.join("game-modules").join(game);
        fs::create_dir_all(game_directory.join(layout::VERSIONS)).expect("versions directory");
        let target = layout::release_directory(&game_directory, version);
        fs::rename(&staged, &target).expect("move staged release");
        MovedRelease { staged, target }
    }

    #[test]
    fn restore_undoes_partial_moves_and_created_entries() {
        let root = tempfile::tempdir().expect("root");
        let new_release = moved_release(root.path(), "new-game", "2.0.0");
        let new_directory = root.path().join("game-modules/new-game");
        let old_release = moved_release(root.path(), "old-game", "2.0.0");
        let old_directory = root.path().join("game-modules/old-game");
        layout::write_current(&old_directory, "2.0.0").expect("simulated switch");
        let old_target = old_release.target.clone();
        let old_staged = old_release.staged.clone();

        restore(
            &[new_release, old_release],
            &[(old_directory.clone(), "1.0.0".to_owned())],
            std::slice::from_ref(&new_directory),
        )
        .expect("restore");

        assert!(fs::symlink_metadata(&new_directory).is_err());
        assert_eq!(
            layout::read_current(&old_directory).expect("restored current"),
            "1.0.0"
        );
        assert!(fs::symlink_metadata(&old_target).is_err());
        assert!(old_staged.join("module.json").is_file());
    }
}
