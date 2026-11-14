//! Milestone 8 exit gate, catalog sentences at the CLI: signed relocation,
//! a foreign signer at the old location, key rotation and its refusal before
//! a statement. Every refusal must leave the registration and catalog bytes
//! unchanged.
#![allow(
    clippy::too_many_lines,
    reason = "each gate sentence reads as one scenario"
)]

#[path = "support/catalog_authoring.rs"]
mod catalog_authoring;
#[path = "support/catalog_mirrors.rs"]
mod catalog_mirrors;
#[path = "support/catalog_rotation.rs"]
mod catalog_rotation;
#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/zip_catalog.rs"]
mod zip_catalog;
#[path = "support/zip_catalog_fixture.rs"]
mod zip_catalog_fixture;

use catalog_authoring::{copy_tree, path_text, stderr};
use catalog_mirrors::{
    keygen, pack_catalog, relocation_json, signed_catalog, snapshot,
};
use catalog_rotation::{rotate, sign};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use zip_catalog::pack;
use zip_catalog_fixture::Fixture;

/// Registration file bytes plus every published catalog file, in sorted order.
fn stored_state(fixture: &Fixture) -> (Vec<u8>, Vec<(String, Vec<u8>)>) {
    (
        fs::read(fixture.registration()).expect("registration bytes"),
        snapshot(&fixture.catalog()),
    )
}

fn registration(fixture: &Fixture) -> Value {
    serde_json::from_slice(&fs::read(fixture.registration()).expect("registration"))
        .expect("registration JSON")
}

fn listed(fixture: &Fixture, name: &str) -> Value {
    let output = fixture.success(&["--json", "bucket", "list"]);
    let entries: Vec<Value> = serde_json::from_str(&output).expect("bucket list JSON");
    entries
        .into_iter()
        .find(|entry| entry["name"] == name)
        .expect("registered bucket")
}

fn author_catalog(fixture: &Fixture, keys: &Path) -> PathBuf {
    signed_catalog(fixture.temp.path(), "community", keys, "author", &[])
}

