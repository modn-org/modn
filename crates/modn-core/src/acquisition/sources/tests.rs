use super::*;
use crate::error::{code_of, ErrorCode};
use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::io::Cursor;
use std::thread::sleep;
use tempfile::tempdir;

const FIRST: &str = "https://first.invalid/mod.zip";
const SECOND: &str = "https://second.invalid/mod.zip";

enum Response {
    Body(Vec<u8>),
    TimeoutAfter(Duration),
    Error(FetchError),
    Partial { bytes: Vec<u8>, kind: io::ErrorKind },
}

#[derive(Default)]
struct Fake {
    responses: RefCell<BTreeMap<String, VecDeque<Response>>>,
    calls: RefCell<Vec<String>>,
}

impl Fake {
    fn with(responses: impl IntoIterator<Item = (&'static str, Vec<Response>)>) -> Self {
        let fake = Self::default();
        for (url, queue) in responses {
            fake.responses
                .borrow_mut()
                .insert(url.to_owned(), queue.into());
        }
        fake
    }

    fn calls(&self) -> usize {
        self.calls.borrow().len()
    }

    fn calls_for(&self, url: &str) -> usize {
        self.calls
            .borrow()
            .iter()
            .filter(|call| *call == url)
            .count()
    }
}

impl SourceTransport for Fake {
    fn open(&self, url: &str, _policy: &SourcePolicy) -> Result<SourceBody, FetchError> {
        self.calls.borrow_mut().push(url.to_owned());
        let mut responses = self.responses.borrow_mut();
        let queue = responses
            .get_mut(url)
            .ok_or_else(|| FetchError::Network("unexpected request".to_owned()))?;
        match queue.pop_front().expect("a queued response") {
            Response::Body(bytes) => Ok(SourceBody {
                reader: Box::new(Cursor::new(bytes)),
                total: None,
            }),
            Response::TimeoutAfter(delay) => Ok(SourceBody {
                reader: Box::new(TimeoutBody { delay, sent: false }),
                total: None,
            }),
            Response::Error(error) => Err(error),
            Response::Partial { bytes, kind } => Ok(SourceBody {
                reader: Box::new(PartialBody {
                    bytes,
                    kind,
                    sent: false,
                }),
                total: None,
            }),
        }
    }
}

struct TimeoutBody {
    delay: Duration,
    sent: bool,
}

impl Read for TimeoutBody {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        if !self.sent {
            self.sent = true;
            sleep(self.delay);
        }
        Err(io::Error::new(io::ErrorKind::TimedOut, "idle timeout"))
    }
}

struct PartialBody {
    bytes: Vec<u8>,
    kind: io::ErrorKind,
    sent: bool,
}

impl Read for PartialBody {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.sent {
            return Err(io::Error::new(self.kind, "body stopped"));
        }
        self.sent = true;
        let length = self.bytes.len().min(buffer.len());
        buffer[..length].copy_from_slice(&self.bytes[..length]);
        Ok(length)
    }
}

fn pin(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn key<'a>(blake3: &'a str) -> CacheKey<'a> {
    CacheKey::new("test-game", "fixture", "1.0", FIRST, blake3)
}

fn acquire<'a>(
    cache: &ArchiveCache,
    blake3: &'a str,
    sources: &[SourceCandidate],
    fake: &Fake,
    policy: SourcePolicy,
) -> Result<SourcesAcquired> {
    cache.acquire_from_sources(
        &key(blake3),
        CacheUse::Reuse,
        sources,
        fake,
        policy,
        None,
        &OperationControl::default(),
    )
}

fn no_partial_files(cache: &ArchiveCache) {
    if let Ok(entries) = fs::read_dir(cache.root()) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            assert!(!name.ends_with(".download"), "partial file kept: {name}");
        }
    }
}

