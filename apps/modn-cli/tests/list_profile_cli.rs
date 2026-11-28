#![allow(
    dead_code,
    reason = "the shared profile fixture exposes helpers for other test crates"
)]

//! `modn list` and `modn doctor` profile output.
//!
//! Every test uses a disposable game installation, manager state and archive
//! cache. The list keeps its array shape and every earlier key; the new keys
//! describe the active profile. Doctor reports profile health without writing.

#[path = "support/profile_fixture.rs"]
mod profile_fixture;

use profile_fixture::ProfileFixture;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

const SHARED: &str = "archive/pc/mod/shared.archive";

fn install(fixture: &ProfileFixture, name: &str, files: &[(&str, &[u8])]) -> Value {
    let archive = fixture.state().join(format!("{name}.zip"));
    let file = File::create(&archive).expect("archive");
    let mut zip = zip::ZipWriter::new(file);
    for (entry, bytes) in files {
        zip.start_file(*entry, SimpleFileOptions::default())
            .expect("archive entry");
        zip.write_all(bytes).expect("archive payload");
    }
    zip.finish().expect("archive finish");
    let report = fixture.json(&[
        "install",
        archive.to_str().expect("archive path"),
        "--path",
        fixture.game().to_str().expect("game path"),
        "--yes",
    ]);
    let cache = fixture.state().join("cache");
    fs::create_dir_all(&cache).expect("cache");
    fs::copy(
        &archive,
        cache.join(format!("cyberpunk-2077#{name}#1.0#0000000.zip")),
    )
    .expect("cache entry");
    report
}

fn cached_archive(fixture: &ProfileFixture, name: &str) -> PathBuf {
    fixture
        .state()
        .join("cache")
        .join(format!("cyberpunk-2077#{name}#1.0#0000000.zip"))
}

fn package_id(report: &Value) -> String {
    report["id"].as_str().expect("package id").to_owned()
}

fn entries(list: &Value) -> &Vec<Value> {
    list.as_array()
        .expect("list JSON stays an array of packages")
}

fn entry<'a>(list: &'a Value, id: &str) -> &'a Value {
    entries(list)
        .iter()
        .find(|entry| entry["id"] == id)
        .expect("listed package")
}

fn keys(value: &Value) -> Vec<&str> {
    let object = value.as_object().expect("JSON object");
    let mut keys = object.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    keys
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    collect(root, root, &mut files);
    files
}

fn collect(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
    for entry in fs::read_dir(directory).expect("directory entries") {
        let entry = entry.expect("directory entry");
        let path = entry.path();
        if entry.file_type().expect("entry type").is_dir() {
            collect(root, &path, files);
        } else {
            let relative = path.strip_prefix(root).expect("relative path").to_owned();
            files.insert(relative, fs::read(&path).expect("file bytes"));
        }
    }
}

fn issue_kinds(doctor: &Value) -> Vec<String> {
    doctor["profile"]["issues"]
        .as_array()
        .expect("profile issues")
        .iter()
        .map(|issue| issue["kind"].as_str().expect("issue kind").to_owned())
        .collect()
}

#[test]
fn list_json_keeps_previous_keys_and_adds_profile_fields() {
    let fixture = ProfileFixture::new();
    let alpha = package_id(&install(
        &fixture,
        "alpha",
        &[("archive/pc/mod/alpha.archive", b"a")],
    ));

    let list = fixture.json(&["list"]);

    let entry = entry(&list, &alpha);
    let keys = keys(entry);
    for key in [
        "archive_blake3",
        "catalog",
        "directories",
        "files",
        "game_installation",
        "id",
        "pinned",
    ] {
        assert!(
            keys.contains(&key),
            "the previous key {key} is missing: {keys:?}"
        );
    }
    assert_eq!(entry["profile"], "default");
    assert_eq!(entry["rank"], 1);
    assert_eq!(entry["enabled"], true);
    assert_eq!(entry["shadowed_files"], 0);
    assert_eq!(entry["priority"], 0);
}

