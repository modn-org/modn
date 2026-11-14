//! Fetch one pinned Git commit without checkout and pack its tree.
//!
//! The commit hash is the integrity pin. Integration tests use a local bare
//! repository through a test-only transport override; release builds accept
//! only GitHub HTTPS remotes.
use super::digest::file_blake3;
use super::{ArchiveCache, CacheUse, Origin};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

mod pack;
use pack::pack_commit;

/// A package source pinned to one full lowercase Git commit on GitHub.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GitSource {
    /// Repository in `owner/name` form.
    pub repository: String,
    /// Full 40-character lowercase commit hash. Branches and tags are refused.
    pub commit: String,
}

/// Verified Git archive cache key. The commit names the source revision.
#[expect(missing_docs, reason = "Field and method names state their meaning")]
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct GitCacheKey<'a> {
    pub game_id: &'a str,
    pub mod_id: &'a str,
    pub version: &'a str,
    pub commit: &'a str,
}

impl<'a> GitCacheKey<'a> {
    /// Name the package release and pinned commit of one cache entry.
    #[must_use]
    pub fn new(game_id: &'a str, mod_id: &'a str, version: &'a str, commit: &'a str) -> Self {
        Self {
            game_id,
            mod_id,
            version,
            commit,
        }
    }
}

/// One fetched archive in the cache, with the digest the pipeline records.
#[expect(missing_docs, reason = "Field names state their meaning")]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct GitAcquired {
    pub path: PathBuf,
    pub blake3: String,
    pub origin: Origin,
}

fn valid_commit(commit: &str) -> bool {
    commit.len() == 40
        && commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Test-only repository base override. Release builds always use GitHub.
#[cfg(any(test, feature = "test-instruction-budget"))]
fn remote_base() -> Option<String> {
    std::env::var("MODN_TEST_GIT_REMOTE_BASE")
        .ok()
        .filter(|base| !base.is_empty())
}

#[cfg(not(any(test, feature = "test-instruction-budget")))]
fn remote_base() -> Option<String> {
    None
}

fn segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

impl GitSource {
    /// Construct a source. Validation happens before any Git call.
    #[must_use]
    pub fn new(repository: String, commit: String) -> Self {
        Self { repository, commit }
    }

    /// Validate the source and return the remote URL.
    ///
    /// Test builds may map the GitHub base to a local `file` URL. Release
    /// builds never read that override and always return GitHub HTTPS.
    pub fn remote_url(&self) -> Result<String> {
        let parts: Vec<&str> = self.repository.split('/').collect();
        if parts.len() != 2 || !parts.iter().all(|part| segment(part)) {
            bail!("invalid Git repository; expected owner/project");
        }
        if !valid_commit(&self.commit) {
            bail!("Git source commit must be exactly 40 lowercase hex characters");
        }
        let base = remote_base().unwrap_or_else(|| "https://github.com".to_owned());
        Ok(format!("{base}/{}.git", self.repository))
    }

    fn test_transport(&self, url: &str) -> bool {
        #[cfg(any(test, feature = "test-instruction-budget"))]
        {
            let _ = self;
            url.starts_with("file://")
        }
        #[cfg(not(any(test, feature = "test-instruction-budget")))]
        {
            let _ = url;
            false
        }
    }

    /// Fetch, verify and pack the pinned commit into a deterministic ZIP in
    /// `staging`. The caller owns `staging` and removes it afterwards.
    pub(crate) fn pack_url(&self, url: &str, staging: &Path) -> Result<pack::PackedArchive> {
        pack_commit(url, &self.commit, staging, self.test_transport(url))
    }

    /// Return a verified cached archive or fetch and pack the commit.
    pub fn acquire(
        &self,
        cache: &ArchiveCache,
        key: &GitCacheKey<'_>,
        cache_use: CacheUse,
    ) -> Result<GitAcquired> {
        let url = self.remote_url()?;
        self.acquire_url(&url, cache, key, cache_use)
    }

    fn acquire_url(
        &self,
        url: &str,
        cache: &ArchiveCache,
        key: &GitCacheKey<'_>,
        cache_use: CacheUse,
    ) -> Result<GitAcquired> {
        if key.commit != self.commit {
            bail!("Git source cache key does not name the pinned commit");
        }
        if cache_use == CacheUse::Reuse {
            if let Some((path, blake3)) = cache.git_cached(key)? {
                return Ok(GitAcquired {
                    path,
                    blake3,
                    origin: Origin::Cache,
                });
            }
        }
        fs::create_dir_all(cache.root())
            .with_context(|| format!("cannot create cache directory {}", cache.root().display()))?;
        let staging = tempfile::Builder::new()
            .prefix("git-source-")
            .tempdir_in(cache.root())?;
        let packed = self.pack_url(url, staging.path())?;
        let path = cache.git_path(key, &packed.blake3);
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("cannot replace cache entry {}", path.display()))?;
        }
        fs::rename(&packed.path, &path)
            .with_context(|| format!("cannot store cache entry {}", path.display()))?;
        Ok(GitAcquired {
            path,
            blake3: packed.blake3,
            origin: Origin::Download,
        })
    }
}

impl ArchiveCache {
    fn git_name(&self, key: &GitCacheKey<'_>, blake3: &str) -> String {
        format!(
            "{}#{}#{}#git-{}-blake3-{blake3}.zip",
            super::entry_name::component(key.game_id),
            super::entry_name::component(key.mod_id),
            super::entry_name::component(key.version),
            key.commit
        )
    }

    /// Cache path for one commit and archive digest.
    #[must_use]
    pub fn git_path(&self, key: &GitCacheKey<'_>, blake3: &str) -> PathBuf {
        self.root.join(self.git_name(key, blake3))
    }

    /// Return a cached Git archive whose bytes match its recorded digest.
    ///
    /// A corrupt entry is never returned; the next fetch replaces it.
    pub fn git_cached(&self, key: &GitCacheKey<'_>) -> Result<Option<(PathBuf, String)>> {
        let directory = self.entries(None)?;
        let prefix = format!(
            "{}#{}#{}#git-{}-",
            super::entry_name::component(key.game_id),
            super::entry_name::component(key.mod_id),
            super::entry_name::component(key.version),
            key.commit
        );
        for entry in directory {
            let name = entry
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let Some(rest) = name.strip_prefix(&prefix) else {
                continue;
            };
            let Some(blake3) = rest.strip_prefix("blake3-") else {
                continue;
            };
            let Some(blake3) = blake3.strip_suffix(".zip") else {
                continue;
            };
            if blake3.len() != 64
                || !blake3
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                continue;
            }
            if entry.bytes > super::MAX_DOWNLOAD_BYTES {
                continue;
            }
            if file_blake3(&entry.path)? == blake3 {
                return Ok(Some((entry.path, blake3.to_owned())));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests;
