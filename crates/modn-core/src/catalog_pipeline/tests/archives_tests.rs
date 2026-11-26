//! Archive resolution and the one-archive-per-digest rule.

use super::fixtures::{self, Fixture, GAME};
use crate::acquisition::{CacheKey, CacheUse, ReleaseDigest};
use crate::buckets::{ArchiveFormat, PackageManifest, PackageSource};
use crate::catalog_pipeline::archives::{self, ResolvedArchives};
use crate::catalog_pipeline::order::{self, OrderedRelease};
use crate::catalog_pipeline::plan::ServedOrigin;
use std::path::PathBuf;

fn resolve(
    fixture: &Fixture,
    manifest: PackageManifest,
    supplied: &[PathBuf],
    cache_use: CacheUse,
) -> anyhow::Result<ResolvedArchives> {
    fixture.add(&manifest);
    let snapshot = fixture.snapshot();
    let selected = fixtures::package(&snapshot, &manifest.id);
    let order = order::resolve(&snapshot, &selected).expect("order");
    let cache = fixture.cache();
    archives::resolve(&order, &selected, supplied, &cache, cache_use)
}

fn resolved_order(fixture: &Fixture, selected_id: &str) -> (Vec<OrderedRelease>, ResolvedArchives) {
    let snapshot = fixture.snapshot();
    let selected = fixtures::package(&snapshot, selected_id);
    let order = order::resolve(&snapshot, &selected).expect("order");
    let cache = fixture.cache();
    let resolved =
        archives::resolve(&order, &selected, &[], &cache, CacheUse::Reuse).expect("archives");
    (order, resolved)
}

#[test]
fn a_supplied_archive_with_the_pinned_digest_serves_its_release() {
    let fixture = Fixture::new();
    let archive = fixture.zip("top");
    let pin = fixtures::digest(&archive);

    let resolved = resolve(
        &fixture,
        fixtures::manifest("top", Some(&pin)),
        std::slice::from_ref(&archive),
        CacheUse::Reuse,
    )
    .expect("archives");

    assert!(resolved.pending.is_empty());
    assert_eq!(resolved.releases.len(), 1);
    assert_eq!(
        resolved.releases[0].digest,
        ReleaseDigest::computed(&pin).expect("digest")
    );
    let served = resolved.releases[0].served.as_ref().expect("served");
    assert_eq!(served.path, archive);
    assert_eq!(served.origin, ServedOrigin::Supplied);
    assert_eq!(
        served.digest,
        ReleaseDigest::computed(&pin).expect("digest")
    );
    assert!(matches!(
        served.source,
        PackageSource::Https { ref url } if url == "https://example.test/top.zip"
    ));
}

#[test]
fn a_supplied_archive_that_matches_no_release_is_refused() {
    let fixture = Fixture::new();
    let archive = fixture.zip("top");

    let error = resolve(
        &fixture,
        fixtures::manifest("top", Some(&"c".repeat(64))),
        std::slice::from_ref(&archive),
        CacheUse::Reuse,
    )
    .expect_err("unmatched archive");

    assert_eq!(
        error.to_string(),
        format!(
            "{}: archive BLAKE3 does not match catalog manifest of main/{GAME}/top or its dependencies",
            archive.display()
        )
    );
}

#[test]
fn a_supplied_archive_with_the_wrong_format_is_refused() {
    let fixture = Fixture::new();
    let archive = fixture.zip("top");
    let pin = fixtures::digest(&archive);
    let mut manifest = fixtures::manifest("top", Some(&pin));
    manifest.archive_format = Some(ArchiveFormat::SevenZ);
    manifest.sources = vec![PackageSource::Manual {
        filename: "top.7z".to_owned(),
    }];

    let error = resolve(&fixture, manifest, &[archive], CacheUse::Reuse).expect_err("format");

    assert_eq!(
        error.to_string(),
        format!("main/{GAME}/top: archive format does not match catalog manifest")
    );
}