#[test]
fn list_json_ranks_follow_the_profile_priority_order() {
    let fixture = ProfileFixture::new();
    let alpha = package_id(&install(
        &fixture,
        "alpha",
        &[("archive/pc/mod/alpha.archive", b"a")],
    ));
    let beta = package_id(&install(
        &fixture,
        "beta",
        &[("archive/pc/mod/beta.archive", b"b")],
    ));

    let list = fixture.json(&["list"]);
    assert_eq!(entry(&list, &beta)["rank"], 1);
    assert_eq!(entry(&list, &alpha)["rank"], 2);

    fixture.json(&["profile", "priority", &alpha, "--rank", "1"]);

    let list = fixture.json(&["list"]);
    assert_eq!(entry(&list, &alpha)["rank"], 1);
    assert_eq!(entry(&list, &beta)["rank"], 2);
    assert_eq!(entry(&list, &alpha)["priority"], 1);
}

#[test]
fn list_json_counts_the_shadowed_files_of_overlapping_packages() {
    let fixture = ProfileFixture::new();
    let first = package_id(&install(&fixture, "first", &[(SHARED, b"first")]));
    let second = package_id(&install(&fixture, "second", &[(SHARED, b"second")]));

    let list = fixture.json(&["list"]);

    assert_eq!(entry(&list, &second)["rank"], 1);
    assert_eq!(entry(&list, &second)["shadowed_files"], 0);
    assert_eq!(entry(&list, &first)["rank"], 2);
    assert_eq!(entry(&list, &first)["shadowed_files"], 1);
}

#[test]
fn list_human_shows_the_profile_header_and_rank_markers() {
    let fixture = ProfileFixture::new();
    let alpha = package_id(&install(
        &fixture,
        "alpha",
        &[("archive/pc/mod/alpha.archive", b"a")],
    ));
    let beta = package_id(&install(
        &fixture,
        "beta",
        &[("archive/pc/mod/beta.archive", b"b")],
    ));

    let output = fixture.run(&["list"]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).expect("utf-8 output");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "Profile: default");
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with(&format!("#1 {beta}"))),
        "{text}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with(&format!("#2 {alpha}"))),
        "{text}"
    );
}

#[test]
fn list_human_marks_shadowed_and_disabled_packages() {
    let fixture = ProfileFixture::new();
    let first = package_id(&install(&fixture, "first", &[(SHARED, b"first")]));
    install(&fixture, "second", &[(SHARED, b"second")]);
    fixture.json(&["profile", "disable", &first, "--yes"]);

    let text = String::from_utf8(fixture.run(&["list"]).stdout).expect("utf-8 output");

    assert!(text.contains("Profile: default"), "{text}");
    assert!(
        text.lines()
            .any(|line| line.starts_with(&format!("disabled {first}"))),
        "{text}"
    );
}

#[test]
fn doctor_json_reports_a_consistent_profile_after_installs() {
    let fixture = ProfileFixture::new();
    install(&fixture, "alpha", &[("archive/pc/mod/alpha.archive", b"a")]);
    install(&fixture, "beta", &[("archive/pc/mod/beta.archive", b"b")]);

    let doctor = fixture.json(&["doctor"]);

    let profile = &doctor["profile"];
    assert_eq!(
        keys(profile),
        [
            "active",
            "consistent",
            "issues",
            "name",
            "overlapping_files",
            "packages"
        ]
    );
    assert_eq!(profile["name"], "default");
    assert_eq!(profile["active"], true);
    assert_eq!(profile["packages"], 2);
    assert_eq!(profile["overlapping_files"], 0);
    assert_eq!(profile["consistent"], true);
    assert_eq!(profile["issues"], serde_json::json!([]));
}

