//! Ordered acquisition of release archives with text reporting and cancellation.

use super::fallback;
use super::key;
use super::nexus_label;
use crate::catalog_install::reference_of;
use crate::git_source;
use crate::nexus_auth;
use crate::nxm::NxmLink;
use crate::progress::Progress;
use modn_core::acquisition::{ArchiveCache, CacheUse, NexusFile, SourceAttempt, SourcePolicy};
use modn_core::buckets::{BucketPackage, PackageSource};
use modn_core::error::{code_of, ErrorCode};
use modn_core::progress::OperationControl;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

/// Acquire the archive of every release the cache does not already hold.
pub(crate) fn download(
    cache: &ArchiveCache,
    cache_use: CacheUse,
    releases: &[&BucketPackage],
    archives: &mut BTreeMap<String, PathBuf>,
    digests: &mut BTreeMap<String, String>,
    links: &[NxmLink],
    json: bool,
    progress: &Progress,
) -> anyhow::Result<Vec<SourceAttempt>> {
    let text = !json && !progress.enabled();
    let mut report = std::io::stderr();
    download_into(
        cache,
        cache_use,
        releases,
        archives,
        digests,
        links,
        text,
        &mut report,
        progress.control(),
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "the download step keeps the verified cache, package set, report sink and cancellation explicit"
)]
pub(super) fn download_into(
    cache: &ArchiveCache,
    cache_use: CacheUse,
    releases: &[&BucketPackage],
    archives: &mut BTreeMap<String, PathBuf>,
    digests: &mut BTreeMap<String, String>,
    links: &[NxmLink],
    text: bool,
    report: &mut dyn Write,
    control: &OperationControl,
) -> anyhow::Result<Vec<SourceAttempt>> {
    let mut attempts = Vec::new();
    for release in releases {
        // A cancellation stops the whole acquisition: no later source, no
        // retry and no cache reuse may continue after it.
        control.checkpoint()?;
        let reference = reference_of(release).to_string();
        if archives.contains_key(&reference) {
            continue;
        }
        if let PackageSource::Git { repository, commit } = release.package.primary_source() {
            if text {
                note(
                    report,
                    format!(
                        "Fetching {reference} {} commit {commit} from {repository}",
                        release.package.version
                    ),
                );
            }
            let archive = git_source::acquire(cache, release, cache_use, control)?;
            archives.insert(reference.clone(), archive.path);
            digests.insert(reference, archive.blake3);
            continue;
        }
        let mut nexus_failure: Option<anyhow::Error> = None;
        if let PackageSource::Nexus {
            game_domain,
            mod_id,
            file_id,
            file_name,
        } = release.package.primary_source()
        {
            let file = NexusFile::new(game_domain, *mod_id, *file_id, file_name)?;
            let link = super::nexus_download::matching_link(links, game_domain, *mod_id, *file_id);
            let url = file.cache_url();
            let cache_key = key(release, &url);
            if text {
                note(
                    report,
                    format!(
                        "Downloading {reference} {} from {}",
                        cache_key.version,
                        nexus_label(game_domain, *mod_id, *file_id, file_name)
                    ),
                );
            }
            if cache_use == CacheUse::Reuse {
                if let Some(path) = cache.cached(&cache_key)? {
                    archives.insert(reference, path);
                    continue;
                }
            }
            let outcome = (|| -> anyhow::Result<()> {
                let api_key = nexus_auth::load_key()?;
                let download_url = super::nexus_download::resolve(&file, &api_key, link)?;
                let acquired = cache.acquire_with_control(
                    &cache_key,
                    cache_use,
                    Some(reference.clone()),
                    control,
                    |_cache_url| super::nexus_download::open(&download_url),
                )?;
                archives.insert(reference.clone(), acquired.path);
                Ok(())
            })();
            if let Err(error) = outcome {
                let label = nexus_label(game_domain, *mod_id, *file_id, file_name);
                attempts.push(SourceAttempt::failed(
                    label,
                    1,
                    modn_core::acquisition::SourceFailure::Refused(format!("{error:#}")),
                ));
                nexus_failure = Some(error);
            } else {
                continue;
            }
        }
        if cache_use == CacheUse::Reuse {
            let mut hit = None;
            for url in fallback::urls(release) {
                if let Some(path) = cache.cached(&key(release, &url))? {
                    hit = Some(path);
                    break;
                }
            }
            if let Some(path) = hit {
                archives.insert(reference, path);
                continue;
            }
        }
        let all_candidates = fallback::candidates(release);
        let candidates = match nexus_failure {
            Some(_) => &all_candidates[1..],
            None => &all_candidates[..],
        };
        if candidates.is_empty() {
            if let Some(error) = nexus_failure {
                return Err(error);
            }
            continue;
        }
        let first_url = fallback::first_url(candidates).map(str::to_owned);
        let has_fetch = first_url.is_some();
        let Some(url) = first_url.or_else(|| candidates.first().map(|c| c.label.clone())) else {
            continue;
        };
        let cache_key = key(release, &url);
        if has_fetch && text {
            note(
                report,
                format!(
                    "Downloading {reference} {} from {}",
                    cache_key.version, cache_key.url
                ),
            );
        }
        let outcome = cache
            .acquire_from_sources(
                &cache_key,
                cache_use,
                candidates,
                &super::HttpTransport,
                SourcePolicy::default(),
                Some(reference.clone()),
                control,
            )
            .map_err(|error| match &nexus_failure {
                Some(nexus) => error.context(format!("the Nexus attempt failed first: {nexus:#}")),
                None => error,
            })?;
        for attempt in &outcome.attempts {
            if text {
                note(
                    report,
                    format!(
                        "Source {reference}: {}: {}",
                        attempt.source,
                        attempt.describe()
                    ),
                );
            }
        }
        attempts.extend(outcome.attempts);
        archives.insert(reference, outcome.acquired.path);
    }
    Ok(attempts)
}

fn note(report: &mut dyn Write, line: String) {
    let _ = writeln!(report, "{line}");
}

#[cfg(test)]
mod tests;
