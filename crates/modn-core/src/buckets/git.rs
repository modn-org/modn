use super::lock::{acquire_lock, LOCK_WAIT};
use super::manifest::valid_id;
use super::mirrors::{
    check_mirror_kinds, read_mirrors, read_relocation, CatalogHost, CatalogSyncTrace,
};
use super::rotation::{CatalogKeyChange, KeyChangeKind};
use super::store::{BucketStore, MODULE_MANIFEST, MODULE_SCRIPT};
use super::trust::ModuleTrustState;
use crate::game_modules::{ignored_catalog_modules, stage_catalog_modules_at, CatalogModuleReport};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};

mod fetch;
mod verify;

use self::fetch::clone_catalog;
pub(super) use verify::verify_catalog_tree;
pub use verify::CatalogSigner;

use self::verify::CatalogVerification;

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
    /// Accepted rotation or revocation, when this revision carried one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_change: Option<CatalogKeyChange>,
    /// Published key statements that added no trust to this pin.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ignored_key_statements: Vec<String>,
    /// Game module releases imported or ignored during this sync.
    pub modules: CatalogModuleReport,
    /// The primary, the source that served the revision and any failures.
    pub trace: CatalogSyncTrace,
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

pub(super) fn git(directory: &Path, arguments: &[&str]) -> Result<String> {
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

pub(super) fn validate_tree(root: &Path, directory: &Path, trust: ModuleTrustState) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() || (!kind.is_dir() && !kind.is_file()) {
            bail!("catalog contains a non-regular entry");
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
                bail!("catalog game modules are only supported under games/<game-id>/");
            }
        }
        if kind.is_dir() {
            validate_tree(root, &entry.path(), trust)?;
        }
    }
    Ok(())
}

fn verify_revision(
    root: &Path,
    name: &str,
    reference: &str,
    previous: Option<&GitCatalogRevision>,
) -> Result<CatalogVerification> {
    verify_catalog_tree(
        root,
        name,
        &format!("Git catalog {name} at {reference}"),
        previous.map(|revision| &revision.signer),
    )
}

impl GitCatalogSource {
    /// Construct a source. Sync validates it before running Git.
    #[must_use]
    pub fn new(remote: String, reference: String) -> Self {
        Self { remote, reference }
    }

    pub(super) fn validate(&self) -> Result<()> {
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
    /// Publication atomically replaces one registration file. An unchanged commit reuses
    /// the published tree; a new commit keeps its own tree.
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
        self.sync_git_bounded(name, source)
    }

