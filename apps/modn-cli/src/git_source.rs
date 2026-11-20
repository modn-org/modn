//! Resolve archives for `git` package sources without bypassing the commit pin.
//!
//! Lookups read the commit-keyed cache only. A fetch runs only when the caller
//! has explicit download or installation consent, like other remote sources.
use anyhow::Result;
use modn_core::acquisition::{ArchiveCache, CacheUse, GitCacheKey, GitSource, Origin};
use modn_core::buckets::BucketPackage;
use modn_core::progress::{OperationControl, ProgressEvent};
use std::fs;
use std::path::PathBuf;

pub(super) struct GitArchive {
    pub(super) path: PathBuf,
    pub(super) blake3: String,
}

fn key<'a>(release: &'a BucketPackage, source: &'a GitSource) -> GitCacheKey<'a> {
    GitCacheKey::new(
        &release.package.game,
        &release.package.id,
        &release.package.version,
        &source.commit,
    )
}

pub(super) fn cached_source(
    cache: &ArchiveCache,
    release: &BucketPackage,
    source: &GitSource,
) -> Result<Option<GitArchive>> {
    let key = key(release, source);
    Ok(cache
        .git_cached(&key)?
        .map(|(path, blake3)| GitArchive { path, blake3 }))
}

/// Names of the cache entries a fetch of `key` may replace.
///
/// A Git entry name ends with `git-<commit>-blake3-<digest>.zip`, so a probe path with
/// a placeholder digest yields the prefix shared by entries of this release.
fn entry_names(cache: &ArchiveCache, key: &GitCacheKey<'_>) -> Result<Vec<String>> {
    let placeholder = "0".repeat(64);
    let probe = cache.git_path(key, &placeholder);
    let Some(name) = probe
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
    else {
        return Ok(Vec::new());
    };
    let Some(prefix) = name.strip_suffix(&format!("{placeholder}.zip")) else {
        return Ok(Vec::new());
    };
    Ok(cache
        .entries(None)?
        .into_iter()
        .filter_map(|entry| {
            entry
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .filter(|name| name.starts_with(prefix))
        .collect())
}

/// Return a verified cached archive or fetch and pack the pinned commit.
///
/// A fetch reports one `download` phase for the package. The phase starts at
/// zero bytes, and ends with the packed archive size. A cancellation before
/// the fetch starts returns [`modn_core::progress::Cancelled`] without running
/// Git. A cancellation observed after the fetch and pack finishes removes the
/// newly fetched cache entry, or keeps a previous entry it replaced, so a
/// cancelled install changes no cache state for either cache use.
/// Return a verified cached archive, or fetch `source`'s pinned commit.
///
/// The ordered download loop names each Git source of a release, so the
/// attempt uses that entry and not the record's primary source.
pub(super) fn acquire_named(
    cache: &ArchiveCache,
    release: &BucketPackage,
    repository: &str,
    commit: &str,
    cache_use: CacheUse,
    control: &OperationControl,
) -> Result<GitArchive> {
    let source = GitSource::new(repository.to_owned(), commit.to_owned());
    acquire_source(cache, release, &source, cache_use, control)
}

fn acquire_source(
    cache: &ArchiveCache,
    release: &BucketPackage,
    source: &GitSource,
    cache_use: CacheUse,
    control: &OperationControl,
) -> Result<GitArchive> {
    let key = key(release, source);
    if cache_use == CacheUse::Reuse {
        if let Some(cached) = cached_source(cache, release, source)? {
            return Ok(cached);
        }
    }
    let reference = release.reference().to_string();
    control.advance(ProgressEvent::Download {
        package: Some(reference.clone()),
        bytes: 0,
        total: None,
    })?;
    let previous = entry_names(cache, &key)?;
    let acquired = source.acquire(cache, &key, cache_use)?;
    let archive = GitArchive {
        path: acquired.path,
        blake3: acquired.blake3,
    };
    if let Err(cancelled) = control.checkpoint() {
        if acquired.origin == Origin::Download {
            let replaced = archive.path.file_name().is_some_and(|name| {
                let name = name.to_string_lossy();
                previous.iter().any(|old| old.as_str() == name.as_ref())
            });
            // An entry with the new name holds the digest that name records,
            // so keeping it preserves the bytes of the entry it replaced.
            if !replaced {
                let _ = fs::remove_file(&archive.path);
            }
        }
        return Err(cancelled.into());
    }
    if acquired.origin == Origin::Download {
        let bytes = fs::metadata(&archive.path).map_or(0, |metadata| metadata.len());
        control.report(&ProgressEvent::Download {
            package: Some(reference),
            bytes,
            total: Some(bytes),
        });
    }
    Ok(archive)
}
