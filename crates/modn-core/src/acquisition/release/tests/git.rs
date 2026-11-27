use super::support::{
    acquire, pin, release, FakeResolver, FakeTransport, GIT_COMMIT, GIT_MISSING, GIT_PROJECT,
};
use crate::acquisition::{ArchiveCache, CacheUse, SourceFailure};
use crate::buckets::PackageSource;
use crate::error::{code_of, ErrorCode};
use crate::progress::OperationControl;
use serde_json::json;
use std::fs;

#[test]
fn a_failed_git_first_source_falls_back_to_the_next_git_source() {
    // Mirrors `a_failed_git_first_source_falls_back_to_the_next_git_source` in
    // `apps/modn-cli/tests/download_acquire_cli.rs`: the failed Git source is
    // reported, then the next Git source serves the packed archive.
    let temporary = tempfile::tempdir().expect("fixture");
    let cache = ArchiveCache::new(temporary.path().join("cache"));
    let git_sources = json!([
        {"kind": "git", "repository": GIT_MISSING, "commit": GIT_COMMIT},
        {"kind": "git", "repository": GIT_PROJECT, "commit": GIT_COMMIT}
    ]);
    let release = release(git_sources, None);
    let bytes = b"packed Git release".to_vec();
    let digest = pin(&bytes);
    let archive = temporary.path().join("packed.zip");
    fs::write(&archive, &bytes).expect("packed archive");
    let resolver = FakeResolver::default();
    resolver.git_fail(GIT_MISSING, "Git fetch failed");
    resolver.git_ok(GIT_PROJECT, archive.clone(), digest.clone());
    let transport = FakeTransport::default();

    let acquired = acquire(
        &cache,
        &release,
        CacheUse::Refresh,
        &transport,
        &resolver,
        &OperationControl::default(),
    )
    .expect("the second Git source serves the archive");

    assert_eq!(acquired.attempts.len(), 2);
    assert_eq!(
        acquired.attempts[0].source,
        format!("git {GIT_MISSING} commit {GIT_COMMIT}")
    );
    assert!(matches!(
        &acquired.attempts[0].failure,
        Some(SourceFailure::Refused(reason)) if reason.contains("Git fetch failed")
    ));
    assert_eq!(
        acquired.attempts[1].source,
        format!("git {GIT_PROJECT} commit {GIT_COMMIT}")
    );
    assert!(acquired.attempts[1].succeeded());
    assert!(matches!(
        &acquired.source,
        PackageSource::Git { repository, .. } if repository == GIT_PROJECT
    ));
    assert_eq!(acquired.digest, digest);
    assert_eq!(acquired.path, archive);
    assert!(transport.calls().is_empty());
}

#[test]
fn an_unconfirmed_git_tree_stop_ends_the_acquisition_without_the_next_source() {
    let temporary = tempfile::tempdir().expect("fixture");
    let cache = ArchiveCache::new(temporary.path().join("cache"));
    let git_sources = json!([
        {"kind": "git", "repository": GIT_MISSING, "commit": GIT_COMMIT},
        {"kind": "git", "repository": GIT_PROJECT, "commit": GIT_COMMIT}
    ]);
    let release = release(git_sources, None);
    let resolver = FakeResolver::default();
    resolver.git_fail_coded(GIT_MISSING, ErrorCode::GitProcessTreeNotStopped);
    let transport = FakeTransport::default();

    // The second Git source is not queued. The fake resolver panics if the
    // loop asks for it, so this test also proves that no fall-over happened.
    let error = acquire(
        &cache,
        &release,
        CacheUse::Refresh,
        &transport,
        &resolver,
        &OperationControl::default(),
    )
    .expect_err("an unconfirmed tree ends the acquisition");

    assert_eq!(
        code_of(&error),
        Some(ErrorCode::GitProcessTreeNotStopped),
        "{error:#}"
    );
    assert!(
        !format!("{error:#}").contains("could not be fetched"),
        "the error must not become a multi-source report: {error:#}"
    );
}
