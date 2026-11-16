//! Ordered source plan for one release's archive.
//!
//! The plan is built without network access. A GitHub release asset resolves
//! its provider metadata only when the ordered loop reaches it, so an unused
//! fallback costs no request.

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

/// One entry of a release's ordered source list.
pub(super) enum Planned {
    /// A pinned Git commit, acquired through the Git path.
    Git { repository: String, commit: String },
    /// A Nexus Mods file, acquired through the Nexus path.
    Nexus {
        game_domain: String,
        mod_id: u64,
        file_id: u64,
        file_name: String,
    },
    /// A fetchable, manual or refused source, in list order.
    Fetch(FetchPlan),
}

/// A source that needs no ordered-attempt path of its own.
pub(super) enum FetchPlan {
    /// A candidate the loop can pass to the archive cache.
    Ready(SourceCandidate),
    /// A GitHub release asset whose metadata stays unresolved until reached.
    Github {
        repository: String,
        tag: String,
        asset: String,
        expected: String,
    },
}

impl FetchPlan {
    /// Resolve this entry into the candidate the loop tries.
    pub(super) fn resolve(self) -> SourceCandidate {
        match self {
            Self::Ready(candidate) => candidate,
            Self::Github {
                repository,
                tag,
                asset,
                expected,
            } => {
                let selector = GithubReleaseAsset::new(repository, tag, asset);
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
        }
    }
}

/// Build the ordered plan for `release` without a network request.
pub(super) fn plan(release: &BucketPackage) -> Vec<Planned> {
    release
        .package
        .sources
        .iter()
        .map(|source| match source {
            PackageSource::Git { repository, commit } => Planned::Git {
                repository: repository.clone(),
                commit: commit.clone(),
            },
            PackageSource::Nexus {
                game_domain,
                mod_id,
                file_id,
                file_name,
            } => Planned::Nexus {
                game_domain: game_domain.clone(),
                mod_id: *mod_id,
                file_id: *file_id,
                file_name: file_name.clone(),
            },
            PackageSource::GithubRelease {
                repository,
                tag,
                asset,
            } => Planned::Fetch(FetchPlan::Github {
                repository: repository.clone(),
                tag: tag.clone(),
                asset: asset.clone(),
                expected: github_url(repository, tag, asset),
            }),
            PackageSource::Https { url } => {
                Planned::Fetch(FetchPlan::Ready(SourceCandidate::fetch(url.clone())))
            }
            PackageSource::Manual { filename } => {
                Planned::Fetch(FetchPlan::Ready(SourceCandidate::manual(filename.clone())))
            }
            _ => Planned::Fetch(FetchPlan::Ready(SourceCandidate::refused(
                "unknown source",
                "unknown release source",
            ))),
        })
        .collect()
}

/// The URL a fetch candidate names, used for the cache entry.
pub(super) fn fetch_url(candidate: &SourceCandidate) -> &str {
    match &candidate.target {
        SourceTarget::Fetch(url) => url.as_str(),
        _ => candidate.label.as_str(),
    }
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
            format!("{error:#}").contains("after 2 attempt(s)"),
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
