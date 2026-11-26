mod acquire;
mod fallback;
mod http;
#[path = "nexus_download.rs"]
pub(super) mod nexus_download;

pub(super) use acquire::download;

use crate::catalog_install::reference_of;
use crate::commands::CacheCommand;
use crate::nxm::NxmLink;
use crate::progress::Progress;
use anyhow::{bail, Result};
pub(crate) use http::fetch;
use http::{fetch_measured, fetch_metadata, HttpTransport};
use modn_core::acquisition::SourceTransport;
use modn_core::acquisition::{
    source_cache_url, ArchiveCache, CacheKey, CacheUse, FetchError, GitCacheKey,
    GithubReleaseAsset, ReleaseDigest, SourceAttempt, SourcePolicy,
};
use modn_core::buckets::{BucketPackage, PackageSource};
use modn_core::manager_roots::ManagerRoots;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::PathBuf;

#[cfg(test)]
use anyhow::Context;

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

fn transport_io(error: &std::io::Error) -> Option<FetchError> {
    if let Some(transient) = match error.kind() {
        std::io::ErrorKind::TimedOut => Some(FetchError::Timeout),
        std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::ConnectionAborted
        | std::io::ErrorKind::BrokenPipe
        | std::io::ErrorKind::UnexpectedEof => Some(FetchError::Network(error.to_string())),
        _ => None,
    } {
        return Some(transient);
    }
    // ureq wraps a body-read timeout as `io::Error` of kind `Other` and keeps
    // the original error as the source. Classify only the transport variants;
    // a decode, policy or non-ureq error stays final.
    let ureq = error.get_ref()?.downcast_ref::<ureq::Error>()?;
    match ureq {
        ureq::Error::Timeout(_) => Some(FetchError::Timeout),
        ureq::Error::ConnectionFailed | ureq::Error::HostNotFound => {
            Some(FetchError::Network(ureq.to_string()))
        }
        ureq::Error::Io(inner) => match inner.kind() {
            std::io::ErrorKind::TimedOut => Some(FetchError::Timeout),
            std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::UnexpectedEof => Some(FetchError::Network(inner.to_string())),
            _ => None,
        },
        _ => None,
    }
}

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
fn key<'a>(release: &'a BucketPackage, url: &'a str) -> CacheKey<'a> {
    let pin = release
        .package
        .pin()
        .expect("an archive source has a validated BLAKE3 pin");
    CacheKey::new(
        &release.package.game,
        &release.package.id,
        &release.package.version,
        url,
        pin,
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

pub(super) fn validate_links(links: &[NxmLink], releases: &[&BucketPackage]) -> Result<()> {
    let packages: Vec<(&str, u64, u64)> = releases
        .iter()
        .flat_map(|release| release.package.sources.iter())
        .filter_map(|source| match source {
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

pub(super) fn source_label(source: &PackageSource) -> Option<String> {
    match source {
        PackageSource::Nexus {
            game_domain,
            mod_id,
            file_id,
            file_name,
        } => Some(nexus_label(game_domain, *mod_id, *file_id, file_name)),
        _ => source_cache_url(source).ok().flatten(),
    }
}

pub(super) fn digest(
    release: &BucketPackage,
    digests: &BTreeMap<String, String>,
) -> Result<ReleaseDigest> {
    match digests.get(&reference_of(release).to_string()) {
        Some(value) => Ok(ReleaseDigest::computed(value)?),
        None => Ok(ReleaseDigest::from_pin(release.package.pin())?),
    }
}

pub(super) fn acquire_source(
    cache: &ArchiveCache,
    game_id: &str,
    mod_id: &str,
    version: &str,
    source: &PackageSource,
    blake3: &str,
    progress: &Progress,
) -> Result<PathBuf> {
    match source {
        PackageSource::Git { commit, .. } => {
            let key = GitCacheKey::new(game_id, mod_id, version, commit);
            match cache.git_cached(&key)? {
                Some((path, cached)) if cached == blake3 => Ok(path),
                _ => Err(cache_miss(
                    "this git release cannot be fetched again; reinstall it from the catalog",
                )),
            }
        }
        PackageSource::Manual { .. } => Err(modn_core::error::coded(
            modn_core::error::ErrorCode::UndoSourceManual,
            "a manual source cannot be fetched again",
        )),
        _ => {
            let Some(url) = source_cache_url(source)? else {
                bail!("unknown release source");
            };
            let key = CacheKey::new(game_id, mod_id, version, &url, blake3);
            if matches!(source, PackageSource::Nexus { .. }) {
                return match cache.cached(&key)? {
                    Some(path) => Ok(path),
                    None => Err(cache_miss(
                        "a Nexus download needs a new nxm link; reinstall it from the catalog",
                    )),
                };
            }
            if let PackageSource::GithubRelease {
                repository,
                tag,
                asset,
            } = source
            {
                if let Some(path) = cache.cached(&key)? {
                    return Ok(path);
                }
                let provider =
                    GithubReleaseAsset::new(repository.clone(), tag.clone(), asset.clone());
                if provider.resolve(fetch_metadata)? != url {
                    bail!("provider source changed during resolution");
                }
            }
            let acquired = cache.acquire_with_control(
                &key,
                CacheUse::Reuse,
                Some(format!("{game_id}/{mod_id}")),
                progress.control(),
                fetch_measured,
            )?;
            Ok(acquired.path)
        }
    }
}

fn cache_miss(detail: &str) -> anyhow::Error {
    modn_core::error::coded(
        modn_core::error::ErrorCode::UndoFailed,
        format!("the recorded archive is no longer cached and {detail}"),
    )
}

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
    served: &mut BTreeMap<String, PackageSource>,
) -> Result<()> {
    for release in releases {
        let reference = reference_of(release).to_string();
        if archives.contains_key(&reference) {
            continue;
        }
        let Some((archive, digest, source)) = acquire::cached_archive(cache, release)? else {
            continue;
        };
        if let Some(digest) = digest {
            digests.insert(reference.clone(), digest);
        }
        served.insert(reference.clone(), source);
        archives.insert(reference, archive);
    }
    Ok(())
}

pub(super) struct SuppliedArchive {
    pub(super) path: PathBuf,
    pub(super) blake3: String,
}

pub(super) fn serving_source(
    reference: &str,
    supplied: &BTreeMap<String, SuppliedArchive>,
    served: &BTreeMap<String, PackageSource>,
    release: &BucketPackage,
) -> PackageSource {
    if let Some(file) = supplied.get(reference) {
        if release.package.pin() == Some(file.blake3.as_str()) {
            if let Some(source) = refetchable_source(&release.package.sources) {
                return source.clone();
            }
        }
        let filename = file
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        return PackageSource::Manual { filename };
    }
    served
        .get(reference)
        .cloned()
        .unwrap_or_else(|| release.package.primary_source().clone())
}

fn refetchable_source(sources: &[PackageSource]) -> Option<&PackageSource> {
    sources.iter().find(|source| {
        matches!(
            source,
            PackageSource::Https { .. } | PackageSource::GithubRelease { .. }
        )
    })
}

pub(super) fn run(command: CacheCommand, roots: &ManagerRoots, json: bool) -> Result<()> {
    let cache = roots.archive_cache();
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
mod serving_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod validation_tests;
