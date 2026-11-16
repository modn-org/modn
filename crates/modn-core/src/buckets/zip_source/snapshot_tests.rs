//! ZIP catalog snapshot retention and failure atomicity.

use super::tests::{
    pack_catalog, published_snapshot, registration_bytes, signing_key, write_catalog, SigningKey,
};
use crate::buckets::{BucketStore, ModuleTrustAck, ZipCatalogSource};
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

const PRIMARY: &str = "https://primary.example/catalog.zip";

fn source() -> ZipCatalogSource {
    ZipCatalogSource::new(PRIMARY.to_owned())
}

fn fetch(bytes: Vec<u8>) -> impl FnMut(&ZipCatalogSource) -> anyhow::Result<Box<dyn Read>> {
    move |_| Ok(Box::new(Cursor::new(bytes.clone())) as Box<dyn Read>)
}

fn snapshot_of(state: &Path, published: &Path) -> PathBuf {
    published
        .ancestors()
        .find(|candidate| candidate.parent() == Some(&state.join("buckets")))
        .map(Path::to_path_buf)
        .expect("published path is inside a snapshot")
}

fn snapshots(state: &Path) -> Vec<PathBuf> {
    fs::read_dir(state.join("buckets"))
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with("zip-catalog-"))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn sync(store: &BucketStore, bytes: Vec<u8>) -> anyhow::Result<crate::buckets::ZipCatalogSync> {
    store.sync_zip_with_signer("community", None, fetch(bytes))
}

fn first_sync(state: &Path, catalog: &Path, key: &SigningKey) -> Vec<u8> {
    write_catalog(catalog, "https://mirror.example/catalog.zip", key);
    let bytes = pack_catalog(catalog);
    let store = BucketStore::new(state);
    store
        .sync_zip_with_signer("community", Some(source()), fetch(bytes.clone()))
        .expect("first sync");
    bytes
}

#[test]
fn an_unchanged_digest_reuses_the_published_snapshot() {
    let state = tempfile::tempdir().expect("state");
    let catalog = tempfile::tempdir().expect("catalog");
    let key = signing_key();
    let bytes = first_sync(state.path(), catalog.path(), &key);
    let before = published_snapshot(state.path());
    let before_registration = registration_bytes(state.path());

    let store = BucketStore::new(state.path());
    sync(&store, bytes).expect("a repeated digest is accepted");

    assert_eq!(published_snapshot(state.path()), before);
    assert_eq!(registration_bytes(state.path()), before_registration);
    assert_eq!(snapshots(state.path()).len(), 1, "one active snapshot");
}

#[test]
fn a_changed_digest_retires_the_superseded_snapshot() {
    let state = tempfile::tempdir().expect("state");
    let catalog = tempfile::tempdir().expect("catalog");
    let key = signing_key();
    let _bytes = first_sync(state.path(), catalog.path(), &key);
    let old = published_snapshot(state.path()).0;

    fs::write(catalog.path().join("extra.txt"), b"new revision").expect("extra file");
    let statement = crate::buckets::catalog_statement(catalog.path()).expect("statement");
    let signature = super::tests::sign_bytes(&key, statement.as_bytes());
    fs::write(catalog.path().join("catalog.sig"), signature).expect("signature");
    let changed = pack_catalog(catalog.path());

    let store = BucketStore::new(state.path());
    sync(&store, changed).expect("a new digest is accepted");

    let (new, _) = published_snapshot(state.path());
    assert_ne!(new, old);
    assert!(new.join("extra.txt").is_file());
    assert!(!old.exists(), "the superseded snapshot is removed");
    assert_eq!(
        snapshots(state.path()),
        vec![snapshot_of(state.path(), &new)]
    );
}

