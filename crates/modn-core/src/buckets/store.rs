mod packages;
mod registration;
#[cfg(test)]
mod roots_tests;

pub(crate) use self::packages::validate_module_source_games;
pub(super) use self::registration::{bounded_json, Registration};

use self::packages::{read_packages, Releases};
use super::manifest::{valid_id, PackageManifest};
use super::trust::ModuleTrustState;
use super::trust_decision::ModuleSourceRegistrationFacts;
use super::BucketSnapshot;
use crate::manager_roots::ManagerRoots;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Path constant for one module manifest inside `games/<game>/`.
pub(super) const MODULE_MANIFEST: &str = "module.json";
/// Path constant for one module script inside `games/<game>/`.
pub(super) const MODULE_SCRIPT: &str = "module.lua";

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
    /// Pinned catalog signer key identifier, when the bucket is a signed catalog.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signer_key_id: Option<String>,
    /// Pinned offline revocation key identifier, when the catalog named one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revocation_key_id: Option<String>,
    /// Key identifiers this catalog rotated away from.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub previous_signers: Vec<String>,
    /// Key identifiers revoked by the offline revocation key.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub revoked_signers: Vec<String>,
    /// `git` or `zip` for a registered catalog; omitted for other buckets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<String>,
    /// Registered primary host of a signed catalog.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary: Option<super::CatalogHost>,
    /// Ordered mirror hosts stored with the latest verified revision.
    pub mirrors: Vec<super::CatalogHost>,
    /// True when this bucket may supply executable game modules.
    pub module_trusted: bool,
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

/// Test clock for code that records or compares wall-clock times.
#[cfg(test)]
pub(super) type TestClock = std::sync::Arc<dyn Fn() -> u64 + Send + Sync>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    schema_version: u32,
    name: String,
    title: String,
}

/// Validate the bucket name and title that `bucket.json` may hold.
///
/// The rules are the ones `BucketStore` applies when it registers or syncs a
/// bucket, so a catalog author can check a value before writing the file.
pub fn validate_bucket_metadata(name: &str, title: &str) -> Result<()> {
    if !valid_id(name) || name == "main" {
        bail!(
            "invalid bucket name: {name}; use lowercase ASCII letters, digits and dashes, at most 64 characters, starting with a letter, and not the reserved name main"
        );
    }
    if !valid_title(title) {
        bail!(
            "invalid bucket title; the title must be non-empty, at most 128 bytes and free of control characters"
        );
    }
    Ok(())
}

/// Whether a registered bucket accepts this title.
fn valid_title(title: &str) -> bool {
    !title.trim().is_empty() && title.len() <= 128 && !title.chars().any(char::is_control)
}

/// Catalog registry, separate from game registrations and ownership state.
pub struct BucketStore {
    pub(super) root: PathBuf,
    pub(super) roots: ManagerRoots,
    /// Test seam: the next registration write fails before any file changes.
    #[cfg(test)]
    pub(super) fail_next_registration_write: std::sync::atomic::AtomicBool,
    /// Test seam: the next registration restore fails before any file changes.
    #[cfg(test)]
    pub(super) fail_next_registration_restore: std::sync::atomic::AtomicBool,
    /// Test seam: the next registration removal fails before any file changes.
    #[cfg(test)]
    pub(super) fail_next_registration_remove: std::sync::atomic::AtomicBool,
    /// Test seam: wall-clock seconds for retired-tree lifetime checks.
    #[cfg(test)]
    pub(super) clock: Option<TestClock>,
}

impl BucketStore {
    /// Construct a handle without creating files or opening game state.
    #[must_use]
    pub fn new(roots: &ManagerRoots) -> Self {
        Self {
            root: roots.bucket_registrations(),
            roots: roots.clone(),
            #[cfg(test)]
            fail_next_registration_write: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            fail_next_registration_restore: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            fail_next_registration_remove: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            clock: None,
        }
    }

