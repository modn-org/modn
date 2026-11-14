//! Documented refusals report their specific error codes.

#[path = "support/catalog.rs"]
mod support;

#[path = "support/main_bucket.rs"]
mod main_bucket;

use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Output;
use support::{error, Fixture};
use zip::write::SimpleFileOptions;

fn error_code(output: &Output) -> String {
    let value: Value = serde_json::from_slice(&output.stderr).expect("JSON error");
    value["error"].as_str().expect("error code").to_owned()
}

fn archive(fixture: &Fixture, member: &str, bytes: &[u8]) -> (PathBuf, String) {
    let path = fixture
        .temp
        .path()
        .join(format!("{}.zip", member.replace('/', "-")));
    let file = fs::File::create(&path).expect("archive");
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file(member, SimpleFileOptions::default())
        .expect("archive entry");
    zip.write_all(bytes).expect("archive payload");
    zip.finish().expect("archive finish");
    let digest = blake3::hash(&fs::read(&path).expect("archive bytes"))
        .to_hex()
        .to_string();
    (path, digest)
}

fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().expect("parent")).expect("fixture folder");
    fs::write(path, bytes).expect("fixture file");
}

#[test]
fn undo_with_a_pending_transaction_reports_pending_transaction() {
    let fixture = Fixture::new();
    let (archive, hash) = fixture.archive();
    fixture.bucket("deps", "test-game", "example", &hash, &[]);
    fixture.register();
    let installed = fixture.run_paths(
        &["--json", "install", "example", "--yes", "--file"],
        &[&archive],
    );
    assert!(installed.status.success(), "{}", error(&installed));
    let connection =
        rusqlite::Connection::open(fixture.state.join("state.sqlite")).expect("manager state");
    connection
        .execute(
            "INSERT INTO pending_transactions
             (id, action, status, game_path, package_id, archive_sha256)
             VALUES ('pending', 'install', 'pending', ?1, 'fixture', NULL)",
            [fixture.game.to_string_lossy().as_ref()],
        )
        .expect("pending transaction");

    let output = fixture.run(&["--json", "undo", "--yes"]);

    assert!(!output.status.success());
    assert_eq!(
        error_code(&output),
        "PENDING_TRANSACTION",
        "{}",
        error(&output)
    );
    assert!(
        error(&output).contains("unfinished transaction"),
        "{}",
        error(&output)
    );
}

#[test]
fn uninstalling_a_needed_package_reports_package_needed() {
    let fixture = Fixture::new();
    let (base_archive, base_hash) = archive(&fixture, "Demo/package/plugins/base.plugin", b"base");
    fixture.bucket("deps", "test-game", "base", &base_hash, &[]);
    let (dependent_archive, dependent_hash) = archive(
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
    for (name, archive) in [("base", &base_archive), ("dependent", &dependent_archive)] {
        let installed =
            fixture.run_paths(&["--json", "install", name, "--yes", "--file"], &[archive]);
        assert!(
            installed.status.success(),
            "install {name}: {}",
            error(&installed)
        );
    }

    let output = fixture.run(&["--json", "uninstall", "base", "--yes"]);

    assert!(!output.status.success());
    assert_eq!(error_code(&output), "PACKAGE_NEEDED", "{}", error(&output));
    assert!(fixture.game.join("addons/base.plugin").is_file());
}

#[test]
fn a_module_with_a_bad_store_identity_reports_module_store_id_invalid() {
    let temp = tempfile::tempdir().expect("fixture");
    let module = temp.path().join("bad-store");
    write(
        &module.join("module.json"),
        br#"{"game_id":"bad-store","version":"0.1.0","api_version":1,"entrypoint":"main.lua","stores":{"steam":"10915a0"}}"#,
    );
    write(
        &module.join("main.lua"),
        br#"return { game_id = "bad-store", api_version = mods_host.api_version }"#,
    );

    let output = main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(temp.path().join("state"))
        .args(["module", "check"])
        .arg(&module)
        .arg("--json")
        .output()
        .expect("run modn");

    assert!(!output.status.success());
    assert_eq!(
        error_code(&output),
        "MODULE_STORE_ID_INVALID",
        "{}",
        error(&output)
    );
}
