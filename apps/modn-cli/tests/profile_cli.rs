#[path = "support/profile_fixture.rs"]
mod profile_fixture;

use profile_fixture::ProfileFixture;
use serde_json::{json, Value};

fn packages(report: &Value) -> &[Value] {
    report["profiles"][0]["packages"]
        .as_array()
        .expect("packages")
}

fn refused(fixture: &ProfileFixture, args: &[&str], code: &str) {
    let before = fixture.profile_list();
    let (actual, message) = fixture.error(args);
    assert_eq!(actual, code, "{message}");
    assert!(!message.is_empty(), "the error message is empty");
    assert_eq!(
        fixture.profile_list(),
        before,
        "profile list changed after {code}"
    );
}

#[test]
fn a_registered_installation_lists_one_active_default_profile() {
    let fixture = ProfileFixture::new();
    let report = fixture.profile_list();
    assert_eq!(report["game"], "cyberpunk-2077");
    assert_eq!(report["path"], fixture.game().to_str().expect("game path"));
    let profiles = report["profiles"].as_array().expect("profiles");
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0]["name"], "default");
    assert_eq!(profiles[0]["active"], true);
    assert!(profiles[0].get("created_at").is_some());
    assert_eq!(profiles[0]["packages"], json!([]));
}

#[test]
fn two_installs_list_ranks_with_the_later_install_first() {
    let fixture = ProfileFixture::new();
    let first = fixture.install("first");
    let second = fixture.install("second");
    let report = fixture.profile_list();
    let packages = packages(&report);
    assert_eq!(packages.len(), 2);
    assert_eq!(packages[0]["package"], second["id"]);
    assert_eq!(packages[0]["rank"], 1);
    assert_eq!(packages[0]["priority"], 1);
    assert_eq!(packages[0]["enabled"], true);
    assert_eq!(packages[0]["shadowed_files"], 0);
    assert_eq!(packages[1]["package"], first["id"]);
    assert_eq!(packages[1]["rank"], 2);
    assert_eq!(packages[1]["priority"], 0);
    assert_eq!(packages[1]["enabled"], true);
}

#[test]
fn create_adds_an_inactive_profile() {
    let fixture = ProfileFixture::new();
    let created = fixture.json(&["profile", "create", "spare"]);
    assert_eq!(created["name"], "spare");
    assert_eq!(created["active"], false);
    assert!(created.get("created_at").is_some());
    assert_eq!(created["packages"], json!([]));
    let report = fixture.profile_list();
    let names = report["profiles"]
        .as_array()
        .expect("profiles")
        .iter()
        .map(|profile| profile["name"].as_str().expect("name"))
        .collect::<Vec<_>>();
    assert_eq!(names, ["default", "spare"]);
}

#[test]
fn clone_copies_packages_ranks_and_enabled_flags() {
    let fixture = ProfileFixture::new();
    let first = fixture.install("first");
    let second = fixture.install("second");
    let connection =
        rusqlite::Connection::open(fixture.state().join("state.sqlite")).expect("state database");
    connection
        .execute(
            "UPDATE profile_packages SET enabled=0 WHERE package_id=?1",
            [second["id"].as_str().expect("id")],
        )
        .expect("disable the second package");
    drop(connection);

    let created = fixture.json(&["profile", "create", "spare", "--clone", "default"]);
    let packages = created["packages"].as_array().expect("packages");
    assert_eq!(packages.len(), 2);
    assert_eq!(packages[0]["package"], second["id"]);
    assert_eq!(packages[0]["rank"], 1);
    assert_eq!(packages[0]["enabled"], false);
    assert_eq!(packages[1]["package"], first["id"]);
    assert_eq!(packages[1]["rank"], 2);
    assert_eq!(packages[1]["enabled"], true);
}

#[test]
fn create_refuses_a_duplicate_name_without_changes() {
    let fixture = ProfileFixture::new();
    fixture.json(&["profile", "create", "spare"]);
    refused(&fixture, &["profile", "create", "spare"], "PROFILE_EXISTS");
}

#[test]
fn delete_removes_an_inactive_profile() {
    let fixture = ProfileFixture::new();
    fixture.json(&["profile", "create", "spare"]);
    let deleted = fixture.json(&["profile", "delete", "spare", "--yes"]);
    assert_eq!(deleted, json!({"deleted": "spare"}));
    let report = fixture.profile_list();
    let profiles = report["profiles"].as_array().expect("profiles");
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0]["name"], "default");
}

#[test]
fn delete_refuses_an_unknown_profile_without_changes() {
    let fixture = ProfileFixture::new();
    refused(
        &fixture,
        &["profile", "delete", "missing", "--yes"],
        "PROFILE_NOT_FOUND",
    );
}

#[test]
fn clone_refuses_an_unknown_source_without_changes() {
    let fixture = ProfileFixture::new();
    refused(
        &fixture,
        &["profile", "create", "spare", "--clone", "missing"],
        "PROFILE_NOT_FOUND",
    );
}

#[test]
fn delete_refuses_the_active_profile_without_changes() {
    let fixture = ProfileFixture::new();
    fixture.json(&["profile", "create", "spare"]);
    refused(
        &fixture,
        &["profile", "delete", "default", "--yes"],
        "PROFILE_ACTIVE",
    );
}

#[test]
fn deleting_the_only_profile_reports_profile_active_because_the_active_check_comes_first() {
    let fixture = ProfileFixture::new();
    refused(
        &fixture,
        &["profile", "delete", "default", "--yes"],
        "PROFILE_ACTIVE",
    );
}

#[test]
fn delete_without_yes_reports_approval_required_without_changes() {
    let fixture = ProfileFixture::new();
    fixture.json(&["profile", "create", "spare"]);
    refused(
        &fixture,
        &["profile", "delete", "spare"],
        "APPROVAL_REQUIRED",
    );
}

#[test]
fn human_list_marks_the_active_profile() {
    let fixture = ProfileFixture::new();
    let output = fixture.run(&["profile", "list"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).expect("utf-8 output");
    assert!(text.contains("PROFILE"));
    assert!(text.contains("ACTIVE"));
    assert!(text
        .lines()
        .any(|line| line.starts_with("default") && line.contains('*')));
}

#[test]
fn human_create_and_delete_name_the_profile() {
    let fixture = ProfileFixture::new();
    let created = fixture.run(&["profile", "create", "spare"]);
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    assert!(String::from_utf8(created.stdout)
        .expect("utf-8 output")
        .contains("spare"));
    let deleted = fixture.run(&["profile", "delete", "spare", "--yes"]);
    assert!(
        deleted.status.success(),
        "{}",
        String::from_utf8_lossy(&deleted.stderr)
    );
    assert!(String::from_utf8(deleted.stdout)
        .expect("utf-8 output")
        .contains("spare"));
}

#[test]
fn game_and_path_select_the_same_installation() {
    let fixture = ProfileFixture::new();
    let by_game = fixture.profile_list();
    let by_path = fixture.json(&[
        "profile",
        "list",
        "--path",
        fixture.game().to_str().expect("game path"),
    ]);
    assert_eq!(by_game, by_path);
}
