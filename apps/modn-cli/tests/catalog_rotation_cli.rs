//! Catalog key rotation and offline revocation at the CLI.

#[path = "support/catalog_authoring.rs"]
mod catalog_authoring;
#[path = "support/catalog_mirrors.rs"]
mod catalog_mirrors;
#[path = "support/catalog_rotation.rs"]
mod catalog_rotation;
#[path = "support/catalog_rotation_state.rs"]
mod catalog_rotation_state;
#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/zip_catalog.rs"]
mod zip_catalog;
#[path = "support/zip_catalog_fixture.rs"]
mod zip_catalog_fixture;

use catalog_authoring::{commit_all, copy_tree, path_text, prepare_catalog, run, stderr};
use catalog_mirrors::{keygen, pack_catalog, signed_catalog};
use catalog_rotation::{revoke, rotate, sign};
use catalog_rotation_state::{
    copy_into, listed_entry, listed_entry_git, registration, state_snapshot, GitFixture,
};
use zip_catalog::pack;
use zip_catalog_fixture::Fixture;

#[test]
fn rotation_is_accepted_with_the_successor_and_trust_follows() {
    let fixture = Fixture::new();
    let temp = fixture.temp.path();
    let keys = temp.join("keys");
    let old = keygen(&keys, "author");
    let next = keygen(&keys, "next");
    let catalog = signed_catalog(temp, "community", &keys, "author", &[]);
    let saved = temp.join("old-catalog");
    copy_tree(&catalog, &saved);
    let source = pack_catalog(&catalog, &temp.join("community.zip"));

    let first = fixture.sync(&source);
    assert!(first.status.success(), "{}", stderr(&first));
    assert!(stderr(&first).contains(&format!("Pinned signer key {old}")));
    fixture.success(&["--json", "bucket", "trust-modules", "community", "--yes"]);

    rotate(&catalog, &keys.join("author.key"), &keys.join("next.pub"));
    sign(&catalog, &keys.join("next.key"));
    pack(&catalog, &source);
    let rotated = fixture.sync(&source);
    assert!(rotated.status.success(), "{}", stderr(&rotated));
    assert!(
        stderr(&rotated).contains(&format!("signing key rotated: {old} -> {next}")),
        "{}",
        stderr(&rotated)
    );
    let stored = registration(&fixture.temp.path().join("state"));
    assert_eq!(stored["zip"]["signer"]["key_id"], next);
    assert_eq!(
        stored["zip"]["signer"]["previous_signers"],
        serde_json::json!([old])
    );
    let entry = listed_entry(&fixture);
    assert_eq!(entry["signer_key_id"], next);
    assert_eq!(entry["previous_signers"], serde_json::json!([old]));
    assert_eq!(entry["module_trusted"], true);

    let repeated = fixture.sync(&source);
    assert!(repeated.status.success(), "{}", stderr(&repeated));
    assert!(!stderr(&repeated).contains("signing key"));

    let before = state_snapshot(&fixture.temp.path().join("state"));
    pack(&saved, &source);
    let refused = fixture.sync(&source);
    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains(&format!(
            "Signing key {old} for bucket community was rotated away"
        )),
        "{}",
        stderr(&refused)
    );
    assert_eq!(before, state_snapshot(&fixture.temp.path().join("state")));
}

#[test]
fn revocation_by_the_offline_key_clears_trust_and_retires_the_signer() {
    let fixture = GitFixture::new();
    let keys = fixture.temp.path().join("keys");
    let old = keygen(&keys, "author");
    let offline = keygen(&keys, "offline");
    let next = keygen(&keys, "next");
    let catalog = prepare_catalog(fixture.temp.path(), "community");
    let resign = run(&[
        "--json",
        "catalog",
        "sign",
        path_text(&catalog),
        "--key",
        path_text(&keys.join("author.key")),
        "--revocation-public-key",
        path_text(&keys.join("offline.pub")),
    ]);
    assert!(resign.status.success(), "{}", stderr(&resign));
    copy_tree(&catalog, &fixture.repo);
    commit_all(&fixture.repo, "first");

    let first = fixture.sync();
    assert!(first.status.success(), "{}", stderr(&first));
    assert!(stderr(&first).contains(&format!("Pinned revocation key {offline}")));
    let entry = listed_entry_git(&fixture);
    assert_eq!(entry["revocation_key_id"], offline);

    fixture.run(&["--json", "bucket", "trust-modules", "community", "--yes"]);
    let saved = fixture.temp.path().join("old-catalog");
    zip_catalog::copy_catalog(&fixture.repo, &saved);

    revoke(
        &catalog,
        &keys.join("offline.key"),
        &old,
        &keys.join("next.pub"),
    );
    sign(&catalog, &keys.join("next.key"));
    copy_into(&catalog, &fixture.repo);
    commit_all(&fixture.repo, "revocation");

    let revoked = fixture.sync();
    assert!(revoked.status.success(), "{}", stderr(&revoked));
    assert!(
        stderr(&revoked).contains(&format!("signing key revoked: {old} replaced by {next}")),
        "{}",
        stderr(&revoked)
    );
    let stored = registration(&fixture.state);
    assert_eq!(stored["git"]["signer"]["key_id"], next);
    assert_eq!(
        stored["git"]["signer"]["revoked_signers"],
        serde_json::json!([old])
    );
    assert!(stored.get("module_trust").is_none());
    let entry = listed_entry_git(&fixture);
    assert_eq!(entry["signer_key_id"], next);
    assert_eq!(entry["revoked_signers"], serde_json::json!([old]));
    assert_eq!(entry["module_trusted"], false);

    let again = fixture.sync();
    assert!(again.status.success(), "{}", stderr(&again));
    assert!(!stderr(&again).contains("signing key"));

    let before = fixture.snapshot();
    copy_into(&saved, &fixture.repo);
    commit_all(&fixture.repo, "old signer");
    let refused = fixture.sync();
    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains(&format!(
            "Signing key {old} for bucket community was revoked"
        )),
        "{}",
        stderr(&refused)
    );
    assert_eq!(before, fixture.snapshot());
}

#[test]
fn a_new_user_registers_a_rotated_catalog_and_the_output_names_the_ignored_statement() {
    let fixture = Fixture::new();
    let temp = fixture.temp.path();
    let keys = temp.join("keys");
    keygen(&keys, "author");
    let next = keygen(&keys, "next");
    let catalog = signed_catalog(temp, "community", &keys, "author", &[]);
    rotate(&catalog, &keys.join("author.key"), &keys.join("next.pub"));
    sign(&catalog, &keys.join("next.key"));
    let source = pack_catalog(&catalog, &temp.join("community.zip"));

    let first = fixture.sync(&source);
    assert!(first.status.success(), "{}", stderr(&first));
    assert!(
        stderr(&first).contains(&format!("Pinned signer key {next}")),
        "{}",
        stderr(&first)
    );
    assert!(
        stderr(&first).contains(
            "Ignored published key change statements (rotation) for bucket community; the first sync pins the signer from catalog.pub and applies no statement."
        ),
        "{}",
        stderr(&first)
    );
    let stored = registration(&fixture.temp.path().join("state"));
    assert_eq!(stored["zip"]["signer"]["key_id"], next);
}
