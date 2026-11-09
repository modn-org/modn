use super::bounded_json;
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

pub(super) fn read_packages(root: &Path, trust: ModuleTrustState) -> Result<Releases> {
    let canonical = root.canonicalize()?;
    let games = root.join("games");
    if !games.exists() {
        match fs::symlink_metadata(&games) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Releases::default())
            }
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
    }
    real_directory(&games, &canonical)?;
    let mut releases = Releases::default();
    let mut count = 0;
    for game in fs::read_dir(&games)? {
        let game = game?;
        let game_id = game.file_name().to_string_lossy().into_owned();
        if !valid_id(&game_id) {
            bail!("invalid game directory name");
        }
        real_directory(&game.path(), &canonical)?;
        let module = game.path().join("module.json");
        let script = game.path().join("module.lua");
        let has_module = fs::symlink_metadata(&module).is_ok();
        let has_script = fs::symlink_metadata(&script).is_ok();
        if has_module || has_script {
            match trust {
                ModuleTrustState::Main => {
                    let info = crate::game_modules::check_module(&game.path())?;
                    if info.game_id != game_id || !has_script {
                        bail!("official game module identity or module.lua mismatch: {game_id}");
                    }
                    let manifest: serde_json::Value = bounded_json(&module)?;
                    if manifest["entrypoint"] != "module.lua" {
                        bail!("official game module entrypoint must be module.lua");
                    }
                }
                ModuleTrustState::Trusted | ModuleTrustState::Untrusted => {
                    // Third-party module files are never packages and are not
                    // parsed or executed while listing or searching a bucket.
                }
                ModuleTrustState::Local => bail!(
                    "only the official main bucket may contain game modules: {}",
                    game.path().display()
                ),
            }
        }
        for entry in fs::read_dir(game.path())? {
            let path = entry?.path();
            if matches!(
                path.file_name().and_then(|n| n.to_str()),
                Some("module.json" | "module.lua")
            ) {
                continue;
            }
            if path.extension().is_none_or(|e| e != "json") {
                bail!("unexpected bucket package entry");
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