#[test]
fn after_the_author_moves_the_catalog_the_next_sync_follows_the_signed_relocation() {
    let fixture = Fixture::new();
    let keys = fixture.temp.path().join("keys");
    let author = keygen(&keys, "author");
    let catalog = author_catalog(&fixture, &keys);
    let old = pack_catalog(&catalog, &fixture.temp.path().join("old-primary.zip"));

    let first = fixture.sync(&old);
    assert!(first.status.success(), "{}", stderr(&first));
    assert!(
        stderr(&first).contains(&format!("Pinned signer key {author}")),
        "{}",
        stderr(&first)
    );

    let new = "https://new.example/catalog.zip";
    let local_target = fixture.temp.path().join("local-target.zip");
    let invalid = signed_catalog(
        fixture.temp.path(),
        "community-invalid-move",
        &keys,
        "author",
        &[(
            "relocation.json",
            &relocation_json(path_text(&local_target)),
        )],
    );
    pack(&invalid, &old);
    let before_invalid = stored_state(&fixture);
    let refused = fixture.run(&["bucket", "sync", "community"]);
    assert!(!refused.status.success(), "a local relocation target was accepted");
    assert!(
        stderr(&refused).contains("a declared ZIP source must use an https:// URL"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(before_invalid, stored_state(&fixture));

    let moved = signed_catalog(
        fixture.temp.path(),
        "community-moved",
        &keys,
        "author",
        &[("relocation.json", &relocation_json(new))],
    );
    pack(&moved, &old);

    let followed = fixture.run(&["--json", "update"]);
    assert!(followed.status.success(), "{}", stderr(&followed));
    assert!(
        stderr(&followed).contains(&format!(
            "catalog community moved from zip {} to zip {new}.",
            path_text(&old)
        )),
        "{}",
        stderr(&followed)
    );
    assert_eq!(registration(&fixture)["zip"]["source"]["url"], new);
    let entry = listed(&fixture, "community");
    assert_eq!(entry["source_kind"], "zip");
    assert_eq!(entry["primary"]["url"], new);
    assert_eq!(entry["signer_key_id"], author.as_str());

    let state_before_old_url = stored_state(&fixture);
    let refused = fixture.run(&[
        "bucket",
        "sync",
        "community",
        "--source",
        path_text(&old),
    ]);
    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains("changing a registered source is refused"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(state_before_old_url, stored_state(&fixture));
}

#[test]
fn modn_refuses_a_catalog_at_the_old_url_that_a_different_key_signed() {
    let fixture = Fixture::new();
    let keys = fixture.temp.path().join("keys");
    let author = keygen(&keys, "author");
    let other = fixture.temp.path().join("other-keys");
    let foreign_key = keygen(&other, "author");
    let zip = pack_catalog(&author_catalog(&fixture, &keys), &fixture.temp.path().join("old.zip"));

    let first = fixture.sync(&zip);
    assert!(first.status.success(), "{}", stderr(&first));
    assert!(
        stderr(&first).contains(&format!("Pinned signer key {author}")),
        "{}",
        stderr(&first)
    );
    let before = stored_state(&fixture);

    let foreign = signed_catalog(
        fixture.temp.path(),
        "community-foreign",
        &other,
        "author",
        &[(
            "relocation.json",
            &relocation_json("https://elsewhere.example/catalog.zip"),
        )],
    );
    pack(&foreign, &zip);

    let refused = fixture.run(&["bucket", "sync", "community"]);
    assert!(!refused.status.success(), "a foreign signer was accepted");
    let message = stderr(&refused);
    assert!(
        message.contains(&format!(
            "Signer key changed from {author} to {foreign_key} for bucket community"
        )),
        "{message}"
    );
    assert!(
        message.contains("A key change needs a signed rotation or revocation statement"),
        "{message}"
    );
    assert_eq!(before, stored_state(&fixture));
    assert_eq!(registration(&fixture)["zip"]["signer"]["key_id"], author);
    assert_eq!(
        listed(&fixture, "community")["signer_key_id"],
        author.as_str()
    );
}

#[test]
fn after_a_signed_handover_modn_accepts_the_successor_keys_catalog() {
    let fixture = Fixture::new();
    let keys = fixture.temp.path().join("keys");
    let old = keygen(&keys, "author");
    let next = keygen(&keys, "next");
    let catalog = author_catalog(&fixture, &keys);
    let zip = pack_catalog(&catalog, &fixture.temp.path().join("community.zip"));
    let before_rotation = fixture.temp.path().join("before-rotation");
    copy_tree(&catalog, &before_rotation);

    let first = fixture.sync(&zip);
    assert!(first.status.success(), "{}", stderr(&first));
    assert!(
        stderr(&first).contains(&format!("Pinned signer key {old}")),
        "{}",
        stderr(&first)
    );

    rotate(&catalog, &keys.join("author.key"), &keys.join("next.pub"));
    sign(&catalog, &keys.join("next.key"));
    pack(&catalog, &zip);

    let rotated = fixture.sync(&zip);
    assert!(rotated.status.success(), "{}", stderr(&rotated));
    assert!(
        stderr(&rotated).contains(&format!("signing key rotated: {old} -> {next}")),
        "{}",
        stderr(&rotated)
    );
    let stored = registration(&fixture);
    assert_eq!(stored["zip"]["signer"]["key_id"], next);
    assert_eq!(
        stored["zip"]["signer"]["previous_signers"],
        serde_json::json!([old])
    );
    let entry = listed(&fixture, "community");
    assert_eq!(entry["signer_key_id"], next.as_str());
    assert_eq!(entry["previous_signers"], serde_json::json!([old]));

    let repeated = fixture.sync(&zip);
    assert!(repeated.status.success(), "{}", stderr(&repeated));
    assert!(!stderr(&repeated).contains("signing key"), "{}", stderr(&repeated));

    let before_retired = stored_state(&fixture);
    pack(&before_rotation, &zip);
    let retired = fixture.run(&["bucket", "sync", "community"]);
    assert!(!retired.status.success(), "the retired signer was accepted");
    assert!(
        stderr(&retired).contains(&format!(
            "Signing key {old} for bucket community was rotated away"
        )),
        "{}",
        stderr(&retired)
    );
    assert_eq!(before_retired, stored_state(&fixture));
}

#[test]
fn modn_refuses_a_catalog_signed_by_the_successor_key_before_any_statement() {
    let fixture = Fixture::new();
    let keys = fixture.temp.path().join("keys");
    let old = keygen(&keys, "author");
    let next = keygen(&keys, "next");
    let catalog = author_catalog(&fixture, &keys);
    let zip = pack_catalog(&catalog, &fixture.temp.path().join("community.zip"));

    let first = fixture.sync(&zip);
    assert!(first.status.success(), "{}", stderr(&first));
    let before = stored_state(&fixture);

    sign(&catalog, &keys.join("next.key"));
    pack(&catalog, &zip);

    let refused = fixture.run(&["bucket", "sync", "community"]);
    assert!(
        !refused.status.success(),
        "a successor without a statement was accepted"
    );
    let message = stderr(&refused);
    assert!(
        message.contains(&format!(
            "Signer key changed from {old} to {next} for bucket community"
        )),
        "{message}"
    );
    assert!(
        message.contains("remove the bucket and sync it again to trust the new key"),
        "{message}"
    );
    assert_eq!(before, stored_state(&fixture));
    assert_eq!(registration(&fixture)["zip"]["signer"]["key_id"], old);
}