    fn sync_git_bounded(
        &self,
        name: &str,
        source: Option<GitCatalogSource>,
    ) -> Result<GitCatalogSync> {
        Self::check_registry_root(&self.root)?;
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
        let primary = CatalogHost::from(&source);
        let mut candidates = vec![source.clone()];
        if previous.is_some() {
            for mirror in self.stored_mirrors(name)? {
                let Some(candidate) = mirror.git_source() else {
                    continue;
                };
                if !candidates.contains(&candidate) {
                    candidates.push(candidate);
                }
            }
        }
        let stage = tempfile::Builder::new()
            .prefix("git-catalog-")
            .tempdir_in(&self.root)?;
        let mut failed_sources = Vec::new();
        let mut served = None;
        for (index, candidate) in candidates.iter().enumerate() {
            let attempt = stage.path().join(format!("source-{index}"));
            fs::create_dir_all(&attempt)?;
            let fetched = clone_catalog(&attempt, candidate);
            match fetched {
                Ok((catalog, commit)) => {
                    served = Some((catalog, commit, candidate.clone()));
                    break;
                }
                Err(error) => {
                    let _ = fs::remove_dir_all(&attempt);
                    failed_sources.push(format!(
                        "{}: {error:#}",
                        CatalogHost::from(candidate).label()
                    ));
                }
            }
        }
        let Some((catalog, commit, served_source)) = served else {
            bail!(
                "catalog {name} could not be fetched from any source:\n{}",
                failed_sources.join("\n")
            );
        };
        let served_by = CatalogHost::from(&served_source);
        let verification =
            verify_revision(&catalog, name, &served_source.reference, previous.as_ref()).map_err(
                |error| anyhow!("catalog source {} failed: {error:#}", served_by.label()),
            )?;
        let trust = match verification.key_change.as_ref().map(|change| change.kind) {
            Some(KeyChangeKind::Revoked) => ModuleTrustState::Untrusted,
            _ => trust,
        };
        let pinned = verification.pinned;
        validate_tree(&catalog, &catalog, trust)
            .map_err(|error| anyhow!("catalog source {} failed: {error:#}", served_by.label()))?;
        let mirrors = read_mirrors(&catalog)
            .map_err(|error| anyhow!("catalog source {} failed: {error:#}", served_by.label()))?;
        check_mirror_kinds(name, &served_by, &mirrors)
            .map_err(|error| anyhow!("catalog source {} failed: {error:#}", served_by.label()))?;
        let relocation = read_relocation(&catalog, &served_by)
            .map_err(|error| anyhow!("catalog source {} failed: {error:#}", served_by.label()))?;
        let moved_to = relocation.filter(|primary_host| primary_host != &primary);
        let published_source = match &moved_to {
            Some(host) => host
                .git_source()
                .context("relocation must name a Git source")?,
            None => source,
        };
        let revision = GitCatalogRevision {
            source: published_source,
            commit,
            signer: verification.signer,
        };
        let key_change = verification.key_change;
        let previous_registration = match &previous {
            Some(_) => Some(self.registration(name)?),
            None => None,
        };
        let unchanged = previous
            .as_ref()
            .is_some_and(|previous| previous.commit == revision.commit)
            && previous_registration
                .as_ref()
                .is_some_and(|registration| same_signing_files(&registration.path, &catalog));
        let published_path = match (&previous_registration, unchanged) {
            (Some(registration), true) => registration.path.clone(),
            _ => catalog.clone(),
        };
        let superseded = match (&previous_registration, unchanged) {
            (Some(registration), false) => Some(registration.path.clone()),
            _ => None,
        };
        let staged = if trust == ModuleTrustState::Trusted {
            Some(stage_catalog_modules_at(
                &self.roots,
                name,
                &revision.signer.key_id,
                &catalog,
            )?)
        } else {
            None
        };
        let ignored = match &staged {
            Some(staged) => staged.ignored().to_vec(),
            None => ignored_catalog_modules(&catalog)?,
        };
        // Validate the whole catalog before any module commit. A commit must not
        // publish releases from a revision that could never be registered.
        self.validate_catalog(name, &published_path, trust)?;
        // Commit the modules first: they were verified against the signed catalog
        // snapshot, so this adds no trust. A failed commit then leaves the previous
        // registration and the previous active releases in place. A failed publish
        // keeps the new releases retained; the next sync publishes the revision and
        // reports those releases as unchanged.
        let imported = match staged {
            Some(staged) => staged.commit()?,
            None => Vec::new(),
        };
        self.publish_git(
            name,
            published_path,
            revision.clone(),
            mirrors.clone(),
            trust == ModuleTrustState::Trusted,
        )?;
        if !(unchanged && previous_registration.is_some()) {
            let _ = stage.keep();
        }
        // Publish before purging: a failed publish leaves the previous registration
        // and tree in place, so only a committed registration purges retired trees.
        self.purge_retired_catalog_snapshots();
        if let Some(superseded) = superseded {
            self.retire_catalog_snapshot(name, &superseded);
        }
        Ok(GitCatalogSync {
            revision,
            pinned,
            key_change,
            ignored_key_statements: verification.ignored_key_statements,
            modules: CatalogModuleReport { imported, ignored },
            trace: CatalogSyncTrace {
                primary: primary.clone(),
                served_by,
                failed_sources,
                moved_from: moved_to.as_ref().map(|_| primary),
                moved_to,
            },
        })
    }
}

/// True when a published tree carries the same pinned key and signature files.
///
/// The catalog digest excludes `catalog.pub` and `catalog.sig`, so a rotation
/// with unchanged content keeps the digest. A tree is reusable only when those
/// two files match too.
pub(super) fn same_signing_files(published: &Path, staged: &Path) -> bool {
    ["catalog.pub", "catalog.sig"].iter().all(|name| {
        match (fs::read(published.join(name)), fs::read(staged.join(name))) {
            (Ok(published), Ok(staged)) => published == staged,
            _ => false,
        }
    })
}

#[cfg(test)]
mod publish_order_tests;
#[cfg(test)]
mod replay_tests;
#[cfg(test)]
mod snapshot_tests;
#[cfg(test)]
mod statement_tests;

#[cfg(test)]
pub(crate) mod tests;
