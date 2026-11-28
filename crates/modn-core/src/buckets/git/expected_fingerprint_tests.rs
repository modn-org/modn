//! Tests for the expected catalog fingerprint of a known-bucket sync.

use super::tests::{remote, source, test_key};
use super::BucketStore;
use crate::buckets::CatalogKeyId;
use crate::error::{code_of, ErrorCode};
use modn_core::manager_roots::ManagerRoots;
use std::fs;

#[test]
fn matching_expected_fingerprint_pins_the_signer() {
    let remote = remote("1", test_key());
    let expected = remote.key.public.key_id();
    let state = tempfile::tempdir().unwrap();
    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));

    let synced = store
        .sync_git_with_expected_fingerprint("community", Some(source(&remote)), Some(&expected))
        .unwrap();

    assert!(synced.pinned);
    assert_eq!(synced.revision.signer.key_id, expected.to_string());
}

#[test]
fn mismatched_expected_fingerprint_is_coded_and_writes_nothing() {
    let remote = remote("1", test_key());
    let actual = remote.key.public.key_id().to_string();
    let expected = CatalogKeyId::parse_hex("0000000000000000").unwrap();
    let state = tempfile::tempdir().unwrap();
    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));

    let error = store
        .sync_git_with_expected_fingerprint("community", Some(source(&remote)), Some(&expected))
        .unwrap_err();

    assert_eq!(code_of(&error), Some(ErrorCode::BucketFingerprintMismatch));
    let message = format!("{error:#}");
    assert!(message.contains("0000000000000000"), "{message}");
    assert!(message.contains(&actual), "{message}");
    assert!(message.contains("nothing was written"), "{message}");
    assert!(!state
        .path()
        .join("buckets/registrations/community.json")
        .exists());
    let entries: Vec<String> = fs::read_dir(state.path().join("buckets"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, [".git-sync.lock"]);
}
