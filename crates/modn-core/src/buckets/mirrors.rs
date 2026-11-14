//! Signed catalog mirrors and relocation records.
//!
//! `mirrors.json` lists other hosts of the same signed catalog. `relocation.json`
//! names the new primary host. Both files sit in the catalog root, so the pinned
//! catalog signature covers them. A sync tries the registered primary first and
//! then the stored mirrors in order. Only a transport failure moves on to the
//! next source; a trust failure stops the sync.

use super::git::GitCatalogSource;
use super::store::{bounded_json, BucketStore, Registration};
use super::zip_source::ZipCatalogSource;
use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Largest number of mirrors one signed catalog may list.
pub const MAX_CATALOG_MIRRORS: usize = 8;

const MIRRORS_FILE: &str = "mirrors.json";
const RELOCATION_FILE: &str = "relocation.json";
const SCHEMA_VERSION: u32 = 1;

/// One host of a signed catalog, tagged by source kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
#[non_exhaustive]
pub enum CatalogHost {
    /// A Git remote and the revision that serves the catalog.
    Git {
        /// HTTPS Git remote.
        remote: String,
        /// Branch, tag or commit.
        revision: String,
    },
    /// An HTTPS URL of a signed catalog ZIP.
    Zip {
        /// HTTPS URL of the catalog ZIP.
        url: String,
    },
}

impl CatalogHost {
    /// Human-readable source label used in sync messages.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Git { remote, revision } => format!("git {remote}@{revision}"),
            Self::Zip { url } => format!("zip {url}"),
        }
    }

    /// Source kind name used in refusals and reports.
    #[must_use]
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Git { .. } => "git",
            Self::Zip { .. } => "zip",
        }
    }

    /// True when both hosts use the same source kind.
    #[must_use]
    pub fn same_kind(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Git { .. }, Self::Git { .. }) | (Self::Zip { .. }, Self::Zip { .. })
        )
    }

    /// ZIP view of this host, when it names a ZIP source.
    #[must_use]
    pub(super) fn zip_source(&self) -> Option<ZipCatalogSource> {
        match self {
            Self::Zip { url } => Some(ZipCatalogSource::new(url.clone())),
            Self::Git { .. } => None,
        }
    }

    /// Git view of this host, when it names a Git source.
    #[must_use]
    pub(super) fn git_source(&self) -> Option<GitCatalogSource> {
        match self {
            Self::Git { remote, revision } => {
                Some(GitCatalogSource::new(remote.clone(), revision.clone()))
            }
            Self::Zip { .. } => None,
        }
    }

    fn validate_declared(&self) -> Result<()> {
        match self {
            Self::Zip { url } => {
                if !url.starts_with("https://") {
                    bail!("a declared ZIP source must use an https:// URL");
                }
                ZipCatalogSource::new(url.clone()).validate()
            }
            Self::Git { remote, revision } => {
                if !remote.starts_with("https://") {
                    bail!("a declared Git source must use an https:// remote");
                }
                GitCatalogSource::new(remote.clone(), revision.clone()).validate()
            }
        }
    }
}

impl From<&GitCatalogSource> for CatalogHost {
    fn from(source: &GitCatalogSource) -> Self {
        Self::Git {
            remote: source.remote.clone(),
            revision: source.reference.clone(),
        }
    }
}

impl From<&ZipCatalogSource> for CatalogHost {
    fn from(source: &ZipCatalogSource) -> Self {
        Self::Zip {
            url: source.url.clone(),
        }
    }
}

/// Where one sync found its verified revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct CatalogSyncTrace {
    /// The registered primary tried first.
    pub primary: CatalogHost,
    /// The source that served the verified revision.
    pub served_by: CatalogHost,
    /// One line per source that failed before the serving source.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failed_sources: Vec<String>,
    /// Previous primary when the revision moved the catalog.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moved_from: Option<CatalogHost>,
    /// New primary when the revision moved the catalog.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moved_to: Option<CatalogHost>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MirrorsFile {
    schema_version: u32,
    mirrors: Vec<CatalogHost>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RelocationFile {
    schema_version: u32,
    primary: CatalogHost,
}

