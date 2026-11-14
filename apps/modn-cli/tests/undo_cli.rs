#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/catalog.rs"]
mod support;

use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use support::{error, Fixture};
use zip::write::SimpleFileOptions;

fn archive(fixture: &Fixture, name: &str, files: &[(&str, &str)]) -> (PathBuf, String) {
    let path = fixture.temp.path().join(format!("{name}.zip"));
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).expect("archive"));
    for (file, contents) in files {
        zip.start_file(
            format!("Demo/package/plugins/{file}"),
            SimpleFileOptions::default(),
        )
        .expect("entry");
        zip.write_all(contents.as_bytes()).expect("payload");
    }
    zip.finish().expect("finish");
    let digest = blake3::hash(&fs::read(&path).expect("bytes"))
        .to_hex()
        .to_string();
    (path, digest)
}

fn manifest(fixture: &Fixture, id: &str, version: &str, hash: &str, source: Value) {
    manifest_extra(fixture, id, version, hash, source, json!({}));
}

fn manifest_extra(
    fixture: &Fixture,
    id: &str,
    version: &str,
    hash: &str,
    source: Value,
    extra: Value,
) {
    let folder = fixture.temp.path().join("deps/mods/test-game");
    fs::create_dir_all(&folder).expect("bucket folder");
    let mut manifest = json!({
        "schema_version": 2, "id": id, "game": "test-game", "name": id,
        "version": version, "archive_format": "zip", "blake3": hash,
        "sources": [source]
    });
    if let (Some(manifest), Value::Object(extra)) = (manifest.as_object_mut(), extra) {
        manifest.extend(extra);
    }
    fs::write(
        folder.join(format!("{id}.json")),
        serde_json::to_vec(&manifest).expect("manifest JSON"),
    )
    .expect("manifest");
}

fn https(url: &str) -> Value {
    json!({"kind": "https", "url": url})
}

fn nexus() -> Value {
    json!({
        "kind": "nexus", "game_domain": "testgame", "mod_id": 12,
        "file_id": 34, "file_name": "example.zip"
    })
}

fn nexus_url() -> &'static str {
    "nexus://testgame/mods/12/files/34/example.zip"
}

/// Put `archive` where the undo cache lookup expects it, without a download.
fn cache(fixture: &Fixture, id: &str, version: &str, url: &str, archive: &Path) {
    let root = fixture.state.join("cache");
    fs::create_dir_all(&root).expect("cache");
    let hash = blake3::hash(url.as_bytes()).to_hex().to_string();
    fs::copy(
        archive,
        root.join(format!("test-game#{id}#{version}#{}.zip", &hash[..7])),
    )
    .expect("cache entry");
}

fn install(fixture: &Fixture, target: &str, files: &[&Path]) -> Output {
    let mut command = main_bucket::command(env!("CARGO_BIN_EXE_modn"));
    command
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .args(["--json", "install", target, "--yes"]);
    for file in files {
        command.arg("--file").arg(file);
    }
    command.output().expect("install")
}

fn json_of(output: &Output) -> Value {
    assert!(output.status.success(), "{}", error(output));
    serde_json::from_slice(&output.stdout).expect("JSON")
}

fn history(fixture: &Fixture) -> Vec<Value> {
    json_of(&fixture.run(&["--json", "history"]))
        .as_array()
        .expect("history array")
        .clone()
}

fn id_of(operation: &Value) -> String {
    operation["id"].as_str().expect("operation ID").to_owned()
}

fn error_code(output: &Output) -> String {
    let value: Value = serde_json::from_slice(&output.stderr)
        .unwrap_or_else(|_| panic!("error JSON: {}", error(output)));
    value["error"].as_str().expect("error code").to_owned()
}

#[test]
fn undo_install_removes_the_package_and_records_the_target() {
    let fixture = Fixture::new();
    let (archive, digest) = fixture.archive();
    fixture.bucket("deps", "test-game", "example", &digest, &[]);
    fixture.register();
    assert!(install(&fixture, "example", &[&archive]).status.success());
    let installed = fixture.game.join("addons/example.plugin");
    assert!(installed.is_file());

    let operations = history(&fixture);
    let target = id_of(&operations[0]);
    let undone = fixture.run(&["--json", "undo", &target, "--yes"]);
    assert!(undone.status.success(), "{}", error(&undone));
    assert!(!installed.exists());
    assert!(json_of(&fixture.run(&["--json", "list"]))
        .as_array()
        .expect("list")
        .is_empty());

    let operations = history(&fixture);
    assert_eq!(operations.len(), 2);
    let record = &operations[0];
    assert_eq!(record["kind"], "uninstall");
    assert_eq!(record["undoes"], target);
    assert_eq!(record["packages"][0]["mod_id"], "example");
    assert!(record["packages"][0]["files"][0]["digest_before"].is_string());
    assert!(record["packages"][0]["files"][0]["digest_after"].is_null());
}

