use super::lock::{acquire_lock, LOCK_WAIT};
use super::manifest::valid_id;
use super::signature::{verify_catalog_directory, CatalogPublicKey};
use super::trust::ModuleTrustState;
use super::{BucketStore, MODULE_MANIFEST, MODULE_SCRIPT};
use crate::game_modules::{stage_catalog_modules_at, CatalogModuleReport};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};

const MAX_CATALOG_PUBLIC_KEY_BYTES: u64 = 4 * 1024;

/// An explicitly selected community catalog remote and branch, tag or commit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GitCatalogSource {
    /// HTTPS remote or absolute local path. Local paths support offline fixtures.
    pub remote: String,
    /// Exact Git revision expression. Option-like and reflog expressions are refused.
    pub reference: String,
}

/// Minisign signer pinned for one Git catalog at first sync.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CatalogSigner {
    /// Minisign key identifier shown as 16 uppercase hexadecimal digits.
    pub key_id: String,
    /// The minisign public key text accepted at first sync.
    pub public_key: String,
}

impl CatalogSigner {
    fn pin(key: &CatalogPublicKey, text: String) -> Self {
        Self {
            key_id: key.key_id().to_string(),
            public_key: text,
        }
    }

    fn pinned_key(&self) -> Result<CatalogPublicKey> {
        let key = CatalogPublicKey::parse(&self.public_key)
            .context("pinned catalog signer is invalid")?;
        if key.key_id().to_string() != self.key_id {
            bail!("pinned catalog signer key ID does not match its public key");
        }
        Ok(key)
    }
}

/// Source, commit and signer published together with a validated catalog pointer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GitCatalogRevision {
    /// Explicit source retained for subsequent updates.
    pub source: GitCatalogSource,
    /// Full resolved Git commit identifier.
    pub commit: String,
    /// Signer key pinned at first sync.
    pub signer: CatalogSigner,
}

/// Result of one explicit Git catalog sync.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct GitCatalogSync {
    /// The published source, commit and signer pin.
    #[serde(flatten)]
    pub revision: GitCatalogRevision,
    /// True when this sync pinned the signer key.
    pub pinned: bool,
    /// Game module releases imported or ignored during this sync.
    pub modules: CatalogModuleReport,
}

/// Stored form that can still read a registration written before signing.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredGitCatalogRevision {
    pub(super) source: GitCatalogSource,
    pub(super) commit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) signer: Option<CatalogSigner>,
}

impl StoredGitCatalogRevision {
    pub(super) fn revision(self, name: &str) -> Result<GitCatalogRevision> {
        let signer = self.signer.with_context(|| {
            format!(
                "Bucket {name} was registered before catalog signing; remove it and sync it again."
            )
        })?;
        Ok(GitCatalogRevision {
            source: self.source,
            commit: self.commit,
            signer,
        })
    }
}

impl From<GitCatalogRevision> for StoredGitCatalogRevision {
    fn from(revision: GitCatalogRevision) -> Self {
        Self {
            source: revision.source,
            commit: revision.commit,
            signer: Some(revision.signer),
        }
    }
}

fn git(directory: &Path, arguments: &[&str]) -> Result<String> {
    let hooks = crate::git::empty_hooks()?;
    crate::git::run(
        &crate::git::GitEnvironment {
            directory,
            hooks: hooks.path(),
            allow_file: true,
        },
        arguments,
    )
}

fn validate_tree(root: &Path, directory: &Path, trust: ModuleTrustState) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() || (!kind.is_dir() && !kind.is_file()) {
            bail!("Git catalog contains a non-regular entry");
        }
        let relative = entry.path();
        let relative = relative
            .strip_prefix(root)
            .context("catalog module path outside root")?;
        let parts: Vec<&OsStr> = relative
            .components()
            .filter_map(|component| match component {
                Component::Normal(name) => Some(name),
                _ => None,
            })
            .collect();
        if parts.first() == Some(&OsStr::new("games")) {
            if !matches!(trust, ModuleTrustState::Main | ModuleTrustState::Trusted) {
                // Untrusted buckets ignore `games/` without reading it.
                continue;
            }
            let valid = match parts.as_slice() {
                [_] => kind.is_dir(),
                [_, game] => kind.is_dir() && valid_id(&game.to_string_lossy()),
                [_, game, name] => {
                    kind.is_file()
                        && valid_id(&game.to_string_lossy())
                        && matches!(name.to_str(), Some(MODULE_MANIFEST | MODULE_SCRIPT))
                }
                _ => false,
            };
            if !valid {
                bail!(
                    "catalog game modules are only supported at games/<game-id>/module.json and module.lua"
                );
            }
        }
        if kind.is_dir() {
            validate_tree(root, &entry.path(), trust)?;
        }
    }
    Ok(())
}

fn unsigned(name: &str, reference: &str) -> anyhow::Error {
    anyhow!("Git catalog {name} at {reference} is unsigned; Modn refuses unsigned catalogs.")
}

