use super::support::{
    acquire, cancelled_control, https_sources, nexus_link, nexus_source, partial_files, pin,
    release, FakeResolver, FakeTransport, FIRST,
};
use crate::acquisition::{ArchiveCache, CacheUse};
use crate::error::{code_of, ErrorCode};
use crate::progress::{CancellationToken, OperationControl};
use serde_json::json;

#[test]
fn a_cancelled_control_stops_before_any_source_call() {
    // Mirrors `a_cancel_before_an_https_attempt_stops_before_the_second_source`
    // in `apps/modn-cli/src/download/acquire/tests.rs`: a cancelled control
    // stops acquisition before any source is opened.
    let temporary = tempfile::tempdir().expect("fixture");
    let cache = ArchiveCache::new(temporary.path().join("cache"));
    let digest = pin(b"unused payload");
    let release = release(https_sources(), Some(&digest));
    let transport = FakeTransport::default();
    let resolver = FakeResolver::default();

    let error = acquire(
        &cache,
        &release,
        CacheUse::Reuse,
        &transport,
        &resolver,
        &cancelled_control(),
    )
    .expect_err("a cancelled control stops acquisition");

    assert_eq!(code_of(&error), Some(ErrorCode::Cancelled));
    assert!(transport.calls().is_empty());
}

#[test]
fn a_cancel_during_a_nexus_attempt_stops_before_the_https_fallback() {
    // Mirrors `a_cancel_during_a_nexus_attempt_stops_before_the_cached_https_fallback`
    // in `apps/modn-cli/src/download/acquire/tests.rs`: a cancellation inside
    // the Nexus body stops before the https source and leaves no partial file.
    let temporary = tempfile::tempdir().expect("fixture");
    let cache = ArchiveCache::new(temporary.path().join("cache"));
    let bytes = b"Nexus payload".to_vec();
    let digest = pin(&bytes);
    let release = release(
        json!([
            nexus_source(123456, "NexusMod.zip"),
            {"kind": "https", "url": FIRST}
        ]),
        Some(&digest),
    );
    let token = CancellationToken::new();
    let control = OperationControl::new(token.clone(), |_event| {});
    let link = nexus_link(123456, "NexusMod.zip");
    let resolver = FakeResolver::default();
    resolver.nexus_download(123456, link.clone());
    resolver.open_cancel(&link, token);
    let transport = FakeTransport::default();

    let error = acquire(
        &cache,
        &release,
        CacheUse::Refresh,
        &transport,
        &resolver,
        &control,
    )
    .expect_err("a cancelled Nexus attempt stops the acquisition");

    assert_eq!(code_of(&error), Some(ErrorCode::Cancelled));
    assert!(transport.calls().is_empty(), "the loop fell back to https");
    assert!(partial_files(&cache).is_empty());
}
