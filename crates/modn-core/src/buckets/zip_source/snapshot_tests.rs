//! ZIP catalog snapshot retention and failure atomicity.

use super::tests::{
    pack_catalog, published_snapshot, registration_bytes, signing_key, write_catalog, SigningKey,
};
use crate::buckets::{BucketStore, ModuleTrustAck, ZipCatalogSource};
use modn_core::manager_roots::ManagerRoots;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const PRIMARY: &str = "https://primary.example/catalog.zip";

pub(super) fn source() -> ZipCatalogSource {
    ZipCatalogSource::new(PRIMARY.to_owned())
}

pub(super) fn fetch(
    bytes: Vec<u8>,
) -> impl FnMut(&ZipCatalogSource) -> anyhow::Result<Box<dyn Read>> {
    move |_| Ok(Box::new(Cursor::new(bytes.clone())) as Box<dyn Read>)
}

pub(super) fn snapshot_of(state: &Path, published: &Path) -> PathBuf {
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

fn use_clock(store: &mut BucketStore, now: &Arc<AtomicU64>) {
    let clock = Arc::clone(now);
    store.clock = Some(Arc::new(move || clock.load(Ordering::SeqCst)));
}

pub(super) fn packed_revision(catalog: &Path, key: &SigningKey, marker: &str) -> Vec<u8> {
    fs::write(catalog.join("extra.txt"), marker.as_bytes()).expect("extra file");
    sign_catalog(catalog, key);
    pack_catalog(catalog)
}

pub(super) fn first_sync(state: &Path, catalog: &Path, key: &SigningKey) -> Vec<u8> {
    write_catalog(catalog, "https://mirror.example/catalog.zip", key);
    let bytes = pack_catalog(catalog);
    let store = BucketStore::new(&ManagerRoots::new(state, None));
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

    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));
    sync(&store, bytes).expect("a repeated digest is accepted");

    assert_eq!(published_snapshot(state.path()), before);
    assert_eq!(registration_bytes(state.path()), before_registration);
    assert_eq!(snapshots(state.path()).len(), 1, "one active snapshot");
}
#[test]
fn a_changed_digest_retires_the_superseded_snapshot_until_the_grace_period_passes() {
    let state = tempfile::tempdir().expect("state");
    let catalog = tempfile::tempdir().expect("catalog");
    let key = signing_key();
    let _bytes = first_sync(state.path(), catalog.path(), &key);
    let old = published_snapshot(state.path()).0;
    let now = Arc::new(AtomicU64::new(1_000_000));
    let mut store = BucketStore::new(&ManagerRoots::new(state.path(), None));
    use_clock(&mut store, &now);

    let changed = packed_revision(catalog.path(), &key, "new revision");
    sync(&store, changed).expect("a new digest is accepted");

    let (new, _) = published_snapshot(state.path());
    assert_ne!(new, old);
    assert!(new.join("extra.txt").is_file());
    assert!(
        old.exists(),
        "the superseded snapshot outlives the registration switch"
    );
    assert_eq!(snapshots(state.path()).len(), 2, "old and active snapshots");

    let changed_again = packed_revision(catalog.path(), &key, "third revision");
    sync(&store, changed_again).expect("a third digest is accepted");

    assert!(
        old.exists(),
        "a retired snapshot waits for the grace period"
    );

    now.store(
        1_000_000 + crate::buckets::retire::RETIRED_GRACE.as_secs(),
        Ordering::SeqCst,
    );
    let changed_once_more = packed_revision(catalog.path(), &key, "fourth revision");
    sync(&store, changed_once_more).expect("a fourth digest is accepted");

    assert!(
        !old.exists(),
        "the first sync after the grace period removes the retired snapshot"
    );
}
#[test]
fn a_malformed_retire_record_never_deletes_a_retired_tree() {
    let state = tempfile::tempdir().expect("state");
    let catalog = tempfile::tempdir().expect("catalog");
    let key = signing_key();
    let _bytes = first_sync(state.path(), catalog.path(), &key);
    let old = published_snapshot(state.path()).0;
    let mut store = BucketStore::new(&ManagerRoots::new(state.path(), None));

    let changed = packed_revision(catalog.path(), &key, "second revision");
    sync(&store, changed).expect("a new digest is accepted");
    let record = state.path().join("buckets/.retired-catalogs");
    fs::write(&record, b"{malformed").expect("malformed record");
    let now = Arc::new(AtomicU64::new(u64::MAX));
    use_clock(&mut store, &now);
    let changed_again = packed_revision(catalog.path(), &key, "third revision");
    sync(&store, changed_again).expect("a third digest is accepted");

    assert!(
        old.is_dir(),
        "a malformed retire record never causes an immediate delete"
    );

    fs::remove_file(&record).expect("remove the retire record");
    let changed_once_more = packed_revision(catalog.path(), &key, "fourth revision");
    sync(&store, changed_once_more).expect("a fourth digest is accepted");

    assert!(
        old.is_dir(),
        "a missing retire record never causes an immediate delete"
    );
}

