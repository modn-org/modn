//! Refusals that report their specific error code (#175).

#[path = "support/catalog.rs"]
mod support;

#[path = "support/main_bucket.rs"]
mod main_bucket;

use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Output;
use support::{error, Fixture};
use zip::write::SimpleFileOptions;

fn error_code(output: &Output) -> String {
    assert!(!output.status.success(), "the command must fail");
    let value: Value = serde_json::from_slice(&output.stderr).expect("JSON error");
    value["error"].as_str().expect("error code").to_owned()
}

fn archive(fixture: &Fixture, member: &str, bytes: &[u8]) -> (PathBuf, String) {
    let path = fixture
        .temp
        .path()
        .join(format!("{}.zip", member.replace('/', "-")));
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).expect("archive"));
    zip.start_file(member, SimpleFileOptions::default())
        .expect("archive entry");
    zip.write_all(bytes).expect("archive payload");
    zip.finish().expect("archive finish");
    let digest = blake3::hash(&fs::read(&path).expect("archive bytes"))
        .to_hex()
        .to_string();
    (path, digest)
}

#[test]
fn a_command_without_a_registered_game_reports_game_not_selected() {
    let fixture = Fixture::new();
    let (archive, hash) = fixture.archive();
    fixture.bucket("deps", "test-game", "example", &hash, &[]);

    let output = fixture.run_paths(
        &["--json", "install", "example", "--dry-run", "--file"],
        &[&archive],
    );

    assert_eq!(
        error_code(&output),
        "GAME_NOT_SELECTED",
        "{}",
        error(&output)
    );
}

#[test]
fn an_unknown_package_reports_package_not_found() {
    let fixture = Fixture::new();
    let (archive, hash) = fixture.archive();
    fixture.bucket("deps", "test-game", "example", &hash, &[]);
    fixture.register();

    let uninstall = fixture.run(&["--json", "uninstall", "missing", "--yes"]);
    let update = fixture.run_paths(
        &["--json", "update", "example", "--yes", "--file"],
        &[&archive],
    );

    assert_eq!(
        error_code(&uninstall),
        "PACKAGE_NOT_FOUND",
        "{}",
        error(&uninstall)
    );
    assert_eq!(
        error_code(&update),
        "PACKAGE_NOT_FOUND",
        "{}",
        error(&update)
    );
}

#[test]
fn an_archive_with_a_blocked_plan_reports_plan_blocked() {
    let fixture = Fixture::new();
    fixture.register();
    let (archive, _) = archive(&fixture, "random/readme.txt", b"not a plugin");

    let output = fixture.run_paths(&["--json", "install", "--yes"], &[&archive]);

    assert_eq!(error_code(&output), "PLAN_BLOCKED", "{}", error(&output));
}

#[test]
fn a_modified_dependency_reports_dependency_missing() {
    let fixture = Fixture::new();
    let (base, base_hash) = archive(&fixture, "Demo/package/plugins/base.plugin", b"base");
    fixture.bucket("deps", "test-game", "base", &base_hash, &[]);
    let (dependent, dependent_hash) = archive(
        &fixture,
        "Demo/package/plugins/dependent.plugin",
        b"dependent",
    );
    let manifest = json!({
        "schema_version":2, "id":"dependent", "game":"test-game",
        "name":"Dependent", "version":"1.0", "archive_format":"zip",
        "blake3":dependent_hash, "sources":[{"kind":"manual","filename":"dependent.zip"}],
        "dependencies":["base"]
    });
    fs::write(
        fixture
            .temp
            .path()
            .join("deps/mods/test-game/dependent.json"),
        serde_json::to_vec(&manifest).expect("manifest JSON"),
    )
    .expect("dependent manifest");
    fixture.register();
    let installed = fixture.run_paths(&["--json", "install", "base", "--yes", "--file"], &[&base]);
    assert!(installed.status.success(), "{}", error(&installed));
    fs::write(fixture.game.join("addons/base.plugin"), b"changed").expect("modify base");

    let output = fixture.run_paths(
        &["--json", "install", "dependent", "--yes", "--file"],
        &[&dependent],
    );

    assert_eq!(
        error_code(&output),
        "DEPENDENCY_MISSING",
        "{}",
        error(&output)
    );
    assert!(!fixture.game.join("addons/dependent.plugin").exists());
}

#[test]
fn a_catalog_reinstall_reports_already_installed() {
    let fixture = Fixture::new();
    let (archive, hash) = fixture.archive();
    fixture.bucket("deps", "test-game", "example", &hash, &[]);
    fixture.register();
    let installed = fixture.run_paths(
        &["--json", "install", "example", "--yes", "--file"],
        &[&archive],
    );
    assert!(installed.status.success(), "{}", error(&installed));

    let output = fixture.run_paths(
        &["--json", "install", "example", "--yes", "--file"],
        &[&archive],
    );

    assert_eq!(
        error_code(&output),
        "ALREADY_INSTALLED",
        "{}",
        error(&output)
    );
}
