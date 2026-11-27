//! Signed sequence cases for Git catalogs.

use super::*;

fn sequence_file(sequence: u64) -> String {
    serde_json::json!({"schema_version":1,"sequence":sequence}).to_string()
}

#[test]
fn sequence_git_first_sync_numbered_is_stored_and_a_lower_number_rolls_back() {
    let remote = remote("1", test_key());
    fs::write(
        remote.directory.path().join("sequence.json"),
        sequence_file(2),
    )
    .unwrap();
    write_signature(remote.directory.path(), &remote.key);
    commit(remote.directory.path(), "sequence 2");
    let state = tempfile::tempdir().unwrap();
    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));
    store
        .sync_git_with_signer("community", Some(source(&remote)))
        .expect("a first numbered sync works");
    let pointer: serde_json::Value =
        serde_json::from_slice(&fs::read(registration(state.path())).unwrap()).unwrap();
    assert_eq!(pointer["accepted"]["sequence"], 2);

    fs::write(
        remote.directory.path().join("sequence.json"),
        sequence_file(3),
    )
    .unwrap();
    seed_catalog(remote.directory.path(), "3");
    write_signature(remote.directory.path(), &remote.key);
    commit(remote.directory.path(), "sequence 3");
    store
        .sync_git_with_signer("community", None)
        .expect("a higher number is accepted");
    let pointer: serde_json::Value =
        serde_json::from_slice(&fs::read(registration(state.path())).unwrap()).unwrap();
    assert_eq!(pointer["accepted"]["sequence"], 3);
    let before = fs::read(registration(state.path())).unwrap();
    let published_before = published(state.path());

    fs::write(
        remote.directory.path().join("sequence.json"),
        sequence_file(2),
    )
    .unwrap();
    seed_catalog(remote.directory.path(), "4");
    write_signature(remote.directory.path(), &remote.key);
    commit(remote.directory.path(), "rolled back");
    let error = store
        .sync_git_with_signer("community", None)
        .expect_err("a lower number is refused");
    assert_eq!(
        crate::error::code_of(&error),
        Some(crate::error::ErrorCode::CatalogRollback)
    );
    assert_eq!(fs::read(registration(state.path())).unwrap(), before);
    assert_eq!(published(state.path()), published_before);
}
