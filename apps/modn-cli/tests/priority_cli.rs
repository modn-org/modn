//! `modn profile priority` and `modn profile apply`: rank changes and the game
//! files that follow them.

#[path = "support/profile_toggle_fixture.rs"]
mod profile_toggle_fixture;

use profile_toggle_fixture::{ToggleFixture, SHARED};
use serde_json::Value;
use std::fs;

const FIRST: &[u8] = b"first";
const SECOND: &[u8] = b"second";

fn package_id(report: &Value) -> String {
    report["id"].as_str().expect("package id").to_owned()
}

fn ranks(list: &Value) -> Vec<(String, u64)> {
    list["profiles"]
        .as_array()
        .expect("profiles")
        .iter()
        .find(|profile| profile["name"] == "default")
        .expect("default profile")["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .map(|entry| {
            (
                entry["package"].as_str().expect("package").to_owned(),
                entry["rank"].as_u64().expect("rank"),
            )
        })
        .collect()
}

fn rank_of(list: &Value, package: &str) -> u64 {
    ranks(list)
        .into_iter()
        .find(|(entry, _)| entry == package)
        .expect("membership")
        .1
}

fn conflict(report: &Value) -> &Value {
    &report["conflicts"].as_array().expect("conflicts")[0]
}

#[test]
fn priority_apply_rank_one_makes_the_lower_priority_mod_active() {
    let fixture = ToggleFixture::new();
    let first = package_id(&fixture.install("first"));
    let second = package_id(&fixture.install("second"));
    assert_eq!(fixture.shared_bytes(), SECOND);

    let report = fixture.json(&[
        "profile", "priority", &first, "--rank", "1", "--apply", "--yes",
    ]);

    assert_eq!(report["profile"], "default");
    assert_eq!(report["package"], first);
    assert_eq!(report["rank"], 1);
    assert_eq!(report["applied"], true);
    assert_eq!(report["dry_run"], false);
    let change = &report["changes"][0];
    assert_eq!(change["path"], SHARED);
    assert_eq!(change["action"], "replace");
    assert_eq!(change["from"], second);
    assert_eq!(change["to"], first);
    assert_eq!(fixture.shared_bytes(), FIRST);
    let conflicts = fixture.json(&["conflicts"]);
    assert_eq!(conflict(&conflicts)["consistent"], true);
    assert_eq!(conflict(&conflicts)["winner"], first);
}

#[test]
fn priority_without_apply_keeps_files_then_apply_repairs() {
    let fixture = ToggleFixture::new();
    let first = package_id(&fixture.install("first"));
    let second = package_id(&fixture.install("second"));

    let report = fixture.json(&["profile", "priority", &first, "--rank", "1"]);

    assert_eq!(report["applied"], false);
    assert_eq!(report["dry_run"], false);
    assert_eq!(report["changes"].as_array().expect("changes").len(), 0);
    assert_eq!(fixture.shared_bytes(), SECOND);
    let conflicts = fixture.json(&["conflicts"]);
    assert_eq!(conflict(&conflicts)["consistent"], false);
    assert_eq!(conflict(&conflicts)["winner"], second);

    let applied = fixture.json(&["profile", "apply", "--yes"]);

    assert_eq!(applied["changes"][0]["to"], first);
    assert_eq!(fixture.shared_bytes(), FIRST);
    let conflicts = fixture.json(&["conflicts"]);
    assert_eq!(conflict(&conflicts)["consistent"], true);
    assert_eq!(conflict(&conflicts)["winner"], first);
}

#[test]
fn priority_before_and_after_forms_move_packages() {
    let fixture = ToggleFixture::new();
    let first = package_id(&fixture.install("first"));
    let second = package_id(&fixture.install("second"));

    let after = fixture.json(&["profile", "priority", &first, "--after", &second]);

    assert_eq!(after["rank"], 2);
    assert_eq!(rank_of(&fixture.profile_list(), &first), 2);
    assert_eq!(rank_of(&fixture.profile_list(), &second), 1);

    let before = fixture.json(&["profile", "priority", &first, "--before", &second]);

    assert_eq!(before["rank"], 1);
    assert_eq!(rank_of(&fixture.profile_list(), &first), 1);
    assert_eq!(rank_of(&fixture.profile_list(), &second), 2);
    assert_eq!(fixture.shared_bytes(), SECOND);
}

