use super::{
    apply_key_statements, check_revocation_key, key_statement_bytes, revocation_statement,
    rotation_statement, PinnedKeys, REVOCATION_FILE, REVOCATION_SIGNATURE_FILE, ROTATION_FILE,
    ROTATION_SIGNATURE_FILE,
};
use crate::buckets::signature::CatalogPublicKey;
use minisign::KeyPair;
use std::fs;
use std::io::Cursor;
use std::path::Path;

const CATALOG_NAME: &str = "community";

struct TestKey {
    pair: KeyPair,
    public: CatalogPublicKey,
    text: String,
}

fn test_key() -> TestKey {
    let pair = KeyPair::generate_unencrypted_keypair().expect("test key");
    let text = pair.pk.to_box().expect("public key box").into_string();
    let public = CatalogPublicKey::parse(&text).expect("public key");
    TestKey { pair, public, text }
}

fn sign(key: &TestKey, bytes: &[u8]) -> String {
    minisign::sign(
        Some(&key.pair.pk),
        &key.pair.sk,
        Cursor::new(bytes),
        Some("rotation test"),
        Some("rotation test signature"),
    )
    .expect("test signature")
    .into_string()
}

fn catalog() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("catalog");
    fs::write(
        directory.path().join("bucket.json"),
        r#"{"schema_version":2,"name":"community","title":"Test"}"#,
    )
    .expect("bucket metadata");
    directory
}

fn write(root: &Path, file: &str, bytes: &[u8]) {
    fs::write(root.join(file), bytes).expect("statement file");
}

fn rotation_body(from: &str, to: &str) -> Vec<u8> {
    rotation_statement(from, to).expect("rotation statement")
}

fn pinned<'a>(signer: &'a TestKey, revocation: Option<&'a TestKey>) -> PinnedKeys<'a> {
    PinnedKeys {
        signer: &signer.public,
        revocation: revocation.map(|key| &key.public),
    }
}

fn accepted(
    directory: &Path,
    signer: &TestKey,
    revocation: Option<&TestKey>,
    revision: &TestKey,
) -> super::KeyStatementOutcome {
    apply_key_statements(
        directory,
        CATALOG_NAME,
        &pinned(signer, revocation),
        Some(&revision.public),
    )
    .expect("statement evaluation")
}

#[test]
fn statements_are_domain_separated_and_name_the_catalog() {
    let body = rotation_body("OLD", "NEW");
    let rotation = key_statement_bytes(super::KeyChangeKind::Rotated, "alpha", &body);
    let revocation = key_statement_bytes(super::KeyChangeKind::Revoked, "alpha", &body);
    assert!(rotation.starts_with(b"modn-rotation-v1\0catalog: alpha\n"));
    assert!(revocation.starts_with(b"modn-revocation-v1\0catalog: alpha\n"));
    assert!(rotation.ends_with(&body));
    assert_ne!(rotation, revocation);
    let other = key_statement_bytes(super::KeyChangeKind::Rotated, "beta", &body);
    assert_ne!(rotation, other);
}

#[test]
fn rotation_signed_by_the_pinned_key_is_accepted() {
    let directory = catalog();
    let signer = test_key();
    let successor = test_key();
    let body = rotation_body(&signer.public.key_id().to_string(), &successor.text);
    write(directory.path(), ROTATION_FILE, &body);
    write(
        directory.path(),
        ROTATION_SIGNATURE_FILE,
        sign(
            &signer,
            &key_statement_bytes(super::KeyChangeKind::Rotated, CATALOG_NAME, &body),
        )
        .as_bytes(),
    );
    let change = accepted(directory.path(), &signer, None, &successor)
        .change
        .expect("change");
    assert_eq!(change.kind, super::KeyChangeKind::Rotated);
    assert_eq!(change.from, signer.public.key_id().to_string());
    assert_eq!(change.to, successor.public.key_id().to_string());
    assert_eq!(change.new_key_text, successor.text);
}

