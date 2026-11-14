use super::{bounded_json, MODULE_MANIFEST, MODULE_SCRIPT};
use crate::buckets::manifest::{valid_id, PackageManifest};
use crate::buckets::trust::ModuleTrustState;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;

const MAX_PACKAGES: usize = 1024;

#[derive(Default)]
pub(super) struct Releases {
    pub(super) current: Vec<PackageManifest>,
    pub(super) other: Vec<PackageManifest>,
}

fn real_directory(path: &Path, root: &Path) -> Result<()> {
    if fs::symlink_metadata(path)?.file_type().is_symlink()
        || !path.is_dir()
        || !path.canonicalize()?.starts_with(root)
    {
        bail!("unsafe bucket directory {}", path.display());
    }
    Ok(())
}

/// Validate `games/` for buckets that may publish game modules.
///
/// `main` and signed catalogs the user trusts for modules may hold only
/// `games/<game-id>/` module manifests and scripts. Every other bucket ignores
/// the folder without reading or validating it.
fn validate_games(root: &Path, canonical: &Path, trust: ModuleTrustState) -> Result<()> {
    if !matches!(trust, ModuleTrustState::Main | ModuleTrustState::Trusted) {
        return Ok(());
    }
    let games = root.join("games");
    match fs::symlink_metadata(&games) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    real_directory(&games, canonical)?;
    for game in fs::read_dir(&games)? {
        let game = game?;
        let game_id = game.file_name().to_string_lossy().into_owned();
        if !valid_id(&game_id) {
            bail!("invalid game directory name");
        }
        if !game.file_type()?.is_dir() {
            bail!(
                "games/ may hold only game module directories: {}",
                game.path().display()
            );
        }
        real_directory(&game.path(), canonical)?;
        let mut has_script = false;
        for entry in fs::read_dir(game.path())? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if !kind.is_file() {
                bail!("games/{game_id}/ may hold only {MODULE_MANIFEST} and {MODULE_SCRIPT}");
            }
            match entry.file_name().to_str() {
                Some(MODULE_SCRIPT) => has_script = true,
                Some(MODULE_MANIFEST) => {}
                _ => bail!("games/{game_id}/ may hold only {MODULE_MANIFEST} and {MODULE_SCRIPT}"),
            }
        }
        if trust == ModuleTrustState::Main {
            let info = crate::game_modules::check_module(&game.path())?;
            if info.game_id != game_id || !has_script {
                bail!("official game module identity or {MODULE_SCRIPT} mismatch: {game_id}");
            }
            let manifest: serde_json::Value = bounded_json(&game.path().join(MODULE_MANIFEST))?;
            if manifest["entrypoint"] != MODULE_SCRIPT {
                bail!("official game module entrypoint must be {MODULE_SCRIPT}");
            }
        }
    }
    Ok(())
}

/// Read `mods/<game-id>/<mod-id>.json` and `mods/<game-id>/<mod-id>@<version>.json`.
fn read_releases(root: &Path, canonical: &Path) -> Result<Releases> {
    let mods = root.join("mods");
    match fs::symlink_metadata(&mods) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Releases::default())
        }
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    real_directory(&mods, canonical)?;
    let mut releases = Releases::default();
    let mut count = 0;
    for game in fs::read_dir(&mods)? {
        let game = game?;
        let game_id = game.file_name().to_string_lossy().into_owned();
        if !valid_id(&game_id) {
            bail!("invalid game directory name");
        }
        if !game.file_type()?.is_dir() {
            bail!("unexpected bucket package entry");
        }
        real_directory(&game.path(), canonical)?;
        for entry in fs::read_dir(game.path())? {
            let path = entry?.path();
            let kind = fs::symlink_metadata(&path)?.file_type();
            if kind.is_symlink() || path.extension().is_none_or(|e| e != "json") {
                bail!("unexpected bucket package entry");
            }
            if !kind.is_file() {
                bail!(
                    "bucket package entry must be a regular file: {}",
                    path.display()
                );
            }
            let stem = path
                .file_stem()
                .context("invalid package path")?
                .to_string_lossy()
                .into_owned();
            let (id, version) = match stem.split_once('@') {
                Some((id, version)) => (id, Some(version)),
                None => (stem.as_str(), None),
            };
            if !valid_id(id) {
                bail!("invalid package filename");
            }
            count += 1;
            if count > MAX_PACKAGES {
                bail!("bucket package limit exceeded");
            }
            let manifest: PackageManifest = bounded_json(&path)?;
            manifest.validate(&game_id, id)?;
            match version {
                None => releases.current.push(manifest),
                Some(version) if version == manifest.version => releases.other.push(manifest),
                Some(_) => bail!("release filename disagrees with its version: {stem}"),
            }
        }
    }
    let key = |p: &PackageManifest| (p.game.clone(), p.id.clone());
    releases.current.sort_by_key(key);
    releases
        .other
        .sort_by_key(|p| (p.game.clone(), p.id.clone(), p.version.clone()));
    for other in &releases.other {
        let current = releases
            .current
            .iter()
            .find(|current| key(current) == key(other))
            .with_context(|| {
                format!(
                    "release {}@{} has no current release",
                    other.id, other.version
                )
            })?;
        if current.version == other.version {
            bail!("duplicate release {}@{}", other.id, other.version);
        }
    }
    Ok(releases)
}

pub(super) fn read_packages(root: &Path, trust: ModuleTrustState) -> Result<Releases> {
    let canonical = root.canonicalize()?;
    validate_games(root, &canonical, trust)?;
    read_releases(root, &canonical)
}
