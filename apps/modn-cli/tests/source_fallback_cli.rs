#[path = "support/main_bucket.rs"]
mod main_bucket;

#[path = "support/catalog.rs"]
mod support;

#[path = "support/tree.rs"]
#[allow(dead_code)]
mod tree;

use serde_json::{json, Value};
use std::fs;
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
