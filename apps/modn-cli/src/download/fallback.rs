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
/// The source retry policy covers the whole metadata request: a timeout, a
/// network failure or an HTTP 5xx is retried once, also when it appears while
/// reading the metadata body. A 404 or a metadata mismatch is final for the
/// source.
fn resolve_metadata(
    selector: &GithubReleaseAsset,
    fetch: impl Fn(&str) -> anyhow::Result<Box<dyn Read>>,
) -> anyhow::Result<String> {
    let policy = SourcePolicy::default();
    let mut attempt = 0;
    loop {
        attempt += 1;
        match selector.resolve(&fetch) {
            Ok(url) => return Ok(url),
            Err(error) => {
                if !transient(&error) || attempt > policy.retries() {
                    return Err(error.context(format!(
                        "GitHub metadata request failed after {attempt} attempt(s)"
                    )));
                }
            }
        }
    }
}

/// Whether `error` is a transport failure the source retry policy covers.
fn transient(error: &anyhow::Error) -> bool {
    if let Some(error) = error.downcast_ref::<FetchError>() {
        return super::retryable(error);
    }
    error
        .downcast_ref::<std::io::Error>()
        .and_then(super::transport_io)
        .is_some_and(|error| super::retryable(&error))
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

#[cfg(test)]
mod tests {
    use super::{resolve_metadata, GithubReleaseAsset};
    use std::cell::Cell;
    use std::io::{self, Cursor, Read};

    const EXPECTED: &str = "https://github.com/fixture/project/releases/download/v1/example.zip";
    const METADATA: &str = r#"{
        "tag_name": "v1",
        "html_url": "https://github.com/fixture/project/releases/tag/v1",
        "assets": [{"name": "example.zip", "browser_download_url": "https://github.com/fixture/project/releases/download/v1/example.zip"}]
    }"#;

    struct FailingBody {
        kind: io::ErrorKind,
        failures: usize,
        body: &'static [u8],
        offset: usize,
    }

    impl Read for FailingBody {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.failures > 0 {
                self.failures -= 1;
                return Err(io::Error::new(self.kind, "fixture metadata body failure"));
            }
            let start = self.offset.min(self.body.len());
            let end = (start + buffer.len()).min(self.body.len());
            buffer[..end - start].copy_from_slice(&self.body[start..end]);
            self.offset = end;
            Ok(end - start)
        }
    }

    fn selector() -> GithubReleaseAsset {
        GithubReleaseAsset::new(
            "fixture/project".to_owned(),
            "v1".to_owned(),
            "example.zip".to_owned(),
        )
    }

    #[test]
    fn a_metadata_body_timeout_is_retried_once() {
        let calls = Cell::new(0_u32);
        let resolved = resolve_metadata(&selector(), |_url| {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                Ok(Box::new(FailingBody {
                    kind: io::ErrorKind::TimedOut,
                    failures: 1,
                    body: METADATA.as_bytes(),
                    offset: 0,
                }) as Box<dyn Read>)
            } else {
                Ok(Box::new(Cursor::new(METADATA.as_bytes())) as Box<dyn Read>)
            }
        })
        .expect("the retried metadata body resolves the asset");
        assert_eq!(resolved, EXPECTED);
        assert_eq!(calls.get(), 2, "the body read was not retried once");
    }

    #[test]
    fn a_metadata_body_that_always_ends_early_reports_the_attempts() {
        let calls = Cell::new(0_u32);
        let error = resolve_metadata(&selector(), |_url| {
            calls.set(calls.get() + 1);
            Ok(Box::new(FailingBody {
                kind: io::ErrorKind::UnexpectedEof,
                failures: usize::MAX,
                body: METADATA.as_bytes(),
                offset: 0,
            }) as Box<dyn Read>)
        })
        .expect_err("an unreadable metadata body refuses the source");
        assert_eq!(calls.get(), 2, "the body read was not retried once");
        assert!(
            format!("{error:#}").contains("after 2 attempts"),
            "the error does not list the attempts: {error:#}"
        );
    }

    #[test]
    fn a_404_metadata_answer_is_not_retried() {
        let calls = Cell::new(0_u32);
        let error = resolve_metadata(&selector(), |_url| {
            calls.set(calls.get() + 1);
            Err(super::FetchError::Http(404).into())
        })
        .expect_err("a 404 metadata answer refuses the source");
        assert_eq!(calls.get(), 1, "a 404 metadata answer was retried");
        assert!(
            format!("{error:#}").contains("http 404"),
            "the 404 is not named: {error:#}"
        );
    }
}
