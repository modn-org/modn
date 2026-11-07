use crate::catalog_install::reference_of;
use crate::commands::CacheCommand;
use crate::git_source;
use anyhow::{bail, Context, Result};
use modn_core::acquisition::{ArchiveCache, CacheKey, CacheUse, GithubReleaseAsset};
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

fn source_url(release: &BucketPackage) -> Option<String> {
    match &release.package.source {
        PackageSource::Https { url } => Some(url.clone()),
        PackageSource::GithubRelease {
            repository,
            tag,
            asset,
        } => Some(format!(
            "https://github.com/{repository}/releases/download/{tag}/{asset}"
        )),
        PackageSource::Manual { .. } | _ => None,
    }
}

fn key<'a>(release: &'a BucketPackage, url: &'a str) -> CacheKey<'a> {
    CacheKey::new(
        &release.package.game,
        &release.package.id,
        &release.package.version,
        url,
        release.package.sha256.as_deref().unwrap_or_default(),
    )
}

pub(super) fn downloadable(release: &BucketPackage) -> bool {
    match release.package.source {
        PackageSource::Https { .. }
        | PackageSource::GithubRelease { .. }
        | PackageSource::Git { .. } => true,
        _ => false,
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
        .or(release.package.sha256.as_deref())
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
        if let Some(path) = source_url(release)
            .map(|url| cache.cached(&key(release, &url)))
            .transpose()?
            .flatten()
        {
            archives.insert(reference, path);
        }
    }
    Ok(())
}

pub(super) fn download(
    cache: &ArchiveCache,
    cache_use: CacheUse,
    releases: &[&BucketPackage],
    archives: &mut BTreeMap<String, PathBuf>,
    digests: &mut BTreeMap<String, String>,
    json: bool,
) -> Result<()> {
    for release in releases {
        let reference = reference_of(release).to_string();
        if archives.contains_key(&reference) {
            continue;
        }
        if let PackageSource::Git {
            repository,
            commit,
        } = &release.package.source
        {
            if !json {
                eprintln!(
                    "Fetching {reference} {} commit {commit} from {repository}",
                    release.package.version
                );
            }
            let archive = git_source::acquire(cache, release, cache_use)?;
            archives.insert(reference.clone(), archive.path);
            digests.insert(reference, archive.sha256);
            continue;
        }
        let Some(url) = source_url(release) else {
            continue;
        };
        if let PackageSource::GithubRelease {
            repository,
            tag,
            asset,
        } = &release.package.source
        {
            let provider = GithubReleaseAsset::new(repository.clone(), tag.clone(), asset.clone());
            let resolved = provider.resolve(fetch_metadata)?;
            if resolved != url {
                bail!("provider source changed during resolution");
            }
        }
        let key = key(release, &url);
        if !json {
            eprintln!("Downloading {reference} {} from {}", key.version, key.url);
        }
        let acquired = cache.acquire(&key, cache_use, fetch)?;
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
    #[cfg(test)]
    if let Some(response) = fixture_response(url) {
        return response;
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_per_read(Some(READ_INACTIVITY_TIMEOUT))
        .user_agent(concat!("modn/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let response = agent.get(url).call()?;
    Ok(Box::new(response.into_body().into_reader()))
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