#[test]
fn rotation_signed_by_an_unrelated_key_is_refused() {
    let directory = catalog();
    let signer = test_key();
    let successor = test_key();
    let unrelated = test_key();
    let body = rotation_body(&signer.public.key_id().to_string(), &successor.text);
    write(directory.path(), ROTATION_FILE, &body);
    write(
        directory.path(),
        ROTATION_SIGNATURE_FILE,
        sign(
            &unrelated,
            &key_statement_bytes(super::KeyChangeKind::Rotated, CATALOG_NAME, &body),
        )
        .as_bytes(),
    );
    let error = apply_key_statements(
        directory.path(),
        CATALOG_NAME,
        &pinned(&signer, None),
        Some(&successor.public),
    )
    .expect_err("forged rotation")
    .to_string();
    assert!(
        error.contains("is not signed by the pinned signing key"),
        "{error}"
    );
    assert!(
        error.contains(&signer.public.key_id().to_string()),
        "{error}"
    );
}

#[test]
fn rotation_signed_by_the_revocation_key_is_refused() {
    let directory = catalog();
    let signer = test_key();
    let revocation = test_key();
    let successor = test_key();
    let body = rotation_body(&signer.public.key_id().to_string(), &successor.text);
    write(directory.path(), ROTATION_FILE, &body);
    write(
        directory.path(),
        ROTATION_SIGNATURE_FILE,
        sign(
            &revocation,
            &key_statement_bytes(super::KeyChangeKind::Rotated, CATALOG_NAME, &body),
        )
        .as_bytes(),
    );
    let error = apply_key_statements(
        directory.path(),
        CATALOG_NAME,
        &pinned(&signer, Some(&revocation)),
        Some(&successor.public),
    )
    .expect_err("revocation key cannot rotate")
    .to_string();
    assert!(
        error.contains("is not signed by the pinned signing key"),
        "{error}"
    );
}

#[test]
fn revocation_signed_by_the_signer_key_is_refused() {
    let directory = catalog();
    let signer = test_key();
    let revocation = test_key();
    let replacement = test_key();
    let body = revocation_statement(&signer.public.key_id().to_string(), &replacement.text)
        .expect("revocation statement");
    write(directory.path(), REVOCATION_FILE, &body);
    write(
        directory.path(),
        REVOCATION_SIGNATURE_FILE,
        sign(
            &signer,
            &key_statement_bytes(super::KeyChangeKind::Revoked, CATALOG_NAME, &body),
        )
        .as_bytes(),
    );
    let error = apply_key_statements(
        directory.path(),
        CATALOG_NAME,
        &pinned(&signer, Some(&revocation)),
        Some(&replacement.public),
    )
    .expect_err("signer cannot revoke")
    .to_string();
    assert!(
        error.contains("is not signed by the pinned revocation key"),
        "{error}"
    );
}

#[test]
fn a_statement_cannot_be_replayed_as_another_kind_or_catalog() {
    let directory = catalog();
    let signer = test_key();
    let revocation = test_key();
    let replacement = test_key();
    let body = revocation_statement(&signer.public.key_id().to_string(), &replacement.text)
        .expect("revocation statement");
    write(directory.path(), REVOCATION_FILE, &body);
    write(
        directory.path(),
        REVOCATION_SIGNATURE_FILE,
        sign(
            &revocation,
            &key_statement_bytes(super::KeyChangeKind::Rotated, CATALOG_NAME, &body),
        )
        .as_bytes(),
    );
    let error = apply_key_statements(
        directory.path(),
        CATALOG_NAME,
        &pinned(&signer, Some(&revocation)),
        Some(&replacement.public),
    )
    .expect_err("cross-kind replay")
    .to_string();
    assert!(error.contains("revocation"), "{error}");

    write(directory.path(), REVOCATION_FILE, &body);
    write(
        directory.path(),
        REVOCATION_SIGNATURE_FILE,
        sign(
            &revocation,
            &key_statement_bytes(super::KeyChangeKind::Revoked, "other", &body),
        )
        .as_bytes(),
    );
    let error = apply_key_statements(
        directory.path(),
        CATALOG_NAME,
        &pinned(&signer, Some(&revocation)),
        Some(&replacement.public),
    )
    .expect_err("cross-catalog replay")
    .to_string();
    assert!(
        error.contains("is not signed by the pinned revocation key"),
        "{error}"
    );
}

