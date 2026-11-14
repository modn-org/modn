//! Verified download cache for archives with `https` artifact sources.
//!
//! The layout follows Scoop's download cache: one file per archive, named
//! `<game>#<mod>#<version>#<url hash><extension>`, kept for reinstalls.
//! Unlike Scoop, Modn checks the BLAKE3 digest pinned by the package manifest
//! before a download becomes a cache entry, and again on every cache hit. A
//! cache entry is therefore always a verified artifact. Fetching is supplied
//! by the caller, so this module needs no network client.
use crate::error::{coded, ErrorCode, ResultExt};
use crate::progress::{DownloadReader, OperationControl};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

mod digest;
mod entry_name;
mod git;
mod index;
mod nexus;
mod provider;
use digest::{copy_blake3, file_blake3, file_has_blake3};
use entry_name::{component, extension, parse_entry_name};
pub use git::{GitAcquired, GitCacheKey, GitSource};
use index::{ArchiveIndex, ArchivePin};
pub use nexus::{
    NexusApiKey, NexusDownloadGrant, NexusDownloadUrl, NexusFile, NexusGameDomain, NexusRequest,
};
pub use provider::GithubReleaseAsset;

/// Largest archive Modn downloads; a longer response is refused.
pub const MAX_DOWNLOAD_BYTES: u64 = 4 << 30;

/// One release's archive, as the package manifest pins it.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct CacheKey<'a> {
    pub game_id: &'a str,
    /// Package identifier within its bucket and game.
    pub mod_id: &'a str,
    pub version: &'a str,
    /// `https` artifact source.
    pub url: &'a str,
    /// BLAKE3 archive pin as 64 lowercase hex characters.
    pub blake3: &'a str,
}

#[expect(missing_docs, reason = "Method names state their behavior")]
impl<'a> CacheKey<'a> {
    #[must_use]
    pub fn new(
        game_id: &'a str,
        mod_id: &'a str,
        version: &'a str,
        url: &'a str,
        blake3: &'a str,
    ) -> Self {
        Self {
            game_id,
            mod_id,
            version,
            url,
            blake3,
        }
    }
}

#[expect(missing_docs, reason = "The name states the item's meaning")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CacheUse {
    /// Use a verified entry; download only when none exists.
    Reuse,
    /// Download again and replace any entry, as Scoop's `--no-cache` does.
    Refresh,
}

#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Origin {
    Cache,
    Download,
}

/// A verified archive in the cache.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Acquired {
    pub path: PathBuf,
    pub origin: Origin,
}

/// One file in the cache, as `modn cache show` lists it.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct CacheEntry {
    pub game_id: String,
    pub mod_id: String,
    pub version: String,
    pub bytes: u64,
    pub path: PathBuf,
}

#[expect(missing_docs, reason = "The name states the item's meaning")]
#[derive(Debug, Clone)]
pub struct ArchiveCache {
    root: PathBuf,
}

#[expect(missing_docs, reason = "Method names state their behavior")]
impl ArchiveCache {
    /// Use `root` as the cache directory; it is created on first download.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Entry path for `key`: `<game>#<mod>#<version>#<url hash><extension>`.
    #[must_use]
    pub fn path(&self, key: &CacheKey<'_>) -> PathBuf {
        let url_hash = &blake3::hash(key.url.as_bytes()).to_hex()[..7];
        self.root.join(format!(
            "{}#{}#{}#{url_hash}{}",
            component(key.game_id),
            component(key.mod_id),
            component(key.version),
            extension(key.url)
        ))
    }