#[test]
fn undo_update_restores_the_previous_release_from_the_cache() {
    let fixture = Fixture::new();
    let (v1, v1_hash) = archive(&fixture, "v1", &[("example.plugin", "one")]);
    let (v2, v2_hash) = archive(&fixture, "v2", &[("example.plugin", "two")]);
    let v1_url = "https://example.invalid/example-1.0.zip";
    let v2_url = "https://example.invalid/example-2.0.zip";
    fixture.bucket("deps", "test-game", "example", &v1_hash, &[]);
    manifest(&fixture, "example", "1.0", &v1_hash, https(v1_url));
    fixture.register();
    assert!(install(&fixture, "example", &[&v1]).status.success());
    cache(&fixture, "example", "1.0", v1_url, &v1);

    manifest(&fixture, "example", "2.0", &v2_hash, https(v2_url));
    let updated = fixture.run_paths(
        &["--json", "update", "example", "--yes", "--file"],
        &[v2.as_path()],
    );
    assert!(updated.status.success(), "{}", error(&updated));
    let addon = fixture.game.join("addons/example.plugin");
    assert_eq!(fs::read_to_string(&addon).expect("v2"), "two");

    let operations = history(&fixture);
    let target = id_of(&operations[0]);
    assert_eq!(operations[0]["kind"], "update");
    let undone = fixture.run(&["--json", "undo", &target, "--yes"]);
    assert!(undone.status.success(), "{}", error(&undone));
    assert_eq!(fs::read_to_string(&addon).expect("v1"), "one");

    let operations = history(&fixture);
    assert_eq!(operations[0]["undoes"], target);
    assert_eq!(
        operations[0]["packages"][0]["release_after"]["version"],
        "1.0"
    );
    assert_eq!(
        operations[0]["packages"][0]["release_before"]["version"],
        "2.0"
    );
}

#[test]
fn undo_uninstall_restores_the_recorded_release() {
    let fixture = Fixture::new();
    let (v1, v1_hash) = archive(&fixture, "v1", &[("example.plugin", "one")]);
    let url = "https://example.invalid/example-1.0.zip";
    fixture.bucket("deps", "test-game", "example", &v1_hash, &[]);
    manifest(&fixture, "example", "1.0", &v1_hash, https(url));
    fixture.register();
    assert!(install(&fixture, "example", &[&v1]).status.success());
    cache(&fixture, "example", "1.0", url, &v1);
    let addon = fixture.game.join("addons/example.plugin");
    assert!(fixture
        .run(&["uninstall", "example", "--yes"])
        .status
        .success());
    assert!(!addon.exists());

    let operations = history(&fixture);
    let target = id_of(&operations[0]);
    let undone = fixture.run(&["--json", "undo", &target, "--yes"]);
    assert!(undone.status.success(), "{}", error(&undone));
    assert_eq!(fs::read_to_string(&addon).expect("restored"), "one");

    let operations = history(&fixture);
    assert_eq!(operations[0]["kind"], "install");
    assert_eq!(operations[0]["undoes"], target);
    assert_eq!(
        operations[0]["packages"][0]["release_after"]["version"],
        "1.0"
    );
}

#[test]
fn undo_uninstall_restores_a_nexus_release_from_the_cache() {
    let fixture = Fixture::new();
    let (archive, digest) = archive(&fixture, "nexus", &[("example.plugin", "one")]);
    fixture.bucket("deps", "test-game", "example", &digest, &[]);
    manifest(&fixture, "example", "1.0", &digest, nexus());
    cache(&fixture, "example", "1.0", nexus_url(), &archive);
    fixture.register();
    let installed = install(&fixture, "example", &[]);
    assert!(installed.status.success(), "{}", error(&installed));
    let addon = fixture.game.join("addons/example.plugin");
    assert!(fixture
        .run(&["uninstall", "example", "--yes"])
        .status
        .success());
    assert!(!addon.exists());

    let uninstall = history(&fixture)[0].clone();
    assert_eq!(
        uninstall["packages"][0]["release_before"]["archive_blake3"],
        digest
    );
    let target = id_of(&uninstall);
    let undone = fixture.run(&["--json", "undo", &target, "--yes"]);
    assert!(undone.status.success(), "{}", error(&undone));
    assert_eq!(fs::read_to_string(&addon).expect("restored"), "one");

    let operations = history(&fixture);
    assert_eq!(operations[0]["kind"], "install");
    assert_eq!(operations[0]["undoes"], target);
    assert_eq!(
        operations[0]["packages"][0]["files"][0]["digest_after"],
        uninstall["packages"][0]["files"][0]["digest_before"]
    );
}