#[test]
fn a_failed_registration_write_keeps_the_registered_snapshot() {
    let state = tempfile::tempdir().expect("state");
    let catalog = tempfile::tempdir().expect("catalog");
    let key = signing_key();
    let _ = first_sync(state.path(), catalog.path(), &key);
    let before = published_snapshot(state.path());
    let before_registration = registration_bytes(state.path());

    fs::write(catalog.path().join("extra.txt"), b"new revision").expect("extra file");
    let statement = crate::buckets::catalog_statement(catalog.path()).expect("statement");
    let signature = super::tests::sign_bytes(&key, statement.as_bytes());
    fs::write(catalog.path().join("catalog.sig"), signature).expect("signature");
    let changed = pack_catalog(catalog.path());

    let store = BucketStore::new(state.path());
    store
        .fail_next_registration_write
        .store(true, Ordering::SeqCst);
    let error = sync(&store, changed)
        .expect_err("the registration write fails")
        .to_string();
    assert!(
        error.contains("injected registration write failure"),
        "{error}"
    );
    assert_eq!(registration_bytes(state.path()), before_registration);
    assert_eq!(published_snapshot(state.path()), before);
    assert_eq!(
        snapshots(state.path()),
        vec![snapshot_of(state.path(), &before.0)]
    );
}

fn write_module(catalog: &Path, version: &str) {
    let directory = catalog.join("games/test-game");
    fs::create_dir_all(&directory).expect("module directory");
    fs::write(
        directory.join("module.json"),
        serde_json::json!({
            "game_id": "test-game",
            "version": version,
            "api_version": 3,
            "entrypoint": "module.lua",
        })
        .to_string(),
    )
    .expect("module manifest");
    fs::write(
        directory.join("module.lua"),
        "return {game_id='test-game', api_version=3}",
    )
    .expect("module script");
}

fn sign_catalog(catalog: &Path, key: &SigningKey) {
    let statement = crate::buckets::catalog_statement(catalog).expect("statement");
    fs::write(
        catalog.join("catalog.sig"),
        super::tests::sign_bytes(key, statement.as_bytes()),
    )
    .expect("signature");
}

#[test]
fn a_failed_module_commit_restores_the_previous_registration() {
    let state = tempfile::tempdir().expect("state");
    let catalog = tempfile::tempdir().expect("catalog");
    let key = signing_key();
    write_module(catalog.path(), "1");
    let _bytes = first_sync(state.path(), catalog.path(), &key);
    let store = BucketStore::new(state.path());
    crate::data_dir::with_data_directory(state.path(), || {
        crate::data_dir::with_profile_directory(state.path(), || {
            fs::create_dir_all(state.path().join("buckets/main")).expect("main bucket");
            fs::write(
                state.path().join("buckets/main/bucket.json"),
                r#"{"schema_version":2,"name":"main","title":"Main"}"#,
            )
            .expect("main metadata");
            store
                .trust_modules(
                    "community",
                    ModuleTrustAck::Acknowledged {
                        expected_fingerprint: None,
                    },
                )
                .expect("trust modules");
        });
    });
    let before = published_snapshot(state.path());
    let before_registration = registration_bytes(state.path());

    write_module(catalog.path(), "2");
    fs::write(catalog.path().join("extra.txt"), b"new revision").expect("extra file");
    sign_catalog(catalog.path(), &key);
    let changed = pack_catalog(catalog.path());
    crate::game_modules::FAIL_NEXT_CATALOG_COMMIT.store(true, Ordering::SeqCst);
    let error = crate::data_dir::with_data_directory(state.path(), || {
        crate::data_dir::with_profile_directory(state.path(), || sync(&store, changed))
    })
    .expect_err("the module commit fails")
    .to_string();
    assert!(
        error.contains("injected catalog module commit failure"),
        "{error}"
    );
    assert_eq!(registration_bytes(state.path()), before_registration);
    assert_eq!(published_snapshot(state.path()), before);
    assert_eq!(
        snapshots(state.path()),
        vec![snapshot_of(state.path(), &before.0)]
    );
}