#[test]
fn doctor_json_reports_a_winner_mismatch_after_a_priority_change() {
    let fixture = ProfileFixture::new();
    let first = package_id(&install(&fixture, "first", &[(SHARED, b"first")]));
    install(&fixture, "second", &[(SHARED, b"second")]);
    fixture.json(&["profile", "priority", &first, "--rank", "1"]);

    let doctor = fixture.json(&["doctor"]);

    assert_eq!(doctor["profile"]["consistent"], false);
    assert_eq!(issue_kinds(&doctor), ["winner_mismatch"]);
    let issue = &doctor["profile"]["issues"][0];
    assert_eq!(keys(issue), ["advice", "detail", "kind", "package", "path"]);
    assert_eq!(issue["path"], SHARED);
    assert_eq!(issue["package"], first);
    assert_eq!(issue["advice"], "modn profile apply --yes");
}

#[test]
fn doctor_json_reports_a_deleted_shadowed_archive() {
    let fixture = ProfileFixture::new();
    let first = package_id(&install(&fixture, "first", &[(SHARED, b"first")]));
    install(&fixture, "second", &[(SHARED, b"second")]);
    fs::remove_file(cached_archive(&fixture, "first")).expect("remove archive");

    let doctor = fixture.json(&["doctor"]);

    assert_eq!(doctor["profile"]["consistent"], false);
    assert_eq!(issue_kinds(&doctor), ["shadowed_claim_unrecoverable"]);
    let issue = &doctor["profile"]["issues"][0];
    assert_eq!(issue["package"], first);
    assert_eq!(issue["path"], SHARED);
    assert_eq!(issue["advice"], "modn cache show");
}

#[test]
fn doctor_json_reports_another_profiles_missing_archive() {
    let fixture = ProfileFixture::new();
    install(&fixture, "alpha", &[("archive/pc/mod/alpha.archive", b"a")]);
    let beta = package_id(&install(
        &fixture,
        "beta",
        &[("archive/pc/mod/beta.archive", b"b")],
    ));
    fixture.json(&["profile", "create", "spare", "--clone", "default"]);
    fs::remove_file(cached_archive(&fixture, "beta")).expect("remove archive");

    let doctor = fixture.json(&["doctor"]);

    assert_eq!(doctor["profile"]["name"], "default");
    assert_eq!(issue_kinds(&doctor), ["other_profile_archive_missing"]);
    let issue = &doctor["profile"]["issues"][0];
    assert_eq!(issue["package"], beta);
    assert_eq!(issue["path"], "archive/pc/mod/beta.archive");
    assert_eq!(issue["advice"], "modn cache show");
    assert!(issue["detail"].as_str().expect("detail").contains("spare"));
}

#[test]
fn doctor_human_lists_profile_issues_with_advice() {
    let fixture = ProfileFixture::new();
    install(&fixture, "first", &[(SHARED, b"first")]);
    install(&fixture, "second", &[(SHARED, b"second")]);
    fs::remove_file(cached_archive(&fixture, "first")).expect("remove archive");

    let output = fixture.run(&["doctor"]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).expect("utf-8 output");
    assert!(text.contains("Profile: default"), "{text}");
    assert!(
        text.contains("profile issue shadowed_claim_unrecoverable:"),
        "{text}"
    );
    assert!(text.contains("Next: modn cache show"), "{text}");
}

#[test]
fn doctor_leaves_the_game_tree_and_state_byte_identical() {
    let fixture = ProfileFixture::new();
    let first = package_id(&install(&fixture, "first", &[(SHARED, b"first")]));
    install(&fixture, "second", &[(SHARED, b"second")]);
    fixture.json(&["profile", "priority", &first, "--rank", "1"]);
    fs::remove_file(cached_archive(&fixture, "first")).expect("remove archive");
    let game_before = snapshot(fixture.game());
    let state_before = snapshot(fixture.state());

    let doctor = fixture.json(&["doctor"]);
    assert_eq!(doctor["profile"]["consistent"], false);
    let output = fixture.run(&["doctor"]);
    assert!(output.status.success());

    assert_eq!(snapshot(fixture.game()), game_before);
    assert_eq!(snapshot(fixture.state()), state_before);
}
