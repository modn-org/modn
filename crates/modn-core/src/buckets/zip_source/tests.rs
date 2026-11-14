use crate::buckets::{BucketStore, ZipCatalogSource};
use std::cell::Cell;
use std::fs;
use std::io::{self, Cursor, Read, Write};
use std::path::Path;

fn source(path: &Path) -> ZipCatalogSource {
    ZipCatalogSource::new(path.to_str().expect("UTF-8 path").to_owned())
}

fn staging_left(state: &Path) -> bool {
    fs::read_dir(state.join("buckets"))
        .map(|mut entries| {
            entries.any(|entry| {
                entry
                    .map(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with("zip-catalog-")
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn assert_no_state(state: &Path) {
    assert!(!state.join("buckets/registrations/community.json").exists());
    assert!(!staging_left(state), "staging directory left behind");
}

#[test]
fn injected_download_failure_keeps_state_and_removes_staging() {
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(state.path());
    let zip = state.path().join("catalog.zip");
    let error = store
        .sync_zip_with_signer("community", Some(source(&zip)), |_| {
            anyhow::bail!("injected download failure")
        })
        .expect_err("download must fail");
    assert!(error.to_string().contains("injected download failure"));
    assert_no_state(state.path());
}

#[test]
fn truncated_download_is_refused_without_state() {
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(state.path());
    let zip = state.path().join("catalog.zip");
    let mut writer = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
    writer
        .start_file("bucket.json", zip::write::SimpleFileOptions::default())
        .expect("entry");
    std::io::Write::write_all(&mut writer, b"{}").expect("bytes");
    let bytes = writer.finish().expect("ZIP").into_inner();
    let truncated = bytes[..bytes.len() / 2].to_vec();
    let error = store
        .sync_zip_with_signer("community", Some(source(&zip)), move |_| {
            Ok(Box::new(Cursor::new(truncated.clone())) as Box<dyn Read>)
        })
        .expect_err("truncated ZIP must fail");
    assert!(error.to_string().contains("invalid ZIP"), "{error}");
    assert_no_state(state.path());
}

#[test]
fn oversized_download_is_refused_without_state() {
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(state.path());
    let zip = state.path().join("catalog.zip");
    let error = store
        .sync_zip_with_signer("community", Some(source(&zip)), |_| {
            Ok(Box::new(io::repeat(0).take(super::MAX_CATALOG_ZIP_BYTES + 1)) as Box<dyn Read>)
        })
        .expect_err("oversized body must fail");
    assert!(error.to_string().contains("256 MiB size limit"), "{error}");
    assert_no_state(state.path());
}

#[test]
fn plain_http_is_refused_before_the_source_is_opened() {
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(state.path());
    let opened = Cell::new(false);
    let error = store
        .sync_zip_with_signer(
            "community",
            Some(ZipCatalogSource::new(
                "http://example.org/catalog.zip".to_owned(),
            )),
            |_| {
                opened.set(true);
                anyhow::bail!("opener must not run")
            },
        )
        .expect_err("plain HTTP must be refused");
    assert!(error.to_string().contains("must use https://"), "{error}");
    assert!(!opened.get(), "the opener ran for a refused source");
    assert_no_state(state.path());
}

const RECORD: &str = r#"{"schema_version":2,"id":"example","game":"test-game","name":"Example","version":"1.0","archive_format":"zip","blake3":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","sources":[{"kind":"manual","filename":"example.zip"}]}"#;

struct SigningKey {
    pair: minisign::KeyPair,
    public: crate::buckets::CatalogPublicKey,
    text: String,
}

fn signing_key() -> SigningKey {
    let pair = minisign::KeyPair::generate_unencrypted_keypair().expect("test key");
    let text = pair.pk.to_box().expect("public key").into_string();
    let public = crate::buckets::CatalogPublicKey::parse(&text).expect("parsed key");
    SigningKey { pair, public, text }
}

fn sign_bytes(key: &SigningKey, bytes: &[u8]) -> String {
    minisign::sign(
        Some(&key.pair.pk),
        &key.pair.sk,
        Cursor::new(bytes),
        Some("zip fixture"),
        Some("zip fixture signature"),
    )
    .expect("signature")
    .into_string()
}

fn write_catalog(root: &Path, mirror: &str, key: &SigningKey) {
    fs::create_dir_all(root.join("mods/test-game")).expect("catalog directory");
    fs::write(
        root.join("bucket.json"),
        r#"{"schema_version":2,"name":"community","title":"Signed"}"#,
    )
    .expect("bucket metadata");
    fs::write(root.join("mods/test-game/example.json"), RECORD).expect("record");
    fs::write(
        root.join("mirrors.json"),
        serde_json::json!({
            "schema_version": 1,
            "mirrors": [{"kind":"zip","url":mirror}]
        })
        .to_string(),
    )
    .expect("mirrors");
    let statement = crate::buckets::catalog_statement(root).expect("statement");
    fs::write(root.join("catalog.pub"), &key.text).expect("public key");
    fs::write(
        root.join("catalog.sig"),
        sign_bytes(key, statement.as_bytes()),
    )
    .expect("signature");
}

fn pack_catalog(root: &Path) -> Vec<u8> {
    fn collect(root: &Path, current: &Path, output: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(current).expect("catalog entries") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                collect(root, &path, output);
            } else {
                output.push((
                    path.strip_prefix(root)
                        .expect("relative path")
                        .to_str()
                        .expect("UTF-8 path")
                        .replace('\\', "/"),
                    fs::read(path).expect("bytes"),
                ));
            }
        }
    }
    let mut entries = Vec::new();
    collect(root, root, &mut entries);
    entries.sort();
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .expect("entry");
        writer.write_all(&bytes).expect("bytes");
    }
    writer.finish().expect("ZIP").into_inner()
}

#[test]
fn rotation_is_accepted_when_a_mirror_serves_the_revision() {
    let state = tempfile::tempdir().expect("state");
    let catalog = tempfile::tempdir().expect("catalog");
    let old = signing_key();
    let new = signing_key();
    let primary = "https://primary.example/catalog.zip";
    let mirror = "https://mirror.example/catalog.zip";
    write_catalog(catalog.path(), mirror, &old);
    let first = pack_catalog(catalog.path());

    let store = BucketStore::new(state.path());
    let first_bytes = first.clone();
    let synced = store
        .sync_zip_with_signer(
            "community",
            Some(ZipCatalogSource::new(primary.to_owned())),
            move |_| Ok(Box::new(Cursor::new(first_bytes.clone())) as Box<dyn Read>),
        )
        .expect("first sync");
    assert!(synced.pinned);
    assert!(synced.key_change.is_none());

    let body = crate::buckets::rotation_statement(&old.public.key_id().to_string(), &new.text)
        .expect("rotation body");
    fs::write(catalog.path().join("rotation.json"), &body).expect("rotation body");
    let bytes = crate::buckets::key_statement_bytes(
        crate::buckets::KeyChangeKind::Rotated,
        "community",
        &body,
    );
    fs::write(
        catalog.path().join("rotation.sig"),
        sign_bytes(&old, &bytes),
    )
    .expect("rotation signature");
    fs::write(catalog.path().join("catalog.pub"), &new.text).expect("new public key");
    let statement = crate::buckets::catalog_statement(catalog.path()).expect("new statement");
    fs::write(
        catalog.path().join("catalog.sig"),
        sign_bytes(&new, statement.as_bytes()),
    )
    .expect("new signature");
    let rotated = pack_catalog(catalog.path());

    let synced = store
        .sync_zip_with_signer("community", None, move |source| {
            if source.url == primary {
                anyhow::bail!("primary is offline");
            }
            Ok(Box::new(Cursor::new(rotated.clone())) as Box<dyn Read>)
        })
        .expect("mirror sync");
    let change = synced.key_change.expect("rotation");
    assert_eq!(change.kind, crate::buckets::KeyChangeKind::Rotated);
    assert_eq!(change.from, old.public.key_id().to_string());
    assert_eq!(change.to, new.public.key_id().to_string());
    assert_eq!(synced.revision.signer.key_id, change.to);
    assert_eq!(
        synced.revision.signer.previous_signers,
        vec![change.from.clone()]
    );
    assert_eq!(synced.trace.served_by.label(), format!("zip {mirror}"));
}
