#[path = "support/catalog.rs"]
mod catalog;
#[path = "support/content_root.rs"]
mod content_root;
#[path = "support/main_bucket.rs"]
mod main_bucket;

use catalog::{error, Fixture};
use serde_json::{json, Value};
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Output;
use zip::write::SimpleFileOptions;

fn archive(fixture: &Fixture, name: &str, files: &[(&str, &str)]) -> (PathBuf, String) {
    let path = fixture.temp.path().join(format!("{name}.zip"));
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).expect("archive"));
    for (file, contents) in files {
        zip.start_file(*file, SimpleFileOptions::default())
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
    let folder = fixture.temp.path().join("deps/mods/test-game");
    fs::create_dir_all(&folder).expect("bucket folder");
    fs::write(
        folder.join(format!("{id}.json")),
        serde_json::to_vec(&json!({
            "schema_version": 2, "id": id, "game": "test-game", "name": id,
            "version": version, "archive_format": "zip", "blake3": hash,
            "sources": [source]
        }))
        .expect("manifest JSON"),
    )
    .expect("manifest");
}

fn https(url: &str) -> Value {
    json!({"kind": "https", "url": url})
}

fn cache(state: &Path, game: &str, id: &str, version: &str, url: &str, archive: &Path) {
    let root = state.join("cache");
    fs::create_dir_all(&root).expect("cache");
    let hash = blake3::hash(url.as_bytes()).to_hex().to_string();
    fs::copy(
        archive,
        root.join(format!("{game}#{id}#{version}#{}.zip", &hash[..7])),
    )
    .expect("cache entry");
}

fn install(fixture: &Fixture, target: &str, archive: &Path) -> Output {
    fixture.run_paths(
        &["--json", "install", target, "--yes", "--file"],
        &[archive],
    )
}

fn install_into(fixture: &Fixture, game: &Path, archive: &Path) -> Output {
    let mut command = main_bucket::command(env!("CARGO_BIN_EXE_modn"));
    command
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .args(["--json", "install", "example", "--yes", "--path"])
        .arg(game)
        .arg("--file")
        .arg(archive);
    command.output().expect("install")
}

fn run_env(fixture: &Fixture, args: &[&str], environment: &[(&str, &OsStr)]) -> Output {
    let mut command = main_bucket::command(env!("CARGO_BIN_EXE_modn"));
    command
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules);
    for (name, value) in environment {
        command.env(name, value);
    }
    command.args(args).output().expect("run modn")
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

#[test]
fn undo_path_selects_only_the_named_installation() {
    let fixture = Fixture::new();
    let (package, digest) = fixture.archive();
    fixture.bucket("deps", "test-game", "example", &digest, &[]);
    fixture.register();
    let game_b = fixture.temp.path().join("game-b");
    fs::create_dir_all(game_b.join("bin")).expect("game b");
    fs::write(game_b.join("bin/TestGame.exe"), b"game-exe").expect("marker b");
    let added = fixture.run_paths(&["game", "add", "test-game", "--path"], &[&game_b]);
    assert!(added.status.success(), "{}", error(&added));

    let first = install_into(&fixture, &fixture.game, &package);
    assert!(first.status.success(), "{}", error(&first));
    let second = install_into(&fixture, &game_b, &package);
    assert!(second.status.success(), "{}", error(&second));
    let addon_a = fixture.game.join("addons/example.plugin");
    let addon_b = game_b.join("addons/example.plugin");
    assert!(addon_a.is_file());
    assert!(addon_b.is_file());

    let operations = history(&fixture);
    assert_eq!(operations.len(), 2);
    let install_a = operations
        .iter()
        .find(|operation| {
            operation["installation"]
                .as_str()
                .is_some_and(|path| !path.contains("game-b"))
        })
        .map(id_of)
        .expect("installation A record");
    let install_b = operations
        .iter()
        .find(|operation| {
            operation["installation"]
                .as_str()
                .is_some_and(|path| path.contains("game-b"))
        })
        .map(id_of)
        .expect("installation B record");

    let undone = fixture.run_paths(&["--json", "undo", "--yes", "--path"], &[&fixture.game]);
    assert!(undone.status.success(), "{}", error(&undone));
    assert!(!addon_a.exists());
    assert!(addon_b.is_file());
    let operations = history(&fixture);
    assert_eq!(operations.len(), 3);
    assert_eq!(operations[0]["kind"], "uninstall");
    assert_eq!(operations[0]["undoes"], install_a);

    let refused = fixture.run_paths(
        &["--json", "undo", &install_b, "--yes", "--path"],
        &[&fixture.game],
    );
    assert!(!refused.status.success());
    assert_eq!(error_code(&refused), "UNDO_NOT_FOUND");
    let message = error(&refused);
    assert!(message.contains("game-b"), "{message}");
    assert!(addon_b.is_file());
    assert_eq!(history(&fixture).len(), 3);
}

