use crate::catalog_install::reference_of;
use crate::commands::CacheCommand;
use anyhow::{bail, Context, Result};
use modn_core::acquisition::{ArchiveCache, CacheKey, CacheUse};
use modn_core::buckets::{BucketPackage, PackageSource};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const READ_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(60);

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

fn key(release: &BucketPackage) -> Option<CacheKey<'_>> {
    match &release.package.source {
        PackageSource::Https { url } => Some(CacheKey::new(
            &release.package.game,
            &release.package.id,
            &release.package.version,
            url,
            &release.package.sha256,
        )),
        _ => None,
    }
}

pub(super) fn downloadable(release: &BucketPackage) -> bool {
    key(release).is_some()
}

pub(super) fn add_cached<'a>(
    cache: &ArchiveCache,
    releases: impl IntoIterator<Item = &'a BucketPackage>,
    archives: &mut BTreeMap<String, PathBuf>,
) -> Result<()> {
    for release in releases {
        let reference = reference_of(release).to_string();
        if archives.contains_key(&reference) {
            continue;
        }
        if let Some(path) = key(release)
            .map(|key| cache.cached(&key))
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
    json: bool,
) -> Result<()> {
    for release in releases {
        let reference = reference_of(release).to_string();
        let Some(key) = key(release) else {
            continue;
        };
        if archives.contains_key(&reference) {
            continue;
        }
        if !json {
            eprintln!("Downloading {reference} {} from {}", key.version, key.url);
        }
        let acquired = cache.acquire(&key, cache_use, fetch)?;
        archives.insert(reference, acquired.path);
    }
    Ok(())
}

pub(super) fn fetch(url: &str) -> Result<impl Read> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_per_read(Some(READ_INACTIVITY_TIMEOUT))
        .user_agent(concat!("modn/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let response = agent.get(url).call()?;
    Ok(response.into_body().into_reader())
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