#[test]
fn undo_uninstall_refuses_a_nexus_release_when_the_archive_is_not_cached() {
    let fixture = Fixture::new();
    let (archive, digest) = archive(&fixture, "nexus", &[("example.plugin", "one")]);
    fixture.bucket("deps", "test-game", "example", &digest, &[]);
    manifest(&fixture, "example", "1.0", &digest, nexus());
    cache(&fixture, "example", "1.0", nexus_url(), &archive);
    fixture.register();
    let installed = install(&fixture, "example", &[]);
    assert!(installed.status.success(), "{}", error(&installed));
    assert!(fixture
        .run(&["uninstall", "example", "--yes"])
        .status
        .success());
    let target = id_of(&history(&fixture)[0]);
    fs::remove_dir_all(fixture.state.join("cache")).expect("clear cache");

    let before_history = history(&fixture);
    let before_packages = json_of(&fixture.run(&["--json", "list"]));
    let marker = fs::read(fixture.game.join("bin/TestGame.exe")).expect("game marker");
    let refused = fixture.run(&["--json", "undo", &target, "--yes"]);
    assert!(!refused.status.success());
    assert_eq!(error_code(&refused), "UNDO_FAILED");
    let message = error(&refused);
    assert!(message.contains("no longer cached"), "{message}");
    assert!(message.contains("nxm"), "{message}");
    assert_eq!(history(&fixture), before_history);
    assert_eq!(json_of(&fixture.run(&["--json", "list"])), before_packages);
    assert_eq!(
        fs::read(fixture.game.join("bin/TestGame.exe")).expect("marker"),
        marker
    );
    assert!(!fixture.game.join("addons/example.plugin").exists());
}

#[test]
fn undo_uninstall_refuses_a_manual_source_before_changes() {
    let fixture = Fixture::new();
    let (archive, digest) = fixture.archive();
    fixture.bucket("deps", "test-game", "example", &digest, &[]);
    fixture.register();
    assert!(install(&fixture, "example", &[&archive]).status.success());
    assert!(fixture
        .run(&["uninstall", "example", "--yes"])
        .status
        .success());
    let target = id_of(&history(&fixture)[0]);

    let refused = fixture.run(&["--json", "undo", &target, "--yes"]);
    assert!(!refused.status.success());
    assert_eq!(error_code(&refused), "UNDO_SOURCE_MANUAL");
    assert!(!fixture.game.join("addons/example.plugin").exists());
    assert_eq!(history(&fixture).len(), 2);
}

#[test]
fn undo_refuses_conflicts_unknown_ids_pins_and_missing_yes() {
    let fixture = Fixture::new();
    let (v1, v1_hash) = archive(&fixture, "v1", &[("example.plugin", "one")]);
    let (v2, v2_hash) = archive(&fixture, "v2", &[("example.plugin", "two")]);
    fixture.bucket("deps", "test-game", "example", &v1_hash, &[]);
    fixture.register();
    assert!(install(&fixture, "example", &[&v1]).status.success());
    let install_id = id_of(&history(&fixture)[0]);

    let unknown = fixture.run(&["--json", "undo", "txn-nope", "--yes"]);
    assert!(!unknown.status.success());
    assert_eq!(error_code(&unknown), "UNDO_NOT_FOUND");

    let no_yes = fixture.run(&["--json", "undo", &install_id]);
    assert!(!no_yes.status.success());
    assert_eq!(error_code(&no_yes), "INVALID_ARGUMENTS");
    assert!(String::from_utf8_lossy(&no_yes.stdout).contains(&install_id));

    let dry_run = fixture.run(&["--json", "undo", &install_id, "--dry-run"]);
    assert!(dry_run.status.success(), "{}", error(&dry_run));
    assert!(fixture.game.join("addons/example.plugin").exists());
    assert_eq!(history(&fixture).len(), 1);

    let packages = json_of(&fixture.run(&["--json", "list"]));
    let package_id = packages[0]["id"].as_str().expect("package ID");
    assert!(fixture.run(&["pin", package_id]).status.success());
    let pinned = fixture.run(&["--json", "undo", &install_id, "--yes"]);
    assert!(!pinned.status.success());
    assert_eq!(error_code(&pinned), "PACKAGE_PINNED");
    assert!(fixture.run(&["unpin", package_id]).status.success());

    manifest(
        &fixture,
        "example",
        "2.0",
        &v2_hash,
        json!({"kind": "manual", "filename": "v2.zip"}),
    );
    let updated = fixture.run_paths(
        &["--json", "update", "example", "--yes", "--file"],
        &[v2.as_path()],
    );
    assert!(updated.status.success(), "{}", error(&updated));
    let update_id = id_of(&history(&fixture)[0]);
    let conflict = fixture.run(&["--json", "undo", &install_id, "--yes"]);
    assert!(!conflict.status.success());
    assert_eq!(error_code(&conflict), "UNDO_CONFLICT");
    assert!(
        error(&conflict).contains(&update_id),
        "{}",
        error(&conflict)
    );
}

