#[path = "nexus_download.rs"]
pub(super) mod nexus_download;

use crate::catalog_install::reference_of;
use crate::commands::CacheCommand;
use crate::git_source;
use crate::nexus_auth;
use crate::nxm::NxmLink;
use crate::progress::Progress;
use anyhow::{bail, Context, Result};
use modn_core::acquisition::{
    ArchiveCache, CacheKey, CacheUse, GitCacheKey, GithubReleaseAsset, NexusFile,
};
use modn_core::buckets::{BucketPackage, PackageSource};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const READ_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(60);

#[cfg(test)]
thread_local! {
    pub(super) static TEST_RESPONSES: std::cell::RefCell<Option<BTreeMap<String, Vec<u8>>>> = const { std::cell::RefCell::new(None) };
    pub(super) static TEST_CALLS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
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
    matches!(
        release.package.primary_source(),
        PackageSource::Https { .. }
            | PackageSource::GithubRelease { .. }
            | PackageSource::Git { .. }
            | PackageSource::Nexus { .. }
    )
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
    sha256: &str,
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
            let key = CacheKey::new(game_id, mod_id, version, &url, sha256);
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
                Some((path, cached)) if cached == sha256 => Ok(path),
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
            let key = CacheKey::new(game_id, mod_id, version, &url, sha256);
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
    let key = CacheKey::new(game_id, mod_id, version, &url, sha256);
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
            digests.insert(reference, archive.sha256);
            continue;
        }
        if let Some(path) = source_url(release)?
            .map(|url| cache.cached(&key(release, &url)))
            .transpose()?
            .flatten()
        {
            archives.insert(reference, path);
        }
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "the download step keeps the verified cache, package set and progress explicit"
)]
pub(super) fn download(
    cache: &ArchiveCache,
    cache_use: CacheUse,
    releases: &[&BucketPackage],
    archives: &mut BTreeMap<String, PathBuf>,
    digests: &mut BTreeMap<String, String>,
    links: &[NxmLink],
    json: bool,
    progress: &Progress,
) -> Result<()> {
    for release in releases {
        let reference = reference_of(release).to_string();
        if archives.contains_key(&reference) {
            continue;
        }
        if let PackageSource::Git { repository, commit } = release.package.primary_source() {
            if !json && !progress.enabled() {
                eprintln!(
                    "Fetching {reference} {} commit {commit} from {repository}",
                    release.package.version
                );
            }
            let archive = git_source::acquire(cache, release, cache_use, progress)?;
            archives.insert(reference.clone(), archive.path);
            digests.insert(reference, archive.sha256);
            continue;
        }
        if let PackageSource::Nexus {
            game_domain,
            mod_id,
            file_id,
            file_name,
        } = release.package.primary_source()
        {
            let file = NexusFile::new(game_domain, *mod_id, *file_id, file_name)?;
            let link = nexus_download::matching_link(links, game_domain, *mod_id, *file_id);
            let url = file.cache_url();
            let key = key(release, &url);
            if !json && !progress.enabled() {
                eprintln!(
                    "Downloading {reference} {} from {}",
                    key.version,
                    nexus_label(game_domain, *mod_id, *file_id, file_name)
                );
            }
            if cache_use == CacheUse::Reuse {
                if let Some(path) = cache.cached(&key)? {
                    archives.insert(reference, path);
                    continue;
                }
            }
            let api_key = nexus_auth::load_key()?;
            let download_url = nexus_download::resolve(&file, &api_key, link)?;
            let acquired = cache.acquire_with_control(
                &key,
                cache_use,
                Some(reference.clone()),
                progress.control(),
                |_cache_url| nexus_download::open(&download_url),
            )?;
            archives.insert(reference, acquired.path);
            continue;
        }
        let Some(url) = source_url(release)? else {
            continue;
        };
        if let PackageSource::GithubRelease {
            repository,
            tag,
            asset,
        } = release.package.primary_source()
        {
            let provider = GithubReleaseAsset::new(repository.clone(), tag.clone(), asset.clone());
            let resolved = provider.resolve(fetch_metadata)?;
            if resolved != url {
                bail!("provider source changed during resolution");
            }
        }
        let key = key(release, &url);
        if !json && !progress.enabled() {
            eprintln!("Downloading {reference} {} from {}", key.version, key.url);
        }
        let acquired = cache.acquire_with_control(
            &key,
            cache_use,
            Some(reference.clone()),
            progress.control(),
            fetch_measured,
        )?;
        archives.insert(reference, acquired.path);
    }
    Ok(())
}

fn fetch_metadata(url: &str) -> Result<Box<dyn Read>> {
    #[cfg(test)]
    if let Some(response) = fixture_response(url) {
        return response;
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(0)
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_per_read(Some(READ_INACTIVITY_TIMEOUT))
        .user_agent(concat!("modn/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let response = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call()?;
    if response.status() != 200 {
        bail!("GitHub metadata request returned {}", response.status());
    }
    Ok(Box::new(response.into_body().into_reader()))
}

pub(super) fn fetch(url: &str) -> Result<Box<dyn Read>> {
    Ok(fetch_measured(url)?.0)
}

fn fetch_measured(url: &str) -> Result<(Box<dyn Read>, Option<u64>)> {
    #[cfg(test)]
    if let Some(response) = fixture_response(url) {
        return Ok((response?, None));
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_per_read(Some(READ_INACTIVITY_TIMEOUT))
        .user_agent(concat!("modn/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let response = agent.get(url).call()?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok());
    Ok((Box::new(response.into_body().into_reader()), total))
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
        println!(
            "{} {} {} ({} bytes)",
            entry.game_id, entry.mod_id, entry.version, entry.bytes
        );
    }
    let verb = if removed { "Removed" } else { "Total" };
    println!("{verb}: {} file(s), {bytes} bytes", entries.len());
    Ok(())
}

#[cfg(test)]
mod tests;
