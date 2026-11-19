//! Bounded clone of one Git catalog mirror.

use super::{git, GitCatalogSource};
use crate::acquisition::GitSource;
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How one catalog mirror clone is bounded, with a test program seam.
#[derive(Clone, Debug)]
pub(crate) struct CatalogFetch {
    /// Inactivity limit of one catalog clone.
    pub(crate) limit: Duration,
    /// Replacement Git program for tests.
    pub(crate) program: Option<PathBuf>,
}

impl CatalogFetch {
    /// The bounded fetch that production and ordinary tests use.
    pub(super) fn bounded() -> Self {
        Self {
            limit: GitSource::catalog_inactivity_limit(),
            program: None,
        }
    }
}

/// Clone and check out one catalog revision through the bounded fetch.
pub(super) fn clone_catalog(
    attempt: &Path,
    source: &GitCatalogSource,
    fetch: &CatalogFetch,
) -> Result<(PathBuf, String)> {
    GitSource::clone_catalog_bounded(
        attempt,
        &source.remote,
        fetch.limit,
        fetch.program.as_deref().map(Path::as_os_str),
    )?;
    let catalog = attempt.join("catalog");
    git(&catalog, &["checkout", "--detach", &source.reference, "--"])?;
    let commit = git(&catalog, &["rev-parse", "--verify", "HEAD"])?;
    fs::remove_dir_all(catalog.join(".git"))?;
    Ok((catalog, commit))
}
