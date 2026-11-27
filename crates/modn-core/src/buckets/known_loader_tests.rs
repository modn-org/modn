use super::test_support::{entry, file_with, install, seed_main, tree_with, VALID};
use super::KNOWN_BUCKETS_FILE;
use crate::buckets::BucketStore;
use crate::manager_roots::ManagerRoots;
use std::fs;

#[test]
fn known_buckets_loader_is_empty_when_the_file_is_absent() {
    let data = tempfile::tempdir().expect("fixture");
    seed_main(data.path(), None);
    let store = BucketStore::new(&ManagerRoots::new(data.path(), None));
    let known = store.known_buckets().expect("loader");
    assert!(known.is_empty());
    assert_eq!(known.len(), 0);
}

#[test]
fn known_buckets_loader_reads_the_official_main_copy() {
    let data = tempfile::tempdir().expect("fixture");
    seed_main(data.path(), Some(VALID));
    let store = BucketStore::new(&ManagerRoots::new(data.path(), None));
    let known = store.known_buckets().expect("loader");
    assert_eq!(known.len(), 2);
    assert_eq!(known.entries()[0].name, "alpha");
}

#[test]
fn known_buckets_loader_reads_the_state_dir_copy() {
    let data = tempfile::tempdir().expect("fixture");
    let profile = tempfile::tempdir().expect("fixture profile");
    seed_main(profile.path(), Some(VALID));
    let store = BucketStore::new(&ManagerRoots::new(data.path(), Some(profile.path())));
    let known = store.known_buckets().expect("loader");
    assert_eq!(known.len(), 2);
    assert!(
        !data.path().join("buckets").exists(),
        "shared data directory stays untouched"
    );
}

#[test]
fn known_buckets_loader_reports_a_malformed_file() {
    let data = tempfile::tempdir().expect("fixture");
    let malformed = file_with(
        "foo",
        entry(
            "git",
            "https://example.com/bucket",
            Some("main"),
            "not-a-key",
            "Bucket.",
        ),
    );
    seed_main(data.path(), Some(&malformed));
    let store = BucketStore::new(&ManagerRoots::new(data.path(), None));
    let error = store
        .known_buckets()
        .expect_err("malformed file")
        .to_string();
    assert!(error.contains("known bucket \"foo\""), "{error}");
    assert!(error.contains("fingerprint"), "{error}");
}

#[test]
fn known_buckets_main_update_accepts_a_root_file() {
    let data = tempfile::tempdir().expect("fixture");
    let tree = tree_with(data.path(), Some(VALID));
    let store = BucketStore::new(&ManagerRoots::new(data.path(), None));
    install(&store, &tree, data.path()).expect("main update");
    let installed = data.path().join("buckets/main").join(KNOWN_BUCKETS_FILE);
    assert_eq!(
        fs::read_to_string(installed).expect("installed file"),
        VALID
    );
    assert_eq!(store.known_buckets().expect("loader").len(), 2);
}

#[test]
fn known_buckets_main_update_refuses_a_malformed_file_and_keeps_the_installed_copy() {
    let data = tempfile::tempdir().expect("fixture");
    let store = BucketStore::new(&ManagerRoots::new(data.path(), None));
    let first = data.path().join("first");
    let valid = tree_with(&first, Some(VALID));
    install(&store, &valid, &first).expect("first main update");
    let installed = data.path().join("buckets/main");
    let before = crate::bucket_digest::directory_digest(&installed).expect("installed digest");

    let malformed = file_with(
        "foo",
        entry(
            "git",
            "https://example.com/bucket",
            Some("main"),
            "too-short",
            "Bucket.",
        ),
    );
    let second = data.path().join("second");
    let tree = tree_with(&second, Some(&malformed));
    let error = install(&store, &tree, &second)
        .expect_err("malformed known buckets")
        .to_string();
    assert!(error.contains("known bucket \"foo\""), "{error}");
    assert!(error.contains("fingerprint"), "{error}");
    assert_eq!(
        crate::bucket_digest::directory_digest(&installed).expect("installed digest"),
        before,
        "installed copy is unchanged"
    );
}

#[test]
fn known_buckets_other_bucket_file_is_never_parsed() {
    let data = tempfile::tempdir().expect("fixture");
    seed_main(data.path(), Some(VALID));
    let store = BucketStore::new(&ManagerRoots::new(data.path(), None));
    let bucket = tempfile::tempdir().expect("fixture bucket");
    fs::write(
        bucket.path().join("bucket.json"),
        r#"{"schema_version":2,"name":"community","title":"Community"}"#,
    )
    .expect("fixture metadata");
    fs::write(bucket.path().join(KNOWN_BUCKETS_FILE), "{not json").expect("fixture file");
    store
        .add("community", bucket.path())
        .expect("registration ignores the root file");
    let known = store.known_buckets().expect("loader");
    assert_eq!(known.len(), 2, "only the main copy is read");
    assert_eq!(known.entries()[0].name, "alpha");
}
