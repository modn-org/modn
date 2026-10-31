use super::manifest::{valid_id, PackageManifest};
use super::module::{
    read_release, valid_version, validate_release, BucketModule, ModuleReleaseManifest,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

const MAX_JSON: u64 = 32 * 1024;
const MAX_PACKAGES: usize = 1024;
const MAX_MODULES: usize = 1024;

#[expect(
    missing_docs,
    reason = "Public catalog record fields are self-describing"
)]
#[derive(Clone, Debug, Serialize)]
#[non_exhaustive]
pub struct BucketInfo {
    pub name: String,
    pub title: String,
    /// None indicates the built-in, read-only main snapshot.
    pub path: Option<PathBuf>,
}

#[expect(
    missing_docs,
    reason = "Public catalog record fields are self-describing"
)]
#[derive(Clone, Debug, Serialize)]
#[non_exhaustive]
pub struct BucketPackage {
    pub bucket: String,
    #[serde(flatten)]
    pub package: PackageManifest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    schema_version: u32,
    name: String,
    title: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    path: PathBuf,
}

/// Catalog registry, separate from game registrations and ownership state.
pub struct BucketStore {
    root: PathBuf,
}

fn bounded_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        bail!("bucket metadata must be a regular file: {}", path.display());
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_JSON + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_JSON {
        bail!("bucket metadata exceeds size limit");
    }
    serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid bucket JSON {}", path.display()))
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

fn read_packages(root: &Path) -> Result<Vec<PackageManifest>> {
    let canonical = root.canonicalize()?;
    let games = root.join("games");
    if !games.exists() {
        // A bucket may publish only game modules.
        match fs::symlink_metadata(&games) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
    }
    real_directory(&games, &canonical)?;
    let mut packages = Vec::new();
    for game in fs::read_dir(&games)? {
        let game = game?;
        let game_id = game.file_name().to_string_lossy().into_owned();
        if !valid_id(&game_id) {
            bail!("invalid game directory name");
        }
        real_directory(&game.path(), &canonical)?;
        for entry in fs::read_dir(game.path())? {
            let path = entry?.path();
            if path.extension().is_none_or(|e| e != "json") {
                bail!("unexpected bucket package entry");
            }
            let id = path
                .file_stem()
                .context("invalid package path")?
                .to_string_lossy();
            if !valid_id(&id) {
                bail!("invalid package filename");
            }
            if packages.len() >= MAX_PACKAGES {
                bail!("bucket package limit exceeded");
            }
            let manifest: PackageManifest = bounded_json(&path)?;
            manifest.validate(&game_id, &id)?;
            packages.push(manifest);
        }
    }
    packages.sort_by(|a, b| (&a.game, &a.id).cmp(&(&b.game, &b.id)));
    Ok(packages)
}

fn read_modules(root: &Path) -> Result<Vec<ModuleReleaseManifest>> {
    let canonical = root.canonicalize()?;
    let modules = root.join("modules");
    match fs::symlink_metadata(&modules) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    real_directory(&modules, &canonical)?;
    let mut releases = Vec::new();
    let mut seen = HashSet::new();
    for game in fs::read_dir(&modules)? {
        let game = game?;
        let id = game.file_name().to_string_lossy().into_owned();
        if !valid_id(&id) {
            bail!("invalid module game directory");
        }
        real_directory(&game.path(), &canonical)?;
        for version in fs::read_dir(game.path())? {
            let version = version?;
            let label = version.file_name().to_string_lossy().into_owned();
            if !valid_version(&label)
                || !seen.insert(format!("{id}/{}", label.to_ascii_lowercase()))
            {
                bail!("invalid or case-colliding module version directory");
            }
            real_directory(&version.path(), &canonical)?;
            if releases.len() >= MAX_MODULES {
                bail!("bucket module release limit exceeded");
            }
            releases.push(read_release(&version.path(), &id, &label)?);
        }
    }
    releases.sort_by(|a, b| (&a.game_id, &a.version).cmp(&(&b.game_id, &b.version)));
    Ok(releases)
}

fn official_modules() -> Result<Vec<ModuleReleaseManifest>> {
    let manifest =
        include_bytes!("../../../../buckets/main/modules/cyberpunk-2077/0.1.0/module.json");
    let script = include_bytes!("../../../../buckets/main/modules/cyberpunk-2077/0.1.0/main.lua");
    Ok(vec![validate_release(
        manifest,
        script,
        "cyberpunk-2077",
        "0.1.0",
    )?])
}

