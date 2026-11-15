//! Ordered source candidates for one release's archive.

use super::http::fetch_metadata;
use modn_core::acquisition::{
    FetchError, GithubReleaseAsset, SourceCandidate, SourcePolicy, SourceTarget,
};
use modn_core::buckets::{BucketPackage, PackageSource};
use std::io::Read;

/// Canonical GitHub release asset URL derived from the pin record.
pub(super) fn github_url(repository: &str, tag: &str, asset: &str) -> String {
    format!("https://github.com/{repository}/releases/download/{tag}/{asset}")
}

/// Every URL Modn can try without provider metadata.
pub(super) fn urls(release: &BucketPackage) -> Vec<String> {
    release
        .package
        .sources
        .iter()
        .filter_map(|source| match source {
            PackageSource::Https { url } => Some(url.clone()),
            PackageSource::GithubRelease {
                repository,
                tag,
                asset,
            } => Some(github_url(repository, tag, asset)),
            _ => None,
        })
        .collect()
}

fn nexus_label(game_domain: &str, mod_id: u64, file_id: u64, file_name: &str) -> String {
    format!("Nexus Mods: {game_domain} mod {mod_id} file {file_id} ({file_name})")
}

/// Resolve GitHub metadata, retrying one transient transport failure.
///
/// The source retry policy covers the metadata request too: a timeout, a
/// network failure or an HTTP 5xx is retried once. A 404 or a metadata
/// mismatch is final for the source.
fn resolve_metadata(
    selector: &GithubReleaseAsset,
    fetch: impl Fn(&str) -> anyhow::Result<Box<dyn Read>>,
) -> anyhow::Result<String> {
    let policy = SourcePolicy::default();
    let mut attempt = 0;
    loop {
        match selector.resolve(&fetch) {
            Ok(url) => return Ok(url),
            Err(error) => {
                let transient = error
                    .downcast_ref::<FetchError>()
                    .is_some_and(super::retryable);
                if !transient || attempt >= policy.retries() {
                    return Err(error);
                }
                attempt += 1;
            }
        }
    }
}

/// Build the ordered candidates for `release`.
///
/// GitHub metadata is resolved here, before the loop starts, so a resolution
/// failure is one refused source and the next source still runs.
pub(super) fn candidates(release: &BucketPackage) -> Vec<SourceCandidate> {
    release
        .package
        .sources
        .iter()
        .map(|source| match source {
            PackageSource::Https { url } => SourceCandidate::fetch(url.clone()),
            PackageSource::GithubRelease {
                repository,
                tag,
                asset,
            } => {
                let expected = github_url(repository, tag, asset);
                let selector =
                    GithubReleaseAsset::new(repository.clone(), tag.clone(), asset.clone());
                match resolve_metadata(&selector, fetch_metadata) {
                    Ok(resolved) if resolved == expected => SourceCandidate::fetch(expected),
                    Ok(_) => SourceCandidate::refused(
                        expected,
                        "provider source changed during resolution",
                    ),
                    Err(error) => SourceCandidate::refused(
                        expected,
                        format!("provider metadata failed: {error:#}"),
                    ),
                }
            }
            PackageSource::Manual { filename } => SourceCandidate::manual(filename.clone()),
            PackageSource::Git { repository, commit } => SourceCandidate::refused(
                format!("git {repository} commit {commit}"),
                "git sources use the pinned-commit path",
            ),
            PackageSource::Nexus {
                game_domain,
                mod_id,
                file_id,
                file_name,
            } => SourceCandidate::refused(
                nexus_label(game_domain, *mod_id, *file_id, file_name),
                "a Nexus download needs an nxm link",
            ),
            _ => SourceCandidate::refused("unknown source", "unknown release source"),
        })
        .collect()
}

/// The first URL the loop would try, used for the cache-entry name.
pub(super) fn first_url(candidates: &[SourceCandidate]) -> Option<&str> {
    candidates
        .iter()
        .find_map(|candidate| match &candidate.target {
            SourceTarget::Fetch(url) => Some(url.as_str()),
            _ => None,
        })
}
