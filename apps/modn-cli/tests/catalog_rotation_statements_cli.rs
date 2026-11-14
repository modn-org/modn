//! Forged statements, revocation-key pinning and author-command refusals.

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

use catalog_authoring::{copy_tree, path_text, prepare_catalog, run, snapshot, stderr};
use catalog_mirrors::{keygen, pack_catalog, signed_catalog};
use catalog_rotation::{forge_revocation, forge_rotation, public_text, rotate, sign};
use catalog_rotation_state::{listed_entry, state_snapshot};
use modn_core::buckets::{revocation_statement, rotation_statement};
use std::fs;
use std::path::Path;
use zip_catalog::pack;
use zip_catalog_fixture::Fixture;

#[test]
#[allow(
    clippy::type_complexity,
    reason = "each case names one statement and one refusal"
)]
fn forged_and_tampered_statements_are_refused_without_changes() {
    let fixture = Fixture::new();
    let temp = fixture.temp.path();
    let state = temp.join("state");
    let keys = temp.join("keys");
    let old = keygen(&keys, "author");
    keygen(&keys, "offline");
    keygen(&keys, "next");
    keygen(&keys, "unrelated");
    let catalog = signed_catalog(temp, "community", &keys, "author", &[]);
    let with_offline = run(&[
        "--json",
        "catalog",
        "sign",
        path_text(&catalog),
        "--key",
        path_text(&keys.join("author.key")),
        "--revocation-public-key",
        path_text(&keys.join("offline.pub")),
    ]);
    assert!(with_offline.status.success(), "{}", stderr(&with_offline));
    let source = pack_catalog(&catalog, &temp.join("community.zip"));
    let first = fixture.sync(&source);
    assert!(first.status.success(), "{}", stderr(&first));
    let base = temp.join("base");
    copy_tree(&catalog, &base);
    let next_text = public_text(&keys.join("next.pub"));

    let cases: Vec<(&str, Box<dyn Fn(&Path)>, &str)> = vec![
        (
            "unrelated rotation",
            Box::new(|case: &Path| {
                forge_rotation(
                    case,
                    &rotation_statement(&old, &next_text).expect("rotation"),
                    &keys.join("unrelated.key"),
                );
            }),
            "is not signed by the pinned signing key",
        ),
        (
            "revocation key rotation",
            Box::new(|case: &Path| {
                forge_rotation(
                    case,
                    &rotation_statement(&old, &next_text).expect("rotation"),
                    &keys.join("offline.key"),
                );
            }),
            "is not signed by the pinned signing key",
        ),
        (
            "signer revocation",
            Box::new(|case: &Path| {
                forge_revocation(
                    case,
                    &revocation_statement(&old, &next_text).expect("revocation"),
                    &keys.join("author.key"),
                );
            }),
            "is not signed by the pinned revocation key",
        ),
        (
            "wrong from",
            Box::new(|case: &Path| {
                forge_rotation(
                    case,
                    &rotation_statement("0000000000000000", &next_text).expect("rotation"),
                    &keys.join("author.key"),
                );
            }),
            "but the pinned signing key is",
        ),
        (
            "to does not match catalog.pub",
            Box::new(|case: &Path| {
                forge_rotation(
                    case,
                    &rotation_statement(&old, &next_text).expect("rotation"),
                    &keys.join("author.key"),
                );
            }),
            "but this revision publishes key",
        ),
        (
            "both statements",
            Box::new(|case: &Path| {
                forge_rotation(
                    case,
                    &rotation_statement(&old, &next_text).expect("rotation"),
                    &keys.join("author.key"),
                );
                forge_revocation(
                    case,
                    &revocation_statement(&old, &next_text).expect("revocation"),
                    &keys.join("offline.key"),
                );
            }),
            "both rotation.json and revocation.json",
        ),
        (
            "tampered body",
            Box::new(|case: &Path| {
                forge_rotation(
                    case,
                    &rotation_statement(&old, &next_text).expect("rotation"),
                    &keys.join("author.key"),
                );
                let path = case.join("rotation.json");
                let mut bytes = fs::read(&path).expect("rotation body");
                bytes.push(b' ');
                fs::write(&path, bytes).expect("tampered body");
            }),
            "rotation",
        ),
    ];
    for (label, prepare, expected) in cases {
        let case = temp.join(format!("case-{label}").replace(' ', "-"));
        copy_tree(&base, &case);
        prepare(&case);
        sign(&case, &keys.join("author.key"));
        pack(&case, &source);
        let before = state_snapshot(&state);
        let output = fixture.run(&["--json", "bucket", "sync", "community"]);
        assert!(!output.status.success(), "{label} was accepted");
        assert!(
            stderr(&output).contains(expected),
            "{label}: expected {expected:?} in {}",
            stderr(&output)
        );
        assert_eq!(before, state_snapshot(&state), "{label} changed state");
    }
}

