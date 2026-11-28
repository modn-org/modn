#[path = "support/profile_fixture.rs"]
mod profile_fixture;

use profile_fixture::ProfileFixture;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

fn keys(value: &Value) -> Vec<&str> {
    let object = value.as_object().expect("JSON object");
    let mut keys = object.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    keys
}

fn install_entry(fixture: &ProfileFixture, name: &str, entry: &str, payload: &[u8]) -> Value {
    let archive = fixture
        .state()
        .parent()
        .expect("fixture root")
        .join(format!("{name}.zip"));
    let file = File::create(&archive).expect("archive");
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file(entry, SimpleFileOptions::default())
        .expect("archive entry");
    zip.write_all(payload).expect("archive payload");
    zip.finish().expect("archive finish");
    fixture.json(&[
        "install",
        archive.to_str().expect("archive path"),
        "--path",
        fixture.game().to_str().expect("game path"),
        "--yes",
    ])
}

fn install_overlap(fixture: &ProfileFixture, name: &str, payload: &[u8]) -> Value {
    install_entry(fixture, name, "archive/pc/mod/shared.archive", payload)
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

#[test]
fn json_reports_the_overlap_and_the_later_install_as_winner() {
    let fixture = ProfileFixture::new();
    let first = install_overlap(&fixture, "first", b"first");
    let second = install_overlap(&fixture, "second", b"second");

    let report = fixture.json(&["conflicts"]);

    assert_eq!(
        keys(&report),
        ["active", "conflicts", "game", "path", "profile"]
    );
    assert_eq!(report["game"], "cyberpunk-2077");
    assert_eq!(report["path"], fixture.game().to_str().expect("game path"));
    assert_eq!(report["profile"], "default");
    assert_eq!(report["active"], true);
    let conflicts = report["conflicts"].as_array().expect("conflicts");
    assert_eq!(conflicts.len(), 1);
    let conflict = &conflicts[0];
    assert_eq!(
        keys(conflict),
        ["claims", "consistent", "path", "root", "winner"]
    );
    assert_eq!(conflict["root"], "game");
    assert_eq!(conflict["path"], "archive/pc/mod/shared.archive");
    assert_eq!(conflict["winner"], second["id"]);
    assert_eq!(conflict["consistent"], true);
    let claims = conflict["claims"].as_array().expect("claims");
    assert_eq!(claims.len(), 2);
    assert_eq!(
        keys(&claims[0]),
        ["blake3", "bytes", "package", "priority", "rank", "winner"]
    );
    assert_eq!(claims[0]["package"], second["id"]);
    assert_eq!(claims[0]["rank"], 1);
    assert_eq!(claims[0]["priority"], 1);
    assert_eq!(claims[0]["winner"], true);
    assert!(claims[0]["blake3"].is_string());
    assert!(claims[0]["bytes"].is_u64());
    assert_eq!(claims[1]["package"], first["id"]);
    assert_eq!(claims[1]["rank"], 2);
    assert_eq!(claims[1]["priority"], 0);
    assert_eq!(claims[1]["winner"], false);
}

#[test]
fn human_output_marks_the_winner_and_counts_overlapping_files() {
    let fixture = ProfileFixture::new();
    let first = install_overlap(&fixture, "first", b"first");
    let second = install_overlap(&fixture, "second", b"second");

    let output = fixture.run(&["conflicts"]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).expect("utf-8 output");
    assert!(text.contains("game/archive/pc/mod/shared.archive"));
    assert!(text.contains(first["id"].as_str().expect("first id")));
    assert!(text.contains(second["id"].as_str().expect("second id")));
    let winner_line = text
        .lines()
        .find(|line| line.contains(second["id"].as_str().expect("second id")))
        .expect("winner line");
    assert!(winner_line.contains("WINNER"));
    assert!(text.contains("1 overlapping files"));
}

#[test]
fn a_cloned_profile_reports_no_stored_winner() {
    let fixture = ProfileFixture::new();
    install_overlap(&fixture, "first", b"first");
    install_overlap(&fixture, "second", b"second");
    fixture.json(&["profile", "create", "spare", "--clone", "default"]);
    let profiles = fixture.profile_list();
    assert!(profiles["profiles"]
        .as_array()
        .expect("profiles")
        .iter()
        .any(|profile| profile["name"] == "spare"));

    let report = fixture.json(&["conflicts", "--profile", "spare"]);

    assert_eq!(report["profile"], "spare");
    assert_eq!(report["active"], false);
    let conflict = &report["conflicts"][0];
    assert!(conflict["winner"].is_null());
    assert_eq!(conflict["consistent"], true);
    assert_eq!(conflict["claims"].as_array().expect("claims").len(), 2);
}

#[test]
fn packages_with_distinct_paths_report_no_conflicts() {
    let fixture = ProfileFixture::new();
    fixture.install("first");
    fixture.install("second");

    let report = fixture.json(&["conflicts"]);

    assert_eq!(report["conflicts"], serde_json::json!([]));
    let output = fixture.run(&["conflicts"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8(output.stdout)
        .expect("utf-8 output")
        .contains("0 overlapping files"));
}

#[test]
fn an_unknown_profile_reports_profile_not_found() {
    let fixture = ProfileFixture::new();

    let (code, message) = fixture.error(&["conflicts", "--profile", "missing"]);

    assert_eq!(code, "PROFILE_NOT_FOUND", "{message}");
}

#[test]
fn path_and_game_select_the_same_installation() {
    let fixture = ProfileFixture::new();
    install_overlap(&fixture, "first", b"first");
    install_overlap(&fixture, "second", b"second");

    let by_game = fixture.json(&["conflicts"]);
    let by_path = fixture.json(&[
        "conflicts",
        "--path",
        fixture.game().to_str().expect("game path"),
    ]);

    assert_eq!(by_game, by_path);
}

#[test]
fn the_command_leaves_the_game_tree_and_state_byte_identical() {
    let fixture = ProfileFixture::new();
    install_overlap(&fixture, "first", b"first");
    install_overlap(&fixture, "second", b"second");
    let game_before = snapshot(fixture.game());
    let state_before = snapshot(fixture.state());

    fixture.json(&["conflicts"]);
    let output = fixture.run(&["conflicts"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(snapshot(fixture.game()), game_before);
    assert_eq!(snapshot(fixture.state()), state_before);
}