#[test]
fn a_hanging_source_is_abandoned_after_the_idle_timeout() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let body = b"served by the second source".to_vec();
    let digest = pin(&body);
    let fake = Fake::with([
        (
            FIRST,
            vec![Response::TimeoutAfter(Duration::from_millis(5))],
        ),
        (SECOND, vec![Response::Body(body.clone())]),
    ]);
    let outcome = acquire(
        &cache,
        &digest,
        &[
            SourceCandidate::fetch(FIRST),
            SourceCandidate::fetch(SECOND),
        ],
        &fake,
        SourcePolicy::new(Duration::from_secs(1), Duration::from_millis(5), 0),
    )
    .unwrap();
    assert_eq!(outcome.acquired.origin, Origin::Download);
    assert_eq!(fs::read(&outcome.acquired.path).unwrap(), body);
    assert_eq!(outcome.attempts.len(), 2);
    assert_eq!(outcome.attempts[0].source, FIRST);
    assert_eq!(outcome.attempts[0].failure, Some(SourceFailure::Timeout));
    assert!(outcome.attempts[1].succeeded());
    assert_eq!(fake.calls_for(FIRST), 1);
}

#[test]
fn network_and_http_failures_fall_through_with_the_documented_retry_counts() {
    let cases = [
        ("network", 2, "network"),
        ("404", 1, "http 404"),
        ("500", 2, "http 500"),
    ];
    for (kind, expected_calls, class) in cases {
        let dir = tempdir().unwrap();
        let cache = ArchiveCache::new(dir.path().join("cache"));
        let body = b"mirror bytes".to_vec();
        let digest = pin(&body);
        let failure = || match kind {
            "network" => Response::Error(FetchError::Network("connection refused".to_owned())),
            "404" => Response::Error(FetchError::Http(404)),
            _ => Response::Error(FetchError::Http(500)),
        };
        let responses = (0..expected_calls).map(|_| failure()).collect();
        let fake = Fake::with([
            (FIRST, responses),
            (SECOND, vec![Response::Body(body.clone())]),
        ]);
        let outcome = acquire(
            &cache,
            &digest,
            &[
                SourceCandidate::fetch(FIRST),
                SourceCandidate::fetch(SECOND),
            ],
            &fake,
            SourcePolicy::new(Duration::from_secs(1), Duration::from_secs(1), 1),
        )
        .unwrap();
        assert_eq!(fs::read(&outcome.acquired.path).unwrap(), body, "{class}");
        assert_eq!(fake.calls_for(FIRST), expected_calls, "{class}");
        assert!(
            outcome.attempts[expected_calls - 1]
                .failure
                .as_ref()
                .unwrap()
                .to_string()
                .contains(class),
            "{class}"
        );
    }
}

#[test]
fn a_digest_mismatch_falls_through_and_is_not_cached_or_retried() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let body = b"the pinned bytes".to_vec();
    let digest = pin(&body);
    let fake = Fake::with([
        (FIRST, vec![Response::Body(b"wrong bytes".to_vec())]),
        (SECOND, vec![Response::Body(body.clone())]),
    ]);
    let outcome = acquire(
        &cache,
        &digest,
        &[
            SourceCandidate::fetch(FIRST),
            SourceCandidate::fetch(SECOND),
        ],
        &fake,
        SourcePolicy::new(Duration::from_secs(1), Duration::from_secs(1), 1),
    )
    .unwrap();
    assert_eq!(fake.calls_for(FIRST), 1, "a mismatch is deterministic");
    let mismatch = outcome.attempts[0].failure.as_ref().unwrap();
    assert_eq!(
        mismatch,
        &SourceFailure::DigestMismatch {
            expected: digest.clone(),
            actual: pin(b"wrong bytes"),
        }
    );
    assert_eq!(fs::read(&outcome.acquired.path).unwrap(), body);
    let entries = cache.entries(None).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(fs::read(&entries[0].path).unwrap(), body);
    no_partial_files(&cache);
}

