#![allow(
    clippy::too_many_lines,
    reason = "behavior tests read as one scenario each"
)]

#[path = "support/catalog_authoring.rs"]
mod catalog_authoring;
#[path = "support/catalog_mirrors.rs"]
mod catalog_mirrors;
#[path = "support/catalog_sequence.rs"]
mod catalog_sequence;
#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/zip_catalog.rs"]
mod zip_catalog;
#[path = "support/zip_catalog_fixture.rs"]
mod zip_catalog_fixture;

use catalog_authoring::{path_text, prepare_catalog, run};
use catalog_mirrors::{keygen, pack_catalog};
use catalog_sequence::{
    catalog_json, error_code, numbered_catalog, numbered_zip, sign, state_snapshot,
    statement_sequence, stderr_text as stderr, stdout, sync, write_sequence,
};
use serde_json::Value;
use std::path::PathBuf;
use zip_catalog::snapshot;
use zip_catalog_fixture::Fixture;

const EXPIRED: &str = "2020-01-01T00:00:00Z";
const FUTURE: &str = "2030-01-01T00:00:00Z";

fn keys(fixture: &Fixture) -> (PathBuf, String) {
    let directory = fixture.temp.path().join("keys");
    let fingerprint = keygen(&directory, "author");
    (directory, fingerprint)
}

#[test]
fn update_refresh_reports_the_accepted_sequence() {
    let fixture = Fixture::new();
    let (keys, _) = keys(&fixture);
    let zip = numbered_zip(&fixture, &keys, "community", "5");
    assert!(sync(&fixture, &zip, false).status.success());

    let output = fixture.run(&["update"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("sequence 5 accepted"),
        "{}",
        stdout(&output)
    );
    let json = fixture.run(&["--json", "update"]);
    let value: Value = serde_json::from_slice(&json.stdout).expect("update JSON");
    let community = value["buckets"]
        .as_array()
        .expect("bucket array")
        .iter()
        .find(|entry| entry["name"] == "community")
        .expect("community refresh");
    assert_eq!(community["sequence"], 5, "{community}");
}

#[test]
fn first_sync_of_a_numbered_catalog_reports_the_accepted_sequence() {
    let fixture = Fixture::new();
    let (keys, _) = keys(&fixture);
    let zip = numbered_zip(&fixture, &keys, "community", "3");
    let output = sync(&fixture, &zip, false);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("sequence 3 accepted"),
        "{}",
        stdout(&output)
    );

    let again = fixture.run(&["--json", "bucket", "sync", "community"]);
    assert!(again.status.success(), "{}", stderr(&again));
    let value: Value = serde_json::from_slice(&again.stdout).expect("sync JSON");
    assert_eq!(value["sequence"], 3, "{value}");
    assert!(value["warnings"].is_null(), "{value}");
}

