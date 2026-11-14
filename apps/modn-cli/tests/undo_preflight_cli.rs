//! `modn undo` runs the same preflight checks in `--dry-run` as in the real
//! run, checks restored bytes before an uninstall undo changes a game file,
//! and honors cancellation while it reads archives for those checks.

#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/catalog.rs"]
mod support;
#[path = "support/tree.rs"]
#[allow(dead_code)]
mod tree;

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
    for (member, contents) in files {
        zip.start_file(*member, SimpleFileOptions::default())
            .expect("entry");
        zip.write_all(contents.as_bytes()).expect("payload");
    }
    zip.finish().expect("finish");
    let digest = blake3::hash(&fs::read(&path).expect("bytes"))
        .to_hex()
        .to_string();
    (path, digest)
}

fn ensure_bucket(fixture: &Fixture, bucket: &str) {
    let root = fixture.temp.path().join(bucket);
    let marker = fixture.temp.path().join(format!(".{bucket}-registered"));
    if marker.exists() {
        return;
    }
    fs::create_dir_all(root.join("mods")).expect("bucket directories");
    fs::write(
        root.join("bucket.json"),
        serde_json::to_vec(&json!({
            "schema_version": 2, "name": bucket, "title": "Undo preflight fixture"
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
            "native_injector": native_injector
        }))
        .expect("manifest JSON"),
    )
    .expect("manifest");
}

fn https(url: &str) -> Value {
    json!({"kind": "https", "url": url})
}

