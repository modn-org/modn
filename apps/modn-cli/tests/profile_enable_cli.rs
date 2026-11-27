#[path = "support/profile_toggle_fixture.rs"]
mod profile_toggle_fixture;

use profile_toggle_fixture::{ToggleFixture, SHARED};
use serde_json::Value;
use std::fs;
use std::path::Path;

const FIRST: &[u8] = b"first";
const SECOND: &[u8] = b"second";

fn package_id(report: &Value) -> String {
    report["id"].as_str().expect("package id").to_owned()
}

fn enabled_in(list: &Value, profile: &str, package: &str) -> bool {
    list["profiles"]
        .as_array()
        .expect("profiles")
        .iter()
        .find(|entry| entry["name"] == profile)
        .expect("profile")["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .find(|entry| entry["package"] == package)
        .expect("package")["enabled"]
        .as_bool()
        .expect("enabled")
}

fn digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

#[test]
fn profile_enable_disable_round_trip_restores_the_tree() {
    let fixture = ToggleFixture::new();
    fixture.install("first");
    let second = package_id(&fixture.install("second"));
    let before = fixture.tree();

    let disabled = fixture.json(&["profile", "disable", &second, "--yes"]);
    assert_eq!(disabled["changes"][0]["action"], "restore");
    assert_eq!(fixture.shared_bytes(), FIRST);
    fixture.json(&["profile", "enable", &second, "--yes"]);

    assert_eq!(fixture.shared_bytes(), SECOND);
    assert_eq!(fixture.tree(), before);
}

#[test]
fn profile_disable_winner_writes_the_first_packages_bytes() {
    let fixture = ToggleFixture::new();
    let first = package_id(&fixture.install("first"));
    let second = package_id(&fixture.install("second"));

    let report = fixture.json(&["profile", "disable", &second, "--yes"]);

    assert_eq!(report["profile"], "default");
    assert_eq!(report["package"], second);
    assert_eq!(report["enabled"], false);
    assert_eq!(report["active"], true);
    assert_eq!(report["dry_run"], false);
    let change = &report["changes"][0];
    assert_eq!(change["path"], SHARED);
    assert_eq!(change["action"], "restore");
    assert_eq!(change["from"], second);
    assert_eq!(change["to"], first);
    assert_eq!(fixture.shared_bytes(), FIRST);
    assert!(!enabled_in(&fixture.profile_list(), "default", &second));
}

#[test]
fn profile_disable_dry_run_json_and_human_change_nothing() {
    let fixture = ToggleFixture::new();
    fixture.install("first");
    let second = package_id(&fixture.install("second"));
    let before = fixture.tree();
    let list = fixture.profile_list();

    let report = fixture.json(&["profile", "disable", &second, "--dry-run"]);
    assert_eq!(report["dry_run"], true);
    assert_eq!(report["changes"].as_array().expect("changes").len(), 1);
    assert_eq!(report["changes"][0]["action"], "restore");
    let human = fixture.run(&["profile", "disable", &second, "--dry-run"]);
    assert!(
        human.status.success(),
        "{}",
        String::from_utf8_lossy(&human.stderr)
    );
    let text = String::from_utf8(human.stdout).expect("UTF-8 output");
    assert!(text.contains("Would change"), "{text}");
    assert!(text.contains("restore"), "{text}");
    assert!(text.contains(SHARED), "{text}");
    assert_eq!(fixture.tree(), before);
    assert_eq!(fixture.profile_list(), list);
}

#[test]
fn profile_disable_missing_cache_refuses_without_change() {
    let fixture = ToggleFixture::new();
    fixture.install("first");
    let second = package_id(&fixture.install("second"));
    fs::remove_dir_all(fixture.cache()).expect("remove cache");
    let before = fixture.tree();
    let list = fixture.profile_list();

    let (code, message) = fixture.error(&["profile", "disable", &second, "--yes"]);

    assert_eq!(code, "PROFILE_ARCHIVE_MISSING");
    assert!(message.contains("modn cache show"), "{message}");
    assert_eq!(fixture.tree(), before);
    assert_eq!(fixture.profile_list(), list);
}

#[test]
fn profile_disable_without_yes_reports_approval_required() {
    let fixture = ToggleFixture::new();
    fixture.install("first");
    let second = package_id(&fixture.install("second"));
    let before = fixture.tree();
    let list = fixture.profile_list();

    let (code, _) = fixture.error(&["profile", "disable", &second]);

    assert_eq!(code, "APPROVAL_REQUIRED");
    assert_eq!(fixture.tree(), before);
    assert_eq!(fixture.profile_list(), list);
}

#[test]
fn profile_disable_of_an_inactive_profile_changes_state_only() {
    let fixture = ToggleFixture::new();
    fixture.install("first");
    let second = package_id(&fixture.install("second"));
    fixture.json(&["profile", "create", "spare", "--clone", "default"]);
    let before = fixture.tree();

    let report = fixture.json(&["profile", "disable", &second, "--profile", "spare"]);

    assert_eq!(report["active"], false);
    assert_eq!(report["changes"].as_array().expect("changes").len(), 0);
    assert_eq!(fixture.shared_bytes(), SECOND);
    assert_eq!(fixture.tree(), before);
    let list = fixture.profile_list();
    assert!(enabled_in(&list, "default", &second));
    assert!(!enabled_in(&list, "spare", &second));
}

#[test]
fn profile_recover_yes_rolls_back_an_interrupted_toggle() {
    let fixture = ToggleFixture::new();
    let first = package_id(&fixture.install("first"));
    let second = package_id(&fixture.install("second"));
    let id = "txn-toggle-crashed";
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
             VALUES (?1, 'profile_toggle', 'pending', ?2, ?3, NULL)",
            rusqlite::params![id, game, second],
        )
        .expect("pending transaction");
    connection
        .execute(
            "INSERT INTO pending_files
             (transaction_id, ord, root, relative_path, blake3, bytes)
             VALUES (?1, 0, 'game', ?2, ?3, ?4)",
            rusqlite::params![id, SHARED, digest(SECOND), SECOND.len() as i64],
        )
        .expect("pending file");
    connection
        .execute(
            "INSERT INTO pending_restores
             (transaction_id, ord, package_id, archive_entry, blake3, bytes)
             VALUES (?1, 0, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, first, SHARED, digest(FIRST), FIRST.len() as i64],
        )
        .expect("pending restore");
    drop(connection);
    fs::write(&temp, SECOND).expect("old winner witness");
    fs::write(&restore, FIRST).expect("restored witness");
    fs::write(&shared, FIRST).expect("interrupted placement");

    let recovered = fixture.json(&["recover", "--yes"]);

    assert_eq!(recovered["recovered"], 1);
    assert_eq!(fixture.shared_bytes(), SECOND);
    assert!(enabled_in(&fixture.profile_list(), "default", &second));
    assert!(!Path::new(&temp).exists());
    assert!(!Path::new(&restore).exists());
}
