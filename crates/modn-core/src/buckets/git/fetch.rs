//! Clone one Git catalog mirror.

use super::{git, GitCatalogSource};
use crate::acquisition::GitSource;
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// Clone and check out one catalog revision.
///
/// The shared Git command bounds a stalled HTTP or HTTPS transfer with its
/// low-speed limits. A timeout is recorded as `timed out` by the catalog sync,
/// which tries the next mirror.
pub(super) fn clone_catalog(
    attempt: &Path,
    source: &GitCatalogSource,
) -> Result<(PathBuf, String)> {
    GitSource::clone_catalog(attempt, &source.remote)?;
    let catalog = attempt.join("catalog");
    git(&catalog, &["checkout", "--detach", &source.reference, "--"])?;
    let commit = git(&catalog, &["rev-parse", "--verify", "HEAD"])?;
    fs::remove_dir_all(catalog.join(".git"))?;
    Ok((catalog, commit))
}