fn official() -> Result<Vec<PackageManifest>> {
    let mut packages = Vec::new();
    for source in [
        include_str!("../../../../buckets/main/games/cyberpunk-2077/cet.json"),
        include_str!("../../../../buckets/main/games/cyberpunk-2077/red4ext.json"),
        include_str!("../../../../buckets/main/games/cyberpunk-2077/redscript.json"),
    ] {
        let manifest: PackageManifest = serde_json::from_str(source)?;
        manifest.validate(&manifest.game, &manifest.id)?;
        packages.push(manifest);
    }
    Ok(packages)
}

impl BucketStore {
    /// Construct a handle without creating files or opening game state.
    #[must_use]
    pub fn new(state_root: &Path) -> Self {
        Self {
            root: state_root.join("buckets"),
        }
    }

    // Refuse redirected manager-state paths, not just linked registry children.
    // The default state path is real; explicit linked state roots must be refused.
    fn check_registry_root(&self) -> Result<()> {
        if self.root.components().any(|c| c == Component::ParentDir) {
            bail!("bucket registry path must not contain parent components");
        }
        for ancestor in self.root.ancestors() {
            match fs::symlink_metadata(ancestor) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    bail!(
                        "linked bucket registry ancestor refused: {}",
                        ancestor.display()
                    );
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
                _ => {}
            }
        }
        Ok(())
    }

    fn registrations(&self) -> Result<BTreeMap<String, PathBuf>> {
        self.check_registry_root()?;
        let root = self.root.join("registrations");
        match fs::symlink_metadata(&root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(BTreeMap::new())
            }
            Err(error) => return Err(error.into()),
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
                bail!("unsafe bucket registry")
            }
            Ok(_) => {}
        }
        let mut registered = BTreeMap::new();
        for entry in fs::read_dir(root)? {
            let path = entry?.path();
            if path.extension().is_none_or(|e| e != "json") {
                bail!("unexpected bucket registration");
            }
            let id = path
                .file_stem()
                .context("invalid registration path")?
                .to_string_lossy();
            if !valid_id(&id) || id == "main" {
                bail!("invalid registered bucket name");
            }
            let registration: Registration = bounded_json(&path)?;
            registered.insert(id.into_owned(), registration.path);
        }
        Ok(registered)
    }

    fn load(
        &self,
        name: &str,
        path: &Path,
    ) -> Result<(BucketInfo, Vec<PackageManifest>, Vec<ModuleReleaseManifest>)> {
        if fs::symlink_metadata(path)?.file_type().is_symlink() {
            bail!("linked bucket root refused");
        }
        let metadata: Metadata = bounded_json(&path.join("bucket.json"))?;
        if metadata.schema_version != 1
            || metadata.name != name
            || metadata.title.trim().is_empty()
            || metadata.title.len() > 128
            || metadata.title.chars().any(char::is_control)
        {
            bail!("bucket name or schema mismatch: {name}");
        }
        let packages = read_packages(path)?;
        let modules = read_modules(path)?;
        Ok((
            BucketInfo {
                name: name.into(),
                title: metadata.title,
                path: Some(path.to_path_buf()),
            },
            packages,
            modules,
        ))
    }

    /// Register one explicitly chosen local bucket. Does not copy or download content.
    pub fn add(&self, name: &str, path: &Path) -> Result<BucketInfo> {
        if !valid_id(name) || name == "main" {
            bail!("invalid or reserved bucket name");
        }
        let path = path.canonicalize()?;
        let (info, _, _) = self.load(name, &path)?;
        if self.registrations()?.contains_key(name) {
            bail!("bucket already registered");
        }
        let dir = self.root.join("registrations");
        if let Ok(metadata) = fs::symlink_metadata(&dir) {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                bail!("unsafe bucket registry");
            }
        }
        fs::create_dir_all(&dir)?;
        let mut temp = tempfile::NamedTempFile::new_in(&self.root)?;
        serde_json::to_writer(&mut temp, &Registration { path })?;
        temp.as_file().sync_all()?;
        temp.persist_noclobber(dir.join(format!("{name}.json")))
            .context("bucket name already registered")?;
        Ok(info)
    }

    /// Forget a registered bucket without touching its source or game files.
    pub fn remove(&self, name: &str) -> Result<()> {
        if !valid_id(name) || name == "main" {
            bail!("cannot remove the reserved main bucket");
        }
        if !self.registrations()?.contains_key(name) {
            bail!("bucket is not registered: {name}");
        }
        let file = self.root.join("registrations").join(format!("{name}.json"));
        fs::remove_file(file)?;
        Ok(())
    }

    /// List the built-in main bucket and registered local buckets.
    pub fn list(&self) -> Result<Vec<BucketInfo>> {
        let mut all = vec![BucketInfo {
            name: "main".into(),
            title: "Modn Official (bundled snapshot)".into(),
            path: None,
        }];
        for (id, path) in self.registrations()? {
            all.push(self.load(&id, &path)?.0);
        }
        Ok(all)
    }

    /// Game IDs represented by built-in package metadata, not hardcoded Rust policy.
    pub fn official_games(&self) -> Result<Vec<String>> {
        let mut ids: Vec<String> = official()?.into_iter().map(|p| p.game).collect();
        ids.extend(official_modules()?.into_iter().map(|m| m.game_id));
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    /// Enumerate all validated manifests, keeping bucket namespaces distinct.
    pub fn packages(&self, game: Option<&str>) -> Result<Vec<BucketPackage>> {
        if game.is_some_and(|id| !valid_id(id)) {
            bail!("invalid game ID");
        }
        let mut packages = Vec::new();
        for package in official()? {
            packages.push(BucketPackage {
                bucket: "main".into(),
                package,
            });
        }
        for (id, path) in self.registrations()? {
            let (_, content, _) = self.load(&id, &path)?;
            for package in content {
                packages.push(BucketPackage {
                    bucket: id.clone(),
                    package,
                });
            }
        }
        packages.retain(|entry| game.is_none_or(|id| entry.package.game == id));
        packages.sort_by(|a, b| {
            (&a.bucket, &a.package.game, &a.package.id).cmp(&(
                &b.bucket,
                &b.package.game,
                &b.package.id,
            ))
        });
        Ok(packages)
    }

    /// Enumerate immutable module metadata in all buckets without executing Lua.
    pub fn modules(&self, game: Option<&str>) -> Result<Vec<BucketModule>> {
        if game.is_some_and(|id| !valid_id(id)) {
            bail!("invalid game ID");
        }
        let mut releases: Vec<BucketModule> = official_modules()?
            .into_iter()
            .map(|module| BucketModule {
                bucket: "main".into(),
                module,
            })
            .collect();
        for (id, path) in self.registrations()? {
            let (_, _, modules) = self.load(&id, &path)?;
            for module in modules {
                releases.push(BucketModule {
                    bucket: id.clone(),
                    module,
                });
            }
        }
        releases.retain(|entry| game.is_none_or(|id| entry.module.game_id == id));
        releases.sort_by(|a, b| {
            (&a.bucket, &a.module.game_id, &a.module.version).cmp(&(
                &b.bucket,
                &b.module.game_id,
                &b.module.version,
            ))
        });
        Ok(releases)
    }

    /// Resolve a bucket/game/version module reference without executing it.
    pub fn show_module(&self, reference: &str) -> Result<BucketModule> {
        let parts: Vec<&str> = reference.split('/').collect();
        if parts.len() != 3
            || !valid_id(parts[0])
            || !valid_id(parts[1])
            || !valid_version(parts[2])
        {
            bail!("expected bucket/game/module-version");
        }
        self.modules(Some(parts[1]))?
            .into_iter()
            .find(|m| m.bucket == parts[0] && m.module.version == parts[2])
            .with_context(|| format!("bucket module not found: {reference}"))
    }

    /// Resolve a complete bucket/game/mod reference without ambiguity.
    pub fn show(&self, reference: &str) -> Result<BucketPackage> {
        let parts: Vec<&str> = reference.split('/').collect();
        if parts.len() != 3 || parts.iter().any(|part| !valid_id(part)) {
            bail!("expected bucket/game/package");
        }
        self.packages(Some(parts[1]))?
            .into_iter()
            .find(|p| p.bucket == parts[0] && p.package.id == parts[2])
            .with_context(|| format!("bucket package not found: {reference}"))
    }
}
