mod packages;

use self::packages::{read_packages, Releases};
use super::manifest::{valid_id, PackageManifest};
use super::BucketSnapshot;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

const MAX_JSON: u64 = 32 * 1024;

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
    /// Pinned Git catalog signer key identifier, when the bucket is Git-backed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signer_key_id: Option<String>,
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

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    git: Option<super::git::StoredGitCatalogRevision>,
}

impl Registration {
    fn revision(&self, name: &str) -> Result<Option<super::GitCatalogRevision>> {
        self.git
            .clone()
            .map(|stored| stored.revision(name))
            .transpose()
    }
}

/// Catalog registry, separate from game registrations and ownership state.
pub struct BucketStore {
    pub(super) root: PathBuf,
}

pub(super) fn bounded_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
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

impl BucketStore {
    /// Construct a handle without creating files or opening game state.
    #[must_use]
    pub fn new(state_root: &Path) -> Self {
        Self {
            root: state_root.join("buckets"),
        }
    }

    pub(super) fn check_registry_root(&self) -> Result<()> {
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

    fn registrations(&self) -> Result<BTreeMap<String, Registration>> {
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
            registered.insert(id.into_owned(), registration);
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
                signer_key_id: None,
            },
            packages,
        ))
    }

    fn official(&self) -> Result<(BucketInfo, Releases)> {
        let path = crate::data_dir::data_directory()?
            .join("buckets")
            .join("main");
        Self::new(&crate::data_dir::data_directory()?).check_registry_root()?;
        self.load("main", &path).with_context(|| {
            format!(
                "cannot load local main bucket; run `modn bucket update`; expected a valid copy at {}",
                path.display()
            )
        })
    }

    pub(super) fn validate_main(&self, path: &Path) -> Result<()> {
        self.load("main", path)?;
        Ok(())
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
        serde_json::to_writer(&mut temp, &Registration { path, git: None })?;
        temp.as_file().sync_all()?;
        temp.persist_noclobber(dir.join(format!("{name}.json")))
            .context("bucket name already registered")?;
        Ok(info)
    }

    pub(super) fn publish_git(
        &self,
        name: &str,
        path: PathBuf,
        git: super::GitCatalogRevision,
    ) -> Result<()> {
        self.load(name, &path)?;
        let dir = self.root.join("registrations");
        if let Ok(meta) = fs::symlink_metadata(&dir) {
            if !meta.is_dir() || meta.file_type().is_symlink() {
                bail!("unsafe bucket registry");
            }
        }
        fs::create_dir_all(&dir)?;
        let mut temp = tempfile::NamedTempFile::new_in(&self.root)?;
        serde_json::to_writer(
            &mut temp,
            &Registration {
                path,
                git: Some(git.into()),
            },
        )?;
        temp.as_file().sync_all()?;
        temp.persist(dir.join(format!("{name}.json")))?;
        Ok(())
    }

    pub(super) fn registered_git(&self, name: &str) -> Result<Option<super::GitCatalogRevision>> {
        if !self.registrations()?.contains_key(name) {
            return Ok(None);
        }
        let registration: Registration =
            bounded_json(&self.root.join("registrations").join(format!("{name}.json")))?;
        registration
            .revision(name)?
            .context("bucket is a local source, not a Git catalog")
            .map(Some)
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
        for (id, registration) in self.registrations()? {
            let signer_key_id = registration
                .revision(&id)?
                .map(|revision| revision.signer.key_id);
            let mut info = self.load(&id, &registration.path)?.0;
            info.signer_key_id = signer_key_id;
            all.push(info);
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
        Ok(self.snapshot(game)?.packages().to_vec())
    }

    /// Read each bucket once and retain current and historical releases together.
    pub fn snapshot(&self, game: Option<&str>) -> Result<BucketSnapshot> {
        if game.is_some_and(|id| !valid_id(id)) {
            bail!("invalid game ID");
        }
        let mut sources = vec![("main".to_owned(), self.official()?.1)];
        for (id, registration) in self.registrations()? {
            registration.revision(&id)?;
            sources.push((id.clone(), self.load(&id, &registration.path)?.1));
        }
        let mut current = Vec::new();
        let mut other = Vec::new();
        for (bucket, releases) in sources {
            let qualify = |package: PackageManifest| BucketPackage {
                bucket: bucket.clone(),
                package,
            };
            let matches = |package: &PackageManifest| game.is_none_or(|id| package.game == id);
            current.extend(releases.current.into_iter().filter(matches).map(qualify));
            other.extend(releases.other.into_iter().filter(matches).map(qualify));
        }
        Ok(BucketSnapshot::new(current, other))
    }

    /// Every release that `bucket` publishes for one mod: the current release
    /// first, then other releases by version string. Only the current release
    /// takes part in search, dependency resolution and plain installs.
    pub fn releases(&self, bucket: &str, game: &str, id: &str) -> Result<Vec<BucketPackage>> {
        self.snapshot(Some(game))?.releases(bucket, game, id)
    }

    /// Resolve a complete bucket/game/mod reference without ambiguity.
    pub fn show(&self, reference: &str) -> Result<BucketPackage> {
        let parts: Vec<&str> = reference.split('/').collect();
        if parts.len() != 3 || parts.iter().any(|part| !valid_id(part)) {
            bail!("expected bucket/game/package");
        }
        self.snapshot(Some(parts[1]))?
            .candidates(reference)?
            .into_iter()
            .next()
            .with_context(|| format!("bucket package not found: {reference}"))
    }
}
