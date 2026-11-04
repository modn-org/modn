use super::manifest::{valid_id, PackageManifest};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

const MAX_JSON: u64 = 32 * 1024;
const MAX_PACKAGES: usize = 1024;

#[expect(
    missing_docs,
    reason = "Public catalog record fields are self-describing"
)]
#[derive(Clone, Debug, Serialize)]
#[non_exhaustive]
pub struct BucketInfo {
    pub name: String,
    pub title: String,
    /// Directory of the read-only catalog source.
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

#[derive(Default)]
struct Releases {
    current: Vec<PackageManifest>,
    other: Vec<PackageManifest>,
}

fn read_packages(root: &Path, official: bool) -> Result<Releases> {
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
            if !official {
                bail!(
                    "only the official main bucket may contain game modules: {}",
                    game.path().display()
                );
            }
            let info = crate::game_modules::check_module(&game.path())?;
            if info.game_id != game_id || !has_script {
                bail!("official game module identity or module.lua mismatch: {game_id}");
            }
            let manifest: serde_json::Value = bounded_json(&module)?;
            if manifest["entrypoint"] != "module.lua" {
                bail!("official game module entrypoint must be module.lua");
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

impl BucketStore {
    /// Construct a handle without creating files or opening game state.
    #[must_use]
    pub fn new(state_root: &Path) -> Self {
        Self {
            root: state_root.join("buckets"),
        }
    }

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

    fn load(&self, name: &str, path: &Path) -> Result<(BucketInfo, Releases)> {
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
        match fs::symlink_metadata(path.join("modules")) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
            Ok(_) => bail!("bucket module directories are unsupported"),
        }
        let packages = read_packages(path, name == "main")?;
        Ok((
            BucketInfo {
                name: name.into(),
                title: metadata.title,
                path: Some(path.to_path_buf()),
            },
            packages,
        ))
    }

    fn official(&self) -> Result<(BucketInfo, Releases)> {
        let path = crate::data_dir::data_directory()?.join("buckets").join("main");
        Self::new(&crate::data_dir::data_directory()?).check_registry_root()?;
        self.load("main", &path).with_context(|| {
            format!(
                "cannot load local main bucket; expected a valid copy at {}",
                path.display()
            )
        })
    }

    /// Return the validated first-party module directory for a supported game.
    pub fn official_module(&self, game: &str) -> Result<PathBuf> {
        if !valid_id(game) {
            bail!("invalid game module identifier");
        }
        let (info, _) = self.official()?;
        let path = info
            .path
            .context("main bucket path missing")?
            .join("games")
            .join(game);
        if !path.join("module.json").is_file() {
            bail!(
                "no first-party game module for {game}; expected {}",
                path.display()
            );
        }
        Ok(path)
    }

    /// Register one explicitly chosen local bucket. Does not copy or download content.
    pub fn add(&self, name: &str, path: &Path) -> Result<BucketInfo> {
        if !valid_id(name) || name == "main" {
            bail!("invalid or reserved bucket name");
        }
        let path = path.canonicalize()?;
        let (info, _) = self.load(name, &path)?;
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

    /// List the local main bucket and registered local buckets.
    pub fn list(&self) -> Result<Vec<BucketInfo>> {
        let mut all = vec![self.official()?.0];
        for (id, path) in self.registrations()? {
            all.push(self.load(&id, &path)?.0);
        }
        Ok(all)
    }

    /// Game IDs with validated first-party modules in the local main bucket.
    pub fn official_games(&self) -> Result<Vec<String>> {
        let (info, _) = self.official()?;
        let games = info.path.context("main bucket path missing")?.join("games");
        let mut ids = Vec::new();
        for entry in fs::read_dir(games)? {
            let entry = entry?;
            if entry.path().join("module.json").is_file() {
                ids.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
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
        for package in self.official()?.1.current {
            packages.push(BucketPackage {
                bucket: "main".into(),
                package,
            });
        }
        for (id, path) in self.registrations()? {
            let (_, content) = self.load(&id, &path)?;
            for package in content.current {
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

    /// Every release that `bucket` publishes for one mod: the current release
    /// first, then other releases by version string. Only the current release
    /// takes part in search, dependency resolution and plain installs.
    pub fn releases(&self, bucket: &str, game: &str, id: &str) -> Result<Vec<BucketPackage>> {
        let mut releases: Vec<BucketPackage> = self
            .packages(Some(game))?
            .into_iter()
            .filter(|p| p.bucket == bucket && p.package.id == id)
            .collect();
        if releases.is_empty() {
            bail!("bucket package not found: {bucket}/{game}/{id}");
        }
        let content = if bucket == "main" {
            Some(self.official()?.1)
        } else if let Some(path) = self.registrations()?.remove(bucket) {
            Some(self.load(bucket, &path)?.1)
        } else {
            None
        };
        if let Some(content) = content {
            releases.extend(
                content
                    .other
                    .into_iter()
                    .filter(|p| p.game == game && p.id == id)
                    .map(|package| BucketPackage {
                        bucket: bucket.into(),
                        package,
                    }),
            );
        }
        Ok(releases)
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
