#[path = "support/catalog.rs"]
mod support;

#[path = "support/main_bucket.rs"]
mod main_bucket;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Output;
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

fn cache_entry(fixture: &Fixture, id: &str) -> PathBuf {
    let url_hash = &sha(url(id).as_bytes())[..7];
    cache_dir(fixture).join(format!("test-game#{id}#1.0#{url_hash}.zip"))
}

/// Count archive entries, ignoring the digest index and its temporary files.
fn archive_entry_count(fixture: &Fixture) -> usize {
    fs::read_dir(cache_dir(fixture))
        .expect("cache dir")
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains('#'))
        .count()
}

fn seed(fixture: &Fixture, id: &str, archive: &Path) {
    fs::create_dir_all(cache_dir(fixture)).expect("cache dir");
    fs::copy(archive, cache_entry(fixture, id)).expect("seed cache");
}

fn modn(fixture: &Fixture, args: &[&str]) -> Output {
    main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .env("MODN_CACHE", cache_dir(fixture))
        .args(args)
        .output()
        .expect("run CLI")
}

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
    assert!(cache_entry(&fixture, "base").is_file());
}

#[test]
fn a_tampered_cache_entry_is_not_used() {
    let (fixture, base, _) = catalog();
    let entry = cache_entry(&fixture, "base");
    seed(&fixture, "base", &base);
    fs::write(&entry, b"tampered").expect("tamper");

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
fn a_nexus_source_needs_a_valid_api_key_without_a_supplied_file() {
    let fixture = Fixture::new();
    let (archive, hash) = plugin_archive(&fixture, "nexus");
    fixture.bucket("deps", "test-game", "nexus", &hash, &[]);
    let manifest = json!({
        "schema_version":1, "id":"nexus", "game":"test-game", "name":"nexus",
        "version":"1.0", "archive_format":"zip", "sha256":hash,
        "source":{"kind":"nexus", "game_domain":"cyberpunk2077", "mod_id":6945,
            "file_id":123456, "file_name":"Equipment-EX-1.2.10.zip"},
        "dependencies":[]
    });
    fs::write(
        fixture.temp.path().join("deps/games/test-game/nexus.json"),
        serde_json::to_vec(&manifest).expect("JSON"),
    )
    .expect("manifest");
    fixture.register();
    let blocked = main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .env("MODN_CACHE", cache_dir(&fixture))
        .env("MODN_NEXUS_API_KEY", "bad\tkey")
        .args(["install", "nexus", "--yes"])
        .output()
        .expect("run CLI");
    assert!(!blocked.status.success());
    let message = error(&blocked);
    assert!(
        message.contains("MODN_NEXUS_API_KEY is not a valid Nexus API key"),
        "{message}"
    );
    assert!(!cache_dir(&fixture).exists());
    let supplied = modn(
        &fixture,
        &[
            "install",
            "nexus",
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

#[test]
fn a_cache_inside_the_game_is_refused_before_any_download() {
    let (fixture, _, _) = catalog();
    let inside = fixture.game.join("modn-cache");
    let refused = main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .env("MODN_CACHE", &inside)
        .args(["install", "base", "--yes"])
        .output()
        .expect("run CLI");
    assert!(!refused.status.success());
    let message = error(&refused);
    assert!(
        message.contains("cache directory must be outside a game installation"),
        "{message}"
    );
    assert!(!message.contains("download failed"), "{message}");
    assert!(!inside.exists());
    assert!(!fixture.game.join("addons").exists());
}

#[test]
fn a_dry_run_downloads_missing_archives_only_on_request() {
    let (fixture, base, top) = catalog();

    let plain = modn(&fixture, &["--json", "install", "top", "--dry-run"]);
    assert!(plain.status.success(), "{}", error(&plain));
    let report: Value = serde_json::from_slice(&plain.stdout).expect("JSON");
    assert_eq!(report["plan"], Value::Null);
    assert!(!cache_dir(&fixture).exists());

    let fetched = modn(&fixture, &["install", "top", "--dry-run", "--download"]);
    assert!(!fetched.status.success());
    let message = error(&fetched);
    assert!(
        message.contains("download failed: https://127.0.0.1:9/releases/base"),
        "{message}"
    );
    assert!(!fixture.game.join("addons").exists());

    seed(&fixture, "base", &base);
    seed(&fixture, "top", &top);
    let cached = modn(
        &fixture,
        &["--json", "install", "top", "--dry-run", "--download"],
    );
    assert!(cached.status.success(), "{}", error(&cached));
    let report: Value = serde_json::from_slice(&cached.stdout).expect("JSON");
    assert_eq!(report["plan"]["status"], "reviewable");
    assert_eq!(
        report["plan"]["operations"][0]["destination"],
        "addons/top.plugin"
    );
    assert!(!fixture.game.join("addons").exists());

    let unpaired = modn(&fixture, &["install", "top", "--yes", "--download"]);
    assert!(!unpaired.status.success());
    assert!(
        error(&unpaired).contains("--dry-run"),
        "{}",
        error(&unpaired)
    );
}

#[test]
fn a_dry_run_download_refuses_a_cache_inside_the_game() {
    let (fixture, _, _) = catalog();
    let inside = fixture.game.join("modn-cache");
    let refused = main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .env("MODN_CACHE", &inside)
        .args(["install", "top", "--dry-run", "--download"])
        .output()
        .expect("run CLI");
    assert!(!refused.status.success());
    let message = error(&refused);
    assert!(
        message.contains("cache directory must be outside a game installation"),
        "{message}"
    );
    assert!(!inside.exists());
}

#[test]
fn list_and_doctor_show_the_folders_modn_created() {
    let (fixture, base, _) = catalog();
    seed(&fixture, "base", &base);
    let installed = modn(&fixture, &["install", "base", "--yes"]);
    assert!(installed.status.success(), "{}", error(&installed));

    let list = modn(&fixture, &["list"]);
    assert!(list.status.success(), "{}", error(&list));
    let text = String::from_utf8_lossy(&list.stdout);
    assert!(
        text.contains("Folders Modn created, removed when empty:\n  addons\n"),
        "{text}"
    );
    let packages: Value =
        serde_json::from_slice(&modn(&fixture, &["--json", "list"]).stdout).expect("JSON");
    assert!(packages.is_array());

    let doctor = modn(&fixture, &["--json", "doctor"]);
    assert!(doctor.status.success(), "{}", error(&doctor));
    let report: Value = serde_json::from_slice(&doctor.stdout).expect("JSON");
    assert_eq!(report["created_directories"], json!(["addons"]));

    let removed = modn(&fixture, &["uninstall", "base", "--yes"]);
    assert!(removed.status.success(), "{}", error(&removed));
    let list = modn(&fixture, &["list"]);
    assert!(!String::from_utf8_lossy(&list.stdout).contains("Folders Modn created"));
}

#[test]
fn provider_preview_is_offline_and_aliases_reuse_verified_bytes() {
    let fixture = Fixture::new();
    let (archive, hash) = plugin_archive(&fixture, "shared");
    fixture.bucket("deps", "test-game", "provider", &hash, &[]);
    let manifest = fixture
        .temp
        .path()
        .join("deps/games/test-game/provider.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["source"] = json!({"kind":"github-release", "repository":"fixture/project", "tag":"v1", "asset":"mod.zip"});
    fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    let preview = modn(
        &fixture,
        &[
            "--game",
            "test-game",
            "--json",
            "install",
            "provider",
            "--path",
            fixture.game.to_str().unwrap(),
            "--dry-run",
        ],
    );
    assert!(preview.status.success(), "{}", error(&preview));
    assert!(!fixture.state.join("state.sqlite").exists());
    assert!(!cache_dir(&fixture).exists());
    assert!(!fixture.game.join("addons").exists());
    fixture.register();
    seed(&fixture, "original", &archive);
    let installed = modn(&fixture, &["install", "provider", "--yes"]);
    assert!(installed.status.success(), "{}", error(&installed));
    assert_eq!(
        fs::read(fixture.game.join("addons/shared.plugin")).unwrap(),
        b"shared"
    );
    let entries = archive_entry_count(&fixture);
    assert_eq!(entries, 1, "digest alias reuse adds no second entry");
    assert!(
        cache_dir(&fixture).join(".digest-index.json").is_file(),
        "the digest index is written beside the archive entry"
    );
    assert!(modn(&fixture, &["uninstall", "provider", "--yes"])
        .status
        .success());
    value["id"] = json!("alias");
    value["source"] = json!({"kind":"https","url": "https://127.0.0.1:9/another-source.zip"});
    fs::write(
        fixture.temp.path().join("deps/games/test-game/alias.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    let alias = modn(&fixture, &["install", "alias", "--yes"]);
    assert!(alias.status.success(), "{}", error(&alias));
    assert_eq!(archive_entry_count(&fixture), 1);
    assert_eq!(
        fs::read(fixture.game.join("addons/shared.plugin")).unwrap(),
        b"shared"
    );
}
