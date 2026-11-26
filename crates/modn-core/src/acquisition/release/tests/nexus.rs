use super::support::{
    acquire, nexus_link, nexus_source, pin, release, FakeResolver, FakeTransport, FIRST,
    NEXUS_LABEL,
};
use crate::acquisition::{ArchiveCache, CacheUse, SourceFailure};
use crate::buckets::PackageSource;
use crate::progress::OperationControl;
use serde_json::json;
use std::fs;

#[test]
fn a_refused_nexus_primary_falls_back_to_the_https_source() {
    // Mirrors `a_failed_nexus_primary_falls_back_to_the_https_source` in
    // `apps/modn-cli/src/download/tests.rs`: the Nexus attempt is first and
    // names its refusal; the https source then serves the archive.
    let temporary = tempfile::tempdir().expect("fixture");
    let cache = ArchiveCache::new(temporary.path().join("cache"));
    let bytes = b"https payload".to_vec();
    let digest = pin(&bytes);
    let release = release(
        json!([
            nexus_source(123456, "NexusMod.zip"),
            {"kind": "https", "url": FIRST}
        ]),
        Some(&digest),
    );
    let resolver = FakeResolver::default();
    resolver.nexus_refused(123456, "Nexus downloads need an API key");
    let transport = FakeTransport::default();
    transport.respond(FIRST, &bytes);

    let acquired = acquire(
        &cache,
        &release,
        CacheUse::Refresh,
        &transport,
        &resolver,
        &OperationControl::default(),
    )
    .expect("the https source serves the archive");

    assert_eq!(acquired.attempts.len(), 2);
    assert_eq!(acquired.attempts[0].source, NEXUS_LABEL);
    assert!(matches!(
        &acquired.attempts[0].failure,
        Some(SourceFailure::Refused(reason))
            if reason.contains("Nexus downloads need an API key")
    ));
    assert!(acquired.attempts[1].succeeded());
    assert!(matches!(
        &acquired.source,
        PackageSource::Https { url } if url == FIRST
    ));
    assert!(resolver.opened().is_empty());
}

#[test]
fn a_resolved_nexus_link_is_cached_and_served_by_the_nexus_source() {
    // Mirrors the Nexus branch of `nexus_archive` in
    // `apps/modn-cli/src/download/acquire.rs`, exercised by
    // `a_cancel_during_a_nexus_attempt_stops_before_the_cached_https_fallback`:
    // a resolved CDN link supplies and caches the archive.
    let temporary = tempfile::tempdir().expect("fixture");
    let cache = ArchiveCache::new(temporary.path().join("cache"));
    let bytes = b"Nexus payload".to_vec();
    let digest = pin(&bytes);
    let release = release(json!([nexus_source(123456, "NexusMod.zip")]), Some(&digest));
    let link = nexus_link(123456, "NexusMod.zip");
    let resolver = FakeResolver::default();
    resolver.nexus_download(123456, link.clone());
    resolver.open_ok(&link, &bytes);
    let transport = FakeTransport::default();

    let acquired = acquire(
        &cache,
        &release,
        CacheUse::Refresh,
        &transport,
        &resolver,
        &OperationControl::default(),
    )
    .expect("the resolved Nexus link serves the archive");

    assert_eq!(acquired.attempts.len(), 1);
    assert_eq!(acquired.attempts[0].source, NEXUS_LABEL);
    assert!(acquired.attempts[0].succeeded());
    assert!(matches!(
        &acquired.source,
        PackageSource::Nexus {
            file_id: 123456,
            ..
        }
    ));
    assert_eq!(acquired.digest, digest);
    assert_eq!(fs::read(&acquired.path).expect("archive bytes"), bytes);
    assert_eq!(resolver.opened(), vec![link.as_str().to_owned()]);
    assert!(transport.calls().is_empty());
}