#[test]
fn a_supplied_path_that_is_not_a_regular_file_is_refused() {
    let fixture = Fixture::new();
    let directory = fixture.data().to_path_buf();

    let error = resolve(
        &fixture,
        fixtures::manifest("top", Some(&"a".repeat(64))),
        std::slice::from_ref(&directory),
        CacheUse::Reuse,
    )
    .expect_err("directory");

    assert_eq!(
        error.to_string(),
        format!("archive must be a regular file: {}", directory.display())
    );
}

#[test]
fn a_verified_cache_entry_serves_the_release() {
    let fixture = Fixture::new();
    let archive = fixture.zip("top");
    let pin = fixtures::digest(&archive);
    let manifest = fixtures::manifest("top", Some(&pin));
    fixture.add(&manifest);
    let snapshot = fixture.snapshot();
    let selected = fixtures::package(&snapshot, "top");
    let order = order::resolve(&snapshot, &selected).expect("order");
    let cache = fixture.cache();
    let key = CacheKey::new(GAME, "top", "1.0", "https://example.test/top.zip", &pin);
    let cached_path = cache.path(&key);
    std::fs::create_dir_all(cache.root()).expect("cache root");
    std::fs::copy(&archive, &cached_path).expect("cache entry");

    let resolved =
        archives::resolve(&order, &selected, &[], &cache, CacheUse::Reuse).expect("archives");

    assert!(resolved.pending.is_empty());
    assert_eq!(
        resolved.releases[0].digest,
        ReleaseDigest::pinned(&pin).expect("pin")
    );
    let served = resolved.releases[0].served.as_ref().expect("served");
    assert_eq!(served.path, cached_path);
    assert_eq!(served.origin, ServedOrigin::Cache);
    assert!(matches!(
        served.source,
        PackageSource::Https { ref url } if url == "https://example.test/top.zip"
    ));
}

#[test]
fn a_cache_entry_with_the_wrong_digest_does_not_serve_the_release() {
    let fixture = Fixture::new();
    let archive = fixture.zip("top");
    let pin = fixtures::digest(&archive);
    let manifest = fixtures::manifest("top", Some(&pin));
    fixture.add(&manifest);
    let snapshot = fixture.snapshot();
    let selected = fixtures::package(&snapshot, "top");
    let order = order::resolve(&snapshot, &selected).expect("order");
    let cache = fixture.cache();
    let key = CacheKey::new(GAME, "top", "1.0", "https://example.test/top.zip", &pin);
    let cached_path = cache.path(&key);
    std::fs::create_dir_all(cache.root()).expect("cache root");
    std::fs::write(&cached_path, b"not the pinned archive").expect("tampered entry");

    let resolved =
        archives::resolve(&order, &selected, &[], &cache, CacheUse::Reuse).expect("archives");

    assert!(resolved.releases[0].served.is_none());
    assert_eq!(resolved.pending.len(), 1);
    assert_eq!(
        resolved.releases[0].digest,
        ReleaseDigest::pinned(&pin).expect("pin")
    );
}

#[test]
fn a_release_without_an_archive_is_pending_with_its_sources_in_order() {
    let fixture = Fixture::new();
    let pin = "a".repeat(64);
    let mut manifest = fixtures::manifest("top", Some(&pin));
    manifest.sources = vec![
        PackageSource::Manual {
            filename: "top.zip".to_owned(),
        },
        PackageSource::Https {
            url: "https://example.test/top.zip".to_owned(),
        },
    ];

    let resolved = resolve(&fixture, manifest, &[], CacheUse::Reuse).expect("archives");

    assert_eq!(resolved.pending.len(), 1);
    let pending = &resolved.pending[0];
    assert_eq!(pending.reference.to_string(), format!("main/{GAME}/top"));
    assert_eq!(pending.version, "1.0");
    assert_eq!(pending.digest, ReleaseDigest::pinned(&pin).expect("pin"));
    assert!(pending.downloadable);
    assert_eq!(pending.sources.len(), 2);
    assert!(matches!(
        pending.sources[0].source,
        PackageSource::Manual { .. }
    ));
    assert!(matches!(
        pending.sources[1].source,
        PackageSource::Https { .. }
    ));
}

