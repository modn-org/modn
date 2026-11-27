//! Signed sequence rollback and conflict cases for ZIP catalogs.

use super::*;

#[test]
fn sequence_mirror_serving_an_older_signed_zip_is_a_rollback() {
    let key = test_key();
    let temp = tempfile::tempdir().expect("temp");
    let two = sequence_json(2);
    let first = signed_zip(
        temp.path(),
        "v2",
        "2.0",
        &key,
        &[
            ("mirrors.json", &mirrors_json(&[MIRROR])),
            ("sequence.json", &two),
        ],
    );
    let state = temp.path().join("state");
    let store = BucketStore::new(&ManagerRoots::new(&state, None));
    let opener = Opener::new(vec![(PRIMARY, Some(first.clone()))]);
    store
        .sync_zip_with_signer("community", Some(zip_source(PRIMARY)), |source| {
            opener.open(source)
        })
        .expect("first numbered sync");
    assert_eq!(stored_accepted(&state)["sequence"], 2);
    let before = state_snapshot(&state);

    let one = sequence_json(1);
    let older = signed_zip(
        temp.path(),
        "older",
        "1.0",
        &key,
        &[
            ("mirrors.json", &mirrors_json(&[MIRROR])),
            ("sequence.json", &one),
        ],
    );
    let opener = Opener::new(vec![(PRIMARY, None), (MIRROR, Some(older))]);
    let error = store
        .sync_zip_with_signer("community", None, |source| opener.open(source))
        .expect_err("an older signed ZIP is refused");
    assert_eq!(
        crate::error::code_of(&error),
        Some(crate::error::ErrorCode::CatalogRollback)
    );
    let message = format!("{error:#}");
    assert!(message.contains("sequence 1"), "{message}");
    assert!(message.contains("accepted sequence 2"), "{message}");
    assert_eq!(before, state_snapshot(&state));

    // The same number with the same bytes is an idempotent re-sync.
    let opener = Opener::new(vec![(PRIMARY, Some(first))]);
    store
        .sync_zip_with_signer("community", None, |source| opener.open(source))
        .expect("equal number and digest");
    assert_eq!(stored_accepted(&state)["sequence"], 2);
}

#[test]
fn sequence_same_number_with_another_digest_is_a_conflict() {
    let key = test_key();
    let temp = tempfile::tempdir().expect("temp");
    let two = sequence_json(2);
    let first = signed_zip(
        temp.path(),
        "v2",
        "2.0",
        &key,
        &[
            ("mirrors.json", &mirrors_json(&[MIRROR])),
            ("sequence.json", &two),
        ],
    );
    let state = temp.path().join("state");
    let store = BucketStore::new(&ManagerRoots::new(&state, None));
    let opener = Opener::new(vec![(PRIMARY, Some(first))]);
    store
        .sync_zip_with_signer("community", Some(zip_source(PRIMARY)), |source| {
            opener.open(source)
        })
        .expect("first numbered sync");
    let before = state_snapshot(&state);

    let republished = signed_zip(
        temp.path(),
        "republished",
        "3.0",
        &key,
        &[
            ("mirrors.json", &mirrors_json(&[MIRROR])),
            ("sequence.json", &two),
        ],
    );
    let opener = Opener::new(vec![(PRIMARY, Some(republished))]);
    let error = store
        .sync_zip_with_signer("community", None, |source| opener.open(source))
        .expect_err("one number cannot name two revisions");
    assert_eq!(
        crate::error::code_of(&error),
        Some(crate::error::ErrorCode::CatalogSequenceConflict)
    );
    assert_eq!(before, state_snapshot(&state));
}

#[test]
fn sequence_numbered_catalog_cannot_become_unnumbered() {
    let key = test_key();
    let temp = tempfile::tempdir().expect("temp");
    let one = sequence_json(1);
    let first = signed_zip(
        temp.path(),
        "v1",
        "1.0",
        &key,
        &[
            ("mirrors.json", &mirrors_json(&[MIRROR])),
            ("sequence.json", &one),
        ],
    );
    let state = temp.path().join("state");
    let store = BucketStore::new(&ManagerRoots::new(&state, None));
    let opener = Opener::new(vec![(PRIMARY, Some(first))]);
    store
        .sync_zip_with_signer("community", Some(zip_source(PRIMARY)), |source| {
            opener.open(source)
        })
        .expect("first numbered sync");
    let before = state_snapshot(&state);

    let unnumbered = signed_zip(
        temp.path(),
        "unnumbered",
        "2.0",
        &key,
        &[("mirrors.json", &mirrors_json(&[MIRROR]))],
    );
    let opener = Opener::new(vec![(PRIMARY, Some(unnumbered))]);
    let error = store
        .sync_zip_with_signer("community", None, |source| opener.open(source))
        .expect_err("a numbered catalog cannot drop its number");
    assert_eq!(
        crate::error::code_of(&error),
        Some(crate::error::ErrorCode::CatalogRollback)
    );
    assert_eq!(before, state_snapshot(&state));
}
