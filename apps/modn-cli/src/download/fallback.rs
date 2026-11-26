use super::http::fetch_metadata;
use modn_core::acquisition::{
    FetchError, GithubReleaseAsset, PlannedAction, SourceCandidate, SourcePolicy,
};
use std::io::Read;

/// Turn one planned core action into the candidate this command fetches.
///
/// GitHub release metadata is resolved here, only when the ordered loop reaches
/// that source. Every other action is already a candidate.
pub(super) fn resolve_candidate(action: PlannedAction) -> SourceCandidate {
    match action {
        PlannedAction::Fetch { candidate } => candidate,
        PlannedAction::Github { selector, expected } => {
            match resolve_metadata(&selector, fetch_metadata) {
                Ok(resolved) if resolved == expected => SourceCandidate::fetch(expected),
                Ok(_) => {
                    SourceCandidate::refused(expected, "provider source changed during resolution")
                }
                Err(error) => SourceCandidate::refused(
                    expected,
                    format!("provider metadata failed: {error:#}"),
                ),
            }
        }
        _ => SourceCandidate::refused("unknown source", "unknown release source"),
    }
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