#[test]
fn undo_refuses_a_policy_that_moves_a_recorded_file_to_another_root() {
    let fixture = Fixture::new();
    let (package, digest) = fixture.archive();
    let url = "https://example.invalid/example-1.0.zip";
    fixture.bucket("deps", "test-game", "example", &digest, &[]);
    manifest(&fixture, "example", "1.0", &digest, https(url));
    write_module(
        &fixture,
        1,
        "Demo/package/plugins/example.plugin",
        "game",
        false,
    );
    fixture.register();
    let installed = install(&fixture, "example", &package);
    assert!(installed.status.success(), "{}", error(&installed));
    cache(&fixture.state, "test-game", "example", "1.0", url, &package);
    let addon = fixture.game.join("addons/example.plugin");
    assert!(addon.is_file());
    assert!(fixture
        .run(&["uninstall", "example", "--yes"])
        .status
        .success());
    let target = id_of(&history(&fixture)[0]);

    let appdata = fixture.temp.path().join("appdata");
    fs::create_dir_all(&appdata).expect("appdata");
    write_module(
        &fixture,
        2,
        "Demo/package/plugins/example.plugin",
        "content",
        true,
    );
    let environment = [
        ("APPDATA", appdata.as_os_str()),
        ("MODN_APPDATA", appdata.as_os_str()),
    ];
    // The dry run uses the same plan-and-record comparison as the real undo,
    // so it proves the refusal happens before any transaction starts.
    let preview = run_env(
        &fixture,
        &["--json", "undo", &target, "--dry-run"],
        &environment,
    );
    assert!(!preview.status.success(), "{}", error(&preview));
    assert_eq!(error_code(&preview), "UNDO_FAILED");
    assert!(error(&preview).contains("record"), "{}", error(&preview));
    let before = history(&fixture);
    let refused = run_env(
        &fixture,
        &["--json", "undo", &target, "--yes"],
        &environment,
    );
    assert!(!refused.status.success(), "{}", error(&refused));
    assert_eq!(error_code(&refused), "UNDO_FAILED");
    assert!(!addon.exists());
    assert!(!appdata.join("Fixture/Mods/addons/example.plugin").exists());
    assert_eq!(history(&fixture), before);
}

#[test]
fn undo_update_refuses_a_changed_member_selection_before_commit() {
    let fixture = Fixture::new();
    let (v1, v1_hash) = archive(
        &fixture,
        "v1",
        &[
            ("v1/example.plugin", "one"),
            ("v1-alt/example.plugin", "evil"),
        ],
    );
    let (v2, v2_hash) = archive(&fixture, "v2", &[("v2/example.plugin", "two")]);
    let v1_url = "https://example.invalid/example-1.0.zip";
    let v2_url = "https://example.invalid/example-2.0.zip";
    fixture.bucket("deps", "test-game", "example", &v1_hash, &[]);
    manifest(&fixture, "example", "1.0", &v1_hash, https(v1_url));
    write_module(&fixture, 1, "v1/example.plugin", "game", false);
    fixture.register();
    let installed = install(&fixture, "example", &v1);
    assert!(installed.status.success(), "{}", error(&installed));
    cache(&fixture.state, "test-game", "example", "1.0", v1_url, &v1);
    let addon = fixture.game.join("addons/example.plugin");
    assert_eq!(fs::read_to_string(&addon).expect("v1"), "one");

    manifest(&fixture, "example", "2.0", &v2_hash, https(v2_url));
    let updated = fixture.run_paths(&["--json", "update", "example", "--yes", "--file"], &[&v2]);
    assert!(updated.status.success(), "{}", error(&updated));
    assert_eq!(fs::read_to_string(&addon).expect("v2"), "two");
    let operations = history(&fixture);
    assert_eq!(operations[0]["kind"], "update");
    let target = id_of(&operations[0]);

    write_module(&fixture, 1, "v1-alt/example.plugin", "game", false);
    let before_history = history(&fixture);
    let before_packages = json_of(&fixture.run(&["--json", "list"]));
    let refused = fixture.run(&["--json", "undo", &target, "--yes"]);
    assert!(!refused.status.success(), "{}", error(&refused));
    assert_eq!(error_code(&refused), "UNDO_FAILED");
    assert!(error(&refused).contains("bytes"), "{}", error(&refused));
    assert_eq!(fs::read_to_string(&addon).expect("unchanged"), "two");
    assert_eq!(history(&fixture), before_history);
    assert_eq!(json_of(&fixture.run(&["--json", "list"])), before_packages);
}

#[test]
fn undo_uninstall_restores_below_the_appdata_content_root() {
    let mut fixture = content_root::Fixture::new();
    fixture.bucket();
    fixture.register();
    let (package, digest) = fixture.archive("package", &[("core.lua", b"content")]);
    let url = "https://example.invalid/content-fixture-1.0.zip";
    let bucket = fixture.temp.path().join("fixture-bucket");
    fs::write(
        bucket.join("mods/content-game/fixture.json"),
        serde_json::to_vec(&json!({
            "schema_version": 2, "id": "fixture", "game": "content-game",
            "name": "Fixture", "version": "1.0", "archive_format": "zip",
            "blake3": digest, "sources": [{"kind": "https", "url": url}]
        }))
        .expect("manifest JSON"),
    )
    .expect("manifest");
    cache(
        &fixture.state,
        "content-game",
        "fixture",
        "1.0",
        url,
        &package,
    );
    fixture.ok(&[
        "install",
        "fixture",
        "--yes",
        "--file",
        package.to_str().expect("archive path"),
    ]);
    let restored = fixture.content("Pkg/core.lua");
    assert!(restored.is_file(), "installed below the content root");

    fixture.ok(&["uninstall", "fixture", "--yes"]);
    assert!(!restored.exists());
    let operations = fixture.json(&["--json", "history"]);
    let target = operations[0]["id"]
        .as_str()
        .expect("operation ID")
        .to_owned();
    assert_eq!(operations[0]["kind"], "uninstall");

    fixture.ok(&["--json", "undo", &target, "--yes"]);
    assert!(restored.is_file(), "restored below the content root");
    assert!(!fixture.game.join("Pkg/core.lua").exists());
}