#[test]
fn priority_and_apply_dry_run_write_nothing() {
    let fixture = ToggleFixture::new();
    let first = package_id(&fixture.install("first"));
    fixture.install("second");
    let tree = fixture.tree();
    let list = fixture.profile_list();

    let priority = fixture.json(&["profile", "priority", &first, "--rank", "1", "--dry-run"]);

    assert_eq!(priority["dry_run"], true);
    assert_eq!(priority["applied"], false);
    assert_eq!(priority["rank"], 1);
    assert_eq!(priority["changes"].as_array().expect("changes").len(), 1);
    assert_eq!(fixture.tree(), tree);
    assert_eq!(fixture.profile_list(), list);

    let apply = fixture.json(&["profile", "apply", "--dry-run"]);

    assert_eq!(apply["dry_run"], true);
    assert_eq!(apply["changes"].as_array().expect("changes").len(), 0);
    assert_eq!(fixture.tree(), tree);
}

#[test]
fn priority_apply_without_yes_is_refused() {
    let fixture = ToggleFixture::new();
    let first = package_id(&fixture.install("first"));
    fixture.install("second");
    let tree = fixture.tree();
    let list = fixture.profile_list();

    let (code, _) = fixture.error(&["profile", "priority", &first, "--rank", "1", "--apply"]);

    assert_eq!(code, "APPROVAL_REQUIRED");
    assert_eq!(fixture.tree(), tree);
    assert_eq!(fixture.profile_list(), list);
}

#[test]
fn priority_apply_missing_cache_refuses_without_change() {
    let fixture = ToggleFixture::new();
    let first = package_id(&fixture.install("first"));
    fixture.install("second");
    fs::remove_dir_all(fixture.cache()).expect("remove cache");
    let tree = fixture.tree();
    let list = fixture.profile_list();

    let (code, message) = fixture.error(&[
        "profile", "priority", &first, "--rank", "1", "--apply", "--yes",
    ]);

    assert_eq!(code, "PROFILE_ARCHIVE_MISSING");
    assert!(message.contains("modn cache show"), "{message}");
    assert_eq!(fixture.tree(), tree);
    assert_eq!(fixture.profile_list(), list);
}

#[test]
fn profile_apply_reports_already_consistent() {
    let fixture = ToggleFixture::new();
    fixture.install("first");
    let tree = fixture.tree();

    let human = fixture.run(&["profile", "apply", "--yes"]);

    assert!(
        human.status.success(),
        "{}",
        String::from_utf8_lossy(&human.stderr)
    );
    let text = String::from_utf8(human.stdout).expect("UTF-8 output");
    assert!(text.contains("already consistent"), "{text}");
    let report = fixture.json(&["profile", "apply", "--yes"]);
    assert_eq!(report["dry_run"], false);
    assert_eq!(report["changes"].as_array().expect("changes").len(), 0);
    assert_eq!(fixture.tree(), tree);
}

#[test]
fn profile_apply_recover_rolls_back_an_interrupted_apply() {
    let fixture = ToggleFixture::new();
    let first = package_id(&fixture.install("first"));
    let second = package_id(&fixture.install("second"));
    let id = "txn-apply-crashed";
    let game = fixture.game().to_string_lossy().into_owned();
    let shared = fixture.game().join(SHARED);
    let temp = shared
        .parent()
        .expect("destination parent")
        .join(format!(".modn-txn-{id}-0.tmp"));
    let restore = shared
        .parent()
        .expect("destination parent")
        .join(format!(".modn-txn-{id}-0.restore"));
    let connection = fixture.database();
    connection
        .execute(
            "INSERT INTO pending_transactions
             (id, action, status, game_path, package_id, archive_blake3)
             VALUES (?1, 'profile_apply', 'pending', ?2, '', NULL)",
            rusqlite::params![id, game],
        )
        .expect("pending transaction");
    connection
        .execute(
            "INSERT INTO pending_files
             (transaction_id, ord, root, relative_path, blake3, bytes)
             VALUES (?1, 0, 'game', ?2, ?3, ?4)",
            rusqlite::params![
                id,
                SHARED,
                blake3::hash(SECOND).to_hex().to_string(),
                SECOND.len() as i64
            ],
        )
        .expect("pending file");
    connection
        .execute(
            "INSERT INTO pending_restores
             (transaction_id, ord, package_id, archive_entry, blake3, bytes)
             VALUES (?1, 0, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                id,
                first,
                SHARED,
                blake3::hash(FIRST).to_hex().to_string(),
                FIRST.len() as i64
            ],
        )
        .expect("pending restore");
    drop(connection);
    fs::write(&temp, SECOND).expect("old winner witness");
    fs::write(&restore, FIRST).expect("restored witness");
    fs::write(&shared, FIRST).expect("interrupted placement");

    let recovered = fixture.json(&["recover", "--yes"]);

    assert_eq!(recovered["recovered"], 1);
    assert_eq!(fixture.shared_bytes(), SECOND);
    assert_eq!(rank_of(&fixture.profile_list(), &second), 1);
    assert!(!temp.exists());
    assert!(!restore.exists());
}
