//! Stage one verified catalog module inside private storage.

use super::super::loader::{self, VerifiedModule};
use super::super::registry::{layout, ImportReport, ModulePin, ModuleRegistry, ReleaseOutcome};
use super::StagedRelease;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;

#[expect(
    clippy::too_many_arguments,
    reason = "Staging keeps one explicit state"
)]
pub(super) fn stage_one(
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
    if manifest.entrypoint != "module.lua" {
        bail!("catalog game module {game} must use entrypoint module.lua");
    }
    let root = source.canonicalize()?;
    let manifest_bytes = loader::bounded_file(
        &source.join("module.json"),
        &root,
        loader::MAX_MANIFEST_BYTES,
    )?;
    let blake3 = layout::digest(&manifest_bytes, script.as_bytes());
    let directory = registry.directory(game)?;
    let current = layout::open_game(&directory, game)?;
    if let Some(active) = current.as_deref() {
        layout::read_release(&directory, game, active)?;
    }
    let version_directory = layout::release_directory(&directory, &manifest.version);
    match fs::symlink_metadata(&version_directory) {
        Ok(_) => {
            let existing = layout::read_release(&directory, game, &manifest.version)?;
            if existing.blake3 == blake3 {
                reports.push(report(&manifest, blake3, ReleaseOutcome::Unchanged));
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
        reports.push(report(&manifest, blake3, ReleaseOutcome::Pinned));
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
        blake3: blake3.clone(),
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
        blake3,
        directory: staged_directory,
        newer,
        current,
    });
    Ok(())
}

fn report(manifest: &VerifiedModule, blake3: String, outcome: ReleaseOutcome) -> ImportReport {
    ImportReport {
        game_id: manifest.game_id.clone(),
        version: manifest.version.clone(),
        blake3,
        outcome,
    }
}
