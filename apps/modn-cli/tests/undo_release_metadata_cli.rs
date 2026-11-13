//! Undo restores the catalog identity, dependency links, declared conflicts
//! and native-injector status that the history record holds.
#![allow(dead_code, reason = "helpers serve different tests in this crate")]

#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/catalog.rs"]
mod support;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Output;
use support::{error, Fixture};
use zip::write::SimpleFileOptions;

fn archive(fixture: &Fixture, name: &str, files: &[(&str, &str)]) -> (PathBuf, String) {
    let path = fixture.temp.path().join(format!("{name}.zip"));
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).expect("archive"));
    for (member, contents) in files {
        zip.start_file(*member, SimpleFileOptions::default())
            .expect("entry");
        zip.write_all(contents.as_bytes()).expect("payload");
    }
    zip.finish().expect("finish");
    let digest = Sha256::digest(fs::read(&path).expect("bytes"));
    (path, format!("{digest:x}"))
}

fn ensure_bucket(fixture: &Fixture, bucket: &str) {
    let root = fixture.temp.path().join(bucket);
    let marker = fixture.temp.path().join(format!(".{bucket}-registered"));
    if marker.exists() {
        return;
    }
    fs::create_dir_all(root.join("games")).expect("bucket directories");
    fs::write(
        root.join("bucket.json"),
        serde_json::to_vec(&json!({
            "schema_version": 1, "name": bucket, "title": "Undo fixture"
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
fn write_manifest(
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
    let folder = fixture.temp.path().join(bucket).join("games").join(game);
    fs::create_dir_all(&folder).expect("bucket folder");
    fs::write(
        folder.join(format!("{id}.json")),
        serde_json::to_vec(&json!({
            "schema_version": 1, "id": id, "game": game, "name": id,
            "version": version, "archive_format": "zip", "sha256": hash,
            "source": source, "dependencies": dependencies,
            "executables": executables, "native_injector": native_injector
        }))
        .expect("manifest JSON"),
    )
    .expect("manifest");
}

fn https(url: &str) -> Value {
    json!({"kind": "https", "url": url})
}

fn declare_conflicts(fixture: &Fixture, bucket: &str, game: &str, id: &str, conflicts: &[&str]) {
    let path = fixture
        .temp
        .path()
        .join(bucket)
        .join("games")
        .join(game)
        .join(format!("{id}.json"));
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(&path).expect("manifest")).expect("manifest JSON");
    manifest["conflicts"] = json!(conflicts);
    fs::write(&path, serde_json::to_vec(&manifest).expect("JSON")).expect("manifest");
}

fn cache(fixture: &Fixture, game: &str, id: &str, version: &str, url: &str, archive: &Path) {
    let root = fixture.state.join("cache");
    fs::create_dir_all(&root).expect("cache");
    let hash = format!("{:x}", Sha256::digest(url.as_bytes()));
    fs::copy(
        archive,
        root.join(format!("{game}#{id}#{version}#{}.zip", &hash[..7])),
    )
    .expect("cache entry");
}

fn run(fixture: &Fixture, args: &[&str], files: &[&Path]) -> Output {
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

fn state_database(fixture: &Fixture) -> rusqlite::Connection {
    rusqlite::Connection::open(fixture.state.join("state.sqlite")).expect("state database")
}

fn remove_declarations(fixture: &Fixture, operation: &str, side: &str) {
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
fn tool_module(fixture: &Fixture) {
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

fn install(fixture: &Fixture, target: &str, archive: &Path, extra: &[&str]) -> Output {
    let mut args = vec!["--json", "install", target, "--yes"];
    args.extend_from_slice(extra);
    run(fixture, &args, &[archive])
}

#[test]
fn undo_of_an_update_restores_the_recorded_dependency_link() {
    let fixture = Fixture::new();
    let (base, base_hash) = archive(
        &fixture,
        "base",
        &[("Demo/package/plugins/base.plugin", "base")],
    );
    let (top_v1, top_v1_hash) = archive(
        &fixture,
        "top-v1",
        &[("Demo/package/plugins/top.plugin", "one")],
    );
    let (top_v2, top_v2_hash) = archive(
        &fixture,
        "top-v2",
        &[("Demo/package/plugins/top.plugin", "two")],
    );
    let (rival, rival_hash) = archive(
        &fixture,
        "rival",
        &[("Demo/package/plugins/rival.plugin", "rival")],
    );
    let base_url = "https://example.invalid/base-1.0.zip";
    let top_url = "https://example.invalid/top-1.0.zip";
    write_manifest(
        &fixture,
        "deps",
        "test-game",
        "base",
        "1.0",
        &base_hash,
        &[],
        &[],
        false,
        https(base_url),
    );
    write_manifest(
        &fixture,
        "deps",
        "test-game",
        "top",
        "1.0",
        &top_v1_hash,
        &["base"],
        &[],
        false,
        https(top_url),
    );
    declare_conflicts(&fixture, "deps", "test-game", "top", &["rival"]);
    write_manifest(
        &fixture,
        "deps",
        "test-game",
        "rival",
        "1.0",
        &rival_hash,
        &[],
        &[],
        false,
        json!({"kind": "manual", "filename": "rival.zip"}),
    );
    fixture.register();

    let installed_base = install(&fixture, "base", &base, &[]);
    assert!(
        installed_base.status.success(),
        "{}",
        error(&installed_base)
    );
    let installed_top = install(&fixture, "top", &top_v1, &[]);
    assert!(installed_top.status.success(), "{}", error(&installed_top));

    write_manifest(
        &fixture,
        "deps",
        "test-game",
        "top",
        "2.0",
        &top_v2_hash,
        &["base"],
        &[],
        false,
        https("https://example.invalid/top-2.0.zip"),
    );
    declare_conflicts(&fixture, "deps", "test-game", "top", &["rival"]);
    let updated = run(&fixture, &["--json", "update", "top", "--yes"], &[&top_v2]);
    assert!(updated.status.success(), "{}", error(&updated));
    cache(&fixture, "test-game", "top", "1.0", top_url, &top_v1);
    let operations = history(&fixture);
    let target = id_of(&operations[0]);
    assert_eq!(operations[0]["kind"], "update");

    let undone = fixture.run(&["--json", "undo", &target, "--yes"]);
    assert!(undone.status.success(), "{}", error(&undone));

    let refused = fixture.run(&["--json", "uninstall", "base", "--yes"]);
    assert!(
        !refused.status.success(),
        "the framework must be needed again"
    );
    assert_eq!(error_code(&refused), "PACKAGE_NEEDED");

    let conflict = install(&fixture, "rival", &rival, &[]);
    assert!(
        !conflict.status.success(),
        "the restored conflict blocks it"
    );
    assert!(
        error(&conflict).contains("conflict"),
        "{}",
        error(&conflict)
    );
    assert!(!fixture.game.join("addons/rival.plugin").exists());
}

#[test]
fn undo_of_an_uninstall_restores_catalog_identity_and_dependency_links() {
    let fixture = Fixture::new();
    let (base, base_hash) = archive(
        &fixture,
        "base",
        &[("Demo/package/plugins/base.plugin", "base")],
    );
    let (top_v1, top_v1_hash) = archive(
        &fixture,
        "top-v1",
        &[("Demo/package/plugins/top.plugin", "one")],
    );
    let (top_v2, top_v2_hash) = archive(
        &fixture,
        "top-v2",
        &[("Demo/package/plugins/top.plugin", "two")],
    );
    let top_url = "https://example.invalid/top-1.0.zip";
    write_manifest(
        &fixture,
        "deps",
        "test-game",
        "base",
        "1.0",
        &base_hash,
        &[],
        &[],
        false,
        https("https://example.invalid/base-1.0.zip"),
    );
    write_manifest(
        &fixture,
        "deps",
        "test-game",
        "top",
        "1.0",
        &top_v1_hash,
        &["base"],
        &[],
        false,
        https(top_url),
    );
    fixture.register();
    assert!(install(&fixture, "base", &base, &[]).status.success());
    assert!(install(&fixture, "top", &top_v1, &[]).status.success());
    cache(&fixture, "test-game", "top", "1.0", top_url, &top_v1);

    let removed = fixture.run(&["--json", "uninstall", "top", "--yes"]);
    assert!(removed.status.success(), "{}", error(&removed));
    let target = id_of(&history(&fixture)[0]);
    let undone = fixture.run(&["--json", "undo", &target, "--yes"]);
    assert!(undone.status.success(), "{}", error(&undone));

    let refused = fixture.run(&["--json", "uninstall", "base", "--yes"]);
    assert!(
        !refused.status.success(),
        "the restored dependency blocks it"
    );
    assert_eq!(error_code(&refused), "PACKAGE_NEEDED");

    let packages = json_of(&fixture.run(&["--json", "list"]));
    let top = packages
        .as_array()
        .expect("list")
        .iter()
        .find(|package| package["mod_id"] == "top")
        .expect("restored top package");
    assert_eq!(top["catalog"]["bucket"], "deps");
    assert_eq!(top["catalog"]["game_id"], "test-game");
    assert_eq!(top["catalog"]["mod_id"], "top");

    write_manifest(
        &fixture,
        "deps",
        "test-game",
        "top",
        "2.0",
        &top_v2_hash,
        &["base"],
        &[],
        false,
        https("https://example.invalid/top-2.0.zip"),
    );
    let updated = run(&fixture, &["--json", "update", "top", "--yes"], &[&top_v2]);
    assert!(updated.status.success(), "{}", error(&updated));
}

#[test]
fn undo_of_an_uninstall_restores_an_approved_tool_executable() {
    let fixture = Fixture::new();
    tool_module(&fixture);
    let (tool, tool_hash) = archive(&fixture, "tool", &[("engine/tools/scc.exe", "compiler")]);
    let url = "https://example.invalid/tool-1.0.zip";
    write_manifest(
        &fixture,
        "deps",
        "test-game",
        "tool",
        "1.0",
        &tool_hash,
        &[],
        &["engine/tools/scc.exe"],
        false,
        https(url),
    );
    fixture.register();
    cache(&fixture, "test-game", "tool", "1.0", url, &tool);
    let installed = install(
        &fixture,
        "tool",
        &tool,
        &["--allow-executable", "engine/tools/scc.exe"],
    );
    assert!(installed.status.success(), "{}", error(&installed));
    let executable = fixture.game.join("engine/tools/scc.exe");
    assert!(executable.is_file());

    assert!(fixture
        .run(&["uninstall", "tool", "--yes"])
        .status
        .success());
    assert!(!executable.exists());
    let target = id_of(&history(&fixture)[0]);
    let undone = fixture.run(&[
        "--json",
        "undo",
        &target,
        "--yes",
        "--allow-executable",
        "engine/tools/scc.exe",
    ]);
    assert!(undone.status.success(), "{}", error(&undone));
    assert!(executable.is_file());

    // The install stored the approval, and uninstall keeps it, so a second
    // restore needs no flag.
    assert!(fixture
        .run(&["uninstall", "tool", "--yes"])
        .status
        .success());
    let target = id_of(&history(&fixture)[0]);
    let stored = fixture.run(&["--json", "undo", &target, "--yes"]);
    assert!(stored.status.success(), "{}", error(&stored));
    assert!(executable.is_file());
}

#[test]
fn undo_of_an_update_and_an_uninstall_enforces_the_native_injector_policy() {
    let fixture = Fixture::new();
    let (v1, v1_hash) = archive(&fixture, "native-v1", &[("winmm.dll", "one")]);
    let (v2, v2_hash) = archive(&fixture, "native-v2", &[("winmm.dll", "two")]);
    let v1_url = "https://example.invalid/native-1.0.zip";
    let v2_url = "https://example.invalid/native-2.0.zip";
    write_manifest(
        &fixture,
        "native",
        "native-injector-game",
        "injector",
        "1.0",
        &v1_hash,
        &[],
        &[],
        true,
        https(v1_url),
    );
    fixture.register_native();
    cache(
        &fixture,
        "native-injector-game",
        "injector",
        "1.0",
        v1_url,
        &v1,
    );
    let installed = install(&fixture, "injector", &v1, &["--allow-native-injector"]);
    assert!(installed.status.success(), "{}", error(&installed));
    let library = fixture.native_game.join("winmm.dll");
    assert_eq!(fs::read_to_string(&library).expect("v1"), "one");

    write_manifest(
        &fixture,
        "native",
        "native-injector-game",
        "injector",
        "2.0",
        &v2_hash,
        &[],
        &[],
        true,
        https(v2_url),
    );
    let updated = run(&fixture, &["--json", "update", "injector", "--yes"], &[&v2]);
    assert!(updated.status.success(), "{}", error(&updated));
    let update = id_of(&history(&fixture)[0]);

    state_database(&fixture)
        .execute("DELETE FROM mod_approvals", [])
        .expect("drop stored approval");
    let refused = fixture.run(&["--json", "undo", &update, "--yes"]);
    assert!(!refused.status.success(), "{}", error(&refused));
    assert_eq!(error_code(&refused), "NATIVE_INJECTOR_NOT_ALLOWED");
    assert_eq!(fs::read_to_string(&library).expect("v2 stays"), "two");
    let allowed = fixture.run(&[
        "--json",
        "undo",
        &update,
        "--yes",
        "--allow-native-injector",
    ]);
    assert!(allowed.status.success(), "{}", error(&allowed));
    assert_eq!(fs::read_to_string(&library).expect("v1 restored"), "one");

    assert!(fixture
        .run(&["uninstall", "injector", "--yes"])
        .status
        .success());
    let uninstall = id_of(&history(&fixture)[0]);
    let refused = fixture.run(&["--json", "undo", &uninstall, "--yes"]);
    assert!(!refused.status.success(), "{}", error(&refused));
    assert_eq!(error_code(&refused), "NATIVE_INJECTOR_NOT_ALLOWED");
    assert!(!library.exists());
    let allowed = fixture.run(&[
        "--json",
        "undo",
        &uninstall,
        "--yes",
        "--allow-native-injector",
    ]);
    assert!(allowed.status.success(), "{}", error(&allowed));
    assert_eq!(fs::read_to_string(&library).expect("restored"), "one");
}

#[test]
fn undo_refuses_a_record_without_release_declarations() {
    let fixture = Fixture::new();
    let (v1, v1_hash) = archive(
        &fixture,
        "v1",
        &[("Demo/package/plugins/example.plugin", "one")],
    );
    let url = "https://example.invalid/example-1.0.zip";
    write_manifest(
        &fixture,
        "deps",
        "test-game",
        "example",
        "1.0",
        &v1_hash,
        &[],
        &[],
        false,
        https(url),
    );
    fixture.register();
    cache(&fixture, "test-game", "example", "1.0", url, &v1);
    assert!(install(&fixture, "example", &v1, &[]).status.success());
    assert!(fixture
        .run(&["uninstall", "example", "--yes"])
        .status
        .success());
    let target = id_of(&history(&fixture)[0]);
    remove_declarations(&fixture, &target, "before");

    let before = history(&fixture);
    let refused = fixture.run(&["--json", "undo", &target, "--yes"]);
    assert!(!refused.status.success(), "{}", error(&refused));
    assert_eq!(error_code(&refused), "UNDO_FAILED");
    let message = error(&refused);
    assert!(message.contains("install the release again"), "{message}");
    assert!(!fixture.game.join("addons/example.plugin").exists());
    assert_eq!(history(&fixture), before);
}

#[test]
fn undo_of_an_update_refuses_a_record_without_release_declarations() {
    let fixture = Fixture::new();
    let (v1, v1_hash) = archive(
        &fixture,
        "v1",
        &[("Demo/package/plugins/example.plugin", "one")],
    );
    let (v2, v2_hash) = archive(
        &fixture,
        "v2",
        &[("Demo/package/plugins/example.plugin", "two")],
    );
    let url = "https://example.invalid/example-1.0.zip";
    write_manifest(
        &fixture,
        "deps",
        "test-game",
        "example",
        "1.0",
        &v1_hash,
        &[],
        &[],
        false,
        https(url),
    );
    fixture.register();
    assert!(install(&fixture, "example", &v1, &[]).status.success());
    cache(&fixture, "test-game", "example", "1.0", url, &v1);
    write_manifest(
        &fixture,
        "deps",
        "test-game",
        "example",
        "2.0",
        &v2_hash,
        &[],
        &[],
        false,
        json!({"kind": "manual", "filename": "v2.zip"}),
    );
    let updated = run(&fixture, &["--json", "update", "example", "--yes"], &[&v2]);
    assert!(updated.status.success(), "{}", error(&updated));
    let target = id_of(&history(&fixture)[0]);
    remove_declarations(&fixture, &target, "before");

    let addon = fixture.game.join("addons/example.plugin");
    let before = history(&fixture);
    let refused = fixture.run(&["--json", "undo", &target, "--yes"]);
    assert!(!refused.status.success(), "{}", error(&refused));
    assert_eq!(error_code(&refused), "UNDO_FAILED");
    assert!(error(&refused).contains("install the release again"));
    assert_eq!(fs::read_to_string(&addon).expect("v2 stays"), "two");
    assert_eq!(history(&fixture), before);
}

#[test]
fn history_reads_an_older_state_without_release_declarations() {
    let fixture = Fixture::new();
    let (package, digest) = fixture.archive();
    fixture.bucket("deps", "test-game", "example", &digest, &[]);
    fixture.register();
    assert!(install(&fixture, "example", &package, &[]).status.success());
    let database = state_database(&fixture);
    database
        .execute_batch(
            "ALTER TABLE managed_package_releases DROP COLUMN catalog;
             ALTER TABLE managed_package_releases DROP COLUMN declarations;",
        )
        .expect("remove additive columns");
    drop(database);

    let database = fixture.state.join("state.sqlite");
    let before = fs::read(&database).expect("state bytes");
    let operations = json_of(&fixture.run(&["--json", "history"]));
    assert_eq!(
        operations.as_array().expect("history").len(),
        1,
        "history reads the older state"
    );
    assert_eq!(
        fs::read(&database).expect("state bytes"),
        before,
        "read-only history writes nothing"
    );
    let connection = rusqlite::Connection::open(&database).expect("state");
    let columns: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('managed_package_releases')
             WHERE name IN ('catalog', 'declarations')",
            [],
            |row| row.get(0),
        )
        .expect("column count");
    assert_eq!(columns, 0, "read-only history adds no columns");
}
