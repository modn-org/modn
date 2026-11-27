#![allow(
    dead_code,
    reason = "shared fixture exposes helpers for different test crates"
)]
use crate::catalog_authoring::{path_text, run, stderr};
use minisign::{PublicKey, SecretKey, SecretKeyBox};
use modn_core::buckets::{key_statement_bytes, KeyChangeKind};
use std::fs;
use std::io::Cursor;
use std::path::Path;

pub fn sign(catalog: &Path, key: &Path) {
    sign_with_revocation(catalog, key, None);
}

pub fn sign_with_revocation(catalog: &Path, key: &Path, revocation_public_key: Option<&Path>) {
    let mut arguments = vec![
        "--json".to_owned(),
        "catalog".to_owned(),
        "sign".to_owned(),
        path_text(catalog).to_owned(),
        "--key".to_owned(),
        path_text(key).to_owned(),
    ];
    if let Some(path) = revocation_public_key {
        arguments.push("--revocation-public-key".to_owned());
        arguments.push(path_text(path).to_owned());
    }
    let borrowed: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let output = run(&borrowed);
    assert!(
        output.status.success(),
        "catalog sign failed: {}",
        stderr(&output)
    );
}

pub fn rotate(catalog: &Path, key: &Path, new_public_key: &Path) {
    let output = run(&[
        "--json",
        "catalog",
        "rotate",
        path_text(catalog),
        "--key",
        path_text(key),
        "--new-public-key",
        path_text(new_public_key),
    ]);
    assert!(
        output.status.success(),
        "catalog rotate failed: {}",
        stderr(&output)
    );
}

pub fn revoke(catalog: &Path, revocation_key: &Path, revoked: &str, replacement_public_key: &Path) {
    let output = run(&[
        "--json",
        "catalog",
        "revoke",
        path_text(catalog),
        "--revocation-key",
        path_text(revocation_key),
        "--revoked",
        revoked,
        "--replacement-public-key",
        path_text(replacement_public_key),
    ]);
    assert!(
        output.status.success(),
        "catalog revoke failed: {}",
        stderr(&output)
    );
}

pub fn forge_rotation(catalog: &Path, body: &[u8], key: &Path) {
    forge(catalog, KeyChangeKind::Rotated, "rotation", body, key);
}

pub fn forge_revocation(catalog: &Path, body: &[u8], key: &Path) {
    forge(catalog, KeyChangeKind::Revoked, "revocation", body, key);
}

fn forge(catalog: &Path, kind: KeyChangeKind, prefix: &str, body: &[u8], key: &Path) {
    let name = serde_json::from_slice::<serde_json::Value>(
        &fs::read(catalog.join("bucket.json")).expect("bucket metadata"),
    )
    .expect("bucket JSON")["name"]
        .as_str()
        .expect("catalog name")
        .to_owned();
    let text = fs::read_to_string(key).expect("secret key");
    let boxed = SecretKeyBox::from_string(&text).expect("secret key box");
    let secret = SecretKey::from_unencrypted_box(boxed).expect("secret key");
    let public = PublicKey::from_secret_key(&secret).expect("public key");
    let bytes = key_statement_bytes(kind, &name, body);
    let signature = minisign::sign(
        Some(&public),
        &secret,
        Cursor::new(bytes),
        Some("fixture rotation"),
        Some("fixture rotation signature"),
    )
    .expect("statement signature")
    .into_string();
    fs::write(catalog.join(format!("{prefix}.json")), body).expect("statement body");
    fs::write(catalog.join(format!("{prefix}.sig")), signature).expect("statement signature");
}

pub fn public_text(path: &Path) -> String {
    fs::read_to_string(path)
        .expect("public key")
        .trim()
        .to_owned()
}

/// Sign a catalog tree with a key without `modn catalog sign`.
///
/// The authoring command refuses to sign a revision older than a published key
/// statement. A test uses this helper to build such a tree and prove that sync
/// refuses it.
pub fn sign_catalog_direct(catalog: &Path, key: &Path) {
    let text = fs::read_to_string(key).expect("secret key");
    let boxed = SecretKeyBox::from_string(&text).expect("secret key box");
    let secret = SecretKey::from_unencrypted_box(boxed).expect("secret key");
    let public = PublicKey::from_secret_key(&secret).expect("public key");
    let statement = modn_core::buckets::catalog_statement(catalog).expect("catalog statement");
    let signature = minisign::sign(
        Some(&public),
        &secret,
        Cursor::new(statement.as_bytes()),
        Some("fixture catalog"),
        Some("fixture catalog signature"),
    )
    .expect("catalog signature")
    .into_string();
    fs::write(
        catalog.join("catalog.pub"),
        public.to_box().expect("public key box").into_string(),
    )
    .expect("catalog public key");
    fs::write(catalog.join("catalog.sig"), signature).expect("catalog signature");
}
