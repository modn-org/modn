//! Ordered acquisition of release archives with text reporting and cancellation.
//!
//! Every source kind is one attempt in the release's list order. Git and
//! Nexus attempts classify their failure like a fetch attempt, a later source
//! still runs, and a cancellation stops the whole acquisition.

use super::fallback::{self, Planned};
use super::key;
use super::nexus_label;
use crate::catalog_install::reference_of;
use crate::git_source;
use crate::nexus_auth;
use crate::nxm::NxmLink;
use crate::progress::Progress;
use modn_core::acquisition::{
    ArchiveCache, CacheUse, FetchError, NexusFile, SourceAttempt, SourceFailure, SourcePolicy,
    SourcesError,
};
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
        if cache_use == CacheUse::Reuse {
            if let Some((path, digest)) = cached(cache, release)? {
                archives.insert(reference.clone(), path);
                if let Some(digest) = digest {
                    digests.insert(reference, digest);
                }
                continue;
            }
        }
        let first = attempts.len();
        let mut served = false;
        for planned in fallback::plan(release) {
            control.checkpoint()?;
            match planned {
                Planned::Git { repository, commit } => {
                    if text {
                        note(
                            report,
                            format!(
                                "Fetching {reference} {} commit {commit} from {repository}",
                                release.package.version
                            ),
                        );
                    }
                    match git_source::acquire_named(
                        cache,
                        release,
                        &repository,
                        &commit,
                        cache_use,
                        control,
                    ) {
                        Ok(archive) => {
                            record(
                                text,
                                report,
                                &reference,
                                &mut attempts,
                                SourceAttempt::ok(format!("git {repository} commit {commit}"), 1),
                            );
                            archives.insert(reference.clone(), archive.path);
                            digests.insert(reference.clone(), archive.blake3);
                            served = true;
                            break;
                        }
                        Err(error) if cancelled(&error) => return Err(error),
                        Err(error) => record(
                            text,
                            report,
                            &reference,
                            &mut attempts,
                            SourceAttempt::failed(
                                format!("git {repository} commit {commit}"),
                                1,
                                classify(&error),
                            ),
                        ),
                    }
                }
                Planned::Nexus {
                    game_domain,
                    mod_id,
                    file_id,
                    file_name,
                } => {
                    let label = nexus_label(&game_domain, mod_id, file_id, &file_name);
                    if text {
                        note(
                            report,
                            format!(
                                "Downloading {reference} {} from {label}",
                                release.package.version
                            ),
                        );
                    }
                    match nexus_archive(
                        cache,
                        cache_use,
                        release,
                        &reference,
                        &game_domain,
                        mod_id,
                        file_id,
                        &file_name,
                        links,
                        control,
                    ) {
                        Ok(path) => {
                            record(
                                text,
                                report,
                                &reference,
                                &mut attempts,
                                SourceAttempt::ok(label, 1),
                            );
                            archives.insert(reference.clone(), path);
                            served = true;
                            break;
                        }
                        Err(error) if cancelled(&error) => return Err(error),
                        Err(error) => record(
                            text,
                            report,
                            &reference,
                            &mut attempts,
                            SourceAttempt::failed(label, 1, classify(&error)),
                        ),
                    }
                }
                Planned::Fetch(fetch) => {
                    let candidate = fetch.resolve();
                    if text && text_announces(&candidate.target) {
                        note(
                            report,
                            format!(
                                "Downloading {reference} {} from {}",
                                release.package.version, candidate.label
                            ),
                        );
                    }
                    let cache_key = key(release, fallback::fetch_url(&candidate));
                    let single = std::slice::from_ref(&candidate);
                    let outcome = cache.acquire_from_sources(
                        &cache_key,
                        cache_use,
                        single,
                        &super::HttpTransport,
                        SourcePolicy::default(),
                        Some(reference.clone()),
                        control,
                    );
                    match outcome {
                        Ok(outcome) => {
                            for attempt in outcome.attempts {
                                note_attempt(text, report, &reference, &attempt);
                                attempts.push(attempt);
                            }
                            archives.insert(reference.clone(), outcome.acquired.path);
                            served = true;
                            break;
                        }
                        Err(error) if cancelled(&error) => return Err(error),
                        Err(error) => match attempts_of(&error) {
                            Some(failed) => {
                                for attempt in failed {
                                    note_attempt(text, report, &reference, &attempt);
                                    attempts.push(attempt);
                                }
                            }
                            None => {
                                return Err(
                                    error.context(format!("source {reference} was not tried"))
                                )
                            }
                        },
                    }
                }
            }
        }
        if !served && attempts.len() > first {
            return Err(anyhow::Error::new(SourcesError::new(
                reference.clone(),
                attempts[first..].to_vec(),
            )));
        }
    }
    Ok(attempts)
}

