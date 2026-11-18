//! Single-use key statements: a replay cannot move the pin back.

use super::statement_tests::write_rotation_files;
use super::tests::{commit, registration, remote, source, test_key, write_signature};
use super::BucketStore;
use std::fs;

#[test]
fn a_replayed_rotation_cannot_roll_the_pin_back_to_a_retired_key() {
    let original = test_key();
    let middle = test_key();
    let remote = remote("1", original.clone());
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(state.path());
    store
        .sync_git_with_signer("community", Some(source(&remote)))
        .expect("first sync");

    // The author rotates A -> B, statement R1, and then B -> A, statement R2.
    write_rotation_files(remote.directory.path(), &original, &middle);
    write_signature(remote.directory.path(), &middle);
    commit(remote.directory.path(), "rotate to the middle key");
    store
        .sync_git_with_signer("community", Some(source(&remote)))
        .expect("R1 applies");
    write_rotation_files(remote.directory.path(), &middle, &original);
    write_signature(remote.directory.path(), &original);
    commit(remote.directory.path(), "rotate back");
    store
        .sync_git_with_signer("community", Some(source(&remote)))
        .expect("R2 applies");
    let pinned = fs::read(registration(state.path())).expect("registration");

    // A holder of the retired key B republishes R1, which A signed before it
    // retired B, and signs the catalog with B.
    write_rotation_files(remote.directory.path(), &original, &middle);
    write_signature(remote.directory.path(), &middle);
    commit(remote.directory.path(), "replay the first rotation");
    let error = store
        .sync_git_with_signer("community", Some(source(&remote)))
        .expect_err("a replayed rotation must not move the pin")
        .to_string();
    assert!(error.contains("already applied"), "{error}");
    assert_eq!(
        fs::read(registration(state.path())).expect("registration"),
        pinned,
        "the pin stays on the current key"
    );
    let again = store
        .sync_git_with_signer("community", Some(source(&remote)))
        .expect_err("the refusal is stable")
        .to_string();
    assert!(again.contains("already applied"), "{again}");
}