#[test]
fn a_body_that_stops_halfway_leaves_no_partial_file() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let body = b"complete bytes".to_vec();
    let digest = pin(&body);
    let fake = Fake::with([
        (
            FIRST,
            vec![Response::Partial {
                bytes: b"partial".to_vec(),
                kind: io::ErrorKind::ConnectionReset,
            }],
        ),
        (SECOND, vec![Response::Body(body.clone())]),
    ]);
    let outcome = acquire(
        &cache,
        &digest,
        &[
            SourceCandidate::fetch(FIRST),
            SourceCandidate::fetch(SECOND),
        ],
        &fake,
        SourcePolicy::new(Duration::from_secs(1), Duration::from_secs(1), 0),
    )
    .unwrap();
    assert_eq!(fs::read(&outcome.acquired.path).unwrap(), body);
    assert!(matches!(
        outcome.attempts[0].failure,
        Some(SourceFailure::Network(_))
    ));
    no_partial_files(&cache);
}

#[test]
fn every_failing_source_is_reported_and_manual_is_never_attempted() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let digest = pin(b"never served");
    let fake = Fake::with([
        (
            FIRST,
            vec![Response::Error(FetchError::Network(
                "connection refused".to_owned(),
            ))],
        ),
        (SECOND, vec![Response::Error(FetchError::Http(404))]),
    ]);
    let error = acquire(
        &cache,
        &digest,
        &[
            SourceCandidate::fetch(FIRST),
            SourceCandidate::fetch(SECOND),
            SourceCandidate::manual("mod.zip"),
        ],
        &fake,
        SourcePolicy::new(Duration::from_secs(1), Duration::from_secs(1), 0),
    )
    .unwrap_err();
    assert_eq!(code_of(&error), Some(ErrorCode::DownloadFailed));
    let message = error.to_string();
    assert!(message.contains(FIRST), "{message}");
    assert!(message.contains("network: connection refused"), "{message}");
    assert!(message.contains(SECOND), "{message}");
    assert!(message.contains("http 404"), "{message}");
    assert!(message.contains("manual"), "{message}");
    assert_eq!(fake.calls(), 2);
    assert!(cache.entries(None).unwrap().is_empty());
    assert_eq!(fs::read_dir(cache.root()).unwrap().count(), 0);
}

#[test]
fn a_verified_cache_hit_makes_no_transport_call() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let body = b"cached bytes".to_vec();
    let digest = pin(&body);
    let first = Fake::with([(FIRST, vec![Response::Body(body.clone())])]);
    acquire(
        &cache,
        &digest,
        &[SourceCandidate::fetch(FIRST)],
        &first,
        SourcePolicy::default(),
    )
    .unwrap();
    let second = Fake::default();
    let outcome = acquire(
        &cache,
        &digest,
        &[
            SourceCandidate::fetch(FIRST),
            SourceCandidate::fetch(SECOND),
        ],
        &second,
        SourcePolicy::default(),
    )
    .unwrap();
    assert_eq!(outcome.acquired.origin, Origin::Cache);
    assert!(outcome.attempts.is_empty());
    assert_eq!(second.calls(), 0);
}

#[test]
fn a_retryable_failure_is_retried_once_then_succeeds() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let body = b"second attempt".to_vec();
    let digest = pin(&body);
    let fake = Fake::with([(
        FIRST,
        vec![
            Response::Error(FetchError::Timeout),
            Response::Body(body.clone()),
        ],
    )]);
    let outcome = acquire(
        &cache,
        &digest,
        &[SourceCandidate::fetch(FIRST)],
        &fake,
        SourcePolicy::new(Duration::from_secs(1), Duration::from_secs(1), 1),
    )
    .unwrap();
    assert_eq!(fs::read(&outcome.acquired.path).unwrap(), body);
    assert_eq!(fake.calls(), 2);
    assert_eq!(outcome.attempts.len(), 2);
    assert_eq!(outcome.attempts[0].failure, Some(SourceFailure::Timeout));
    assert!(outcome.attempts[1].succeeded());
}