fn cache(fixture: &Fixture, game: &str, id: &str, version: &str, url: &str, archive: &Path) {
    let root = fixture.state.join("cache");
    fs::create_dir_all(&root).expect("cache");
    let hash = blake3::hash(url.as_bytes()).to_hex().to_string();
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

fn state_counts(fixture: &Fixture) -> Vec<(String, i64)> {
    let database = state_database(fixture);
    [
        "managed_packages",
        "managed_files",
        "managed_directories",
        "history_operations",
        "history_packages",
        "history_files",
        "pending_transactions",
        "pending_stacks",
        "pending_stack_packages",
        "mod_approvals",
    ]
    .into_iter()
    .map(|table| {
        let count = database
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or_else(|_| panic!("count {table}"));
        (table.to_owned(), count)
    })
    .collect()
}

fn policy_script(api_version: u32, selected: &str, root: &str) -> String {
    format!(
        r#"
local fs = mods_host.fs

return {{
    game_id = "test-game",
    api_version = {api_version},
    marker = "bin/TestGame.exe",
    common_paths = {{}},
    validate = function()
        local ok, info = pcall(fs.metadata, 1, "bin/TestGame.exe")
        return ok and info.kind == "file"
    end,
    steam_name = function() return "Test Game" end,
    is_epic = function(name) return false end,
    scan = function()
        return {{ mods = {{}}, framework_signals = {{}}, incomplete = false, redmod_available = false }}
    end,
    doctor = function() return {{}} end,
    plan = function(files)
        local operations = {{}}
        for _, file in ipairs(files) do
            if file.source == "{selected}" or file.source == "v2/example.plugin" then
                table.insert(operations, {{
                    source = file.source,
                    destination = "addons/example.plugin",
                    kind = "test-plugin",
                    package_root = "",
                    root = "{root}",
                }})
            end
        end
        return {{ schema_version = 1, operations = operations, blockers = {{}} }}
    end,
}}
"#
    )
}

fn write_module(fixture: &Fixture, api_version: u32, selected: &str, root: &str, content: bool) {
    let dir = fixture.modules.join("test-game");
    let declared = if content {
        r#","content_root":{"base":"appdata","path":"Fixture/Mods"}"#
    } else {
        ""
    };
    fs::write(
        dir.join("module.json"),
        format!(
            r#"{{"game_id":"test-game","version":"0.1.0","api_version":{api_version},"entrypoint":"main.lua"{declared}}}"#
        ),
    )
    .expect("module manifest");
    fs::write(
        dir.join("main.lua"),
        policy_script(api_version, selected, root),
    )
    .expect("module script");
}

fn refused_assignment(fixture: &Fixture) -> (PathBuf, PathBuf) {
    let (v1, v1_hash) = archive(
        fixture,
        "v1",
        &[
            ("v1/example.plugin", "one"),
            ("v1-alt/example.plugin", "evil"),
        ],
    );
    let (v2, v2_hash) = archive(fixture, "v2", &[("v2/example.plugin", "two")]);
    let v1_url = "https://example.invalid/example-1.0.zip";
    let v2_url = "https://example.invalid/example-2.0.zip";
    write_manifest(
        fixture,
        "deps",
        "test-game",
        "example",
        "1.0",
        &v1_hash,
        &[],
        false,
        https(v1_url),
    );
    write_module(fixture, 1, "v1/example.plugin", "game", false);
    fixture.register();
    let installed = run(fixture, &["--json", "install", "example", "--yes"], &[&v1]);
    assert!(installed.status.success(), "{}", error(&installed));
    cache(fixture, "test-game", "example", "1.0", v1_url, &v1);
    write_manifest(
        fixture,
        "deps",
        "test-game",
        "example",
        "2.0",
        &v2_hash,
        &[],
        false,
        https(v2_url),
    );
    (v1, v2)
}

#[test]
fn dry_run_update_undo_refuses_a_native_injector_without_approval() {
    let fixture = Fixture::new();
    let (v1, v1_hash) = archive(&fixture, "native-v1", &[("winmm.dll", "one")]);
    let (v2, v2_hash) = archive(&fixture, "native-v2", &[("winmm.dll", "two")]);
    let v1_url = "https://example.invalid/native-1.0.zip";
    let v2_url = "https://example.invalid/native-2.0.zip";
    write_manifest(
        &fixture,
        "deps",
        "native-injector-game",
        "injector",
        "1.0",
        &v1_hash,
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
    let installed = run(
        &fixture,
        &[
            "--json",
            "install",
            "injector",
            "--yes",
            "--allow-native-injector",
        ],
        &[&v1],
    );
    assert!(installed.status.success(), "{}", error(&installed));
    write_manifest(
        &fixture,
        "deps",
        "native-injector-game",
        "injector",
        "2.0",
        &v2_hash,
        &[],
        true,
        https(v2_url),
    );
    let updated = run(&fixture, &["--json", "update", "injector", "--yes"], &[&v2]);
    assert!(updated.status.success(), "{}", error(&updated));
    let target = id_of(&history(&fixture)[0]);
    state_database(&fixture)
        .execute("DELETE FROM mod_approvals", [])
        .expect("drop stored approval");

    let before_tree = tree::snapshot(&fixture.native_game);
    let before_history = history(&fixture);
    let before_counts = state_counts(&fixture);

    let refused = fixture.run(&["--json", "undo", &target, "--dry-run"]);
    assert!(!refused.status.success(), "{}", error(&refused));
    assert_eq!(error_code(&refused), "NATIVE_INJECTOR_NOT_ALLOWED");
    assert_eq!(tree::snapshot(&fixture.native_game), before_tree);
    assert_eq!(history(&fixture), before_history);
    assert_eq!(state_counts(&fixture), before_counts);

    let allowed = fixture.run(&[
        "--json",
        "undo",
        &target,
        "--dry-run",
        "--allow-native-injector",
    ]);
    assert!(allowed.status.success(), "{}", error(&allowed));
    assert_eq!(tree::snapshot(&fixture.native_game), before_tree);
}

#[test]
fn dry_run_update_undo_refuses_a_changed_member_selection() {
    let fixture = Fixture::new();
    let (_v1, v2) = refused_assignment(&fixture);
    let updated = run(&fixture, &["--json", "update", "example", "--yes"], &[&v2]);
    assert!(updated.status.success(), "{}", error(&updated));
    let target = id_of(&history(&fixture)[0]);
    write_module(&fixture, 1, "v1-alt/example.plugin", "game", false);

    let before_tree = tree::snapshot(&fixture.game);
    let before_history = history(&fixture);
    let before_list = json_of(&fixture.run(&["--json", "list"]));
    let before_counts = state_counts(&fixture);
    let refused = fixture.run(&["--json", "undo", &target, "--dry-run"]);
    assert!(!refused.status.success(), "{}", error(&refused));
    assert_eq!(error_code(&refused), "UNDO_FAILED");
    assert!(error(&refused).contains("bytes"), "{}", error(&refused));
    assert_eq!(tree::snapshot(&fixture.game), before_tree);
    assert_eq!(history(&fixture), before_history);
    assert_eq!(json_of(&fixture.run(&["--json", "list"])), before_list);
    assert_eq!(state_counts(&fixture), before_counts);
}

#[test]
fn uninstall_undo_checks_restored_bytes_before_any_change() {
    let fixture = Fixture::new();
    let (_v1, _v2) = refused_assignment(&fixture);
    let removed = fixture.run(&["uninstall", "example", "--yes"]);
    assert!(removed.status.success(), "{}", error(&removed));
    let target = id_of(&history(&fixture)[0]);
    assert_eq!(history(&fixture)[0]["kind"], "uninstall");
    write_module(&fixture, 1, "v1-alt/example.plugin", "game", false);

    let before_tree = tree::snapshot(&fixture.game);
    let before_history = history(&fixture);
    let before_list = json_of(&fixture.run(&["--json", "list"]));
    let before_counts = state_counts(&fixture);
    let refused = fixture.run(&["--json", "undo", &target, "--yes"]);
    assert!(!refused.status.success(), "{}", error(&refused));
    assert_eq!(error_code(&refused), "UNDO_FAILED");
    assert!(error(&refused).contains("bytes"), "{}", error(&refused));
    assert!(
        error(&refused).contains("no change made"),
        "{}",
        error(&refused)
    );
    assert_eq!(tree::snapshot(&fixture.game), before_tree);
    assert_eq!(history(&fixture), before_history);
    assert_eq!(json_of(&fixture.run(&["--json", "list"])), before_list);
    assert_eq!(state_counts(&fixture), before_counts);
    assert_eq!(
        state_counts(&fixture)
            .into_iter()
            .find(|(table, _)| table == "pending_transactions")
            .expect("pending count")
            .1,
        0
    );
}

fn cancelled_undo(fixture: &Fixture, target: &str) -> Output {
    let mut command = main_bucket::command(env!("CARGO_BIN_EXE_modn"));
    command
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .args(["--json", "--progress", "json", "undo", target, "--yes"])
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
    child.wait_with_output().expect("modn output")
}

#[test]
fn cancelled_undo_changes_no_file_or_manager_record() {
    let fixture = Fixture::new();
    let (_v1, v2) = refused_assignment(&fixture);
    let updated = run(&fixture, &["--json", "update", "example", "--yes"], &[&v2]);
    assert!(updated.status.success(), "{}", error(&updated));
    let target = id_of(&history(&fixture)[0]);

    let before_tree = tree::snapshot(&fixture.game);
    let before_history = history(&fixture);
    let before_list = json_of(&fixture.run(&["--json", "list"]));
    let before_counts = state_counts(&fixture);
    let before_cache = tree::snapshot(&fixture.state.join("cache"));

    let cancelled = cancelled_undo(&fixture, &target);
    assert!(!cancelled.status.success());
    let stderr = String::from_utf8_lossy(&cancelled.stderr);
    assert!(stderr.contains(r#""error":"CANCELLED""#), "{stderr}");
    assert_eq!(tree::snapshot(&fixture.game), before_tree);
    assert_eq!(history(&fixture), before_history);
    assert_eq!(json_of(&fixture.run(&["--json", "list"])), before_list);
    assert_eq!(state_counts(&fixture), before_counts);
    assert_eq!(tree::snapshot(&fixture.state.join("cache")), before_cache);
}
