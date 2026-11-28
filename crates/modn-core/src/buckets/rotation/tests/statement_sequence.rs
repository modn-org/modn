//! Tests for the sequence number that a key statement names.
//!
//! A statement that moves the pin must be published in the revision it names,
//! so an older number there is a replay. A statement that stays history keeps
//! the number it was signed with, and a number from the future never appears.

use super::super::{
    apply_key_statements, key_statement_bytes, rotation_statement_with_sequence,
    validate_historical_statements, KeyChangeKind, PinnedKeys, ROTATION_FILE,
    ROTATION_SIGNATURE_FILE,
};
use super::{catalog, pinned, sign, test_key, write};
use std::path::Path;

const CATALOG_NAME: &str = "community";

fn rotation(root: &Path, from: &str, to: &str, sequence: u64, signer: &super::TestKey) -> Vec<u8> {
    let body = rotation_statement_with_sequence(from, to, sequence).expect("rotation body");
    write(root, ROTATION_FILE, &body);
    write(
        root,
        ROTATION_SIGNATURE_FILE,
        sign(
            signer,
            &key_statement_bytes(KeyChangeKind::Rotated, CATALOG_NAME, &body),
        )
        .as_bytes(),
    );
    body
}

#[test]
fn a_statement_that_applies_from_an_older_revision_is_a_rollback() {
    let directory = catalog();
    let signer = test_key();
    let successor = test_key();
    rotation(
        directory.path(),
        &signer.public.key_id().to_string(),
        &successor.text,
        2,
        &signer,
    );
    let error = apply_key_statements(
        directory.path(),
        CATALOG_NAME,
        &pinned(&signer, None),
        Some(&successor.public),
        Some(3),
    )
    .expect_err("a statement from an older revision is a replay");
    assert_eq!(
        crate::error::code_of(&error),
        Some(crate::error::ErrorCode::CatalogRollback)
    );
    assert!(
        error.to_string().contains("older revision is a replay"),
        "{error}"
    );
}

#[test]
fn a_statement_that_applies_at_the_revision_sequence_is_accepted() {
    let directory = catalog();
    let signer = test_key();
    let successor = test_key();
    rotation(
        directory.path(),
        &signer.public.key_id().to_string(),
        &successor.text,
        2,
        &signer,
    );
    let outcome = apply_key_statements(
        directory.path(),
        CATALOG_NAME,
        &pinned(&signer, None),
        Some(&successor.public),
        Some(2),
    )
    .expect("a statement at the revision sequence applies");
    assert!(outcome.change.is_some());
}

#[test]
fn a_statement_newer_than_the_revision_is_refused() {
    let directory = catalog();
    let signer = test_key();
    let successor = test_key();
    rotation(
        directory.path(),
        &signer.public.key_id().to_string(),
        &successor.text,
        3,
        &signer,
    );
    let error = apply_key_statements(
        directory.path(),
        CATALOG_NAME,
        &pinned(&signer, None),
        Some(&successor.public),
        Some(2),
    )
    .expect_err("a statement cannot come from a later revision");
    assert!(
        error
            .to_string()
            .contains("newer than the catalog revision's sequence 2"),
        "{error}"
    );
}

#[test]
fn a_published_statement_from_an_older_revision_stays_history() {
    let directory = catalog();
    let statement_key = test_key();
    let pinned_key = test_key();
    let successor = test_key();
    rotation(
        directory.path(),
        &statement_key.public.key_id().to_string(),
        &successor.text,
        2,
        &statement_key,
    );
    let outcome = apply_key_statements(
        directory.path(),
        CATALOG_NAME,
        &pinned(&pinned_key, None),
        Some(&pinned_key.public),
        Some(5),
    )
    .expect("a statement from an earlier revision adds no trust");
    assert!(outcome.change.is_none());
    assert_eq!(outcome.ignored, vec!["rotation".to_owned()]);
}

#[test]
fn an_applied_statement_republished_from_an_older_revision_stays_history() {
    let directory = catalog();
    let statement_key = test_key();
    let pinned_key = test_key();
    let body = rotation(
        directory.path(),
        &statement_key.public.key_id().to_string(),
        &pinned_key.text,
        1,
        &statement_key,
    );
    let digest = blake3::hash(&body).to_hex().to_string();
    let keys = PinnedKeys {
        signer: &pinned_key.public,
        revocation: None,
        applied: vec![digest.as_str()],
    };
    let outcome = apply_key_statements(
        directory.path(),
        CATALOG_NAME,
        &keys,
        Some(&pinned_key.public),
        Some(4),
    )
    .expect("an applied statement is history");
    assert!(outcome.change.is_none());
    assert_eq!(outcome.ignored, vec!["rotation".to_owned()]);
}

#[test]
fn the_first_sync_accepts_history_and_refuses_a_statement_from_the_future() {
    let directory = catalog();
    let signer = test_key();
    let successor = test_key();
    rotation(
        directory.path(),
        &signer.public.key_id().to_string(),
        &successor.text,
        2,
        &signer,
    );
    let ignored = validate_historical_statements(directory.path(), CATALOG_NAME, Some(4))
        .expect("a statement from an earlier revision is history");
    assert_eq!(ignored, vec!["rotation".to_owned()]);
    let error = validate_historical_statements(directory.path(), CATALOG_NAME, Some(1))
        .expect_err("a statement cannot come from a later revision");
    assert!(
        error
            .to_string()
            .contains("newer than the catalog revision's sequence 1"),
        "{error}"
    );
}