fn has_signature(root: &Path) -> Result<bool> {
    match fs::symlink_metadata(root.join("catalog.sig")) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn read_public_key(root: &Path) -> Result<Option<(CatalogPublicKey, String)>> {
    let path = root.join("catalog.pub");
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.file_type().is_file() {
        bail!(
            "catalog public key must be a regular file: {}",
            path.display()
        );
    }
    if metadata.len() > MAX_CATALOG_PUBLIC_KEY_BYTES {
        bail!("catalog public key exceeds size limit: {}", path.display());
    }
    let text = fs::read_to_string(&path)
        .with_context(|| format!("cannot read catalog public key: {}", path.display()))?;
    let key = CatalogPublicKey::parse(&text)?;
    Ok(Some((key, text)))
}

fn verify_revision(
    root: &Path,
    name: &str,
    reference: &str,
    previous: Option<&GitCatalogRevision>,
) -> Result<CatalogSigner> {
    if !has_signature(root)? {
        return Err(unsigned(name, reference));
    }
    let Some(previous) = previous else {
        let Some((key, text)) = read_public_key(root)? else {
            return Err(unsigned(name, reference));
        };
        verify_catalog_directory(root, &key)?;
        return Ok(CatalogSigner::pin(&key, text));
    };
    let pinned = previous.signer.pinned_key()?;
    if let Some((present, _)) = read_public_key(root)? {
        if present.key_id() != pinned.key_id() {
            bail!(
                "Signer key changed from {} to {} for bucket {name}. Modn does not rotate keys yet (#122); remove the bucket and sync it again to trust the new key.",
                pinned.key_id(),
                present.key_id()
            );
        }
    }
    verify_catalog_directory(root, &pinned)?;
    Ok(previous.signer.clone())
}

impl GitCatalogSource {
    /// Construct a source. Sync validates it before running Git.
    #[must_use]
    pub fn new(remote: String, reference: String) -> Self {
        Self { remote, reference }
    }

    fn validate(&self) -> Result<()> {
        let local = PathBuf::from(&self.remote);
        let https = self.remote.strip_prefix("https://").is_some_and(|rest| {
            let host = rest.split('/').next().unwrap_or_default();
            !host.is_empty() && !host.contains(['@', '?', '#', '\\', ':'])
        });
        if (!https && !local.is_absolute())
            || self.remote.chars().any(char::is_control)
            || self.remote.contains('\\') && https
            || self.reference.is_empty()
            || self.reference.len() > 200
            || self.reference.starts_with('-')
            || self.reference.contains("..")
            || !self
                .reference
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b))
        {
            bail!("invalid Git catalog remote or revision");
        }
        Ok(())
    }
}

impl BucketStore {
    /// Read the published Git revision without fetching. Local registrations return an error.
    pub fn git_revision(&self, name: &str) -> Result<Option<GitCatalogRevision>> {
        if !valid_id(name) || name == "main" {
            bail!("invalid or reserved Git catalog name");
        }
        self.registered_git(name)
    }

    /// Explicitly fetch, check out, verify and validate a community catalog before publication.
    /// New catalogs require `source`; updates reuse the registered source when it is absent.
    /// A failed fetch, checkout, verification or validation leaves the old registration and tree intact.
    /// Publication atomically replaces one registration file. Old trees are retained.
    pub fn sync_git(
        &self,
        name: &str,
        source: Option<GitCatalogSource>,
    ) -> Result<GitCatalogRevision> {
        Ok(self.sync_git_with_signer(name, source)?.revision)
    }

    /// Same as [`Self::sync_git`], and reports whether this sync pinned the signer key.
    pub fn sync_git_with_signer(
        &self,
        name: &str,
        source: Option<GitCatalogSource>,
    ) -> Result<GitCatalogSync> {
        self.check_registry_root()?;
        if !valid_id(name) || name == "main" {
            bail!("invalid or reserved Git catalog name");
        }
        fs::create_dir_all(&self.root)?;
        let lock_path = self.root.join(".git-sync.lock");
        let _lock = acquire_lock(
            &lock_path,
            "unsafe Git catalog lock",
            "Git catalog sync already running",
            LOCK_WAIT,
        )?;
        let previous = self.git_revision(name)?;
        let trust = match &previous {
            None => ModuleTrustState::Untrusted,
            Some(_) => self.module_trust(name)?,
        };
        let source = match (source, previous.as_ref()) {
            (Some(source), None) => source,
            (None, Some(previous)) => previous.source.clone(),
            (Some(source), Some(previous)) if source == previous.source => source,
            _ => {
                bail!("provide a source for registration; changing a registered source is refused")
            }
        };
        source.validate()?;
        let stage = tempfile::Builder::new()
            .prefix("git-catalog-")
            .tempdir_in(&self.root)?;
        git(
            stage.path(),
            &[
                "clone",
                "--no-local",
                "--no-checkout",
                "--",
                &source.remote,
                "catalog",
            ],
        )?;
        let catalog = stage.path().join("catalog");
        git(&catalog, &["checkout", "--detach", &source.reference, "--"])?;
        let commit = git(&catalog, &["rev-parse", "--verify", "HEAD"])?;
        fs::remove_dir_all(catalog.join(".git"))?;
        let signer = verify_revision(&catalog, name, &source.reference, previous.as_ref())?;
        validate_tree(&catalog, &catalog, trust)?;
        let pinned = previous.is_none();
        let revision = GitCatalogRevision {
            source,
            commit,
            signer,
        };
        let staged = if trust == ModuleTrustState::Trusted {
            Some(stage_catalog_modules_at(
                &crate::data_dir::profile_directory()?,
                name,
                &revision.signer.key_id,
                &catalog,
            )?)
        } else {
            None
        };
        let ignored = match &staged {
            Some(staged) => staged.ignored().to_vec(),
            None => Vec::new(),
        };
        self.publish_git(name, catalog, revision.clone())?;
        let imported = match staged {
            Some(staged) => staged.commit()?,
            None => Vec::new(),
        };
        let _retained = stage.keep();
        Ok(GitCatalogSync {
            revision,
            pinned,
            modules: CatalogModuleReport { imported, ignored },
        })
    }
}

#[cfg(test)]
mod tests;
