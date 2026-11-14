//! Catalog key rotation and offline revocation at the CLI.
#![allow(
    clippy::too_many_lines,
    reason = "behavior tests read as one scenario each"
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

use catalog_authoring::{
    commit_all, copy_tree, init_repo, path_text, prepare_catalog, run, snapshot, stderr,
};
use catalog_mirrors::{keygen, pack_catalog, signed_catalog};
use catalog_rotation::{forge_revocation, forge_rotation, public_text, revoke, rotate, sign};
use modn_core::buckets::{revocation_statement, rotation_statement};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;
use zip_catalog::pack;
use zip_catalog_fixture::Fixture;

fn registration(state: &Path) -> Value {
    serde_json::from_slice(
        &fs::read(state.join("buckets/registrations/community.json")).expect("registration"),
    )
    .expect("registration JSON")
}

fn published_catalog(state: &Path) -> PathBuf {
    let value = registration(state);
    PathBuf::from(value["path"].as_str().expect("published path"))
}

fn state_snapshot(state: &Path) -> (Vec<u8>, Vec<(String, Vec<u8>)>) {
    (
        fs::read(state.join("buckets/registrations/community.json")).expect("registration bytes"),
        snapshot(&published_catalog(state)),
    )
}

fn entry<'a>(entries: &'a [Value], name: &str) -> &'a Value {
    entries
        .iter()
        .find(|entry| entry["name"] == name)
        .expect("registered bucket")
}

fn listed_entry(fixture: &Fixture) -> Value {
    let output = fixture.success(&["--json", "bucket", "list"]);
    let entries: Vec<Value> = serde_json::from_str(&output).expect("bucket list JSON");
    entry(&entries, "community").clone()
}

fn copy_into(source: &Path, target: &Path) {
    for entry in fs::read_dir(target).expect("target entries") {
        let entry = entry.expect("target entry");
        if entry.file_name() == ".git" {
            continue;
        }
        if entry.file_type().expect("target type").is_dir() {
            fs::remove_dir_all(entry.path()).expect("remove old directory");
        } else {
            fs::remove_file(entry.path()).expect("remove old file");
        }
    }
    copy_tree(source, target);
}

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

struct GitFixture {
    temp: TempDir,
    state: PathBuf,
    data: PathBuf,
    repo: PathBuf,
}

impl GitFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("fixture");
        let data = temp.path().join("data");
        fs::create_dir_all(&data).expect("data");
        main_bucket::seed(&data);
        let repo = init_repo(temp.path());
        let state = temp.path().join("state");
        Self {
            temp,
            state,
            data,
            repo,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(main_bucket::modn_binary())
            .env("LOCALAPPDATA", &self.data)
            .env(
                "MODN_TEST_MAIN_BUCKET_PUBLIC_KEY",
                main_bucket::test_public_key(),
            )
            .env_remove("MODN_GAME_MODULES_DIR")
            .args(["--state-dir", path_text(&self.state)])
            .args(args)
            .output()
            .expect("run fixture command")
    }

    fn sync(&self) -> Output {
        self.run(&[
            "--json",
            "bucket",
            "sync",
            "community",
            "--source",
            path_text(&self.repo),
            "--revision",
            "HEAD",
        ])
    }

    fn snapshot(&self) -> (Vec<u8>, Vec<(String, Vec<u8>)>) {
        state_snapshot(&self.state)
    }
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

fn listed_entry_git(fixture: &GitFixture) -> Value {
    let output = fixture.run(&["--json", "bucket", "list"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let entries: Vec<Value> = serde_json::from_slice(&output.stdout).expect("bucket list JSON");
    entry(&entries, "community").clone()
}

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