    pub(super) fn check_registry_root(root: &Path) -> Result<()> {
        if root.components().any(|c| c == Component::ParentDir) {
            bail!("bucket registry path must not contain parent components");
        }
        for ancestor in root.ancestors() {
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

    pub(super) fn registrations(&self) -> Result<BTreeMap<String, Registration>> {
        Self::check_registry_root(&self.root)?;
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

    fn load(
        &self,
        name: &str,
        path: &Path,
        trust: ModuleTrustState,
    ) -> Result<(BucketInfo, Releases)> {
        if fs::symlink_metadata(path)?.file_type().is_symlink() {
            bail!("linked bucket root refused");
        }
        let metadata: Metadata = bounded_json(&path.join("bucket.json"))?;
        if metadata.schema_version != 2 {
            bail!(
                "bucket {name} uses bucket.json schema_version {}; Modn requires bucket contract v2 and does not read v1 buckets. The bucket must be republished for Modn contract v2.",
                metadata.schema_version
            );
        }
        if metadata.name != name || !valid_title(&metadata.title) {
            bail!("bucket name or metadata mismatch: {name}");
        }
        match fs::symlink_metadata(path.join("modules")) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
            Ok(_) => bail!("bucket module directories are unsupported"),
        }
        let packages = read_packages(path, trust)?;
        Ok((
            BucketInfo {
                name: name.into(),
                title: metadata.title,
                path: Some(path.to_path_buf()),
                signer_key_id: None,
                revocation_key_id: None,
                previous_signers: Vec::new(),
                revoked_signers: Vec::new(),
                source_kind: None,
                primary: None,
                mirrors: Vec::new(),
                module_trusted: matches!(trust, ModuleTrustState::Main | ModuleTrustState::Trusted),
            },
            packages,
        ))
    }

    fn official(&self) -> Result<(BucketInfo, Releases)> {
        let path = self.roots.official_main();
        Self::check_registry_root(&self.roots.official_bucket_root())?;
        self.load("main", &path, ModuleTrustState::Main)
            .with_context(|| {
                format!(
                    "cannot load local main bucket; run `modn bucket update`; expected a valid copy at {}",
                    path.display()
                )
            })
    }

    pub(super) fn validate_main(&self, path: &Path) -> Result<()> {
        self.load("main", path, ModuleTrustState::Main)?;
        super::known::validate_root(path)?;
        Ok(())
    }
    /// Validate one staged community catalog before publication.
    pub(super) fn validate_catalog(
        &self,
        name: &str,
        path: &Path,
        trust: ModuleTrustState,
    ) -> Result<()> {
        self.load(name, path, trust)?;
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
        if !path.join(MODULE_MANIFEST).is_file() {
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
        let (info, _) = self.load(name, &path, ModuleTrustState::Local)?;
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
        serde_json::to_writer(
            &mut temp,
            &Registration {
                path,
                git: None,
                zip: None,
                module_trust: None,
                mirrors: None,
                accepted: None,
            },
        )?;
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
        mirrors: Vec<super::CatalogHost>,
        trusted: bool,
        accepted: Option<super::sequence::StoredSequence>,
    ) -> Result<()> {
        let signer_key_id = git.signer.key_id.clone();
        let module_trust = trusted.then_some(super::ModuleSourceTrust { signer_key_id });
        let trust = if module_trust.is_some() {
            ModuleTrustState::Trusted
        } else {
            ModuleTrustState::Untrusted
        };
        self.load(name, &path, trust)?;
        self.write_registration(
            name,
            &Registration {
                path,
                git: Some(git.into()),
                zip: None,
                module_trust,
                mirrors: Some(mirrors),
                accepted,
            },
        )
    }

    /// Read one registration without creating registry directories.
    pub(super) fn registration(&self, name: &str) -> Result<Registration> {
        if !valid_id(name) || name == "main" {
            bail!("invalid or reserved bucket name");
        }
        if !self.registrations()?.contains_key(name) {
            bail!("bucket is not registered: {name}");
        }
        bounded_json(&self.root.join("registrations").join(format!("{name}.json")))
    }

    /// List the local main bucket and registered local buckets.
    pub fn list(&self) -> Result<Vec<BucketInfo>> {
        let mut all = vec![self.official()?.0];
        for (id, registration) in self.registrations()? {
            let revision = registration.revision(&id)?;
            let trust = ModuleSourceRegistrationFacts::from_registration(&registration)
                .bucket_trust_state();
            let signer_key_id = match registration.zip.as_ref() {
                Some(zip) => Some(zip.signer.key_id.clone()),
                None => revision.map(|revision| revision.signer.key_id),
            };
            let signer = registration.catalog_signer();
            let revocation_key_id = signer.and_then(|signer| signer.revocation_key_id.clone());
            let previous_signers = signer
                .map(|signer| signer.previous_signers.clone())
                .unwrap_or_default();
            let revoked_signers = signer
                .map(|signer| signer.revoked_signers.clone())
                .unwrap_or_default();
            let source_kind = match (&registration.git, &registration.zip) {
                (Some(_), None) => Some("git".to_owned()),
                (None, Some(_)) => Some("zip".to_owned()),
                _ => None,
            };
            let mut info = self.load(&id, &registration.path, trust)?.0;
            info.signer_key_id = signer_key_id;
            info.revocation_key_id = revocation_key_id;
            info.previous_signers = previous_signers;
            info.revoked_signers = revoked_signers;
            info.source_kind = source_kind;
            info.primary = super::mirrors::registration_primary(&registration);
            info.mirrors = super::mirrors::registration_mirrors(&registration);
            all.push(info);
        }
        Ok(all)
    }

    /// Game IDs with validated first-party modules in the local main bucket.
    pub fn official_games(&self) -> Result<Vec<String>> {
        let (info, _) = self.official()?;
        let games = info.path.context("main bucket path missing")?.join("games");
        let mut ids = Vec::new();
        let entries = match fs::read_dir(games) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(ids),
            Err(error) => return Err(error.into()),
            Ok(entries) => entries,
        };
        for entry in entries {
            let entry = entry?;
            if entry.path().join(MODULE_MANIFEST).is_file() {
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
            let trust = ModuleSourceRegistrationFacts::from_registration(&registration)
                .bucket_trust_state();
            sources.push((id.clone(), self.load(&id, &registration.path, trust)?.1));
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