fn read_declared<T: DeserializeOwned>(root: &Path, file: &str) -> Result<Option<T>> {
    let path = root.join(file);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
        Ok(_) => bounded_json(&path)
            .with_context(|| format!("cannot read {file}"))
            .map(Some),
    }
}

/// Read and validate `mirrors.json` from one verified catalog root.
pub(super) fn read_mirrors(root: &Path) -> Result<Vec<CatalogHost>> {
    let Some(file): Option<MirrorsFile> = read_declared(root, MIRRORS_FILE)? else {
        return Ok(Vec::new());
    };
    if file.schema_version != SCHEMA_VERSION {
        bail!(
            "{MIRRORS_FILE} uses schema_version {}; Modn supports {SCHEMA_VERSION}",
            file.schema_version
        );
    }
    if file.mirrors.len() > MAX_CATALOG_MIRRORS {
        bail!(
            "{MIRRORS_FILE} lists {} mirrors; the limit is {MAX_CATALOG_MIRRORS}",
            file.mirrors.len()
        );
    }
    for (index, mirror) in file.mirrors.iter().enumerate() {
        mirror
            .validate_declared()
            .with_context(|| format!("invalid mirror {index} in {MIRRORS_FILE}"))?;
        if file.mirrors[..index].contains(mirror) {
            bail!("{MIRRORS_FILE} lists {} twice", mirror.label());
        }
    }
    Ok(file.mirrors)
}

/// Read and validate `relocation.json` from one verified catalog root.
pub(super) fn read_relocation(
    root: &Path,
    current: &CatalogHost,
) -> Result<Option<CatalogHost>> {
    let Some(file): Option<RelocationFile> = read_declared(root, RELOCATION_FILE)? else {
        return Ok(None);
    };
    if file.schema_version != SCHEMA_VERSION {
        bail!(
            "{RELOCATION_FILE} uses schema_version {}; Modn supports {SCHEMA_VERSION}",
            file.schema_version
        );
    }
    file.primary
        .validate_declared()
        .with_context(|| format!("invalid relocation target in {RELOCATION_FILE}"))?;
    if !file.primary.same_kind(current) {
        bail!(
            "{RELOCATION_FILE} must name a {} source for this catalog",
            current.kind_name()
        );
    }
    Ok(Some(file.primary))
}

/// Refuse a mirror list that names a different source kind than the catalog.
pub(super) fn check_mirror_kinds(
    name: &str,
    served_by: &CatalogHost,
    mirrors: &[CatalogHost],
) -> Result<()> {
    if mirrors.iter().all(|mirror| mirror.same_kind(served_by)) {
        return Ok(());
    }
    bail!(
        "{MIRRORS_FILE} in catalog {name} must list {} sources only; a mirror names another host of the same catalog",
        served_by.kind_name()
    )
}

/// The registered primary of one catalog registration.
pub(super) fn registration_primary(registration: &Registration) -> Option<CatalogHost> {
    match (&registration.git, &registration.zip) {
        (Some(git), None) => Some(CatalogHost::from(&git.source)),
        (None, Some(zip)) => Some(CatalogHost::from(&zip.source)),
        _ => None,
    }
}

/// The stored mirror list of one catalog registration.
pub(super) fn registration_mirrors(registration: &Registration) -> Vec<CatalogHost> {
    registration.mirrors.clone().unwrap_or_default()
}

impl BucketStore {
    /// The ordered mirrors stored for one registered catalog.
    pub(super) fn stored_mirrors(&self, name: &str) -> Result<Vec<CatalogHost>> {
        Ok(registration_mirrors(&self.registration(name)?))
    }
}

#[cfg(test)]
mod tests;
