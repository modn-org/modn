use super::super::loader;
use super::super::registry::admission::{admit, Admission, Candidate, RegistryState};
use super::super::registry::{ImportReport, ModuleRegistry, ModuleSource, ReleaseOutcome};
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

/// One checked catalog release waiting for the catalog publication to succeed.
pub(super) struct StagedRelease {
    pub(super) game_id: String,
    pub(super) version: String,
    pub(super) blake3: String,
    pub(super) directory: PathBuf,
    pub(super) newer: bool,
    pub(super) current: Option<String>,
}

pub(super) fn stage_one(
    registry: &ModuleRegistry,
    staging: &Path,
    source: &ModuleSource,
    game: &str,
    directory: &Path,
    staged: &mut Vec<StagedRelease>,
    reports: &mut Vec<ImportReport>,
) -> Result<()> {
    if !directory.join("module.json").is_file() || !directory.join("module.lua").is_file() {
        bail!("game module {game} must contain module.json and module.lua");
    }
    let (manifest, script) = loader::read_verified_module(directory)?;
    if manifest.game_id != game {
        bail!(
            "catalog directory {game} declares game_id {}",
            manifest.game_id
        );
    }
    if manifest.entrypoint != "module.lua" {
        bail!("catalog game module {game} must use entrypoint module.lua");
    }
    let root = directory.canonicalize()?;
    let manifest_bytes = loader::bounded_file(
        &directory.join("module.json"),
        &root,
        loader::MAX_MANIFEST_BYTES,
    )?;
    let registry_directory = registry.directory(game)?;
    let staged_directory = staging.join(game);
    let admission = admit(Candidate {
        directory: &registry_directory,
        manifest: &manifest,
        script: &script,
        manifest_bytes: &manifest_bytes,
        source,
        state: RegistryState::read(&registry_directory, game)?,
        staged: &staged_directory,
    })?;
    match admission {
        Admission::Unchanged { blake3 } => {
            reports.push(report(&manifest, blake3, ReleaseOutcome::Unchanged));
        }
        Admission::Refused { blake3 } => {
            reports.push(report(&manifest, blake3, ReleaseOutcome::Pinned));
        }
        Admission::Admitted(release) => staged.push(StagedRelease {
            game_id: game.to_owned(),
            version: manifest.version.clone(),
            blake3: release.blake3,
            directory: staged_directory,
            newer: release.newer,
            current: release.current,
        }),
    }
    Ok(())
}

fn report(
    manifest: &loader::VerifiedModule,
    blake3: String,
    outcome: ReleaseOutcome,
) -> ImportReport {
    ImportReport {
        game_id: manifest.game_id.clone(),
        version: manifest.version.clone(),
        blake3,
        outcome,
    }
}
