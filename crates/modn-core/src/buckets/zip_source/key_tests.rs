//! Attacks on the pinned catalog signer key and its revision `catalog.pub`.

use super::tests::{
    pack_catalog, published_snapshot, reformatted_key_text, registration_bytes, same_id_key_text,
    sign_bytes, signing_key, write_catalog, write_revocation_statement, write_rotation,
};
use crate::buckets::{catalog_statement, BucketStore, ZipCatalogSource};
use modn_core::manager_roots::ManagerRoots;
use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;

fn first_sync_with_revocation_key(
    state: &Path,
    catalog: &Path,
    primary: &str,
    mirror: &str,
    signer: &super::tests::SigningKey,
    revocation: &super::tests::SigningKey,
) {
    fs::write(catalog.join("revocation.pub"), &revocation.text).expect("revocation key");
    write_catalog(catalog, mirror, signer);
    let first = pack_catalog(catalog);
    let store = BucketStore::new(&ManagerRoots::new(state, None));
    store
        .sync_zip_with_signer(
            "community",
            Some(ZipCatalogSource::new(primary.to_owned())),
            move |_| Ok(Box::new(Cursor::new(first.clone())) as Box<dyn Read>),
        )
        .expect("first sync");
}

#[test]
fn mirror_revocation_with_a_swapped_catalog_public_key_is_refused() {
    let state = tempfile::tempdir().expect("state");
    let catalog = tempfile::tempdir().expect("catalog");
    let old = signing_key();
    let revocation = signing_key();
    let replacement = signing_key();
    let donor = signing_key();
    let primary = "https://primary.example/catalog.zip";
    let mirror = "https://mirror.example/catalog.zip";
    first_sync_with_revocation_key(
        state.path(),
        catalog.path(),
        primary,
        mirror,
        &old,
        &revocation,
    );
    let before_registration = registration_bytes(state.path());
    let before_published = published_snapshot(state.path());

    write_revocation_statement(catalog.path(), &revocation, &old, &replacement);
    fs::write(
        catalog.path().join("catalog.pub"),
        same_id_key_text(&replacement, &donor),
    )
    .expect("swapped public key");
    let statement = catalog_statement(catalog.path()).expect("statement");
    fs::write(
        catalog.path().join("catalog.sig"),
        sign_bytes(&replacement, statement.as_bytes()),
    )
    .expect("replacement signature");
    let rotated = pack_catalog(catalog.path());

    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));
    let error = store
        .sync_zip_with_signer("community", None, move |source| {
            if source.url == primary {
                anyhow::bail!("primary is offline");
            }
            Ok(Box::new(Cursor::new(rotated.clone())) as Box<dyn Read>)
        })
        .expect_err("a swapped replacement key must be refused")
        .to_string();
    assert!(
        error.contains("publishes a different public key"),
        "{error}"
    );
    assert_eq!(registration_bytes(state.path()), before_registration);
    assert_eq!(published_snapshot(state.path()), before_published);
}

#[test]
fn ordinary_sync_with_a_swapped_catalog_public_key_is_refused() {
    let state = tempfile::tempdir().expect("state");
    let catalog = tempfile::tempdir().expect("catalog");
    let pinned = signing_key();
    let donor = signing_key();
    let source = "https://primary.example/catalog.zip";
    write_catalog(catalog.path(), source, &pinned);
    let first = pack_catalog(catalog.path());
    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));
    store
        .sync_zip_with_signer(
            "community",
            Some(ZipCatalogSource::new(source.to_owned())),
            move |_| Ok(Box::new(Cursor::new(first.clone())) as Box<dyn Read>),
        )
        .expect("first sync");
    let before_registration = registration_bytes(state.path());
    let before_published = published_snapshot(state.path());

    fs::write(
        catalog.path().join("mods/test-game/example.json"),
        super::tests::RECORD.replace("\"1.0\"", "\"2.0\""),
    )
    .expect("new record");
    let statement = catalog_statement(catalog.path()).expect("statement");
    fs::write(
        catalog.path().join("catalog.sig"),
        sign_bytes(&pinned, statement.as_bytes()),
    )
    .expect("signature");
    fs::write(
        catalog.path().join("catalog.pub"),
        same_id_key_text(&pinned, &donor),
    )
    .expect("swapped public key");
    let revision = pack_catalog(catalog.path());

    let error = store
        .sync_zip_with_signer("community", None, move |_| {
            Ok(Box::new(Cursor::new(revision.clone())) as Box<dyn Read>)
        })
        .expect_err("a swapped pinned key must be refused")
        .to_string();
    assert!(
        error.contains("publishes a different public key"),
        "{error}"
    );
    assert_eq!(registration_bytes(state.path()), before_registration);
    assert_eq!(published_snapshot(state.path()), before_published);
}

#[test]
fn reformatted_catalog_public_key_with_the_same_bytes_is_accepted() {
    let state = tempfile::tempdir().expect("state");
    let catalog = tempfile::tempdir().expect("catalog");
    let old = signing_key();
    let new = signing_key();
    let source = "https://primary.example/catalog.zip";
    write_catalog(catalog.path(), source, &old);
    let first = pack_catalog(catalog.path());
    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));
    store
        .sync_zip_with_signer(
            "community",
            Some(ZipCatalogSource::new(source.to_owned())),
            move |_| Ok(Box::new(Cursor::new(first.clone())) as Box<dyn Read>),
        )
        .expect("first sync");

    write_rotation(catalog.path(), &old, &new);
    fs::write(
        catalog.path().join("catalog.pub"),
        reformatted_key_text(&new),
    )
    .expect("reformatted public key");
    let rotated = pack_catalog(catalog.path());

    let synced = store
        .sync_zip_with_signer("community", None, move |_| {
            Ok(Box::new(Cursor::new(rotated.clone())) as Box<dyn Read>)
        })
        .expect("the same key bytes in another text form are accepted");
    assert_eq!(
        synced.revision.signer.key_id,
        new.public.key_id().to_string()
    );
    assert_eq!(synced.revision.signer.public_key, new.text);
}
