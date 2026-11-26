use super::support::{
    acquire, https_sources, pin, release, FakeResolver, FakeTransport, FIRST, SECOND,
};
use crate::acquisition::{ArchiveCache, CacheUse, FetchError, SourceFailure, SourcesError};
use crate::buckets::PackageSource;
use crate::error::{code_of, ErrorCode};
use crate::progress::OperationControl;
use serde_json::json;
use std::fs;

#[test]
fn a_failed_first_source_falls_back_to_the_next_planned_source() {
    // Mirrors `a_failed_nexus_primary_falls_back_to_the_https_source` in
    // `apps/modn-cli/src/download/tests.rs`: the failed source is reported,
    // then the next planned source serves the archive.
    let temporary = tempfile::tempdir().expect("fixture");
    let cache = ArchiveCache::new(temporary.path().join("cache"));
    let bytes = b"fallback payload".to_vec();
    let digest = pin(&bytes);
    let release = release(https_sources(), Some(&digest));
    let transport = FakeTransport::default();
    transport.fail(FIRST, FetchError::Http(404));
    transport.respond(SECOND, &bytes);
    let resolver = FakeResolver::default();

    let acquired = acquire(
        &cache,
        &release,
        CacheUse::Refresh,
        &transport,
        &resolver,
        &OperationControl::default(),
    )
    .expect("the second planned source serves the archive");

    assert_eq!(transport.calls(), vec![FIRST.to_owned(), SECOND.to_owned()]);
    assert_eq!(acquired.attempts.len(), 2);
    assert_eq!(acquired.attempts[0].source, FIRST);
    assert_eq!(acquired.attempts[0].failure, Some(SourceFailure::Http(404)));
    assert!(acquired.attempts[1].succeeded());
    assert!(matches!(
        &acquired.source,
        PackageSource::Https { url } if url == SECOND
    ));
    assert_eq!(fs::read(&acquired.path).expect("archive bytes"), bytes);
    assert_eq!(acquired.digest, digest);
}

#[test]
fn every_failing_source_is_named_in_one_error_and_nothing_is_cached() {
    // Mirrors `every_failing_source_is_named_in_one_error_and_nothing_is_cached`
    // in `apps/modn-cli/tests/source_fallback_cli.rs`: one error names every
    // source and its reason, and no partial file remains.
    let temporary = tempfile::tempdir().expect("fixture");
    let cache = ArchiveCache::new(temporary.path().join("cache"));
    let digest = pin(b"unused payload");
    let release = release(
        json!([
            {"kind": "https", "url": FIRST},
            {"kind": "https", "url": SECOND},
            {"kind": "manual", "filename": "example.zip"}
        ]),
        Some(&digest),
    );
    let transport = FakeTransport::default();
    transport.fail_twice(FIRST, FetchError::Network("connection reset".to_owned()));
    transport.fail_twice(SECOND, FetchError::Network("connection reset".to_owned()));
    let resolver = FakeResolver::default();

    let error = acquire(
        &cache,
        &release,
        CacheUse::Refresh,
        &transport,
        &resolver,
        &OperationControl::default(),
    )
    .expect_err("every source fails");

    assert_eq!(code_of(&error), Some(ErrorCode::DownloadFailed));
    let sources = error
        .downcast_ref::<SourcesError>()
        .expect("the failure report is a SourcesError");
    let text = sources.to_string();
    for expected in [FIRST, SECOND, "network", "2 attempts", "manual"] {
        assert!(text.contains(expected), "missing {expected:?}: {text}");
    }
    assert_eq!(sources.attempts().len(), 5);
    assert!(super::support::partial_files(&cache).is_empty());
}
