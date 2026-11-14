//! Shared helpers for the split undo release metadata test files.
#![allow(
    dead_code,
    reason = "each split test file uses a different subset of these helpers"
)]

use crate::main_bucket;
use crate::support::{error, Fixture};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Output;
use zip::write::SimpleFileOptions;

pub fn archive(fixture: &Fixture, name: &str, files: &[(&str, &str)]) -> (PathBuf, String) {
    let path = fixture.temp.path().join(format!("{name}.zip"));
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).expect("archive"));
    for (member, contents) in files {
        zip.start_file(*member, SimpleFileOptions::default())
            .expect("entry");
        zip.write_all(contents.as_bytes()).expect("payload");
    }
    zip.finish().expect("finish");
    let digest = blake3::hash(&fs::read(&path).expect("bytes"));
    (path, digest.to_hex().to_string())
}

pub fn ensure_bucket(fixture: &Fixture, bucket: &str) {
    let root = fixture.temp.path().join(bucket);
    let marker = fixture.temp.path().join(format!(".{bucket}-registered"));
    if marker.exists() {
        return;
    }
    fs::create_dir_all(root.join("games")).expect("bucket directories");
    fs::write(
        root.join("bucket.json"),
        serde_json::to_vec(&json!({
            "schema_version": 2, "name": bucket, "title": "Undo fixture"
        }))
        .expect("bucket JSON"),
    )
    .expect("bucket metadata");
    let added = fixture.run_paths(&["bucket", "add", bucket], &[&root]);
    assert!(added.status.success(), "{}", error(&added));
    fs::write(marker, b"1").expect("bucket marker");
}

#[expect(
    clippy::too_many_arguments,
    reason = "the manifest fixture names every field a test varies"
)]
pub fn write_manifest(
    fixture: &Fixture,
    bucket: &str,
    game: &str,
    id: &str,
    version: &str,
    hash: &str,
    dependencies: &[&str],
    executables: &[&str],
    native_injector: bool,
    source: Value,
) {
    ensure_bucket(fixture, bucket);
    let folder = fixture.temp.path().join(bucket).join("mods").join(game);
    fs::create_dir_all(&folder).expect("bucket folder");
    fs::write(
        folder.join(format!("{id}.json")),
        serde_json::to_vec(&json!({
            "schema_version": 2, "id": id, "game": game, "name": id,
            "version": version, "archive_format": "zip", "blake3": hash,
            "sources": [source], "dependencies": dependencies,
            "executables": executables, "native_injector": native_injector
        }))
        .expect("record JSON"),
    )
    .expect("record");
}

pub fn https(url: &str) -> Value {
    json!({"kind": "https", "url": url})
}

pub fn declare_conflicts(
    fixture: &Fixture,
    bucket: &str,
    game: &str,
    id: &str,
    conflicts: &[&str],
) {
    let path = fixture
        .temp
        .path()
        .join(bucket)
        .join("mods")
        .join(game)
        .join(format!("{id}.json"));
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(&path).expect("record")).expect("record JSON");
    manifest["conflicts"] = json!(conflicts);
    fs::write(&path, serde_json::to_vec(&manifest).expect("JSON")).expect("record");
}

pub fn cache(fixture: &Fixture, game: &str, id: &str, version: &str, url: &str, archive: &Path) {
    let root = fixture.state.join("cache");
    fs::create_dir_all(&root).expect("cache");
    let hash = format!("{:x}", Sha256::digest(url.as_bytes()));
    fs::copy(
        archive,
        root.join(format!("{game}#{id}#{version}#{}.zip", &hash[..7])),
    )
    .expect("cache entry");
}

pub fn run(fixture: &Fixture, args: &[&str], files: &[&Path]) -> Output {
    let mut command = main_bucket::command(env!("CARGO_BIN_EXE_modn"));
    command
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .args(args);
    for file in files {
        command.arg("--file").arg(file);
    }
    command.output().expect("run modn")
}

pub fn json_of(output: &Output) -> Value {
    assert!(output.status.success(), "{}", error(output));
    serde_json::from_slice(&output.stdout).expect("JSON")
}

pub fn history(fixture: &Fixture) -> Vec<Value> {
    json_of(&fixture.run(&["--json", "history"]))
        .as_array()
        .expect("history array")
        .clone()
}

pub fn id_of(operation: &Value) -> String {
    operation["id"].as_str().expect("operation ID").to_owned()
}

pub fn error_code(output: &Output) -> String {
    let value: Value = serde_json::from_slice(&output.stderr)
        .unwrap_or_else(|_| panic!("error JSON: {}", error(output)));
    value["error"].as_str().expect("error code").to_owned()
}

pub fn state_database(fixture: &Fixture) -> rusqlite::Connection {
    rusqlite::Connection::open(fixture.state.join("state.sqlite")).expect("state database")
}

pub fn remove_declarations(fixture: &Fixture, operation: &str, side: &str) {
    let database = state_database(fixture);
    let column = format!("release_{side}");
    let text: String = database
        .query_row(
            &format!("SELECT {column} FROM history_packages WHERE operation_id=?1"),
            [operation],
            |row| row.get(0),
        )
        .expect("release JSON");
    let mut release: Value = serde_json::from_str(&text).expect("release object");
    release
        .as_object_mut()
        .expect("release object")
        .remove("declarations");
    database
        .execute(
            &format!("UPDATE history_packages SET {column}=?1 WHERE operation_id=?2"),
            rusqlite::params![release.to_string(), operation],
        )
        .expect("rewrite release");
}

/// API 1 test-game policy that maps a declared tool executable beside plugins.
pub fn tool_module(fixture: &Fixture) {
    fs::write(
        fixture.modules.join("test-game/main.lua"),
        r#"
local fs = mods_host.fs

local policy = {
    game_id = "test-game",
    api_version = 1,
    marker = "bin/TestGame.exe",
    common_paths = {},
}
function policy.validate()
    local ok, info = pcall(fs.metadata, 1, "bin/TestGame.exe")
    return ok and info.kind == "file"
end
function policy.steam_name() return "Test Game" end
function policy.is_epic() return false end
function policy.scan()
    return { mods = {}, framework_signals = {}, incomplete = false, redmod_available = false }
end
function policy.doctor() return {} end
function policy.plan(files)
    local operations = {}
    for _, file in ipairs(files) do
        if file.source == "engine/tools/scc.exe" then
            table.insert(operations, {
                source = file.source,
                destination = "engine/tools/scc.exe",
                kind = "tool",
                package_root = "",
            })
        elseif file.source == "Demo/package/plugins/example.plugin" then
            table.insert(operations, {
                source = file.source,
                destination = "addons/example.plugin",
                kind = "test-plugin",
                package_root = "",
            })
        end
    end
    return { schema_version = 1, operations = operations, blockers = {} }
end
return policy
"#,
    )
    .expect("tool module");
}

pub fn install(fixture: &Fixture, target: &str, archive: &Path, extra: &[&str]) -> Output {
    let mut args = vec!["--json", "install", target, "--yes"];
    args.extend_from_slice(extra);
    run(fixture, &args, &[archive])
}