#[test]
fn revocation_key_pinning_rules_are_enforced() {
    let fixture = Fixture::new();
    let temp = fixture.temp.path();
    let state = temp.join("state");
    let keys = temp.join("keys");
    keygen(&keys, "author");
    keygen(&keys, "offline");
    keygen(&keys, "next");
    let catalog = signed_catalog(temp, "community", &keys, "author", &[]);
    let source = pack_catalog(&catalog, &temp.join("community.zip"));
    let first = fixture.sync(&source);
    assert!(first.status.success(), "{}", stderr(&first));
    let base = temp.join("base");
    copy_tree(&catalog, &base);

    let add = temp.join("add");
    copy_tree(&base, &add);
    fs::copy(keys.join("offline.pub"), add.join("revocation.pub")).expect("revocation key");
    sign(&add, &keys.join("author.key"));
    pack(&add, &source);
    let before = state_snapshot(&state);
    let refused = fixture.run(&["--json", "bucket", "sync", "community"]);
    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains("must be present at the first sync"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(before, state_snapshot(&state));

    let pinned = Fixture::new();
    let temp = pinned.temp.path();
    let state = temp.join("state");
    let keys = temp.join("keys");
    keygen(&keys, "author");
    keygen(&keys, "offline");
    keygen(&keys, "next");
    let catalog = signed_catalog(temp, "community", &keys, "author", &[]);
    let with_offline = run(&[
        "--json",
        "catalog",
        "sign",
        path_text(&catalog),
        "--key",
        path_text(&keys.join("author.key")),
        "--revocation-public-key",
        path_text(&keys.join("offline.pub")),
    ]);
    assert!(with_offline.status.success(), "{}", stderr(&with_offline));
    let source = pack_catalog(&catalog, &temp.join("community.zip"));
    assert!(pinned.sync(&source).status.success());
    let base = temp.join("base");
    copy_tree(&catalog, &base);

    for (label, prepare, expected) in [
        (
            "drop",
            Box::new(|case: &Path| {
                fs::remove_file(case.join("revocation.pub")).expect("remove revocation key");
            }) as Box<dyn Fn(&Path)>,
            "removes revocation.pub",
        ),
        (
            "change",
            Box::new(|case: &Path| {
                fs::copy(keys.join("next.pub"), case.join("revocation.pub"))
                    .expect("changed revocation key");
            }) as Box<dyn Fn(&Path)>,
            "Revocation key changed",
        ),
    ] {
        let case = temp.join(label);
        copy_tree(&base, &case);
        prepare(&case);
        sign(&case, &keys.join("author.key"));
        pack(&case, &source);
        let before = state_snapshot(&state);
        let output = pinned.run(&["--json", "bucket", "sync", "community"]);
        assert!(!output.status.success(), "{label} accepted");
        assert!(
            stderr(&output).contains(expected),
            "{label}: expected {expected:?} in {}",
            stderr(&output)
        );
        assert_eq!(before, state_snapshot(&state));
    }
}

#[test]
fn rotation_works_during_modn_update() {
    let fixture = Fixture::new();
    let temp = fixture.temp.path();
    let keys = temp.join("keys");
    let old = keygen(&keys, "author");
    let next = keygen(&keys, "next");
    let catalog = signed_catalog(temp, "community", &keys, "author", &[]);
    let source = pack_catalog(&catalog, &temp.join("community.zip"));
    let first = fixture.sync(&source);
    assert!(first.status.success(), "{}", stderr(&first));

    rotate(&catalog, &keys.join("author.key"), &keys.join("next.pub"));
    sign(&catalog, &keys.join("next.key"));
    pack(&catalog, &source);
    let refreshed = fixture.success(&["update"]);
    assert!(
        refreshed.contains(&format!("signing key rotated: {old} -> {next}")),
        "{refreshed}"
    );
    assert_eq!(listed_entry(&fixture)["signer_key_id"], next.as_str());
}

#[test]
fn author_commands_refuse_a_secret_key_inside_the_catalog_and_write_nothing() {
    let temp = tempfile::tempdir().expect("fixture");
    let keys = temp.path().join("keys");
    keygen(&keys, "author");
    let catalog = prepare_catalog(temp.path(), "community");
    fs::copy(keys.join("author.key"), catalog.join("inside.key")).expect("inner key");
    fs::copy(keys.join("author.pub"), catalog.join("next.pub")).expect("public key");
    let before = snapshot(&catalog);
    for command in [
        vec![
            "catalog",
            "rotate",
            path_text(&catalog),
            "--key",
            path_text(&catalog.join("inside.key")),
            "--new-public-key",
            path_text(&catalog.join("next.pub")),
        ],
        vec![
            "catalog",
            "sign",
            path_text(&catalog),
            "--key",
            path_text(&catalog.join("inside.key")),
            "--revocation-public-key",
            path_text(&catalog.join("next.pub")),
        ],
    ] {
        let output = run(&command);
        assert!(!output.status.success());
        assert!(
            stderr(&output).contains("must live outside the catalog directory"),
            "{}",
            stderr(&output)
        );
        assert_eq!(before, snapshot(&catalog));
    }
    let revoke_output = run(&[
        "catalog",
        "revoke",
        path_text(&catalog),
        "--revocation-key",
        path_text(&catalog.join("inside.key")),
        "--revoked",
        "0123456789ABCDEF",
        "--replacement-public-key",
        path_text(&catalog.join("next.pub")),
    ]);
    assert!(!revoke_output.status.success());
    assert!(
        stderr(&revoke_output).contains("must live outside the catalog directory"),
        "{}",
        stderr(&revoke_output)
    );
    assert_eq!(before, snapshot(&catalog));
}
