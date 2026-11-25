use anyhow::Result;
use minisign::{sign, KeyPair};
use modn_core::acquisition::{
    ArchiveCache, CacheKey, CacheUse, FetchError, Origin, SourceBody, SourceCandidate,
    SourceFailure, SourcePolicy, SourceTransport,
};
use modn_core::buckets::{
    catalog_statement, verify_catalog_directory, CatalogPublicKey, PackageManifest, PackageSource,
};
use modn_core::progress::OperationControl;
use serde_json::json;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

const FIRST: &str = "https://origin.example/fixture-1.0.zip";
const SECOND: &str = "https://mirror.example/fixture-1.0.zip";
const NAME: &str = "community";

struct SignedCatalog {
    root: tempfile::TempDir,
    manifest: PackageManifest,
    public: CatalogPublicKey,
}

impl SignedCatalog {
    fn root(&self) -> PathBuf {
        self.root.path().join("catalog")
    }
}

fn write_catalog_dir(root: &Path, archive_pin: &str) -> PathBuf {
    let catalog = root.join("catalog");
    fs::create_dir_all(catalog.join("mods/test-game")).expect("catalog directories");
    fs::write(
        catalog.join("bucket.json"),
        serde_json::to_vec(&json!({
            "schema_version": 2,
            "name": NAME,
            "title": "Community fixture"
        }))
        .expect("catalog metadata JSON"),
    )
    .expect("catalog metadata");
    fs::write(
        catalog.join("mods/test-game/fixture.json"),
        serde_json::to_vec(&json!({
            "schema_version": 2,
            "id": "fixture",
            "game": "test-game",
            "name": "Fixture",
            "version": "1.0",
            "archive_format": "zip",
            "blake3": archive_pin,
            "sources": [
                {"kind": "https", "url": FIRST},
                {"kind": "https", "url": SECOND}
            ]
        }))
        .expect("manifest JSON"),
    )
    .expect("manifest");
    catalog
}

fn signed_catalog(archive_pin: &str) -> SignedCatalog {
    let holder = tempfile::tempdir().expect("fixture root");
    let catalog = write_catalog_dir(holder.path(), archive_pin);
    let pair = KeyPair::generate_unencrypted_keypair().expect("fixture key pair");
    let public_text = pair.pk.to_box().expect("public key").into_string();
    fs::write(catalog.join("catalog.pub"), &public_text).expect("catalog public key");
    let statement = catalog_statement(&catalog).expect("catalog statement");
    let signature = sign(
        Some(&pair.pk),
        &pair.sk,
        Cursor::new(statement.as_bytes()),
        Some("fixture catalog"),
        Some("fixture catalog signature"),
    )
    .expect("catalog signature")
    .into_string();
    fs::write(catalog.join("catalog.sig"), signature).expect("catalog signature file");

    let public = CatalogPublicKey::parse(&public_text).expect("parsed public key");
    let verified = verify_catalog_directory(&catalog, &public).expect("verified signed catalog");
    assert_eq!(verified.key_id, public.key_id());

    let bytes = fs::read(catalog.join("mods/test-game/fixture.json")).expect("verified manifest");
    let manifest: PackageManifest = serde_json::from_slice(&bytes).expect("manifest record");
    SignedCatalog {
        root: holder,
        manifest,
        public,
    }
}

#[derive(Default)]
struct RemovedOrigin {
    failures: BTreeMap<String, FetchError>,
    bodies: BTreeMap<String, Vec<u8>>,
    calls: RefCell<Vec<String>>,
}

impl SourceTransport for RemovedOrigin {
    fn open(&self, url: &str, _policy: &SourcePolicy) -> Result<SourceBody, FetchError> {
        self.calls.borrow_mut().push(url.to_owned());
        if let Some(failure) = self.failures.get(url) {
            return Err(failure.clone());
        }
        let body = self.bodies.get(url).cloned().unwrap_or_default();
        Ok(SourceBody::new(Box::new(Cursor::new(body)), None))
    }
}

fn candidates(manifest: &PackageManifest) -> Vec<SourceCandidate> {
    manifest
        .sources
        .iter()
        .map(|source| match source {
            PackageSource::Https { url } => SourceCandidate::fetch(url.clone()),
            other => SourceCandidate::refused(
                format!("{other:?}"),
                "the gate fixture lists archive origins only",
            ),
        })
        .collect()
}

