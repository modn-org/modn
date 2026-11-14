#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/catalog.rs"]
mod support;
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Output;
use support::{error, Fixture};
use zip::write::SimpleFileOptions;

fn archive(fixture: &Fixture, name: &str, contents: &str) -> (PathBuf, String) {
    let path = fixture.temp.path().join(format!("{name}.zip"));
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).expect("archive"));
    zip.start_file(
        "Demo/package/plugins/example.plugin",
        SimpleFileOptions::default(),
    )
    .expect("entry");
    zip.write_all(contents.as_bytes()).expect("payload");
    zip.finish().expect("finish");
    let digest = blake3::hash(&fs::read(&path).expect("bytes"))
        .to_hex()
        .to_string();
    (path, digest)
}

fn manifest(fixture: &Fixture, id: &str, version: &str, hash: &str, filename: &str) {
    let folder = fixture.temp.path().join("deps/mods/test-game");
    fs::create_dir_all(&folder).expect("bucket folder");
    fs::write(
        folder.join(format!("{id}.json")),
        serde_json::to_vec(&json!({
            "schema_version": 2, "id": id, "game": "test-game", "name": id,
            "version": version, "archive_format": "zip", "blake3": hash,
            "sources": [{"kind": "manual", "filename": filename}]
        }))
        .expect("manifest JSON"),
    )
    .expect("manifest");
}

fn json_of(output: &Output) -> Value {
    assert!(output.status.success(), "{}", error(output));
    serde_json::from_slice(&output.stdout).expect("JSON")
}

fn history(fixture: &Fixture, extra: &[&str]) -> Vec<Value> {
    let mut args = vec!["--json", "history"];
    args.extend_from_slice(extra);
    json_of(&fixture.run(&args))
        .as_array()
        .expect("history array")
        .clone()
}

fn release(operation: &Value, key: &str) -> Value {
    operation["packages"][0][key].clone()
}

#[test]
fn install_update_and_uninstall_each_record_one_operation() {
    let fixture = Fixture::new();
    let (v1, v1_hash) = archive(&fixture, "v1", "one");
    manifest(&fixture, "example", "1.0", &v1_hash, "v1.zip");
    fixture.bucket("deps", "test-game", "example", &v1_hash, &[]);
    fixture.register();
    let installed = fixture.run_paths(
        &["--json", "install", "example", "--yes", "--file"],
        &[v1.as_path()],
    );
    assert!(installed.status.success(), "{}", error(&installed));

    let operations = history(&fixture, &[]);
    assert_eq!(operations.len(), 1, "{operations:?}");
    let install = &operations[0];
    assert_eq!(install["kind"], "install");
    assert_eq!(install["game_id"], "test-game");
    assert_eq!(install["packages"][0]["mod_id"], "example");
    assert_eq!(release(install, "release_after")["version"], "1.0");
    assert_eq!(release(install, "release_after")["archive_blake3"], v1_hash);
    assert_eq!(
        install["packages"][0]["files"][0]["digest_before"],
        Value::Null
    );
    assert!(install["packages"][0]["files"][0]["digest_after"].is_string());

    let (v2, v2_hash) = archive(&fixture, "v2", "two");
    manifest(&fixture, "example", "2.0", &v2_hash, "v2.zip");
    let updated = fixture.run_paths(
        &["--json", "update", "example", "--yes", "--file"],
        &[v2.as_path()],
    );
    assert!(updated.status.success(), "{}", error(&updated));
    let operations = history(&fixture, &[]);
    assert_eq!(operations.len(), 2);
    assert_eq!(operations[0]["kind"], "update");
    assert_eq!(release(&operations[0], "release_before")["version"], "1.0");
    assert_eq!(release(&operations[0], "release_after")["version"], "2.0");
    assert!(operations[0]["packages"][0]["files"][0]["digest_before"].is_string());
    assert!(operations[0]["packages"][0]["files"][0]["digest_after"].is_string());

    let removed = fixture.run(&["uninstall", "example", "--yes"]);
    assert!(removed.status.success(), "{}", error(&removed));
    let operations = history(&fixture, &[]);
    assert_eq!(operations.len(), 3);
    assert_eq!(operations[0]["kind"], "uninstall");
    assert_eq!(release(&operations[0], "release_before")["version"], "2.0");
    assert!(release(&operations[0], "release_after").is_null());
    assert!(operations[0]["packages"][0]["files"][0]["digest_before"].is_string());
    assert!(operations[0]["packages"][0]["files"][0]["digest_after"].is_null());

    assert_eq!(history(&fixture, &["--game", "test-game"]).len(), 3);
    assert!(history(&fixture, &["--game", "other-game"]).is_empty());
    let text = fixture.run(&["history"]);
    assert!(text.status.success(), "{}", error(&text));
    let text = String::from_utf8(text.stdout).expect("text");
    assert!(text.contains("ID") && text.contains("KIND") && text.contains("uninstall"));
}

#[test]
fn dry_run_refusal_and_empty_state_write_no_history() {
    let fixture = Fixture::new();
    let (archive, hash) = archive(&fixture, "example", "one");
    manifest(&fixture, "example", "1.0", &hash, "example.zip");
    fixture.bucket("deps", "test-game", "example", &hash, &[]);
    fixture.register();
    assert!(history(&fixture, &[]).is_empty());

    let dry_run = fixture.run_paths(
        &["install", "example", "--dry-run", "--file"],
        &[archive.as_path()],
    );
    assert!(dry_run.status.success(), "{}", error(&dry_run));
    assert!(history(&fixture, &[]).is_empty());

    let refused = fixture.run(&["--json", "uninstall", "example"]);
    assert!(!refused.status.success());
    assert!(history(&fixture, &[]).is_empty());

    let fresh = tempfile::tempdir().expect("fresh state");
    let state = fresh.path().join("state");
    let output = main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&state)
        .args(["--json", "history"])
        .output()
        .expect("history command");
    assert!(output.status.success(), "{}", error(&output));
    let operations: Value = serde_json::from_slice(&output.stdout).expect("JSON");
    assert_eq!(operations, json!([]));
    assert!(!Path::new(&state).exists());
}