#[test]
fn a_retire_record_without_a_time_gets_a_fresh_time() {
    let state = tempfile::tempdir().expect("state");
    let catalog = tempfile::tempdir().expect("catalog");
    let key = signing_key();
    let _bytes = first_sync(state.path(), catalog.path(), &key);
    let old = published_snapshot(state.path()).0;
    let old_name = snapshot_of(state.path(), &old)
        .file_name()
        .expect("snapshot name")
        .to_string_lossy()
        .into_owned();
    let now = Arc::new(AtomicU64::new(1_000_000));
    let mut store = BucketStore::new(&ManagerRoots::new(state.path(), None));
    use_clock(&mut store, &now);
    let changed = packed_revision(catalog.path(), &key, "second revision");
    sync(&store, changed).expect("a new digest is accepted");

    let record = state.path().join("buckets/.retired-catalogs");
    fs::write(&record, serde_json::json!([old_name.clone()]).to_string()).expect("legacy record");
    let changed_again = packed_revision(catalog.path(), &key, "third revision");
    sync(&store, changed_again).expect("a third digest is accepted");

    assert!(old.is_dir(), "the legacy record is kept, not deleted");
    let entries: serde_json::Value =
        serde_json::from_slice(&fs::read(&record).expect("retire record")).expect("JSON");
    let entry = entries
        .as_array()
        .expect("retire records")
        .iter()
        .find(|entry| entry["name"] == old_name)
        .expect("the legacy entry is kept");
    assert_eq!(entry["retired_at"], 1_000_000);
}

#[test]
fn a_retired_snapshot_survives_two_syncs_while_a_reader_still_needs_it() {
    let state = tempfile::tempdir().expect("state");
    let catalog = tempfile::tempdir().expect("catalog");
    let key = signing_key();
    let _bytes = first_sync(state.path(), catalog.path(), &key);
    let old = published_snapshot(state.path()).0;
    let now = Arc::new(AtomicU64::new(1_000_000));
    let mut store = BucketStore::new(&ManagerRoots::new(state.path(), None));
    use_clock(&mut store, &now);

    let (loaded, loaded_reader) = std::sync::mpsc::channel();
    let (release, released_reader) = std::sync::mpsc::channel();
    let reader_state = state.path().to_path_buf();
    let reader = std::thread::spawn(move || {
        let store = BucketStore::new(&ManagerRoots::new(&reader_state, None));
        let registration = store.registration("community").expect("registration");
        let path = registration.path.clone();
        loaded.send(()).expect("reader ready");
        released_reader.recv().expect("syncs finished");
        store
            .validate_catalog(
                "community",
                &path,
                crate::buckets::ModuleTrustState::Untrusted,
            )
            .map(|()| path)
    });
    loaded_reader.recv().expect("reader ready");

    let changed = packed_revision(catalog.path(), &key, "second revision");
    sync(&store, changed).expect("a new digest is accepted");
    let changed_again = packed_revision(catalog.path(), &key, "third revision");
    sync(&store, changed_again).expect("a third digest is accepted");

    assert!(
        old.is_dir(),
        "two syncs inside the grace period keep the retired tree"
    );
    release.send(()).expect("release the reader");

    let read = reader
        .join()
        .expect("reader thread")
        .expect("the reader keeps its tree after both syncs");
    assert_eq!(read, old);

    now.store(
        1_000_000 + crate::buckets::retire::RETIRED_GRACE.as_secs(),
        Ordering::SeqCst,
    );
    let changed_once_more = packed_revision(catalog.path(), &key, "fourth revision");
    sync(&store, changed_once_more).expect("a fourth digest is accepted");

    assert!(
        !old.exists(),
        "the first sync after the grace period removes the retired tree"
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

    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));
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
}
#[test]
fn a_failed_module_commit_restores_the_previous_registration() {
    let state = tempfile::tempdir().expect("state");
    let catalog = tempfile::tempdir().expect("catalog");
    let key = signing_key();
    write_module(catalog.path(), "1");
    let _bytes = first_sync(state.path(), catalog.path(), &key);
    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));
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
    let error = sync(&store, changed)
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
    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));
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
    store
        .fail_next_registration_restore
        .store(true, Ordering::SeqCst);
    let error = sync(&store, changed).expect_err("the module commit and the restore fail");
    let error = format!("{error:#}");
    assert!(
        error.contains("injected catalog module commit failure"),
        "{error}"
    );
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
    assert!(error.contains("could not be restored"), "{error}");
    assert!(error.contains("the new catalog tree is kept"), "{error}");
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
    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));
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
