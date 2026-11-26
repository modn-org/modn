use super::support::{
    acquire, https_sources, nexus_source, pin, release, seed_cache, FakeResolver, FakeTransport,
    FIRST, GIT_COMMIT, GIT_PROJECT,
};
use crate::acquisition::{ArchiveCache, CacheUse, GitCacheKey, NexusFile};
use crate::buckets::PackageSource;
use crate::progress::OperationControl;
use serde_json::json;
use std::fs;

#[test]
fn a_verified_cache_hit_serves_without_a_transport_call() {
    // Mirrors `a_verified_cache_hit_installs_without_a_transport_call` in
    // `apps/modn-cli/tests/source_fallback_cli.rs`: a verified entry is served
    // with no transport or resolver call and an empty attempt list.
    let temporary = tempfile::tempdir().expect("fixture");
    let cache = ArchiveCache::new(temporary.path().join("cache"));
    let bytes = b"cached payload".to_vec();
    let digest = pin(&bytes);
    let release = release(https_sources(), Some(&digest));
    let expected = seed_cache(&cache, &release, FIRST, &bytes);
    let transport = FakeTransport::default();
    let resolver = FakeResolver::default();

    let acquired = acquire(
        &cache,
        &release,
        CacheUse::Reuse,
        &transport,
        &resolver,
        &OperationControl::default(),
    )
    .expect("the verified cache entry serves the release");

    assert!(acquired.attempts.is_empty());
    assert_eq!(acquired.path, expected);
    assert_eq!(acquired.digest, digest);
    assert!(matches!(
        &acquired.source,
        PackageSource::Https { url } if url == FIRST
    ));
    assert!(transport.calls().is_empty());
}

#[test]
fn a_cached_secondary_nexus_archive_is_served_by_its_own_source() {
    // Mirrors `a_cached_secondary_nexus_archive_installs_without_a_key_or_request`
    // in `apps/modn-cli/src/download/acquire/tests.rs`: the cache hit names the
    // second Nexus source, and no provider call happens.
    let temporary = tempfile::tempdir().expect("fixture");
    let cache = ArchiveCache::new(temporary.path().join("cache"));
    let bytes = b"secondary Nexus payload".to_vec();
    let digest = pin(&bytes);
    let release = release(
        json!([
            nexus_source(111111, "First.zip"),
            nexus_source(222222, "Second.zip")
        ]),
        Some(&digest),
    );
    let second = NexusFile::new("cyberpunk2077", 6945, 222222, "Second.zip")
        .expect("fixture file")
        .cache_url();
    seed_cache(&cache, &release, &second, &bytes);
    let transport = FakeTransport::default();
    let resolver = FakeResolver::default();

    let acquired = acquire(
        &cache,
        &release,
        CacheUse::Reuse,
        &transport,
        &resolver,
        &OperationControl::default(),
    )
    .expect("the cached second Nexus archive serves the release");

    assert!(acquired.attempts.is_empty());
    assert!(matches!(
        &acquired.source,
        PackageSource::Nexus {
            file_id: 222222,
            ..
        }
    ));
    assert!(transport.calls().is_empty());
}

#[test]
fn a_cached_git_archive_serves_without_the_git_resolver() {
    // Mirrors the undo half of
    // `a_git_fallback_is_recorded_as_the_serving_source_and_undo_uses_its_cache`
    // in `apps/modn-cli/tests/source_fallback_cli.rs`: a cached packed commit
    // is served without running Git.
    let temporary = tempfile::tempdir().expect("fixture");
    let cache = ArchiveCache::new(temporary.path().join("cache"));
    let release = release(
        json!([{"kind": "git", "repository": GIT_PROJECT, "commit": GIT_COMMIT}]),
        None,
    );
    let bytes = b"cached Git release".to_vec();
    let digest = pin(&bytes);
    fs::create_dir_all(cache.root()).expect("cache directory");
    let key = GitCacheKey::new(
        &release.package.game,
        &release.package.id,
        &release.package.version,
        GIT_COMMIT,
    );
    let expected = cache.git_path(&key, &digest);
    fs::write(&expected, &bytes).expect("git cache entry");
    let transport = FakeTransport::default();
    let resolver = FakeResolver::default();

    let acquired = acquire(
        &cache,
        &release,
        CacheUse::Reuse,
        &transport,
        &resolver,
        &OperationControl::default(),
    )
    .expect("the cached Git archive serves the release");

    assert!(acquired.attempts.is_empty());
    assert_eq!(acquired.path, expected);
    assert_eq!(acquired.digest, digest);
    assert!(matches!(
        &acquired.source,
        PackageSource::Git { repository, .. } if repository == GIT_PROJECT
    ));
    assert!(transport.calls().is_empty());
}