fn key<'a>(url: &'a str, blake3: &'a str) -> CacheKey<'a> {
    CacheKey::new("test-game", "fixture", "1.0", url, blake3)
}

fn policy() -> SourcePolicy {
    SourcePolicy::new(Duration::from_secs(1), Duration::from_secs(1), 0)
}

fn cache_listing(cache: &ArchiveCache) -> Vec<String> {
    fs::read_dir(cache.root())
        .map(|entries| {
            let mut names: Vec<String> = entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        })
        .unwrap_or_default()
}

#[test]
fn with_the_first_origin_removed_the_install_succeeds_from_the_second_archive_origin() {
    let archive = b"the fixture archive served by the second origin".to_vec();
    let pin = blake3::hash(&archive).to_hex().to_string();
    let fixture = signed_catalog(&pin);
    assert_eq!(fixture.public.key_id().to_string().len(), 16);
    assert_eq!(fixture.manifest.sources.len(), 2);
    assert!(fixture.root().join("catalog.sig").is_file());

    let removed = RemovedOrigin {
        failures: BTreeMap::from([(FIRST.to_owned(), FetchError::Http(404))]),
        bodies: BTreeMap::from([(SECOND.to_owned(), archive.clone())]),
        ..RemovedOrigin::default()
    };
    let cache_root = tempfile::tempdir().expect("cache root");
    let cache = ArchiveCache::new(cache_root.path().join("cache"));
    let outcome = cache
        .acquire_from_sources(
            &key(FIRST, &pin),
            CacheUse::Reuse,
            &candidates(&fixture.manifest),
            &removed,
            policy(),
            None,
            &OperationControl::default(),
        )
        .expect("second origin supplies the verified archive");

    assert_eq!(outcome.acquired.origin, Origin::Download);
    assert_eq!(
        fs::read(&outcome.acquired.path).expect("verified archive"),
        archive
    );
    assert_eq!(outcome.attempts.len(), 2);
    assert_eq!(outcome.attempts[0].source, FIRST);
    assert_eq!(outcome.attempts[0].failure, Some(SourceFailure::Http(404)));
    assert!(outcome.attempts[1].succeeded());
    assert_eq!(
        *removed.calls.borrow(),
        vec![FIRST.to_owned(), SECOND.to_owned()]
    );
    assert_eq!(outcome.acquired.path, cache.path(&key(SECOND, &pin)));
    assert!(!cache.path(&key(FIRST, &pin)).exists());
    for name in cache_listing(&cache) {
        assert!(!name.ends_with(".download"), "partial file kept: {name}");
    }
}

#[test]
fn a_second_origin_that_does_not_match_the_pin_leaves_no_cache_entry() {
    let archive = b"the archive the catalog pins".to_vec();
    let pin = blake3::hash(&archive).to_hex().to_string();
    let fixture = signed_catalog(&pin);
    let removed = RemovedOrigin {
        failures: BTreeMap::from([(FIRST.to_owned(), FetchError::Http(404))]),
        bodies: BTreeMap::from([(
            SECOND.to_owned(),
            b"bytes that the catalog did not pin".to_vec(),
        )]),
        ..RemovedOrigin::default()
    };
    let cache_root = tempfile::tempdir().expect("cache root");
    let cache = ArchiveCache::new(cache_root.path().join("cache"));
    let error = cache
        .acquire_from_sources(
            &key(FIRST, &pin),
            CacheUse::Reuse,
            &candidates(&fixture.manifest),
            &removed,
            policy(),
            None,
            &OperationControl::default(),
        )
        .expect_err("a mismatched body must not install");
    let message = format!("{error:#}");
    assert!(message.contains("digest mismatch"), "{message}");
    assert!(message.contains(&pin), "{message}");
    assert!(!cache.path(&key(FIRST, &pin)).exists());
    assert!(!cache.path(&key(SECOND, &pin)).exists());
    for name in cache_listing(&cache) {
        assert!(!name.ends_with(".download"), "partial file kept: {name}");
    }
    assert_eq!(
        *removed.calls.borrow(),
        vec![FIRST.to_owned(), SECOND.to_owned()]
    );
}