    /// Return the verified entry for `key`, if one exists.
    ///
    /// This only reads. An entry whose digest differs from the manifest is
    /// never returned; the next download replaces it.
    pub fn cached(&self, key: &CacheKey<'_>) -> Result<Option<PathBuf>> {
        let path = self.path(key);
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            return self.cached_alias(key);
        };
        if !metadata.is_file() {
            bail!("cache entry is not a regular file: {}", path.display());
        }
        if file_blake3(&path)? == key.blake3 {
            return Ok(Some(path));
        }
        self.cached_alias(key)
    }

    /// Find an entry with the requested digest through the on-disk index.
    ///
    /// A recorded entry is used only after its bytes match the pin again. A
    /// missing, stale or corrupt index falls back to a scan whose result stays
    /// in memory. A lookup never writes, so a preflight, a cancelled operation
    /// or a refused one leaves every cache file unchanged.
    fn cached_alias(&self, key: &CacheKey<'_>) -> Result<Option<PathBuf>> {
        let Some(pin) = ArchivePin::blake3(key.blake3) else {
            return Ok(None);
        };
        let existing = ArchiveIndex::read(&self.root);
        if let Some(name) = existing
            .as_ref()
            .and_then(|index| index.file_name(&pin).map(str::to_owned))
        {
            let path = self.root.join(name);
            if file_has_blake3(&path, key.blake3)? {
                return Ok(Some(path));
            }
        }
        self.scan_for(key.blake3)
    }

    /// Hash each listed entry once and return the first whose bytes match.
    fn scan_for(&self, blake3: &str) -> Result<Option<PathBuf>> {
        for entry in self.entries(None)? {
            let Ok(digest) = file_blake3(&entry.path) else {
                continue;
            };
            if digest == blake3 {
                return Ok(Some(entry.path));
            }
        }
        Ok(None)
    }

    /// Return a verified archive for `key`, downloading it when needed.
    ///
    /// `fetch` opens the response body for the URL. The body is written to
    /// `<entry>.download`, checked against the pinned digest and the size
    /// limit, then renamed into place. A failed check removes the partial file.
    pub fn acquire<R: Read>(
        &self,
        key: &CacheKey<'_>,
        cache_use: CacheUse,
        fetch: impl FnOnce(&str) -> Result<R>,
    ) -> Result<Acquired> {
        self.acquire_limited(key, cache_use, MAX_DOWNLOAD_BYTES, fetch)
    }

    /// Return a verified archive for `key`, reporting progress and honouring
    /// cancellation through `control`.
    ///
    /// `fetch` returns the response body and its expected length, when the
    /// source states one. Every [`crate::progress::ProgressEvent::Download`]
    /// event names `package`. A cancellation before the commit point stops
    /// the read, and the cache keeps no partial entry.
    ///
    /// # Errors
    ///
    /// Returns the error of `fetch`, a [`crate::progress::Cancelled`] wrapped
    /// in an I/O error, or [`ErrorCode::DigestMismatch`].
    pub fn acquire_with_control<R: Read>(
        &self,
        key: &CacheKey<'_>,
        cache_use: CacheUse,
        package: Option<String>,
        control: &OperationControl,
        fetch: impl FnOnce(&str) -> Result<(R, Option<u64>)>,
    ) -> Result<Acquired> {
        self.acquire(key, cache_use, |url| {
            let (body, total) = fetch(url)?;
            Ok(DownloadReader::new(
                body,
                total,
                package.clone(),
                control.clone(),
            )?)
        })
    }

    fn acquire_limited<R: Read>(
        &self,
        key: &CacheKey<'_>,
        cache_use: CacheUse,
        limit: u64,
        fetch: impl FnOnce(&str) -> Result<R>,
    ) -> Result<Acquired> {
        if cache_use == CacheUse::Reuse {
            if let Some(path) = self.cached(key)? {
                return Ok(Acquired {
                    path,
                    origin: Origin::Cache,
                });
            }
        }
        fs::create_dir_all(&self.root)
            .with_context(|| format!("cannot create cache directory {}", self.root.display()))?;
        let path = self.path(key);
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let mut partial = tempfile::Builder::new()
            .prefix(&format!("{name}."))
            .suffix(".download")
            .tempfile_in(&self.root)?;
        let body = fetch(key.url)
            .with_context(|| format!("download failed: {}", key.url))
            .code(ErrorCode::DownloadFailed)?;
        let (bytes, actual) = copy_blake3(body, partial.as_file_mut(), limit)
            .with_context(|| format!("download failed: {}", key.url))
            .code(ErrorCode::DownloadFailed)?;
        if bytes > limit {
            return Err(coded(
                ErrorCode::DownloadFailed,
                format!("download exceeds {limit} bytes: {}", key.url),
            ));
        }
        if actual != key.blake3 {
            return Err(coded(
                ErrorCode::DigestMismatch,
                format!(
                    "BLAKE3 check failed for {}/{} {}\nURL:      {}\nExpected: {}\nActual:   {}",
                    key.game_id, key.mod_id, key.version, key.url, key.blake3, actual
                ),
            ));
        }
        partial.as_file().sync_all()?;
        partial
            .persist(&path)
            .with_context(|| format!("cannot store cache entry {}", path.display()))?;
        if let (Some(pin), Some(name)) = (ArchivePin::blake3(key.blake3), path.file_name()) {
            ArchiveIndex::record(&self.root, pin, name.to_string_lossy().into_owned());
        }
        Ok(Acquired {
            path,
            origin: Origin::Download,
        })
    }

    /// List cache entries, optionally only those of one mod, sorted by name.
    pub fn entries(&self, mod_id: Option<&str>) -> Result<Vec<CacheEntry>> {
        let reader = match fs::read_dir(&self.root) {
            Ok(reader) => reader,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err.into()),
        };
        let mut entries = Vec::new();
        for item in reader {
            let item = item?;
            let metadata = item.metadata()?;
            let name = item.file_name().to_string_lossy().into_owned();
            let Some([game_id, entry_mod, version]) = parse_entry_name(&name) else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            if mod_id.is_some_and(|wanted| component(wanted) != *entry_mod) {
                continue;
            }
            entries.push(CacheEntry {
                game_id: game_id.to_owned(),
                mod_id: entry_mod.to_owned(),
                version: version.to_owned(),
                bytes: metadata.len(),
                path: item.path(),
            });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }

    /// Remove the entries [`ArchiveCache::entries`] lists for the same filter.
    pub fn remove(&self, mod_id: Option<&str>) -> Result<Vec<CacheEntry>> {
        let entries = self.entries(mod_id)?;
        for entry in &entries {
            fs::remove_file(&entry.path)
                .with_context(|| format!("cannot remove {}", entry.path.display()))?;
        }
        Ok(entries)
    }
}

#[cfg(test)]
mod tests;
