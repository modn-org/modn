mod acquire;
mod fallback;
mod http;
#[path = "nexus_download.rs"]
pub(super) mod nexus_download;

pub(super) use acquire::download;

use crate::catalog_install::reference_of;
use crate::commands::CacheCommand;
use crate::git_source;
use crate::nxm::NxmLink;
use crate::progress::Progress;
use anyhow::{bail, Context, Result};
pub(crate) use http::fetch;
use http::{fetch_measured, fetch_metadata, HttpTransport};
use modn_core::acquisition::SourceTransport;
use modn_core::acquisition::{
    ArchiveCache, CacheKey, CacheUse, FetchError, GitCacheKey, GithubReleaseAsset, NexusFile,
    SourceAttempt, SourcePolicy,
};
use modn_core::buckets::{BucketPackage, PackageSource};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

#[cfg(test)]
thread_local! {
    pub(super) static TEST_RESPONSES: std::cell::RefCell<Option<BTreeMap<String, Vec<u8>>>> = const { std::cell::RefCell::new(None) };
    pub(super) static TEST_CALLS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    pub(super) static TEST_METADATA_FAILURES: std::cell::RefCell<Option<BTreeMap<String, Vec<FetchError>>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn take_metadata_failure(url: &str) -> Option<FetchError> {
    TEST_METADATA_FAILURES.with(|failures| {
        let mut failures = failures.borrow_mut();
        let queue = failures.as_mut()?.get_mut(url)?;
        if queue.is_empty() {
            return None;
        }
        TEST_CALLS.with(|calls| calls.borrow_mut().push(url.to_owned()));
        Some(queue.remove(0))
    })
}

#[cfg(test)]
fn fixture_response(url: &str) -> Option<Result<Box<dyn Read>>> {
    TEST_RESPONSES.with(|responses| {
        responses.borrow().as_ref().map(|responses| {
            TEST_CALLS.with(|calls| calls.borrow_mut().push(url.to_owned()));
            responses
                .get(url)
                .cloned()
                .map(|bytes| Box::new(std::io::Cursor::new(bytes)) as Box<dyn Read>)
                .with_context(|| format!("unexpected fixture request: {url}"))
        })
    })
}

fn retryable(error: &FetchError) -> bool {
    match error {
        FetchError::Timeout | FetchError::Network(_) => true,
        FetchError::Http(status) => (500..600).contains(status),
        _ => false,
    }
}

/// Classify an I/O failure from a metadata body read like its transport call.
///
/// The HTTP layer returns body reads as plain I/O errors. A timeout, a reset,
/// an aborted connection, a broken pipe or an unexpected end of body is the
/// same transient transport failure as an equivalent error from the open step.
fn transport_io(error: &std::io::Error) -> Option<FetchError> {
    match error.kind() {
        std::io::ErrorKind::TimedOut => Some(FetchError::Timeout),
        std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::ConnectionAborted
        | std::io::ErrorKind::BrokenPipe
        | std::io::ErrorKind::UnexpectedEof => Some(FetchError::Network(error.to_string())),
        _ => None,
    }
}
/// Fetch one catalog ZIP with the shared HTTPS client and source retry policy.
pub(super) fn fetch_catalog(url: &str) -> anyhow::Result<Box<dyn Read>> {
    let policy = SourcePolicy::default();
    let mut attempt = 0;
    loop {
        match HttpTransport.open(url, &policy) {
            Ok(body) => return Ok(body.reader),
            Err(error) if attempt < policy.retries() && retryable(&error) => attempt += 1,
            Err(error) => bail!("catalog download failed: {error}"),
        }
    }
}
pub(super) fn cache_for(state_path: &Path) -> Result<ArchiveCache> {
    let root = match std::env::var_os("MODN_CACHE") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => state_path
            .parent()
            .context("invalid manager state directory")?
            .join("cache"),
    };
    Ok(ArchiveCache::new(root))
}

