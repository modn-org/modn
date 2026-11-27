use anyhow::Result;
use modn_core::acquisition::{ArchiveCache, CacheUse, GitAcquired, GitCacheKey, GitSource, Origin};
use modn_core::buckets::BucketPackage;
use modn_core::progress::{OperationControl, ProgressEvent};
use std::fs;

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

/// Fetch the planned Git commit, or reuse its verified cache entry.
pub(super) fn acquire(
    cache: &ArchiveCache,
    release: &BucketPackage,
    repository: &str,
    key: &GitCacheKey<'_>,
    cache_use: CacheUse,
    control: &OperationControl,
) -> Result<GitAcquired> {
    let source = GitSource::new(repository.to_owned(), key.commit.to_owned());
    if cache_use == CacheUse::Reuse && cache.git_cached(key)?.is_some() {
        return source.acquire(cache, key, cache_use);
    }
    let reference = release.reference().to_string();
    control.advance(ProgressEvent::Download {
        package: Some(reference.clone()),
        bytes: 0,
        total: None,
    })?;
    let previous = entry_names(cache, key)?;
    let acquired = source.acquire_with_control(cache, key, cache_use, control)?;
    if let Err(cancelled) = control.checkpoint() {
        if acquired.origin == Origin::Download {
            let replaced = acquired.path.file_name().is_some_and(|name| {
                let name = name.to_string_lossy();
                previous.iter().any(|old| old.as_str() == name.as_ref())
            });
            if !replaced {
                let _ = fs::remove_file(&acquired.path);
            }
        }
        return Err(cancelled.into());
    }
    if acquired.origin == Origin::Download {
        let bytes = fs::metadata(&acquired.path).map_or(0, |metadata| metadata.len());
        control.report(&ProgressEvent::Download {
            package: Some(reference),
            bytes,
            total: Some(bytes),
        });
    }
    Ok(acquired)
}