#[test]
fn a_release_with_only_manual_sources_is_not_downloadable() {
    let fixture = Fixture::new();
    let mut manifest = fixtures::manifest("top", Some(&"a".repeat(64)));
    manifest.sources = vec![PackageSource::Manual {
        filename: "top.zip".to_owned(),
    }];

    let resolved = resolve(&fixture, manifest, &[], CacheUse::Reuse).expect("archives");

    assert_eq!(resolved.pending.len(), 1);
    assert!(!resolved.pending[0].downloadable);
}

#[test]
fn releases_that_share_an_archive_are_placed_once() {
    let fixture = Fixture::new();
    let pin = "a".repeat(64);
    let mut top = fixtures::manifest("top", Some(&"b".repeat(64)));
    top.dependencies = vec!["a".to_owned(), "b".to_owned()];
    fixture.add(&fixtures::manifest("a", Some(&pin)));
    fixture.add(&fixtures::manifest("b", Some(&pin)));
    fixture.add(&top);
    let (order, resolved) = resolved_order(&fixture, "top");

    let kept =
        archives::one_archive_per_digest(&order, &resolved, &[0, 1]).expect("shared archive");

    assert_eq!(kept, [0, 2]);
}

#[test]
fn releases_that_share_an_archive_must_declare_the_same_requirements() {
    let fixture = Fixture::new();
    let pin = "a".repeat(64);
    let mut a = fixtures::manifest("a", Some(&pin));
    a.conflicts = vec!["other".to_owned()];
    let mut top = fixtures::manifest("top", Some(&"b".repeat(64)));
    top.dependencies = vec!["a".to_owned(), "b".to_owned()];
    fixture.add(&a);
    fixture.add(&fixtures::manifest("b", Some(&pin)));
    fixture.add(&top);
    let (order, resolved) = resolved_order(&fixture, "top");

    let error = archives::one_archive_per_digest(&order, &resolved, &[0, 1])
        .expect_err("different declarations");

    assert_eq!(
        error.to_string(),
        format!(
            "main/{GAME}/a and main/{GAME}/b share one archive but declare different dependencies or conflicts; install them separately"
        )
    );
}

#[test]
fn the_selected_release_cannot_share_an_archive_with_a_dependency() {
    let fixture = Fixture::new();
    let pin = "a".repeat(64);
    let mut top = fixtures::manifest("top", Some(&pin));
    top.dependencies = vec!["a".to_owned()];
    fixture.add(&fixtures::manifest("a", Some(&pin)));
    fixture.add(&top);
    let (order, resolved) = resolved_order(&fixture, "top");

    let error = archives::one_archive_per_digest(&order, &resolved, &[0])
        .expect_err("shared selected archive");

    assert_eq!(
        error.to_string(),
        format!(
            "main/{GAME}/top: its archive is also the archive of a missing dependency; install that dependency first"
        )
    );
}

#[test]
fn releases_without_a_digest_are_never_combined() {
    let fixture = Fixture::new();
    let mut top = fixtures::git_manifest("top", &"b".repeat(40));
    top.dependencies = vec!["a".to_owned(), "b".to_owned()];
    fixture.add(&fixtures::git_manifest("a", &"a".repeat(40)));
    fixture.add(&fixtures::git_manifest("b", &"a".repeat(40)));
    fixture.add(&top);
    let (order, resolved) = resolved_order(&fixture, "top");
    assert!(resolved
        .releases
        .iter()
        .all(|release| release.digest.is_unknown()));

    let kept =
        archives::one_archive_per_digest(&order, &resolved, &[0, 1]).expect("unknown digests");

    assert_eq!(kept, [0, 1, 2]);
}
