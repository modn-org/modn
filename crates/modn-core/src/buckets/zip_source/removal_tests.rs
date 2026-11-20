//! Catalog tree removal when a bucket ends.
use super::snapshot_tests::{fetch, first_sync, packed_revision, snapshot_of};
use super::tests::{pack_catalog, published_snapshot, signing_key, write_catalog, RECORD};
use crate::buckets::{BucketStore, ZipCatalogSource};
use std::fs;
use std::sync::atomic::Ordering;

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
        &store
            .registration("second")
            .expect("second registration")
            .path,
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

#[test]
fn a_failed_registration_removal_keeps_the_snapshot() {
    let state = tempfile::tempdir().expect("state");
    let catalog = tempfile::tempdir().expect("catalog");
    let key = signing_key();
    write_catalog(catalog.path(), "https://mirror.example/one.zip", &key);
    let store = BucketStore::new(state.path());
    store
        .sync_zip_with_signer(
            "community",
            Some(ZipCatalogSource::new(
                "https://primary.example/one.zip".to_owned(),
            )),
            fetch(pack_catalog(catalog.path())),
        )
        .expect("community sync");
    let snapshot = snapshot_of(state.path(), &published_snapshot(state.path()).0);

    store
        .fail_next_registration_remove
        .store(true, Ordering::SeqCst);
    let error = store
        .remove("community")
        .expect_err("the failed removal is reported")
        .to_string();
    assert!(
        error.contains("injected registration removal failure"),
        "{error}"
    );
    assert!(snapshot.is_dir(), "a failed removal keeps the snapshot");
    assert!(state
        .path()
        .join("buckets/registrations/community.json")
        .is_file());

    store.remove("community").expect("a later removal succeeds");
    assert!(!snapshot.exists());
}

#[test]
fn removing_a_bucket_removes_its_live_and_retired_snapshots() {
    let state = tempfile::tempdir().expect("state");
    let catalog = tempfile::tempdir().expect("catalog");
    let key = signing_key();
    let _bytes = first_sync(state.path(), catalog.path(), &key);
    let retired = snapshot_of(state.path(), &published_snapshot(state.path()).0);
    let store = BucketStore::new(state.path());
    let changed = packed_revision(catalog.path(), &key, "second revision");
    store
        .sync_zip_with_signer("community", None, fetch(changed))
        .expect("a changed digest sync");
    let live = snapshot_of(state.path(), &published_snapshot(state.path()).0);
    assert!(retired.is_dir() && live.is_dir());

    store.remove("community").expect("remove");

    assert!(
        !retired.exists(),
        "the removed bucket's retired snapshot is gone"
    );
    assert!(!live.exists(), "the removed bucket's live snapshot is gone");
}

#[test]
fn removing_a_bucket_keeps_a_retired_tree_another_registration_names() {
    let state = tempfile::tempdir().expect("state");
    let catalog = tempfile::tempdir().expect("catalog");
    let key = signing_key();
    let _bytes = first_sync(state.path(), catalog.path(), &key);
    let retired = snapshot_of(state.path(), &published_snapshot(state.path()).0);
    let store = BucketStore::new(state.path());
    let changed = packed_revision(catalog.path(), &key, "second revision");
    store
        .sync_zip_with_signer("community", None, fetch(changed))
        .expect("a changed digest sync");
    let live = snapshot_of(state.path(), &published_snapshot(state.path()).0);

    let second = state.path().join("buckets/registrations/second.json");
    fs::write(&second, serde_json::json!({"path": retired}).to_string())
        .expect("second registration");
    store.remove("community").expect("remove");

    assert!(
        retired.is_dir(),
        "a tree that another registration names stays"
    );
    assert!(!live.exists());
    assert!(second.is_file());
}

#[test]
fn removing_a_local_bucket_named_like_a_snapshot_keeps_every_file() {
    let state = tempfile::tempdir().expect("state");
    let state_root = state.path().canonicalize().expect("canonical state");
    let root = state_root.join("buckets");
    fs::create_dir_all(&root).expect("bucket root");
    for name in ["git-catalog-local", "zip-catalog-local"] {
        let folder = root.join(name);
        fs::create_dir_all(&folder).expect("local folder");
        fs::write(
            folder.join("bucket.json"),
            format!(r#"{{"schema_version":2,"name":"{name}","title":"Local"}}"#),
        )
        .expect("bucket metadata");
        let payload = folder.join("user.txt");
        fs::write(&payload, b"user data").expect("user file");

        let store = BucketStore::new(&state_root);
        store.add(name, &folder).expect("local bucket");
        store.remove(name).expect("remove the local bucket");

        assert_eq!(
            fs::read(&payload).expect("user file after removal"),
            b"user data",
            "{name} keeps every user file byte-identical"
        );
        assert!(folder.join("bucket.json").is_file());
    }
}