fn source_url(release: &BucketPackage) -> Result<Option<String>> {
    Ok(match release.package.primary_source() {
        PackageSource::Https { url } => Some(url.clone()),
        PackageSource::GithubRelease {
            repository,
            tag,
            asset,
        } => Some(format!(
            "https://github.com/{repository}/releases/download/{tag}/{asset}"
        )),
        PackageSource::Nexus {
            game_domain,
            mod_id,
            file_id,
            file_name,
        } => Some(NexusFile::new(game_domain, *mod_id, *file_id, file_name)?.cache_url()),
        PackageSource::Manual { .. } | _ => None,
    })
}

fn key<'a>(release: &'a BucketPackage, url: &'a str) -> CacheKey<'a> {
    CacheKey::new(
        &release.package.game,
        &release.package.id,
        &release.package.version,
        url,
        release.package.pin().unwrap_or_default(),
    )
}

pub(super) fn downloadable(release: &BucketPackage) -> bool {
    release.package.sources.iter().any(|source| {
        matches!(
            source,
            PackageSource::Https { .. }
                | PackageSource::GithubRelease { .. }
                | PackageSource::Git { .. }
                | PackageSource::Nexus { .. }
        )
    })
}

/// Refuse `--nxm` links that match no package this command installs or updates.
pub(super) fn validate_links(links: &[NxmLink], releases: &[&BucketPackage]) -> Result<()> {
    let packages: Vec<(&str, u64, u64)> = releases
        .iter()
        .filter_map(|release| match release.package.primary_source() {
            PackageSource::Nexus {
                game_domain,
                mod_id,
                file_id,
                ..
            } => Some((game_domain.as_str(), *mod_id, *file_id)),
            _ => None,
        })
        .collect();
    nexus_download::validate_links(links, &packages)
}

fn nexus_label(game_domain: &str, mod_id: u64, file_id: u64, file_name: &str) -> String {
    format!("Nexus Mods: {game_domain} mod {mod_id} file {file_id} ({file_name})")
}

/// A download consent label that never includes a secret-bearing URL.
pub(super) fn source_label(release: &BucketPackage) -> Option<String> {
    match release.package.primary_source() {
        PackageSource::Nexus {
            game_domain,
            mod_id,
            file_id,
            file_name,
        } => Some(nexus_label(game_domain, *mod_id, *file_id, file_name)),
        _ => source_url(release).ok().flatten(),
    }
}

/// Archive digest the pipeline uses for one release.
///
/// Git sources publish no manifest digest; their packed ZIP digest becomes
/// known after a cache hit or fetch and is stored in `digests`.
pub(super) fn digest<'a>(
    release: &'a BucketPackage,
    digests: &'a BTreeMap<String, String>,
) -> Option<&'a str> {
    digests
        .get(&reference_of(release).to_string())
        .map(String::as_str)
        .or(release.package.pin())
}

/// Acquire the archive of one recorded release source for undo.
///
/// A verified cache entry needs no network. A `manual` source is refused
/// before this call. `git` and `nexus` sources are restored only from the
/// archive cache; a miss refuses before any change.
pub(super) fn acquire_source(
    cache: &ArchiveCache,
    game_id: &str,
    mod_id: &str,
    version: &str,
    source: &PackageSource,
    blake3: &str,
    progress: &Progress,
) -> Result<PathBuf> {
    let url = match source {
        PackageSource::Https { url } => url.clone(),
        PackageSource::GithubRelease {
            repository,
            tag,
            asset,
        } => {
            let url = format!("https://github.com/{repository}/releases/download/{tag}/{asset}");
            let key = CacheKey::new(game_id, mod_id, version, &url, blake3);
            if let Some(path) = cache.cached(&key)? {
                return Ok(path);
            }
            let provider = GithubReleaseAsset::new(repository.clone(), tag.clone(), asset.clone());
            if provider.resolve(fetch_metadata)? != url {
                bail!("provider source changed during resolution");
            }
            url
        }
        PackageSource::Git { commit, .. } => {
            let key = GitCacheKey::new(game_id, mod_id, version, commit);
            return match cache.git_cached(&key)? {
                Some((path, cached)) if cached == blake3 => Ok(path),
                _ => Err(cache_miss(
                    "this git release cannot be fetched again; reinstall it from the catalog",
                )),
            };
        }
        PackageSource::Nexus {
            game_domain,
            mod_id: nexus_mod_id,
            file_id,
            file_name,
        } => {
            let url = NexusFile::new(game_domain, *nexus_mod_id, *file_id, file_name)?.cache_url();
            let key = CacheKey::new(game_id, mod_id, version, &url, blake3);
            return match cache.cached(&key)? {
                Some(path) => Ok(path),
                None => Err(cache_miss(
                    "a Nexus download needs a new nxm link; reinstall it from the catalog",
                )),
            };
        }
        PackageSource::Manual { .. } => {
            return Err(modn_core::error::coded(
                modn_core::error::ErrorCode::UndoSourceManual,
                "a manual source cannot be fetched again",
            ));
        }
        _ => bail!("unknown release source"),
    };
    let key = CacheKey::new(game_id, mod_id, version, &url, blake3);
    let acquired = cache.acquire_with_control(
        &key,
        CacheUse::Reuse,
        Some(format!("{game_id}/{mod_id}")),
        progress.control(),
        fetch_measured,
    )?;
    Ok(acquired.path)
}

