use super::http::fetch_metadata;
use modn_core::acquisition::{
    FetchError, GithubReleaseAsset, NexusFile, SourceCandidate, SourcePolicy, SourceTarget,
};
use modn_core::buckets::{BucketPackage, PackageSource};
use std::io::Read;

pub(super) fn github_url(repository: &str, tag: &str, asset: &str) -> String {
    format!("https://github.com/{repository}/releases/download/{tag}/{asset}")
}

pub(super) fn cache_url(source: &PackageSource) -> anyhow::Result<Option<String>> {
    Ok(match source {
        PackageSource::Https { url } => Some(url.clone()),
        PackageSource::GithubRelease {
            repository,
            tag,
            asset,
        } => Some(github_url(repository, tag, asset)),
        PackageSource::Nexus {
            game_domain,
            mod_id,
            file_id,
            file_name,
        } => Some(NexusFile::new(game_domain, *mod_id, *file_id, file_name)?.cache_url()),
        _ => None,
    })
}

pub(super) fn resolve_metadata(
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

fn transient(error: &anyhow::Error) -> bool {
    if let Some(error) = error.downcast_ref::<FetchError>() {
        return super::retryable(error);
    }
    error
        .downcast_ref::<std::io::Error>()
        .and_then(super::transport_io)
        .is_some_and(|error| super::retryable(&error))
}

pub(super) enum Planned {
    Git {
        repository: String,
        commit: String,
    },
    Nexus {
        game_domain: String,
        mod_id: u64,
        file_id: u64,
        file_name: String,
    },
    Fetch(FetchPlan),
}

pub(super) enum FetchPlan {
    Ready(SourceCandidate),
    Github {
        repository: String,
        tag: String,
        asset: String,
        expected: String,
    },
}

impl FetchPlan {
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

pub(super) fn plan(release: &BucketPackage) -> Vec<(Planned, PackageSource)> {
    release
        .package
        .sources
        .iter()
        .map(|source| {
            let planned = match source {
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
            };
            (planned, source.clone())
        })
        .collect()
}

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