#[test]
fn undo_a_stack_install_removes_and_restores_every_member_atomically() {
    let fixture = Fixture::new();
    let (base, base_hash) = archive(&fixture, "base", &[("base.plugin", "base")]);
    let (middle, middle_hash) = archive(&fixture, "middle", &[("middle.plugin", "middle")]);
    let (top, top_hash) = archive(&fixture, "top", &[("top.plugin", "top")]);
    let base_url = "https://example.invalid/base-1.0.zip";
    let middle_url = "https://example.invalid/middle-1.0.zip";
    let top_url = "https://example.invalid/top-1.0.zip";
    fixture.bucket("deps", "test-game", "base", &base_hash, &[]);
    manifest(&fixture, "base", "1.0", &base_hash, https(base_url));
    manifest_extra(
        &fixture,
        "middle",
        "1.0",
        &middle_hash,
        https(middle_url),
        json!({"dependencies": ["base"]}),
    );
    manifest_extra(
        &fixture,
        "top",
        "1.0",
        &top_hash,
        https(top_url),
        json!({"dependencies": ["middle"]}),
    );
    cache(&fixture, "base", "1.0", base_url, &base);
    cache(&fixture, "middle", "1.0", middle_url, &middle);
    cache(&fixture, "top", "1.0", top_url, &top);
    fixture.register();
    let installed = install(&fixture, "top", &[]);
    assert!(installed.status.success(), "{}", error(&installed));
    for file in ["base.plugin", "middle.plugin", "top.plugin"] {
        assert!(fixture.game.join("addons").join(file).is_file());
    }
    let operations = history(&fixture);
    assert_eq!(operations.len(), 1);
    assert_eq!(
        operations[0]["packages"]
            .as_array()
            .expect("packages")
            .len(),
        3
    );
    let target = id_of(&operations[0]);

    let undone = fixture.run(&["--json", "undo", &target, "--yes"]);
    assert!(undone.status.success(), "{}", error(&undone));
    for file in ["base.plugin", "middle.plugin", "top.plugin"] {
        assert!(!fixture.game.join("addons").join(file).exists());
    }
    let operations = history(&fixture);
    assert_eq!(operations[0]["undoes"], target);
    assert_eq!(
        operations[0]["packages"]
            .as_array()
            .expect("packages")
            .len(),
        3
    );
    assert_eq!(operations[0]["kind"], "uninstall");

    let uninstall_id = id_of(&operations[0]);
    let restored = fixture.run(&["--json", "undo", &uninstall_id, "--yes"]);
    assert!(restored.status.success(), "{}", error(&restored));
    for file in ["base.plugin", "middle.plugin", "top.plugin"] {
        assert!(fixture.game.join("addons").join(file).is_file());
    }
    let operations = history(&fixture);
    assert_eq!(operations[0]["undoes"], uninstall_id);
    assert_eq!(
        operations[0]["packages"]
            .as_array()
            .expect("packages")
            .len(),
        3
    );
    assert_eq!(operations[0]["kind"], "install");
}

#[test]
fn a_cancel_line_stops_undo_before_the_commit_point() {
    let fixture = Fixture::new();
    let (archive, digest) = fixture.archive();
    fixture.bucket("deps", "test-game", "example", &digest, &[]);
    fixture.register();
    assert!(install(&fixture, "example", &[&archive]).status.success());
    let target = id_of(&history(&fixture)[0]);

    let mut command = main_bucket::command(env!("CARGO_BIN_EXE_modn"));
    command
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .args(["--json", "--progress", "json", "undo", &target, "--yes"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn modn");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"cancel\n")
        .expect("cancel line");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("modn output");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(r#""error":"CANCELLED""#), "{stderr}");
    assert!(fixture.game.join("addons/example.plugin").exists());
    assert_eq!(history(&fixture).len(), 1);
}