#[test]
fn both_statements_wrong_from_and_wrong_schema_are_refused() {
    let directory = catalog();
    let signer = test_key();
    let successor = test_key();
    let rotation = rotation_body(&signer.public.key_id().to_string(), &successor.text);
    let revocation = revocation_statement(&signer.public.key_id().to_string(), &successor.text)
        .expect("revocation statement");
    write(directory.path(), ROTATION_FILE, &rotation);
    write(
        directory.path(),
        ROTATION_SIGNATURE_FILE,
        sign(
            &signer,
            &key_statement_bytes(super::KeyChangeKind::Rotated, CATALOG_NAME, &rotation),
        )
        .as_bytes(),
    );
    write(directory.path(), REVOCATION_FILE, &revocation);
    write(
        directory.path(),
        REVOCATION_SIGNATURE_FILE,
        sign(
            &signer,
            &key_statement_bytes(super::KeyChangeKind::Revoked, CATALOG_NAME, &revocation),
        )
        .as_bytes(),
    );
    let error = apply_key_statements(
        directory.path(),
        CATALOG_NAME,
        &pinned(&signer, None),
        Some(&successor.public),
    )
    .expect_err("both statements")
    .to_string();
    assert!(
        error.contains("both rotation.json and revocation.json"),
        "{error}"
    );

    fs::remove_file(directory.path().join(REVOCATION_FILE)).expect("remove revocation");
    fs::remove_file(directory.path().join(REVOCATION_SIGNATURE_FILE)).expect("remove signature");
    let wrong_from = rotation_body("0000000000000000", &successor.text);
    write(directory.path(), ROTATION_FILE, &wrong_from);
    write(
        directory.path(),
        ROTATION_SIGNATURE_FILE,
        sign(
            &signer,
            &key_statement_bytes(super::KeyChangeKind::Rotated, CATALOG_NAME, &wrong_from),
        )
        .as_bytes(),
    );
    let historical = apply_key_statements(
        directory.path(),
        CATALOG_NAME,
        &pinned(&signer, None),
        Some(&successor.public),
    )
    .expect("a statement that names another key is history");
    assert!(historical.change.is_none());
    assert_eq!(historical.ignored, vec!["rotation".to_owned()]);

    let unknown = br#"{"schema_version":2,"kind":"rotation","from":"x","to":"y"}"#;
    write(directory.path(), ROTATION_FILE, unknown);
    let error = apply_key_statements(
        directory.path(),
        CATALOG_NAME,
        &pinned(&signer, None),
        Some(&successor.public),
    )
    .expect_err("unknown version")
    .to_string();
    assert!(error.contains("schema_version 2"), "{error}");
}

#[test]
fn a_to_key_that_does_not_match_catalog_pub_is_refused() {
    let directory = catalog();
    let signer = test_key();
    let successor = test_key();
    let other = test_key();
    let body = rotation_body(&signer.public.key_id().to_string(), &successor.text);
    write(directory.path(), ROTATION_FILE, &body);
    write(
        directory.path(),
        ROTATION_SIGNATURE_FILE,
        sign(
            &signer,
            &key_statement_bytes(super::KeyChangeKind::Rotated, CATALOG_NAME, &body),
        )
        .as_bytes(),
    );
    let error = apply_key_statements(
        directory.path(),
        CATALOG_NAME,
        &pinned(&signer, None),
        Some(&other.public),
    )
    .expect_err("revision key mismatch")
    .to_string();
    assert!(
        error.contains("publishes a different public key"),
        "{error}"
    );
}

#[test]
fn revocation_key_must_be_pinned_and_cannot_change() {
    let pinned_key = test_key();
    let other = test_key();
    let error = check_revocation_key("community", None, Some(&other.public))
        .expect_err("added later")
        .to_string();
    assert!(
        error.contains("must be present at the first sync"),
        "{error}"
    );
    let error = check_revocation_key("community", Some(&pinned_key.public), None)
        .expect_err("removed")
        .to_string();
    assert!(error.contains("removes revocation.pub"), "{error}");
    let error = check_revocation_key("community", Some(&pinned_key.public), Some(&other.public))
        .expect_err("changed")
        .to_string();
    assert!(error.contains("Revocation key changed"), "{error}");
}
