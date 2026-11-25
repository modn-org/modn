#![allow(
    dead_code,
    reason = "each integration-test crate uses a different subset of these helpers"
)]
use crate::main_bucket;
use crate::support::{error, Fixture};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Output;
use zip::write::SimpleFileOptions;

pub fn archive(fixture: &Fixture, name: &str, files: &[(&str, &str)]) -> (PathBuf, String) {
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

pub fn manifest(fixture: &Fixture, id: &str, version: &str, hash: &str, source: Value) {
    manifest_extra(fixture, id, version, hash, source, json!({}));
}

pub fn manifest_extra(
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

pub fn https(url: &str) -> Value {
    json!({"kind": "https", "url": url})
}

pub fn nexus() -> Value {
    json!({
        "kind": "nexus", "game_domain": "testgame", "mod_id": 12,
        "file_id": 34, "file_name": "example.zip"
    })
}

pub fn nexus_url() -> &'static str {
    "nexus://testgame/mods/12/files/34/example.zip"
}

pub fn cache(fixture: &Fixture, id: &str, version: &str, url: &str, archive: &Path) {
    let root = fixture.state.join("cache");
    fs::create_dir_all(&root).expect("cache");
    let hash = blake3::hash(url.as_bytes()).to_hex().to_string();
    fs::copy(
        archive,
        root.join(format!("test-game#{id}#{version}#{}.zip", &hash[..7])),
    )
    .expect("cache entry");
}

pub fn install(fixture: &Fixture, target: &str, files: &[&Path]) -> Output {
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
