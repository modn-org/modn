#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/zip_catalog.rs"]
mod zip_catalog;
#[path = "support/zip_catalog_fixture.rs"]
mod zip_catalog_fixture;
use serde_json::Value;
use std::fs;
use std::process::Command;
use zip_catalog::{fixture_zip, pack};
use zip_catalog_fixture::{assert_refused_without_state, listed, stderr, Fixture};
const SIGNED_KEY: &str = "2A7E5A370638B878";
const REKEYED_KEY: &str = "DC69368386F82F95";
#[test]
fn valid_zip_registers_pins_and_replaces_atomically() {
    let fixture = Fixture::new();
    let first = fixture.sync(&fixture_zip(fixture.temp.path(), "v2"));
    assert!(first.status.success(), "{}", stderr(&first));
    assert!(
        stderr(&first).contains(&format!("Pinned signer key {SIGNED_KEY}")),
        "{}",
        stderr(&first)
    );
    let synced: Value = serde_json::from_slice(&first.stdout).expect("sync JSON");
    assert_eq!(synced["signer"]["key_id"], SIGNED_KEY);
    assert_eq!(synced["pinned"], true);
    let entry = listed(&fixture)
        .into_iter()
        .find(|entry| entry["name"] == "community")
        .expect("registered bucket");
    assert_eq!(entry["source_kind"], "zip");
    assert_eq!(entry["signer_key_id"], SIGNED_KEY);
    let registration: Value =
        serde_json::from_slice(&fs::read(fixture.registration()).expect("registration"))
            .expect("registration JSON");
    assert_eq!(registration["zip"]["signer"]["key_id"], SIGNED_KEY);
    assert!(registration["zip"]["source"]["url"]
        .as_str()
        .expect("ZIP source")
        .ends_with(".zip"));
    let before = fixture.state_snapshot();
    let updated = fixture.temp.path().join("v1-update");
    zip_catalog::copy_catalog(&zip_catalog::catalog_fixture("v1"), &updated);
    let source = fixture_zip(fixture.temp.path(), "v2");
    pack(&updated, &source);
    let second = fixture.sync(&source);
    assert!(second.status.success(), "{}", stderr(&second));
    assert!(!stderr(&second).contains("Pinned signer key"));
    let after = fixture.state_snapshot();
    assert_ne!(before.0, after.0, "registration must change");
    let catalog = fixture.catalog();
    let record = fs::read_to_string(catalog.join("mods/test-game/example.json")).expect("record");
    assert!(record.contains("\"version\":\"1.0\""), "{record}");
}
#[test]
fn unsigned_zip_is_refused_and_writes_nothing() {
    let fixture = Fixture::new();
    let directory = fixture.temp.path().join("unsigned");
    zip_catalog::copy_catalog(&zip_catalog::catalog_fixture("v2"), &directory);
    fs::remove_file(directory.join("catalog.sig")).expect("remove signature");
    let zip = fixture.temp.path().join("unsigned.zip");
    pack(&directory, &zip);
    let output = fixture.sync(&zip);
    assert_refused_without_state(
        &fixture,
        &output,
        "is unsigned; Modn refuses unsigned catalogs",
    );
}
#[test]
fn tampered_zip_keeps_previous_registration_and_catalog() {
    let fixture = Fixture::new();
    assert!(fixture
        .sync(&fixture_zip(fixture.temp.path(), "v2"))
        .status
        .success());
    let before = fixture.state_snapshot();
    let directory = fixture.temp.path().join("tampered");
    zip_catalog::copy_catalog(&zip_catalog::catalog_fixture("v1"), &directory);
    let record = directory.join("mods/test-game/example.json");
    let mut bytes = fs::read(&record).expect("record");
    let last = bytes.len() - 2;
    bytes[last] ^= 1;
    fs::write(&record, bytes).expect("tampered record");
    let source = fixture.temp.path().join("v2.zip");
    pack(&directory, &source);
    let output = fixture.sync(&source);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("catalog signature verification failed"),
        "{}",
        stderr(&output)
    );
    assert_eq!(before, fixture.state_snapshot());
}
#[test]
fn wrong_or_missing_public_key_is_refused() {
    let fixture = Fixture::new();
    let directory = fixture.temp.path().join("wrong-key");
    zip_catalog::copy_catalog(&zip_catalog::catalog_fixture("v2"), &directory);
    fs::copy(
        zip_catalog::catalog_fixture("v2-rekeyed").join("catalog.pub"),
        directory.join("catalog.pub"),
    )
    .expect("foreign public key");
    let zip = fixture.temp.path().join("wrong-key.zip");
    pack(&directory, &zip);
    let output = fixture.sync(&zip);
    assert_refused_without_state(&fixture, &output, "does not match public key");
    let directory = fixture.temp.path().join("missing-key");
    zip_catalog::copy_catalog(&zip_catalog::catalog_fixture("v2"), &directory);
    fs::remove_file(directory.join("catalog.pub")).expect("remove public key");
    let zip = fixture.temp.path().join("missing-key.zip");
    pack(&directory, &zip);
    let output = fixture.sync(&zip);
    assert_refused_without_state(
        &fixture,
        &output,
        "is unsigned; Modn refuses unsigned catalogs",
    );
}
#[test]
fn different_signer_on_resync_is_refused() {
    let fixture = Fixture::new();
    assert!(fixture
        .sync(&fixture_zip(fixture.temp.path(), "v2"))
        .status
        .success());
    let before = fixture.state_snapshot();
    let directory = fixture.temp.path().join("rekeyed-update");
    zip_catalog::copy_catalog(&zip_catalog::catalog_fixture("v2-rekeyed"), &directory);
    let source = fixture.temp.path().join("v2.zip");
    pack(&directory, &source);
    let output = fixture.sync(&source);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains(&format!(
            "Signer key changed from {SIGNED_KEY} to {REKEYED_KEY}"
        )),
        "{}",
        stderr(&output)
    );
    assert_eq!(before, fixture.state_snapshot());
}
#[test]
fn revision_and_plain_http_are_refused_without_network() {
    let fixture = Fixture::new();
    let zip = fixture_zip(fixture.temp.path(), "v2");
    let output = fixture.run(&[
        "bucket",
        "sync",
        "community",
        "--source",
        zip.to_str().expect("ZIP path"),
        "--revision",
        "main",
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("`--revision` cannot be used with a ZIP catalog source"),
        "{}",
        stderr(&output)
    );
    let output = fixture.run(&[
        "bucket",
        "sync",
        "community",
        "--source",
        "http://example.org/catalog.zip",
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("must use https://; plain HTTP is refused"),
        "{}",
        stderr(&output)
    );
    assert!(!fixture.registration().exists());
}
#[test]
fn update_refreshes_zip_catalog_and_keeps_previous_on_failure() {
    let fixture = Fixture::new();
    let source = fixture_zip(fixture.temp.path(), "v2");
    assert!(fixture.sync(&source).status.success());
    let updated = fixture_zip(fixture.temp.path(), "v1");
    fs::copy(&updated, &source).expect("replace hosted ZIP");
    let report: Value =
        serde_json::from_str(&fixture.success(&["--json", "update"])).expect("update JSON");
    assert_eq!(report["failed"], 0);
    let community = report["buckets"]
        .as_array()
        .expect("bucket array")
        .iter()
        .find(|entry| entry["name"] == "community")
        .expect("community refresh");
    assert_eq!(community["kind"], "zip");
    assert_eq!(community["changed"], true);
    let digest = community["digest"].as_str().expect("digest").to_owned();
    let before = fixture.state_snapshot();
    let directory = fixture.temp.path().join("tampered");
    zip_catalog::copy_catalog(&zip_catalog::catalog_fixture("v2"), &directory);
    let record = directory.join("mods/test-game/example.json");
    let mut bytes = fs::read(&record).expect("record");
    let last = bytes.len() - 2;
    bytes[last] ^= 1;
    fs::write(&record, bytes).expect("tampered record");
    let tampered = fixture.temp.path().join("tampered.zip");
    pack(&directory, &tampered);
    fs::copy(&tampered, &source).expect("host tampered ZIP");
    let output = fixture.run(&["update"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("1 of 2 catalog refreshes failed"),
        "{}",
        stderr(&output)
    );
    assert_eq!(before, fixture.state_snapshot());
    let registration: Value =
        serde_json::from_slice(&fs::read(fixture.registration()).expect("registration"))
            .expect("registration JSON");
    assert_eq!(registration["zip"]["digest"], digest);
}
#[test]
fn contract_v2_violations_are_refused() {
    let fixture = Fixture::new();
    let keys = fixture.temp.path().join("keys");
    let keygen = Command::new(main_bucket::modn_binary())
        .args(["catalog", "keygen", "--out"])
        .arg(&keys)
        .output()
        .expect("keygen");
    assert!(keygen.status.success(), "{}", stderr(&keygen));
    let directory = fixture.temp.path().join("v1-contract");
    zip_catalog::copy_catalog(&zip_catalog::catalog_fixture("v2"), &directory);
    fs::write(
        directory.join("bucket.json"),
        r#"{"schema_version":1,"name":"community","title":"Old"}"#,
    )
    .expect("v1 metadata");
    let sign = Command::new(main_bucket::modn_binary())
        .args(["catalog", "sign"])
        .arg(&directory)
        .arg("--key")
        .arg(keys.join("catalog.key"))
        .output()
        .expect("sign v1");
    assert!(sign.status.success(), "{}", stderr(&sign));
    let zip = fixture.temp.path().join("v1-contract.zip");
    pack(&directory, &zip);
    let output = fixture.sync(&zip);
    assert_refused_without_state(&fixture, &output, "requires bucket contract v2");
    let directory = fixture.temp.path().join("empty-sources");
    zip_catalog::copy_catalog(&zip_catalog::catalog_fixture("v2"), &directory);
    fs::write(
        directory.join("mods/test-game/example.json"),
        r#"{"schema_version":2,"id":"example","game":"test-game","name":"Example","version":"1.0","archive_format":"zip","blake3":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","sources":[]}"#,
    )
    .expect("empty sources record");
    let sign = Command::new(main_bucket::modn_binary())
        .args(["catalog", "sign"])
        .arg(&directory)
        .arg("--key")
        .arg(keys.join("catalog.key"))
        .output()
        .expect("sign empty sources");
    assert!(sign.status.success(), "{}", stderr(&sign));
    let zip = fixture.temp.path().join("empty-sources.zip");
    pack(&directory, &zip);
    let output = fixture.sync(&zip);
    assert!(!output.status.success());
    assert!(!fixture.registration().exists());
}
