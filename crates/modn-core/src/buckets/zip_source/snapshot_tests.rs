//! ZIP catalog snapshot retention and failure atomicity.

use super::tests::{
    pack_catalog, published_snapshot, registration_bytes, signing_key, write_catalog, SigningKey,
    RECORD,
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
fn a_changed_digest_retires_the_superseded_snapshot_at_the_next_sync() {
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
    assert!(
        old.exists(),
        "the superseded snapshot outlives the registration switch"
    );
    assert_eq!(snapshots(state.path()).len(), 2, "old and active snapshots");

    fs::write(catalog.path().join("extra.txt"), b"third revision").expect("extra file");
    let statement = crate::buckets::catalog_statement(catalog.path()).expect("statement");
    let signature = super::tests::sign_bytes(&key, statement.as_bytes());
    fs::write(catalog.path().join("catalog.sig"), signature).expect("signature");
    let changed_again = pack_catalog(catalog.path());
    sync(&store, changed_again).expect("a third digest is accepted");

    assert!(!old.exists(), "the next sync removes the retired snapshot");
    assert!(
        new.exists(),
        "a tree retired by this sync waits for the next one"
    );
}

#[test]
fn a_reader_that_loaded_the_registration_before_a_sync_keeps_its_tree() {
    let state = tempfile::tempdir().expect("state");
    let catalog = tempfile::tempdir().expect("catalog");
    let key = signing_key();
    let _bytes = first_sync(state.path(), catalog.path(), &key);
    let old = published_snapshot(state.path()).0;
    let store = BucketStore::new(state.path());

    let (loaded, loaded_reader) = std::sync::mpsc::channel();
    let (release, released_reader) = std::sync::mpsc::channel();
    let reader_state = state.path().to_path_buf();
    let reader = std::thread::spawn(move || {
        let store = BucketStore::new(&reader_state);
        let registration = store.registration("community").expect("registration");
        let path = registration.path.clone();
        loaded.send(()).expect("reader ready");
        released_reader.recv().expect("sync finished");
        store
            .validate_catalog(
                "community",
                &path,
                crate::buckets::ModuleTrustState::Untrusted,
            )
            .map(|()| path)
    });
    loaded_reader.recv().expect("reader ready");

    fs::write(catalog.path().join("extra.txt"), b"new revision").expect("extra file");
    let statement = crate::buckets::catalog_statement(catalog.path()).expect("statement");
    let signature = super::tests::sign_bytes(&key, statement.as_bytes());
    fs::write(catalog.path().join("catalog.sig"), signature).expect("signature");
    let changed = pack_catalog(catalog.path());
    sync(&store, changed).expect("a new digest is accepted");
    release.send(()).expect("release the reader");

    let read = reader
        .join()
        .expect("reader thread")
        .expect("the reader keeps its tree after the switch");
    assert_eq!(read, old);
}

#[test]
fn cleanup_keeps_a_tree_that_the_registration_names() {
    let state = tempfile::tempdir().expect("state");
    let catalog = tempfile::tempdir().expect("catalog");
    let key = signing_key();
    let _bytes = first_sync(state.path(), catalog.path(), &key);
    let (active, _) = published_snapshot(state.path());
    let snapshot = snapshot_of(state.path(), &active);
    let name = snapshot
        .file_name()
        .expect("snapshot name")
        .to_string_lossy();
    let marker = state.path().join("buckets/.retired-catalogs");
    fs::write(&marker, serde_json::json!([name]).to_string()).expect("marker");

    let store = BucketStore::new(state.path());
    store.purge_retired_catalog_snapshots();

    assert!(active.is_dir(), "the registered tree is never removed");
    assert!(marker.is_file(), "the entry waits for a later sync");
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

/// Give the store a local main bucket and trust `community` for modules.
fn trust_modules(state: &Path, store: &BucketStore) {
    crate::data_dir::with_data_directory(state, || {
        crate::data_dir::with_profile_directory(state, || {
            fs::create_dir_all(state.join("buckets/main")).expect("main bucket");
            fs::write(
                state.join("buckets/main/bucket.json"),
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
}

#[test]
fn a_failed_module_commit_restores_the_previous_registration() {
    let state = tempfile::tempdir().expect("state");
    let catalog = tempfile::tempdir().expect("catalog");
    let key = signing_key();
    write_module(catalog.path(), "1");
    let _bytes = first_sync(state.path(), catalog.path(), &key);
    let store = BucketStore::new(state.path());
    trust_modules(state.path(), &store);
    let before = published_snapshot(state.path());
    let before_registration = registration_bytes(state.path());

    write_module(catalog.path(), "2");
    fs::write(catalog.path().join("extra.txt"), b"new revision").expect("extra file");
    sign_catalog(catalog.path(), &key);
    let changed = pack_catalog(catalog.path());
    crate::game_modules::FAIL_NEXT_CATALOG_COMMIT
        .lock()
        .expect("seam")
        .insert(state.path().join("game-modules"));
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

#[test]
fn a_failed_registration_restore_keeps_the_published_snapshot() {
    let state = tempfile::tempdir().expect("state");
    let catalog = tempfile::tempdir().expect("catalog");
    let key = signing_key();
    write_module(catalog.path(), "1");
    let _bytes = first_sync(state.path(), catalog.path(), &key);
    let store = BucketStore::new(state.path());
    trust_modules(state.path(), &store);
    let before = published_snapshot(state.path()).0;

    write_module(catalog.path(), "2");
    fs::write(catalog.path().join("extra.txt"), b"new revision").expect("extra file");
    sign_catalog(catalog.path(), &key);
    let changed = pack_catalog(catalog.path());
    crate::game_modules::FAIL_NEXT_CATALOG_COMMIT
        .lock()
        .expect("seam")
        .insert(state.path().join("game-modules"));
    let error = crate::data_dir::with_data_directory(state.path(), || {
        crate::data_dir::with_profile_directory(state.path(), || {
            store
                .fail_next_registration_restore
                .store(true, Ordering::SeqCst);
            sync(&store, changed)
        })
    })
    .expect_err("the module commit and the restore fail");
    let error = format!("{error:#}");
    assert!(
        error.contains("injected catalog module commit failure"),
        "{error}"
    );
    assert!(error.contains("could not be restored"), "{error}");
    assert!(error.contains("the new catalog tree is kept"), "{error}");

    let registration = store.registration("community").expect("registration");
    assert_ne!(
        registration.path, before,
        "the registration points at the new tree"
    );
    store
        .validate_catalog(
            "community",
            &registration.path,
            crate::buckets::ModuleTrustState::Untrusted,
        )
        .expect("the registration still resolves");
}

#[test]
fn a_signer_rotation_with_unchanged_content_publishes_a_new_snapshot() {
    let state = tempfile::tempdir().expect("state");
    let catalog = tempfile::tempdir().expect("catalog");
    let old = signing_key();
    let new = signing_key();
    write_catalog(catalog.path(), "https://mirror.example/catalog.zip", &old);
    let body = crate::buckets::rotation_statement(&old.public.key_id().to_string(), &new.text)
        .expect("rotation body");
    fs::write(catalog.path().join("rotation.json"), &body).expect("rotation body");
    fs::write(
        catalog.path().join("rotation.sig"),
        super::tests::sign_bytes(
            &old,
            &crate::buckets::key_statement_bytes(
                crate::buckets::KeyChangeKind::Rotated,
                "community",
                &body,
            ),
        ),
    )
    .expect("rotation signature");
    sign_catalog(catalog.path(), &old);
    let first = pack_catalog(catalog.path());
    let store = BucketStore::new(state.path());
    store
        .sync_zip_with_signer("community", Some(source()), fetch(first))
        .expect("first sync pins the old key");
    let before = published_snapshot(state.path()).0;

    sign_catalog(catalog.path(), &new);
    fs::write(catalog.path().join("catalog.pub"), &new.text).expect("new public key");
    let rotated = pack_catalog(catalog.path());
    store
        .sync_zip_with_signer("community", None, fetch(rotated))
        .expect("the rotation applies");

    let (after, _) = published_snapshot(state.path());
    assert_ne!(after, before, "a rotation publishes its own tree");
    assert_eq!(
        fs::read(after.join("catalog.pub")).expect("new public key"),
        new.text.as_bytes()
    );
    let revision = store.zip_revision("community").expect("revision");
    assert_eq!(
        revision.expect("zip revision").signer.key_id,
        new.public.key_id().to_string()
    );
}

#[test]
fn removing_a_bucket_removes_only_its_own_snapshot() {
    let state = tempfile::tempdir().expect("state");
    let first = tempfile::tempdir().expect("first catalog");
    let key = signing_key();
    write_catalog(first.path(), "https://mirror.example/one.zip", &key);
    let store = BucketStore::new(state.path());
    store
        .sync_zip_with_signer(
            "community",
            Some(ZipCatalogSource::new(
                "https://primary.example/one.zip".to_owned(),
            )),
            fetch(pack_catalog(first.path())),
        )
        .expect("community sync");
    let community = snapshot_of(state.path(), &published_snapshot(state.path()).0);

    let second = tempfile::tempdir().expect("second catalog");
    fs::create_dir_all(second.path().join("mods/test-game")).expect("catalog directory");
    fs::write(
        second.path().join("bucket.json"),
        r#"{"schema_version":2,"name":"second","title":"Second"}"#,
    )
    .expect("bucket metadata");
    fs::write(second.path().join("mods/test-game/example.json"), RECORD).expect("record");
    let statement = crate::buckets::catalog_statement(second.path()).expect("statement");
    fs::write(second.path().join("catalog.pub"), &key.text).expect("public key");
    fs::write(
        second.path().join("catalog.sig"),
        super::tests::sign_bytes(&key, statement.as_bytes()),
    )
    .expect("signature");
    store
        .sync_zip_with_signer(
            "second",
            Some(ZipCatalogSource::new(
                "https://primary.example/two.zip".to_owned(),
            )),
            fetch(pack_catalog(second.path())),
        )
        .expect("second sync");
    let other = snapshot_of(
        state.path(),
        &store.registration("second").expect("second registration").path,
    );

    let local = tempfile::tempdir().expect("local bucket");
    fs::write(
        local.path().join("bucket.json"),
        r#"{"schema_version":2,"name":"local","title":"Local"}"#,
    )
    .expect("local metadata");
    store.add("local", local.path()).expect("local bucket");

    store.remove("community").expect("remove");

    assert!(!state
        .path()
        .join("buckets/registrations/community.json")
        .exists());
    assert!(!community.exists(), "the removed bucket's snapshot is gone");
    assert!(other.is_dir(), "another bucket's snapshot stays");
    assert!(local.path().is_dir(), "a local folder source stays");
}
