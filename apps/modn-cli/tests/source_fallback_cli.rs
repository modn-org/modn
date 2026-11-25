#[path = "support/git_fixture.rs"]
mod git_fixture;

#[path = "support/main_bucket.rs"]
mod main_bucket;

#[path = "support/catalog.rs"]
mod support;

#[path = "support/tree.rs"]
#[allow(dead_code)]
mod tree;

use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::process::Output;
use support::{error, Fixture};

const FIRST: &str = "https://127.0.0.1:1/primary.zip";
const SECOND: &str = "https://127.0.0.1:1/mirror.zip";

fn add_bucket(fixture: &Fixture, sources: Value, pin: &str) {
    let root = fixture.temp.path().join("mirrors");
    let records = root.join("mods/test-game");
    fs::create_dir_all(&records).expect("bucket directory");
    fs::write(
        root.join("bucket.json"),
        br#"{"schema_version":2,"name":"mirrors","title":"Mirrors"}"#,
    )
    .expect("bucket metadata");
    let manifest = json!({
        "schema_version": 2,
        "id": "example",
        "game": "test-game",
        "name": "Example",
        "version": "1.0",
        "archive_format": "zip",
        "blake3": pin,
        "sources": sources,
    });
    fs::write(
        records.join("example.json"),
        serde_json::to_vec(&manifest).expect("manifest JSON"),
    )
    .expect("manifest");
    let result = fixture.run_paths(&["bucket", "add", "mirrors"], &[&root]);
    assert!(result.status.success(), "{}", error(&result));
}

fn install(fixture: &Fixture, extra: &[&str]) -> std::process::Output {
    let mut args = vec!["--json", "install", "test-game/example"];
    args.extend_from_slice(extra);
    args.extend_from_slice(&["--path", fixture.game.to_str().expect("game path")]);
    fixture.run(&args)
}