#[test]
fn a_stale_signed_catalog_is_refused_and_changes_nothing() {
    let fixture = Fixture::new();
    let (keys, _) = keys(&fixture);
    let zip = numbered_zip(&fixture, &keys, "community", "2");
    assert!(sync(&fixture, &zip, false).status.success());
    let before = state_snapshot(&fixture);

    let _ = numbered_zip(&fixture, &keys, "community-stale", "1");
    let refused = sync(&fixture, &zip, true);
    assert!(
        !refused.status.success(),
        "a stale sequence must be refused"
    );
    assert_eq!(
        error_code(&refused),
        "CATALOG_ROLLBACK",
        "{}",
        stderr(&refused)
    );
    assert!(
        stderr(&refused).contains("already accepted sequence 2"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(before, state_snapshot(&fixture));
}

#[test]
fn the_same_sequence_with_changed_content_is_refused() {
    let fixture = Fixture::new();
    let (keys, _) = keys(&fixture);
    let zip = fixture.temp.path().join("community.zip");
    let first = numbered_catalog(
        fixture.temp.path(),
        "community",
        &keys,
        "author",
        "2",
        None,
        &[],
    );
    pack_catalog(&first, &zip);
    assert!(sync(&fixture, &zip, false).status.success());
    let before = state_snapshot(&fixture);

    let changed = numbered_catalog(
        fixture.temp.path(),
        "community-changed",
        &keys,
        "author",
        "2",
        None,
        &[("notes.txt", "another revision")],
    );
    pack_catalog(&changed, &zip);
    let refused = sync(&fixture, &zip, true);
    assert!(!refused.status.success());
    assert_eq!(
        error_code(&refused),
        "CATALOG_SEQUENCE_CONFLICT",
        "{}",
        stderr(&refused)
    );
    assert!(
        stderr(&refused).contains("names one revision"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(before, state_snapshot(&fixture));
}

#[test]
fn an_expired_catalog_syncs_with_a_warning() {
    let fixture = Fixture::new();
    let (keys, _) = keys(&fixture);
    let catalog = prepare_catalog(fixture.temp.path(), "community");
    write_sequence(
        &catalog,
        &format!("{{\"schema_version\":1,\"sequence\":1,\"expires\":\"{EXPIRED}\"}}"),
    );
    let signed = sign(&catalog, &keys.join("author.key"), None, None);
    assert!(signed.status.success(), "{}", stderr(&signed));
    let zip = pack_catalog(&catalog, &fixture.temp.path().join("community.zip"));
    let output = sync(&fixture, &zip, false);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("sequence 1 accepted"), "{text}");
    assert!(
        text.contains(&format!(
            "warning: catalog community expired on {EXPIRED}; its publisher asks users not to rely on it. The sync completed."
        )),
        "{text}"
    );

    let again = fixture.run(&["--json", "bucket", "sync", "community"]);
    let value: Value = serde_json::from_slice(&again.stdout).expect("sync JSON");
    assert_eq!(value["warnings"][0]["kind"], "expired", "{value}");
}

#[test]
fn sign_sequence_next_increments_and_a_lower_number_is_refused() {
    let fixture = Fixture::new();
    let keys = fixture.temp.path().join("keys");
    keygen(&keys, "author");
    let key = keys.join("author.key");
    let catalog = prepare_catalog(fixture.temp.path(), "community");

    let first = sign(&catalog, &key, Some("1"), None);
    assert!(first.status.success(), "{}", stderr(&first));
    assert_eq!(catalog_json(&catalog, "sequence.json")["sequence"], 1);
    let next = sign(&catalog, &key, Some("next"), None);
    assert!(next.status.success(), "{}", stderr(&next));
    assert_eq!(catalog_json(&catalog, "sequence.json")["sequence"], 2);

    let before = snapshot(&catalog);
    let lower = sign(&catalog, &key, Some("1"), None);
    assert!(!lower.status.success(), "a lower number must be refused");
    assert!(
        stderr(&lower).contains("never goes down"),
        "{}",
        stderr(&lower)
    );
    assert_eq!(before, snapshot(&catalog));
}

#[test]
fn sign_rewrites_sequence_json_completely_from_the_flags() {
    let fixture = Fixture::new();
    let keys = fixture.temp.path().join("keys");
    keygen(&keys, "author");
    let key = keys.join("author.key");
    let catalog = prepare_catalog(fixture.temp.path(), "community");

    let stamped = sign(&catalog, &key, Some("2"), Some(FUTURE));
    assert!(stamped.status.success(), "{}", stderr(&stamped));
    assert_eq!(catalog_json(&catalog, "sequence.json")["expires"], FUTURE);
    let next = sign(&catalog, &key, Some("next"), None);
    assert!(next.status.success(), "{}", stderr(&next));
    let value = catalog_json(&catalog, "sequence.json");
    assert_eq!(value["sequence"], 3);
    assert!(value.get("expires").is_none(), "{value}");

    let past = sign(&catalog, &key, Some("next"), Some(EXPIRED));
    assert!(!past.status.success(), "a past expiry must be refused");
    assert!(
        stderr(&past).contains("must be in the future"),
        "{}",
        stderr(&past)
    );
}

#[test]
fn verify_prints_the_sequence_and_the_expiry() {
    let fixture = Fixture::new();
    let (keys, _) = keys(&fixture);
    let catalog = numbered_catalog(
        fixture.temp.path(),
        "community",
        &keys,
        "author",
        "4",
        Some(FUTURE),
        &[],
    );
    let output = run(&[
        "catalog",
        "verify",
        path_text(&catalog),
        "--key",
        path_text(&catalog.join("catalog.pub")),
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("sequence: 4"), "{text}");
    assert!(text.contains(&format!("expires: {FUTURE}")), "{text}");

    let json = run(&[
        "--json",
        "catalog",
        "verify",
        path_text(&catalog),
        "--key",
        path_text(&catalog.join("catalog.pub")),
    ]);
    let value: Value = serde_json::from_slice(&json.stdout).expect("verify JSON");
    assert_eq!(value["sequence"], 4, "{value}");
    assert_eq!(value["expires"], FUTURE, "{value}");

    let plain = prepare_catalog(fixture.temp.path(), "community-plain");
    assert!(sign(&plain, &keys.join("author.key"), None, None)
        .status
        .success());
    let plain = run(&[
        "catalog",
        "verify",
        path_text(&plain),
        "--key",
        path_text(&plain.join("catalog.pub")),
    ]);
    assert!(
        stdout(&plain).contains("sequence: unnumbered"),
        "{}",
        stdout(&plain)
    );
}

#[test]
fn rotate_and_revoke_write_the_statement_sequence() {
    let fixture = Fixture::new();
    let (keys, author) = keys(&fixture);
    let next = fixture.temp.path().join("next-keys");
    keygen(&next, "author");
    let revocation = fixture.temp.path().join("revocation-keys");
    keygen(&revocation, "revocation");
    let catalog = numbered_catalog(
        fixture.temp.path(),
        "community",
        &keys,
        "author",
        "3",
        None,
        &[],
    );

    let before = snapshot(&catalog);
    let mismatched = run(&[
        "--json",
        "catalog",
        "rotate",
        path_text(&catalog),
        "--key",
        path_text(&keys.join("author.key")),
        "--new-public-key",
        path_text(&next.join("author.pub")),
        "--sequence",
        "4",
    ]);
    assert!(!mismatched.status.success());
    assert!(
        stderr(&mismatched).contains("differs from sequence.json sequence 3"),
        "{}",
        stderr(&mismatched)
    );
    assert_eq!(before, snapshot(&catalog));

    let rotated = run(&[
        "--json",
        "catalog",
        "rotate",
        path_text(&catalog),
        "--key",
        path_text(&keys.join("author.key")),
        "--new-public-key",
        path_text(&next.join("author.pub")),
    ]);
    assert!(rotated.status.success(), "{}", stderr(&rotated));
    assert_eq!(statement_sequence(&catalog, "rotation.json"), Some(3));

    let revoked = run(&[
        "--json",
        "catalog",
        "revoke",
        path_text(&catalog),
        "--revocation-key",
        path_text(&revocation.join("revocation.key")),
        "--revoked",
        &author,
        "--replacement-public-key",
        path_text(&next.join("author.pub")),
    ]);
    assert!(revoked.status.success(), "{}", stderr(&revoked));
    assert_eq!(statement_sequence(&catalog, "revocation.json"), Some(3));
}

#[test]
fn an_unnumbered_legacy_catalog_still_syncs() {
    let fixture = Fixture::new();
    let (keys, _) = keys(&fixture);
    let catalog = prepare_catalog(fixture.temp.path(), "community");
    let signed = sign(&catalog, &keys.join("author.key"), None, None);
    assert!(signed.status.success(), "{}", stderr(&signed));
    assert!(stderr(&signed).contains("rollback"), "{}", stderr(&signed));
    let zip = pack_catalog(&catalog, &fixture.temp.path().join("community.zip"));
    let output = sync(&fixture, &zip, false);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!stdout(&output).contains("sequence"), "{}", stdout(&output));

    let json = fixture.run(&["--json", "bucket", "sync", "community"]);
    let value: Value = serde_json::from_slice(&json.stdout).expect("sync JSON");
    assert!(value.get("sequence").is_none(), "{value}");
}

#[test]
fn a_downgrade_to_unnumbered_after_a_numbered_sync_is_refused() {
    let fixture = Fixture::new();
    let (keys, _) = keys(&fixture);
    let zip = fixture.temp.path().join("community.zip");
    let numbered = numbered_catalog(
        fixture.temp.path(),
        "community",
        &keys,
        "author",
        "2",
        None,
        &[],
    );
    pack_catalog(&numbered, &zip);
    assert!(sync(&fixture, &zip, false).status.success());
    let before = state_snapshot(&fixture);

    let plain = prepare_catalog(fixture.temp.path(), "community-plain");
    assert!(sign(&plain, &keys.join("author.key"), None, None)
        .status
        .success());
    pack_catalog(&plain, &zip);
    let refused = sync(&fixture, &zip, true);
    assert!(!refused.status.success());
    assert_eq!(
        error_code(&refused),
        "CATALOG_ROLLBACK",
        "{}",
        stderr(&refused)
    );
    assert!(
        stderr(&refused).contains("is unnumbered"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(before, state_snapshot(&fixture));
}

#[test]
fn bucket_remove_then_readd_accepts_an_older_number() {
    let fixture = Fixture::new();
    let (keys, _) = keys(&fixture);
    let zip = fixture.temp.path().join("community.zip");
    let numbered = numbered_catalog(
        fixture.temp.path(),
        "community",
        &keys,
        "author",
        "2",
        None,
        &[],
    );
    pack_catalog(&numbered, &zip);
    assert!(sync(&fixture, &zip, false).status.success());
    let stale = numbered_catalog(
        fixture.temp.path(),
        "community-stale",
        &keys,
        "author",
        "1",
        None,
        &[],
    );
    pack_catalog(&stale, &zip);
    assert!(!sync(&fixture, &zip, true).status.success());

    let removed = fixture.run(&["bucket", "remove", "community"]);
    assert!(removed.status.success(), "{}", stderr(&removed));
    let output = sync(&fixture, &zip, false);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("sequence 1 accepted"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn bucket_update_reports_the_official_accepted_sequence() {
    let fixture = Fixture::numbered_main(1);
    let output = fixture.run(&["bucket", "update"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("sequence 1 accepted"),
        "{}",
        stdout(&output)
    );

    let again = fixture.run(&["--json", "bucket", "update"]);
    assert!(again.status.success(), "{}", stderr(&again));
    let value: Value = serde_json::from_slice(&again.stdout).expect("update JSON");
    assert_eq!(value["sequence"], 1, "{value}");
    assert!(
        value["warnings"].is_null() || value["warnings"] == serde_json::json!([]),
        "{value}"
    );
}