/// Return a verified cache entry for `release`, in source list order.
///
/// A Git entry also reports the packed digest that its name records.
fn cached(
    cache: &ArchiveCache,
    release: &BucketPackage,
) -> anyhow::Result<Option<(PathBuf, Option<String>)>> {
    if let Some(archive) = git_source::cached(cache, release)? {
        return Ok(Some((archive.path, Some(archive.blake3))));
    }
    match release.package.primary_source() {
        PackageSource::Nexus {
            game_domain,
            mod_id,
            file_id,
            file_name,
        } => {
            let url = NexusFile::new(game_domain, *mod_id, *file_id, file_name)?.cache_url();
            if let Some(path) = cache.cached(&key(release, &url))? {
                return Ok(Some((path, None)));
            }
        }
        _ => {}
    }
    for url in fallback::urls(release) {
        if let Some(path) = cache.cached(&key(release, &url))? {
            return Ok(Some((path, None)));
        }
    }
    Ok(None)
}

#[expect(
    clippy::too_many_arguments,
    reason = "the Nexus attempt keeps the cache, release, provider fields, links and cancellation explicit"
)]
fn nexus_archive(
    cache: &ArchiveCache,
    cache_use: CacheUse,
    release: &BucketPackage,
    reference: &str,
    game_domain: &str,
    mod_id: u64,
    file_id: u64,
    file_name: &str,
    links: &[NxmLink],
    control: &OperationControl,
) -> anyhow::Result<PathBuf> {
    let file = NexusFile::new(game_domain, mod_id, file_id, file_name)?;
    let link = super::nexus_download::matching_link(links, game_domain, mod_id, file_id);
    let url = file.cache_url();
    let cache_key = key(release, &url);
    let api_key = nexus_auth::load_key()?;
    let download_url = super::nexus_download::resolve(&file, &api_key, link)?;
    let acquired = cache.acquire_with_control(
        &cache_key,
        cache_use,
        Some(reference.to_owned()),
        control,
        |_cache_url| super::nexus_download::open(&download_url),
    )?;
    Ok(acquired.path)
}

/// Whether one cancelled error ends the whole acquisition.
fn cancelled(error: &anyhow::Error) -> bool {
    code_of(error) == Some(ErrorCode::Cancelled)
}

/// Classify a Git or Nexus failure like any other source attempt.
fn classify(error: &anyhow::Error) -> SourceFailure {
    if let Some(fetch) = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<FetchError>())
    {
        return fetch.clone().into();
    }
    SourceFailure::Refused(format!("{error:#}"))
}

/// The attempts of a failed ordered fetch, when the cache reported them.
fn attempts_of(error: &anyhow::Error) -> Option<Vec<SourceAttempt>> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<SourcesError>())
        .map(|sources| sources.attempts().to_vec())
}

/// Whether the ordered loop announces this candidate before it tries it.
fn text_announces(target: &modn_core::acquisition::SourceTarget) -> bool {
    matches!(target, modn_core::acquisition::SourceTarget::Fetch(_))
}

fn record(
    text: bool,
    report: &mut dyn Write,
    reference: &str,
    attempts: &mut Vec<SourceAttempt>,
    attempt: SourceAttempt,
) {
    note_attempt(text, report, reference, &attempt);
    attempts.push(attempt);
}

fn note_attempt(text: bool, report: &mut dyn Write, reference: &str, attempt: &SourceAttempt) {
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

fn note(report: &mut dyn Write, line: String) {
    let _ = writeln!(report, "{line}");
}

#[cfg(test)]
mod tests;
