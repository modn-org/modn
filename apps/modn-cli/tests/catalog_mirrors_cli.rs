#![allow(
    clippy::too_many_lines,
    reason = "behavior tests read as one scenario each"
)]

#[path = "support/catalog_authoring.rs"]
mod catalog_authoring;
#[path = "support/catalog_mirrors.rs"]
mod catalog_mirrors;
#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/zip_catalog.rs"]
mod zip_catalog;
#[path = "support/zip_catalog_fixture.rs"]
mod zip_catalog_fixture;

use catalog_authoring::path_text;
use catalog_mirrors::{
    keygen, mirrors_json, pack_catalog, relocation_json, signed_catalog, snapshot, MIRROR,
    NEW_PRIMARY, SECOND_MIRROR,
};
use serde_json::Value;
use std::fs;
use std::path::Path;
use zip_catalog_fixture::Fixture;

fn listed(fixture: &Fixture, name: &str) -> Value {
    let output = fixture.success(&["--json", "bucket", "list"]);
    let value: Value = serde_json::from_str(&output).expect("bucket list JSON");
    value
        .as_array()
        .expect("bucket list array")
        .iter()
        .find(|entry| entry["name"] == name)
        .cloned()
        .expect("registered bucket")
}

fn state_snapshot(fixture: &Fixture) -> (Vec<u8>, Vec<(String, Vec<u8>)>) {
    (
        fs::read(fixture.registration()).expect("registration bytes"),
        snapshot(&fixture.catalog()),
    )
}

fn registration(fixture: &Fixture) -> Value {
    serde_json::from_slice(&fs::read(fixture.registration()).expect("registration"))
        .expect("registration JSON")
}

fn first_catalog(fixture: &Fixture, keys: &Path, extra: &[(&str, &str)]) -> std::path::PathBuf {
    let catalog = signed_catalog(fixture.temp.path(), "community", keys, "author", extra);
    pack_catalog(&catalog, &fixture.temp.path().join("community.zip"))
}

#[test]
fn list_json_reports_the_primary_and_the_mirror_list() {
    let fixture = Fixture::new();
    let keys = fixture.temp.path().join("keys");
    keygen(&keys, "author");
    let zip = first_catalog(
        &fixture,
        &keys,
        &[("mirrors.json", &mirrors_json(&[MIRROR, SECOND_MIRROR]))],
    );
    fixture.success(&["bucket", "sync", "community", "--source", path_text(&zip)]);

    let entry = listed(&fixture, "community");
    assert_eq!(entry["source_kind"], "zip");
    assert_eq!(entry["primary"]["kind"], "zip");
    assert_eq!(entry["primary"]["url"], path_text(&zip));
    assert_eq!(
        entry["mirrors"],
        serde_json::json!([
            {"kind":"zip","url":MIRROR},
            {"kind":"zip","url":SECOND_MIRROR}
        ])
    );
    let text = fixture.success(&["bucket", "list"]);
    assert!(
        text.contains(&format!("primary zip {}", path_text(&zip))),
        "{text}"
    );
    assert!(text.contains("2 mirrors"), "{text}");
}

#[test]
fn sync_follows_a_signed_relocation_and_reports_it() {
    let fixture = Fixture::new();
    let keys = fixture.temp.path().join("keys");
    keygen(&keys, "author");
    let zip = first_catalog(
        &fixture,
        &keys,
        &[
            ("relocation.json", &relocation_json(NEW_PRIMARY)),
            ("mirrors.json", &mirrors_json(&[MIRROR])),
        ],
    );
    let output = fixture.success(&["bucket", "sync", "community", "--source", path_text(&zip)]);
    assert!(
        output.contains(&format!(
            "catalog community moved from zip {} to zip {NEW_PRIMARY}.",
            path_text(&zip)
        )),
        "{output}"
    );
    let entry = listed(&fixture, "community");
    assert_eq!(entry["primary"]["url"], NEW_PRIMARY);
    assert_eq!(
        entry["mirrors"],
        serde_json::json!([{"kind":"zip","url":MIRROR}])
    );
    assert_eq!(registration(&fixture)["zip"]["source"]["url"], NEW_PRIMARY);
}

#[test]
fn update_refreshes_through_the_relocation_path() {
    let fixture = Fixture::new();
    let keys = fixture.temp.path().join("keys");
    keygen(&keys, "author");
    let zip = first_catalog(&fixture, &keys, &[]);
    fixture.success(&["bucket", "sync", "community", "--source", path_text(&zip)]);

    let moved = signed_catalog(
        fixture.temp.path(),
        "community-moved",
        &keys,
        "author",
        &[("relocation.json", &relocation_json(NEW_PRIMARY))],
    );
    pack_catalog(&moved, &zip);

    let output = fixture.success(&["update"]);
    assert!(
        output.contains(&format!(
            "catalog community moved from zip {} to zip {NEW_PRIMARY}.",
            path_text(&zip)
        )),
        "{output}"
    );
    let entry = listed(&fixture, "community");
    assert_eq!(entry["primary"]["url"], NEW_PRIMARY);
}

#[test]
fn foreign_signer_refuses_the_relocation_and_keeps_the_state() {
    let fixture = Fixture::new();
    let keys = fixture.temp.path().join("keys");
    keygen(&keys, "author");
    let other = fixture.temp.path().join("other-keys");
    keygen(&other, "author");
    let zip = first_catalog(&fixture, &keys, &[]);
    fixture.success(&["bucket", "sync", "community", "--source", path_text(&zip)]);
    let before = state_snapshot(&fixture);

    let moved = signed_catalog(
        fixture.temp.path(),
        "community-rekeyed",
        &other,
        "author",
        &[("relocation.json", &relocation_json(NEW_PRIMARY))],
    );
    pack_catalog(&moved, &zip);
    let refused = fixture.run(&["bucket", "sync", "community"]);
    assert!(!refused.status.success());
    let message = String::from_utf8_lossy(&refused.stderr);
    assert!(
        message.contains(&format!("catalog source zip {}", path_text(&zip))),
        "{message}"
    );
    assert!(message.contains("Signer key changed"), "{message}");
    assert_eq!(before, state_snapshot(&fixture));
    assert_eq!(
        registration(&fixture)["zip"]["source"]["url"],
        path_text(&zip)
    );
}

#[test]
fn invalid_mirrors_refuse_the_revision_and_keep_the_state() {
    let fixture = Fixture::new();
    let keys = fixture.temp.path().join("keys");
    keygen(&keys, "author");
    let zip = first_catalog(&fixture, &keys, &[]);
    fixture.success(&["bucket", "sync", "community", "--source", path_text(&zip)]);
    let before = state_snapshot(&fixture);

    let invalid = signed_catalog(
        fixture.temp.path(),
        "community-invalid",
        &keys,
        "author",
        &[("mirrors.json", &mirrors_json(&["C:/local/catalog.zip"]))],
    );
    pack_catalog(&invalid, &zip);
    let refused = fixture.run(&["bucket", "sync", "community"]);
    assert!(!refused.status.success());
    let message = String::from_utf8_lossy(&refused.stderr);
    assert!(message.contains("invalid mirror"), "{message}");
    assert!(message.contains("mirrors.json"), "{message}");
    assert_eq!(before, state_snapshot(&fixture));
}