fn cache_files(fixture: &Fixture) -> Vec<String> {
    let Ok(entries) = fs::read_dir(fixture.state.join("cache")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn a_dry_run_lists_the_ordered_sources_without_a_transport_call_or_write() {
    let fixture = Fixture::new();
    add_bucket(
        &fixture,
        json!([
            {"kind": "https", "url": FIRST},
            {"kind": "https", "url": SECOND}
        ]),
        &"0".repeat(64),
    );
    fixture.register();
    let output = install(&fixture, &["--dry-run"]);
    assert!(output.status.success(), "{}", error(&output));
    let value: Value = serde_json::from_slice(&output.stdout).expect("dry-run JSON");
    assert_eq!(value["sources"], json!([FIRST, SECOND]));
    assert!(
        cache_files(&fixture).is_empty(),
        "a dry run wrote cache files"
    );
}

#[test]
fn every_failing_source_is_named_in_one_error_and_nothing_is_cached() {
    let fixture = Fixture::new();
    add_bucket(
        &fixture,
        json!([
            {"kind": "https", "url": FIRST},
            {"kind": "https", "url": SECOND},
            {"kind": "manual", "filename": "example.zip"}
        ]),
        &"0".repeat(64),
    );
    fixture.register();
    let output = install(&fixture, &["--yes"]);
    assert!(!output.status.success(), "{}", error(&output));
    let stderr = error(&output);
    for expected in [FIRST, SECOND, "network", "2 attempts", "manual"] {
        assert!(stderr.contains(expected), "missing {expected:?}: {stderr}");
    }
    assert!(
        cache_files(&fixture)
            .iter()
            .all(|name| !name.ends_with(".download")),
        "a failed attempt kept a partial file"
    );
    assert!(!fixture.game.join("addons/example.plugin").exists());
}

#[test]
fn a_verified_cache_hit_installs_without_a_transport_call() {
    let fixture = Fixture::new();
    let (archive, pin) = fixture.archive();
    add_bucket(&fixture, json!([{"kind": "https", "url": FIRST}]), &pin);
    fixture.register();
    let cache = fixture.state.join("cache");
    fs::create_dir_all(&cache).expect("cache directory");
    let url_hash = &blake3::hash(FIRST.as_bytes()).to_hex()[..7];
    fs::write(
        cache.join(format!("test-game#example#1.0#{url_hash}.zip")),
        fs::read(&archive).expect("archive bytes"),
    )
    .expect("seeded cache entry");
    let output = install(&fixture, &["--yes"]);
    assert!(output.status.success(), "{}", error(&output));
    assert_eq!(
        fs::read(fixture.game.join("addons/example.plugin")).expect("installed plugin"),
        b"installed-by-modn"
    );
    assert!(cache_files(&fixture)
        .iter()
        .all(|name| !name.ends_with(".download")));
}

#[test]
fn a_dry_run_names_the_first_fetchable_source_not_the_manual_one() {
    let fixture = Fixture::new();
    add_bucket(
        &fixture,
        json!([
            {"kind": "manual", "filename": "Example 1.0.zip"},
            {"kind": "https", "url": FIRST}
        ]),
        &"0".repeat(64),
    );
    fixture.register();

    let output = install(&fixture, &["--dry-run"]);
    assert!(output.status.success(), "{}", error(&output));
    let value: Value = serde_json::from_slice(&output.stdout).expect("dry-run JSON");
    let needed = value["archive_needed"]
        .as_str()
        .expect("dry run names the needed download");
    assert!(needed.contains(FIRST), "{needed}");
    assert!(!needed.contains("manually"), "{needed}");

    let text = fixture.run(&[
        "install",
        "test-game/example",
        "--dry-run",
        "--path",
        fixture.game.to_str().expect("game path"),
    ]);
    assert!(text.status.success(), "{}", error(&text));
    let stdout = String::from_utf8_lossy(&text.stdout);
    assert!(stdout.contains("when you install it"), "{stdout}");
    assert!(!stdout.contains("then add --file <archive>"), "{stdout}");
    assert!(stdout.contains(FIRST), "{stdout}");
    assert!(cache_files(&fixture).is_empty());
}

#[test]
fn a_manual_only_source_is_reported_without_a_download() {
    let fixture = Fixture::new();
    add_bucket(
        &fixture,
        json!([{"kind": "manual", "filename": "example.zip"}]),
        &"0".repeat(64),
    );
    fixture.register();
    let output = install(&fixture, &["--yes"]);
    assert!(!output.status.success());
    let stderr = error(&output);
    assert!(stderr.contains("manually"), "{stderr}");
    assert!(cache_files(&fixture).is_empty());
}

fn seed_url_cache(fixture: &Fixture, url: &str, archive: &std::path::Path) {
    let cache = fixture.state.join("cache");
    fs::create_dir_all(&cache).expect("cache directory");
    let url_hash = &blake3::hash(url.as_bytes()).to_hex()[..7];
    fs::write(
        cache.join(format!("test-game#example#1.0#{url_hash}.zip")),
        fs::read(archive).expect("archive bytes"),
    )
    .expect("seeded cache entry");
}

fn history_source(fixture: &Fixture) -> Value {
    let output = fixture.run(&["--json", "history"]);
    assert!(output.status.success(), "{}", error(&output));
    let operations: Value = serde_json::from_slice(&output.stdout).expect("history JSON");
    operations[0]["packages"][0]["release_after"]["source"].clone()
}

#[test]
fn a_https_fallback_is_recorded_as_the_serving_source_and_undo_uses_its_cache() {
    let fixture = Fixture::new();
    let (archive, pin) = fixture.archive();
    add_bucket(
        &fixture,
        json!([
            {"kind": "manual", "filename": "Example 1.0.zip"},
            {"kind": "https", "url": FIRST}
        ]),
        &pin,
    );
    seed_url_cache(&fixture, FIRST, &archive);
    fixture.register();

    let output = install(&fixture, &["--yes"]);
    assert!(output.status.success(), "{}", error(&output));
    let source = history_source(&fixture);
    assert_eq!(source["kind"], "https", "{source}");
    assert_eq!(source["url"], FIRST);

    let removed = fixture.run(&[
        "uninstall",
        "example",
        "--path",
        fixture.game.to_str().expect("game path"),
        "--yes",
    ]);
    assert!(removed.status.success(), "{}", error(&removed));
    assert!(!fixture.game.join("addons/example.plugin").exists());

    let undone = fixture.run(&["undo", "--yes"]);
    assert!(
        undone.status.success(),
        "undo must restore the https archive from the cache: {}",
        error(&undone)
    );
    assert!(fixture.game.join("addons/example.plugin").is_file());
}

#[test]
fn a_user_supplied_archive_records_the_first_fetchable_catalog_source() {
    let fixture = Fixture::new();
    let (archive, pin) = fixture.archive();
    add_bucket(
        &fixture,
        json!([
            {"kind": "manual", "filename": "Example 1.0.zip"},
            {"kind": "https", "url": FIRST}
        ]),
        &pin,
    );
    seed_url_cache(&fixture, FIRST, &archive);
    fixture.register();

    let output = fixture.run_paths(
        &["--json", "install", "test-game/example", "--yes", "--file"],
        &[archive.as_path()],
    );
    assert!(output.status.success(), "{}", error(&output));
    let source = history_source(&fixture);
    assert_eq!(source["kind"], "https", "{source}");
    assert_eq!(source["url"], FIRST, "{source}");

    let removed = fixture.run(&[
        "uninstall",
        "example",
        "--path",
        fixture.game.to_str().expect("game path"),
        "--yes",
    ]);
    assert!(removed.status.success(), "{}", error(&removed));
    let undone = fixture.run(&["undo", "--yes"]);
    assert!(
        undone.status.success(),
        "undo must restore the pinned catalog archive from the cache: {}",
        error(&undone)
    );
    assert!(fixture.game.join("addons/example.plugin").is_file());
}

#[test]
fn a_dry_run_reports_a_cached_secondary_nexus_archive() {
    let fixture = Fixture::new();
    let (archive, pin) = fixture.archive();
    add_bucket(
        &fixture,
        json!([
            {
                "kind": "nexus",
                "game_domain": "cyberpunk2077",
                "mod_id": 6945,
                "file_id": 111111,
                "file_name": "First.zip"
            },
            {
                "kind": "nexus",
                "game_domain": "cyberpunk2077",
                "mod_id": 6945,
                "file_id": 222222,
                "file_name": "Second.zip"
            }
        ]),
        &pin,
    );
    seed_url_cache(
        &fixture,
        "nexus://cyberpunk2077/mods/6945/files/222222/Second.zip",
        &archive,
    );
    fixture.register();

    let output = install(&fixture, &["--dry-run"]);
    assert!(output.status.success(), "{}", error(&output));
    let value: Value = serde_json::from_slice(&output.stdout).expect("dry-run JSON");
    assert!(
        !value["plan"].is_null(),
        "the dry run did not report the cached archive: {value}"
    );
    assert!(value["archive_needed"].is_null(), "{value}");
}

fn add_git_fallback_bucket(fixture: &Fixture, root: &Path, commit: &str) {
    let bucket = root.join("git-fallback-fixture");
    let game = bucket.join("mods/test-game");
    fs::create_dir_all(&game).expect("bucket directory");
    fs::write(
        bucket.join("bucket.json"),
        br#"{"schema_version":2,"name":"git-fallback-fixture","title":"Git fallback fixture"}"#,
    )
    .expect("bucket metadata");
    let manifest = json!({
        "schema_version": 2,
        "id": "git-package",
        "game": "test-game",
        "name": "Git Package",
        "version": "1.0",
        "sources": [
            {"kind": "git", "repository": "fixture/missing", "commit": commit},
            {"kind": "git", "repository": "fixture/project", "commit": commit},
        ],
    });
    fs::write(
        game.join("git-package.json"),
        serde_json::to_vec(&manifest).expect("manifest JSON"),
    )
    .expect("manifest");
    let result = fixture.run_paths(&["bucket", "add", "git-fallback-fixture"], &[&bucket]);
    assert!(result.status.success(), "{}", error(&result));
}

fn run_git(fixture: &Fixture, base: &Path, args: &[&str]) -> Output {
    let mut command = main_bucket::command(env!("CARGO_BIN_EXE_modn"));
    command
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .env("MODN_TEST_GIT_REMOTE_BASE", git_fixture::file_url(base))
        .args(args);
    command.output().expect("run CLI")
}

#[test]
fn a_git_fallback_is_recorded_as_the_serving_source_and_undo_uses_its_cache() {
    let fixture = Fixture::new();
    let repository = git_fixture::repository(fixture.temp.path());
    add_git_fallback_bucket(&fixture, fixture.temp.path(), &repository.commit);
    fixture.register();
    let game = fixture.game.to_str().expect("game path");
    let plugin = fixture.game.join("addons/git-source.plugin");

    let installed = run_git(
        &fixture,
        &repository.base,
        &[
            "--json",
            "install",
            "test-game/git-package",
            "--path",
            game,
            "--yes",
        ],
    );
    assert!(installed.status.success(), "{}", error(&installed));
    let source = history_source(&fixture);
    assert_eq!(source["kind"], "git", "{source}");
    assert_eq!(source["repository"], "fixture/project", "{source}");
    assert_eq!(source["commit"], repository.commit, "{source}");
    assert_eq!(
        fs::read(&plugin).expect("installed plugin"),
        b"pinned-release"
    );

    let removed = fixture.run(&["uninstall", "git-package", "--path", game, "--yes"]);
    assert!(removed.status.success(), "{}", error(&removed));
    assert!(!plugin.exists());
    let hidden = fixture.temp.path().join("project.git-hidden");
    fs::rename(repository.base.join("fixture/project.git"), &hidden).expect("hide the remote");

    let undone = fixture.run(&["undo", "--yes"]);
    assert!(
        undone.status.success(),
        "undo must restore the serving Git source archive from the cache: {}",
        error(&undone)
    );
    assert_eq!(
        fs::read(&plugin).expect("restored plugin"),
        b"pinned-release"
    );
}
