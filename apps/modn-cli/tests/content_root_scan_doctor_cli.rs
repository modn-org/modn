//! End-to-end scan and doctor coverage for API 2 content roots.
//!
//! Every command runs the real `modn` binary against a temporary game folder,
//! a temporary appdata base and a temporary state directory. `MODN_APPDATA`
//! and `APPDATA` both point at the temporary base, so no test reads or writes
//! the user's real `%APPDATA%`.

#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/content_root.rs"]
mod support;
#[allow(
    dead_code,
    reason = "shared fixture exposes helpers for different test crates"
)]
#[path = "support/tree.rs"]
mod tree;

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use support::{expected_content_root, Fixture};

fn install_content(fixture: &Fixture) -> PathBuf {
    let (archive, _) = fixture.archive("content-scan", &[("core.lua", b"content")]);
    fixture.ok(&[
        "install",
        archive.to_str().expect("archive"),
        "--yes",
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    fixture.content("Pkg/core.lua")
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("UTF-8 output")
}

fn link_directory(link: &Path, target: &Path) {
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(target, link).expect("junction fixture");
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).expect("symlink fixture");
}

fn diagnostics(report: &Value) -> Vec<&str> {
    report["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .filter_map(|item| item["code"].as_str())
        .collect()
}

#[test]
fn scan_labels_both_roots_and_preserves_game_root_json() {
    let fixture = Fixture::new();
    fixture.register();
    let game_file = fixture.game.join("GameMods/Present.lua");
    fs::create_dir_all(game_file.parent().expect("game mods")).expect("game folder");
    fs::write(&game_file, b"game content").expect("game mod");
    let owned = install_content(&fixture);
    let manual = fixture.content("Manual/Content.lua");
    fs::create_dir_all(manual.parent().expect("manual")).expect("manual folder");
    fs::write(&manual, b"manual content").expect("manual mod");

    let report = fixture.json(&[
        "--json",
        "scan",
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    assert_eq!(
        Path::new(report["content_root"].as_str().expect("content root")),
        expected_content_root(&fixture)
    );
    let mods = report["mods"].as_array().expect("mods");
    let game_entry = mods
        .iter()
        .find(|entry| entry["relative_path"] == "GameMods/Present.lua")
        .expect("game entry");
    assert!(game_entry.get("root").is_none(), "{game_entry}");
    let owned_entry = mods
        .iter()
        .find(|entry| entry["relative_path"] == "Pkg/core.lua")
        .expect("owned content entry");
    assert_eq!(owned_entry["root"], "content");
    assert_eq!(owned_entry["managed"], true);
    assert_eq!(owned_entry["integrity"], "intact");
    let manual_entry = mods
        .iter()
        .find(|entry| entry["relative_path"] == "Manual/Content.lua")
        .expect("manual content entry");
    assert_eq!(manual_entry["root"], "content");
    assert_eq!(manual_entry["managed"], false);

    let owned_files = report["owned_files"].as_array().expect("owned files");
    assert!(owned_files.iter().any(|file| {
        file["root"] == "content"
            && file["relative_path"] == "Pkg/core.lua"
            && file["status"] == "intact"
    }));
    assert!(fs::read(&owned).expect("owned file") == b"content");

    let text = stdout(&fixture.ok(&["scan", "--path", fixture.game.to_str().expect("game")]));
    assert!(text.contains("Content root:"), "{text}");
    assert!(
        text.contains("game-mod: GameMods/Present.lua [unmanaged]"),
        "{text}"
    );
    assert!(
        text.contains("content content-mod: Pkg/core.lua [managed, intact]"),
        "{text}"
    );
    assert!(
        text.contains("content content-mod: Manual/Content.lua [unmanaged]"),
        "{text}"
    );
}

#[test]
fn doctor_reports_a_missing_content_root_and_writes_nothing() {
    let fixture = Fixture::new();
    fixture.register();
    let game_before = tree::snapshot(&fixture.game);
    let appdata_before = tree::snapshot(&fixture.appdata);

    let report = fixture.json(&[
        "--json",
        "doctor",
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    assert!(
        diagnostics(&report).contains(&"CONTENT_ROOT_MISSING"),
        "{report}"
    );
    assert_eq!(
        Path::new(report["scan"]["content_root"].as_str().expect("root")),
        expected_content_root(&fixture)
    );
    let text = stdout(&fixture.ok(&["doctor", "--path", fixture.game.to_str().expect("game")]));
    assert!(text.contains("CONTENT_ROOT_MISSING"), "{text}");
    assert_eq!(tree::snapshot(&fixture.game), game_before);
    assert_eq!(tree::snapshot(&fixture.appdata), appdata_before);
}

#[test]
fn doctor_reports_a_content_root_that_is_not_a_folder() {
    let fixture = Fixture::new();
    fixture.register();
    fs::create_dir_all(fixture.appdata.join("Fixture")).expect("fixture folder");
    fs::write(fixture.appdata.join(support::CONTENT_RELATIVE), b"file").expect("blocking file");

    let report = fixture.json(&[
        "--json",
        "doctor",
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    assert!(
        diagnostics(&report).contains(&"CONTENT_ROOT_NOT_DIRECTORY"),
        "{report}"
    );
}

#[test]
fn doctor_reports_owned_files_missing_from_the_content_root() {
    let fixture = Fixture::new();
    fixture.register();
    install_content(&fixture);
    fs::remove_file(fixture.content("Pkg/core.lua")).expect("remove owned content");

    let scan = fixture.json(&[
        "--json",
        "scan",
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    assert!(scan["owned_files"]
        .as_array()
        .expect("owned")
        .iter()
        .any(|file| {
            file["root"] == "content"
                && file["relative_path"] == "Pkg/core.lua"
                && file["status"] == "missing"
        }));

    let report = fixture.json(&[
        "--json",
        "doctor",
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    let codes = diagnostics(&report);
    assert!(
        codes.contains(&"CONTENT_ROOT_OWNED_FILE_MISSING"),
        "{report}"
    );
    assert!(!codes.contains(&"CONTENT_ROOT_MISSING"), "{report}");
}

#[test]
fn doctor_reports_a_moved_content_root() {
    let fixture = Fixture::new();
    fixture.register();
    install_content(&fixture);
    let parent = fixture.appdata.join("Fixture");
    let moved = fixture.appdata.join("FixtureMoved");
    fs::rename(&parent, &moved).expect("move content root parent");
    link_directory(&parent, &moved);

    let report = fixture.json(&[
        "--json",
        "doctor",
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    assert!(
        diagnostics(&report).contains(&"CONTENT_ROOT_MOVED"),
        "{report}"
    );
    let scan = fixture.json(&[
        "--json",
        "scan",
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    assert!(scan["mods"]
        .as_array()
        .expect("mods")
        .iter()
        .all(|entry| entry.get("root").is_none()));
}

#[test]
fn doctor_reports_a_linked_content_root() {
    let fixture = Fixture::new();
    fixture.register();
    install_content(&fixture);
    let outside = fixture.temp.path().join("outside-mods");
    fs::create_dir_all(&outside).expect("outside folder");
    let root = fixture.content_root();
    fs::remove_dir_all(&root).expect("remove content root");
    link_directory(&root, &outside);

    let report = fixture.json(&[
        "--json",
        "doctor",
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    assert!(
        diagnostics(&report).contains(&"CONTENT_ROOT_LINKED"),
        "{report}"
    );
    assert!(
        !diagnostics(&report).contains(&"CONTENT_ROOT_MOVED"),
        "{report}"
    );
}
