use super::*;
use crate::buckets::store::BucketStore;
use crate::error::code_of;
use crate::manager_roots::ManagerRoots;

fn signed(root: &Path, body: &str) -> Option<SignedSequence> {
    fs::write(root.join(SEQUENCE_FILE), body).expect("sequence file");
    read_sequence(root).expect("valid sequence")
}

fn temp() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp")
}

fn record(sequence: u64, digest: &str) -> StoredSequence {
    StoredSequence::new(sequence, digest)
}

#[test]
fn sequence_first_sync_unnumbered_stores_nothing() {
    let directory = temp();
    assert_eq!(read_sequence(directory.path()).expect("absent"), None);
    let (accepted, warnings) =
        accept_sequence("community", None, None, "blake3:aa", 0).expect("first sync");
    assert_eq!(accepted, None);
    assert!(warnings.is_empty());
}

#[test]
fn sequence_first_sync_numbered_is_stored() {
    let directory = temp();
    let signed = signed(directory.path(), r#"{"schema_version":1,"sequence":3}"#);
    let (accepted, warnings) =
        accept_sequence("community", None, signed.as_ref(), "blake3:aa", 0).expect("first sync");
    assert_eq!(accepted, Some(record(3, "blake3:aa")));
    assert!(warnings.is_empty());
}

#[test]
fn sequence_higher_number_is_accepted_and_stored() {
    let directory = temp();
    let signed = signed(directory.path(), r#"{"schema_version":1,"sequence":4}"#);
    let stored = record(3, "blake3:aa");
    let (accepted, _) =
        accept_sequence("community", Some(&stored), signed.as_ref(), "blake3:bb", 0)
            .expect("higher number");
    assert_eq!(accepted, Some(record(4, "blake3:bb")));
}

#[test]
fn sequence_lower_number_is_a_rollback() {
    let directory = temp();
    let signed = signed(directory.path(), r#"{"schema_version":1,"sequence":2}"#);
    let stored = record(3, "blake3:aa");
    let error = accept_sequence("community", Some(&stored), signed.as_ref(), "blake3:bb", 0)
        .expect_err("lower number");
    assert_eq!(code_of(&error), Some(ErrorCode::CatalogRollback));
    let message = format!("{error:#}");
    assert!(message.contains("community"), "{message}");
    assert!(message.contains("sequence 2"), "{message}");
    assert!(message.contains("accepted sequence 3"), "{message}");
    assert!(message.contains("nothing was written"), "{message}");
}

#[test]
fn sequence_numbered_to_unnumbered_is_a_rollback() {
    let directory = temp();
    assert_eq!(read_sequence(directory.path()).expect("absent"), None);
    let stored = record(3, "blake3:aa");
    let error = accept_sequence("community", Some(&stored), None, "blake3:aa", 0)
        .expect_err("dropped number");
    assert_eq!(code_of(&error), Some(ErrorCode::CatalogRollback));
    assert!(format!("{error:#}").contains("unnumbered"));
}

#[test]
fn sequence_unnumbered_to_numbered_is_accepted() {
    let directory = temp();
    let signed = signed(directory.path(), r#"{"schema_version":1,"sequence":1}"#);
    let (accepted, _) =
        accept_sequence("community", None, signed.as_ref(), "blake3:aa", 0).expect("upgrade");
    assert_eq!(accepted, Some(record(1, "blake3:aa")));
}

#[test]
fn sequence_equal_number_and_digest_is_idempotent() {
    let directory = temp();
    let signed = signed(directory.path(), r#"{"schema_version":1,"sequence":3}"#);
    let stored = record(3, "blake3:aa");
    let (accepted, _) =
        accept_sequence("community", Some(&stored), signed.as_ref(), "blake3:aa", 0)
            .expect("re-sync");
    assert_eq!(accepted, Some(record(3, "blake3:aa")));
}

#[test]
fn sequence_equal_number_with_another_digest_is_a_conflict() {
    let directory = temp();
    let signed = signed(directory.path(), r#"{"schema_version":1,"sequence":3}"#);
    let stored = record(3, "blake3:aa");
    let error = accept_sequence("community", Some(&stored), signed.as_ref(), "blake3:bb", 0)
        .expect_err("republished number");
    assert_eq!(code_of(&error), Some(ErrorCode::CatalogSequenceConflict));
    let message = format!("{error:#}");
    assert!(message.contains("blake3:aa"), "{message}");
    assert!(message.contains("blake3:bb"), "{message}");
    assert!(message.contains("nothing was written"), "{message}");
}

#[test]
fn sequence_expiry_is_a_warning_not_a_refusal() {
    let directory = temp();
    let signed = signed(
        directory.path(),
        r#"{"schema_version":1,"sequence":2,"expires":"2020-01-01T00:00:00Z"}"#,
    );
    // 2020-01-01T00:00:00Z is 1577836800; a later clock sees the expiry.
    let expired = signed.as_ref().expect("signed");
    let (accepted, warnings) =
        accept_sequence("community", None, Some(expired), "blake3:aa", 1_600_000_000)
            .expect("expired catalog still syncs");
    assert_eq!(accepted, Some(record(2, "blake3:aa")));
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].kind, CatalogWarningKind::Expired);
    assert!(warnings[0].message.contains("2020-01-01T00:00:00Z"));
}

#[test]
fn sequence_parser_refuses_unknown_fields_and_bad_values() {
    for body in [
        r#"{"schema_version":1}"#,
        r#"{"schema_version":2,"sequence":1}"#,
        r#"{"schema_version":1,"sequence":0}"#,
        r#"{"schema_version":1,"sequence":1,"extra":true}"#,
        r#"{"schema_version":1,"sequence":1,"expires":"2026-12-31"}"#,
        r#"{"schema_version":1,"sequence":1,"expires":"2026-02-30T00:00:00Z"}"#,
        r#"{"schema_version":1,"sequence":1,"expires":"2026-12-31T24:00:00Z"}"#,
        r#"{"schema_version":1,"sequence":1,"expires":"2026-12-31T00:00:00+00:00"}"#,
    ] {
        let directory = temp();
        fs::write(directory.path().join(SEQUENCE_FILE), body).expect("sequence file");
        assert!(read_sequence(directory.path()).is_err(), "accepted {body}");
    }
}

#[test]
fn sequence_expiry_parser_accepts_the_utc_form_and_leap_days() {
    assert_eq!(
        parse_expiry("2020-01-01T00:00:00Z").expect("epoch case"),
        1_577_836_800
    );
    assert_eq!(
        parse_expiry("2024-02-29T12:34:56Z").expect("leap day"),
        1_709_210_096
    );
    assert!(parse_expiry("2023-02-29T00:00:00Z").is_err());
}

#[test]
fn registration_without_accepted_still_parses_and_keeps_its_bytes() {
    let directory = temp();
    let store = BucketStore::new(&ManagerRoots::new(directory.path(), None));
    let path = directory
        .path()
        .join("buckets/registrations/community.json");
    fs::create_dir_all(path.parent().expect("parent")).expect("registry");
    let legacy = br#"{"path":"/catalogs/community"}"#;
    fs::write(&path, legacy).expect("legacy registration");
    assert_eq!(store.stored_sequence("community").expect("parse"), None);
    let registration = store.registration("community").expect("registration");
    assert_eq!(registration.accepted, None);
    assert_eq!(store.stored_sequence("missing").expect("absent"), None);
}

#[test]
fn main_sequence_record_is_stored_beside_the_official_tree() {
    let directory = temp();
    let store = BucketStore::new(&ManagerRoots::new(directory.path(), None));
    assert_eq!(store.main_sequence().expect("absent"), None);
    store
        .write_main_sequence(&record(7, "blake3:main"))
        .expect("record");
    assert_eq!(
        store.main_sequence().expect("stored"),
        Some(record(7, "blake3:main"))
    );
    assert!(directory.path().join("buckets/main.json").is_file());
}
