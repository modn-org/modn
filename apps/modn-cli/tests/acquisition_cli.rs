//! Catalog installs from the verified download cache, without network access.
//!
//! Each `https` source points at a closed loopback port, so any attempted
//! download fails at once. A pre-populated cache entry must avoid that.
#[path = "support/catalog.rs"]
mod support;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use support::{error, Fixture};
use zip::write::SimpleFileOptions;

fn url(name: &str) -> String {
    format!("https://127.0.0.1:9/releases/{name}-1.0.zip")
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn plugin_archive(fixture: &Fixture, name: &str) -> (PathBuf, String) {
    let path = fixture.temp.path().join(format!("{name}.zip"));
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).expect("archive"));
    zip.start_file(
        format!("Demo/package/plugins/{name}.plugin"),
        SimpleFileOptions::default(),
    )
    .expect("entry");
    zip.write_all(name.as_bytes()).expect("payload");
    zip.finish().expect("finish");
    let digest = sha(&fs::read(&path).expect("bytes"));
    (path, digest)
}

/// Write a manifest with an `https` source into the `deps` bucket.
fn https_manifest(fixture: &Fixture, id: &str, hash: &str, dependencies: &[&str]) {
    let manifest = json!({
        "schema_version":1, "id":id, "game":"test-game", "name":id,
        "version":"1.0", "archive_format":"zip", "sha256":hash,
        "source":{"kind":"https","url":url(id)}, "dependencies":dependencies
    });
    fs::write(
        fixture
            .temp
            .path()
            .join(format!("deps/games/test-game/{id}.json")),
        serde_json::to_vec(&manifest).expect("JSON"),
    )
    .expect("manifest");
}

fn cache_dir(fixture: &Fixture) -> PathBuf {
    fixture.temp.path().join("cache")
}

/// The entry name `modn` uses: `<game>#<mod>#<version>#<url hash>.zip`.
fn cache_entry(fixture: &Fixture, id: &str) -> PathBuf {
    let url_hash = &sha(url(id).as_bytes())[..7];
    cache_dir(fixture).join(format!("test-game#{id}#1.0#{url_hash}.zip"))
}

fn seed(fixture: &Fixture, id: &str, archive: &Path) {
    fs::create_dir_all(cache_dir(fixture)).expect("cache dir");
    fs::copy(archive, cache_entry(fixture, id)).expect("seed cache");
}

fn modn(fixture: &Fixture, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .env("MODN_CACHE", cache_dir(fixture))
        .args(args)
        .output()
        .expect("run CLI")
}

/// `top` (https) depends on `base` (https); neither is installed.
fn catalog() -> (Fixture, PathBuf, PathBuf) {
    let fixture = Fixture::new();
    let (base, base_hash) = plugin_archive(&fixture, "base");
    let (top, top_hash) = plugin_archive(&fixture, "top");
    fixture.bucket("deps", "test-game", "seedling", &"0".repeat(64), &[]);
    https_manifest(&fixture, "base", &base_hash, &[]);
    https_manifest(&fixture, "top", &top_hash, &["base"]);
    fixture.register();
    (fixture, base, top)
}

#[test]
fn verified_cache_entries_install_a_stack_without_downloading() {
    let (fixture, base, top) = catalog();
    seed(&fixture, "base", &base);
    seed(&fixture, "top", &top);

    // A dry run reports the cached archives and creates nothing.
    let preview = modn(&fixture, &["--json", "install", "top", "--dry-run"]);
    assert!(preview.status.success(), "{}", error(&preview));
    let plan: Value = serde_json::from_slice(&preview.stdout).expect("JSON");
    assert_eq!(
        plan["dependencies"][0]["archive"],
        json!(cache_entry(&fixture, "base"))
    );
    assert_eq!(plan["plan"]["status"], "reviewable");
    assert!(!fixture.game.join("addons").exists());

    let installed = modn(&fixture, &["--json", "install", "top", "--yes"]);
    assert!(installed.status.success(), "{}", error(&installed));
    let report: Value = serde_json::from_slice(&installed.stdout).expect("JSON");
    assert_eq!(report["stack"][0]["package"], "deps/test-game/base");
    assert_eq!(report["stack"][1]["package"], "deps/test-game/top");
    assert!(fixture.game.join("addons/base.plugin").is_file());
    assert!(fixture.game.join("addons/top.plugin").is_file());
    // Cache entries are kept for a later reinstall, as Scoop keeps them.
    assert!(cache_entry(&fixture, "base").is_file());
}

