//! Verified download cache for archives with `https` artifact sources.
//!
//! The layout follows Scoop's download cache: one file per archive, named
//! `<game>#<mod>#<version>#<url hash><extension>`, kept for reinstalls.
//! Unlike Scoop, Modn checks the SHA-256 digest pinned by the package manifest
//! before a download becomes a cache entry, and again on every cache hit. A
//! cache entry is therefore always a verified artifact. Fetching is supplied
//! by the caller, so this module needs no network client.
use anyhow::{bail, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

mod git;
mod provider;
pub use git::{GitAcquired, GitCacheKey, GitSource};
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
    pub sha256: &'a str,
}

#[expect(missing_docs, reason = "Method names state their behavior")]
impl<'a> CacheKey<'a> {
    #[must_use]
    pub fn new(
        game_id: &'a str,
        mod_id: &'a str,
        version: &'a str,
        url: &'a str,
        sha256: &'a str,
    ) -> Self {
        Self {
            game_id,
            mod_id,
            version,
            url,
            sha256,
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
        let url_hash = &hex(&Sha256::digest(key.url.as_bytes()))[..7];
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
        if file_sha256(&path)? == key.sha256 {
            return Ok(Some(path));
        }
        self.cached_alias(key)
    }

    fn cached_alias(&self, key: &CacheKey<'_>) -> Result<Option<PathBuf>> {
        for entry in self.entries(None)? {
            if fs::symlink_metadata(&entry.path)?.file_type().is_file()
                && file_sha256(&entry.path)? == key.sha256
            {
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
        let body = fetch(key.url).with_context(|| format!("download failed: {}", key.url))?;
        let (bytes, actual) = copy_hashed(body, partial.as_file_mut(), limit)
            .with_context(|| format!("download failed: {}", key.url))?;
        if bytes > limit {
            bail!("download exceeds {limit} bytes: {}", key.url);
        }
        if actual != key.sha256 {
            bail!(
                "hash check failed for {}/{} {}\nURL:      {}\nExpected: {}\nActual:   {}",
                key.game_id,
                key.mod_id,
                key.version,
                key.url,
                key.sha256,
                actual
            );
        }
        partial.as_file().sync_all()?;
        partial
            .persist(&path)
            .with_context(|| format!("cannot store cache entry {}", path.display()))?;
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

fn component(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn extension(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    let name = path.rsplit('/').next().unwrap_or_default();
    match name.rsplit_once('.') {
        Some((_, ext)) if (1..=8).contains(&ext.len()) => format!(".{}", component(ext)),
        _ => String::new(),
    }
}

fn parse_entry_name(name: &str) -> Option<[&str; 3]> {
    let parts: Vec<&str> = name.split('#').collect();
    let [game_id, mod_id, version, rest] = parts.as_slice() else {
        return None;
    };
    let named = |value: &str| {
        !value.is_empty()
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    };
    let hex = |value: &str, len: usize| {
        value.len() == len
            && value
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    };
    let length_and_extension = |ext: &str| {
        ext.is_empty()
            || ext.strip_prefix('.').is_some_and(|ext| {
                (1..=8).contains(&ext.len())
                    && ext
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
            })
    };
    let rest = *rest;
    let valid_rest = if let Some(git) = rest.strip_prefix("git-") {
        git.split_once('-').is_some_and(|(commit, archive)| {
            hex(commit, 40)
                && archive
                    .strip_suffix(".zip")
                    .is_some_and(|digest| hex(digest, 64))
        })
    } else {
        rest.split_at_checked(7).is_some_and(|(url_hash, ext)| {
            hex(url_hash, 7) && length_and_extension(ext)
        })
    };
    (named(game_id) && named(mod_id) && named(version) && valid_rest)
        .then_some([*game_id, *mod_id, *version])
}

fn copy_hashed(body: impl Read, output: &mut File, limit: u64) -> Result<(u64, String)> {
    let mut reader = body.take(limit + 1);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut total = 0;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        output.write_all(&buffer[..read])?;
        total += read as u64;
    }
    Ok((total, hex(&hasher.finalize())))
}

fn file_sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests;