fn cache_miss(detail: &str) -> anyhow::Error {
    modn_core::error::coded(
        modn_core::error::ErrorCode::UndoFailed,
        format!("the recorded archive is no longer cached and {detail}"),
    )
}

/// JSON outcomes for a list of source attempts.
pub(super) fn downloads_json(attempts: &[SourceAttempt]) -> Vec<serde_json::Value> {
    attempts
        .iter()
        .map(|attempt| {
            serde_json::json!({
                "source": attempt.source,
                "attempt": attempt.attempt,
                "outcome": if attempt.succeeded() { "ok" } else { "failed" },
                "reason": attempt.failure.as_ref().map(ToString::to_string),
            })
        })
        .collect()
}

pub(super) fn add_cached<'a>(
    cache: &ArchiveCache,
    releases: impl IntoIterator<Item = &'a BucketPackage>,
    archives: &mut BTreeMap<String, PathBuf>,
    digests: &mut BTreeMap<String, String>,
) -> Result<()> {
    for release in releases {
        let reference = reference_of(release).to_string();
        if archives.contains_key(&reference) {
            continue;
        }
        if let Some(archive) = git_source::cached(cache, release)? {
            archives.insert(reference.clone(), archive.path);
            digests.insert(reference, archive.blake3);
            continue;
        }
        for url in fallback::urls(release) {
            if let Some(path) = cache.cached(&key(release, &url))? {
                archives.insert(reference, path);
                break;
            }
        }
    }
    Ok(())
}

pub(super) fn run(command: CacheCommand, state_path: &Path, json: bool) -> Result<()> {
    let cache = cache_for(state_path)?;
    let (entries, removed) = match command {
        CacheCommand::Show { mod_id } => (cache.entries(mod_id.as_deref())?, false),
        CacheCommand::Rm { mod_id, all } => {
            let filter = match (mod_id, all) {
                (Some(_), true) => bail!("choose a mod ID or --all, not both"),
                (None, false) => bail!("name a mod ID to remove, or pass --all"),
                (mod_id, _) => mod_id,
            };
            (cache.remove(filter.as_deref())?, true)
        }
    };
    let bytes: u64 = entries.iter().map(|entry| entry.bytes).sum();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "cache": cache.root(),
                "removed": removed,
                "entries": entries,
                "bytes": bytes,
            }))?
        );
        return Ok(());
    }
    for entry in &entries {
        let retired = if entry.retired { ", retired" } else { "" };
        println!(
            "{} {} {} ({} bytes{retired})",
            entry.game_id, entry.mod_id, entry.version, entry.bytes
        );
    }
    let verb = if removed { "Removed" } else { "Total" };
    println!("{verb}: {} file(s), {bytes} bytes", entries.len());
    Ok(())
}

#[cfg(test)]
mod tests;