#[test]
fn a_tampered_cache_entry_is_not_used() {
    let (fixture, base, _) = catalog();
    let entry = cache_entry(&fixture, "base");
    seed(&fixture, "base", &base);
    fs::write(&entry, b"tampered").expect("tamper");

    // The entry fails verification, so Modn tries to download and fails.
    let failed = modn(&fixture, &["install", "base", "--yes"]);
    assert!(!failed.status.success());
    let message = error(&failed);
    assert!(
        message.contains("download failed: https://127.0.0.1:9/"),
        "{message}"
    );
    assert!(!fixture.game.join("addons").exists());
    assert_eq!(fs::read(&entry).expect("entry"), b"tampered");
    let partial = entry.with_file_name(format!(
        "{}.download",
        entry.file_name().expect("name").to_string_lossy()
    ));
    assert!(!partial.exists());
}

#[test]
fn no_cache_downloads_again_and_never_uses_the_entry() {
    let (fixture, base, _) = catalog();
    seed(&fixture, "base", &base);
    let failed = modn(&fixture, &["install", "base", "--yes", "--no-cache"]);
    assert!(!failed.status.success());
    assert!(
        error(&failed).contains("download failed"),
        "{}",
        error(&failed)
    );
    assert!(!fixture.game.join("addons").exists());

    let cached = modn(&fixture, &["install", "base", "--yes"]);
    assert!(cached.status.success(), "{}", error(&cached));
    assert!(fixture.game.join("addons/base.plugin").is_file());
}

#[test]
fn a_manual_source_is_never_downloaded() {
    let fixture = Fixture::new();
    let (archive, hash) = plugin_archive(&fixture, "manual");
    fixture.bucket("deps", "test-game", "manual", &hash, &[]);
    fixture.register();
    let blocked = modn(&fixture, &["install", "manual", "--yes"]);
    assert!(!blocked.status.success());
    let message = error(&blocked);
    assert!(
        message.contains("download \"fixture.zip\" manually"),
        "{message}"
    );
    assert!(message.contains("--file <archive>"), "{message}");
    assert!(!cache_dir(&fixture).exists());
    let supplied = modn(
        &fixture,
        &[
            "install",
            "manual",
            "--yes",
            "--file",
            archive.to_str().expect("UTF-8"),
        ],
    );
    assert!(supplied.status.success(), "{}", error(&supplied));
}

#[test]
fn cache_show_and_rm_follow_scoop() {
    let (fixture, base, top) = catalog();
    seed(&fixture, "base", &base);
    seed(&fixture, "top", &top);
    fs::write(cache_dir(&fixture).join("notes.txt"), b"keep").expect("unrelated");

    let show = modn(&fixture, &["--json", "cache", "show"]);
    assert!(show.status.success(), "{}", error(&show));
    let listed: Value = serde_json::from_slice(&show.stdout).expect("JSON");
    assert_eq!(listed["entries"].as_array().expect("entries").len(), 2);
    assert_eq!(listed["entries"][0]["mod_id"], "base");

    let missing_target = modn(&fixture, &["cache", "rm"]);
    assert!(!missing_target.status.success());
    assert!(
        error(&missing_target).contains("--all"),
        "{}",
        error(&missing_target)
    );

    let one = modn(&fixture, &["--json", "cache", "rm", "top"]);
    assert!(one.status.success(), "{}", error(&one));
    assert!(!cache_entry(&fixture, "top").exists());
    assert!(cache_entry(&fixture, "base").is_file());

    let all = modn(&fixture, &["cache", "rm", "--all"]);
    assert!(all.status.success(), "{}", error(&all));
    assert!(String::from_utf8_lossy(&all.stdout).contains("Removed: 1 file(s)"));
    assert!(!cache_entry(&fixture, "base").exists());
    assert!(cache_dir(&fixture).join("notes.txt").is_file());
}
