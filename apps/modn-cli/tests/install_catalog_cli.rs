//! End-to-end catalog installation coverage with disposable fake games and local catalogs.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;

struct Fixture {
    temp: TempDir,
    state: PathBuf,
    modules: PathBuf,
    game: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("fixture");
        let state = temp.path().join("manager");
        let modules = temp.path().join("modules");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/test-game");
        let module = modules.join("test-game");
        fs::create_dir_all(&module).expect("module dir");
        for file in ["main.lua", "module.json"] {
            fs::copy(fixture.join(file), module.join(file)).expect("module source");
        }
        let cyberpunk = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/modn-core/game-modules/cyberpunk-2077");
        let cyberpunk_target = modules.join("cyberpunk-2077");
        fs::create_dir_all(&cyberpunk_target).expect("cyberpunk module");
        for file in ["main.lua", "module.json"] {
            fs::copy(cyberpunk.join(file), cyberpunk_target.join(file)).expect("cyberpunk source");
        }
        let game = temp.path().join("game");
        fs::create_dir_all(game.join("bin")).expect("game");
        fs::write(game.join("bin/TestGame.exe"), b"game-exe").expect("marker");
        Self {
            temp,
            state,
            modules,
            game,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_modn"))
            .arg("--state-dir")
            .arg(&self.state)
            .env("MODN_GAME_MODULES_DIR", &self.modules)
            .args(args)
            .output()
            .expect("run CLI")
    }

    fn run_paths(&self, args: &[&str], paths: &[&Path]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_modn"));
        command
            .arg("--state-dir")
            .arg(&self.state)
            .env("MODN_GAME_MODULES_DIR", &self.modules)
            .args(args);
        for path in paths {
            command.arg(path);
        }
        command.output().expect("run CLI")
    }

    fn register(&self) {
        let result = self.run_paths(&["game", "add", "test-game", "--path"], &[&self.game]);
        assert!(result.status.success(), "{}", error(&result));
    }

    fn bucket(&self, name: &str, game: &str, id: &str, hash: &str, dependencies: &[&str]) {
        let root = self.temp.path().join(name);
        let manifest_dir = root.join("games").join(game);
        fs::create_dir_all(&manifest_dir).expect("bucket dirs");
        fs::write(
            root.join("bucket.json"),
            serde_json::to_vec(&json!({
                "schema_version": 1, "name":name, "title":"Fixture"
            }))
            .expect("JSON"),
        )
        .expect("bucket metadata");
        fs::write(
            manifest_dir.join(format!("{id}.json")),
            serde_json::to_vec(&json!({
                "schema_version": 1, "id":id, "game":game, "name":"Fixture",
                "version":"1.0", "archive_format":"zip", "sha256":hash,
                "source":{"kind":"manual","filename":"fixture.zip"},
                "dependencies":dependencies
            }))
            .expect("JSON"),
        )
        .expect("manifest");
        let result = self.run_paths(&["bucket", "add", name], &[&root]);
        assert!(result.status.success(), "{}", error(&result));
    }

    fn archive(&self) -> (PathBuf, String) {
        let path = self.temp.path().join("fixture.zip");
        let file = fs::File::create(&path).expect("archive");
        let mut zip = zip::ZipWriter::new(file);
        use std::io::Write;
        zip.start_file(
            "Demo/package/plugins/example.plugin",
            SimpleFileOptions::default(),
        )
        .expect("entry");
        zip.write_all(b"installed-by-modn").expect("payload");
        zip.finish().expect("zip finish");
        let digest = Sha256::digest(fs::read(&path).expect("bytes"));
        (path, format!("{digest:x}"))
    }
}

fn error(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn adds_first_party_game_and_supports_development_source() {
    let fixture = Fixture::new();
    let missing = fixture.run(&["game", "install", "test-game", "--path", "not-a-game"]);
    assert!(!missing.status.success());
    assert!(!fixture.state.exists());

    fixture.register();
    let entries = fixture.run(&["game", "list", "--json"]);
    let list: Value = serde_json::from_slice(&entries.stdout).expect("game list JSON");
    assert_eq!(list[0]["game_id"], "test-game");
    assert_eq!(
        fs::read(fixture.game.join("bin/TestGame.exe")).expect("marker"),
        b"game-exe"
    );

    // Explicit --src takes a single directory rather than an external module package.
    let other = Fixture::new();
    let source = other.modules.join("test-game");
    let result = Command::new(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&other.state)
        .arg("--src")
        .arg(&source)
        .args(["game", "add", "test-game", "--path"])
        .arg(&other.game)
        .output()
        .expect("run");
    assert!(result.status.success(), "{}", error(&result));
}

#[test]
fn short_and_qualified_names_resolve_and_ambiguous_names_fail_in_json() {
    let fixture = Fixture::new();
    let fake_hash = "0".repeat(64);
    fixture.bucket("community", "minecraft", "cet", &fake_hash, &[]);
    let output = fixture.run(&["--json", "install", "cet"]);
    assert!(!output.status.success());
    let data: Value = serde_json::from_slice(&output.stderr).expect("structured ambiguity");
    assert_eq!(data["error"], "ambiguous_target");
    assert_eq!(data["query"], "cet");
    let candidates = data["candidates"].as_array().expect("candidates");
    assert!(candidates.iter().any(|x| x == "main/cyberpunk-2077/cet"));
    assert!(candidates.iter().any(|x| x == "community/minecraft/cet"));
    assert!(!fixture.state.join("state.sqlite").exists());

    for reference in ["minecraft/cet", "community/minecraft/cet"] {
        let result = fixture.run(&["install", reference]);
        assert!(!result.status.success());
        assert!(error(&result).contains("download"));
        assert!(!fixture.state.join("state.sqlite").exists());
    }
    let unknown = fixture.run(&["--json", "install", "not-listed"]);
    assert!(!unknown.status.success());
    assert!(!fixture.state.join("state.sqlite").exists());
}

#[test]
fn local_manifest_hash_verified_then_preview_and_install_reuse_rust_transactions() {
    let fixture = Fixture::new();
    let (archive, digest) = fixture.archive();
    fixture.bucket("community", "test-game", "example", &digest, &[]);
    fixture.register();
    let state_db = fixture.state.join("state.sqlite");
    assert!(state_db.is_file());
    let expected_file = fixture.game.join("addons/example.plugin");

    let missing = fixture.run(&["install", "example"]);
    assert!(!missing.status.success());
    assert!(error(&missing).contains("--file"));
    assert!(!expected_file.exists());

    let bad = fixture.run_paths(&["install", "example", "--file"], &[&archive]);
    assert!(!bad.status.success());
    assert!(error(&bad).contains("--yes"));
    assert!(!expected_file.exists());

    let preview = fixture.run_paths(
        &["--json", "install", "test-game/example", "--file"],
        &[&archive],
    );
    assert!(!preview.status.success(), "unapproved add must not proceed");

    let direct = Command::new(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .args([
            "--json",
            "install",
            "community/test-game/example",
            "--dry-run",
            "--file",
        ])
        .arg(&archive)
        .output()
        .expect("preview");
    assert!(direct.status.success(), "{}", error(&direct));
    let report: Value = serde_json::from_slice(&direct.stdout).expect("preview JSON");
    assert_eq!(report["package"], "community/test-game/example");
    assert_eq!(report["plan"]["status"], "reviewable");
    assert!(!expected_file.exists());

    let installed = Command::new(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .args(["--json", "install", "example", "--yes", "--file"])
        .arg(&archive)
        .output()
        .expect("install");
    assert!(installed.status.success(), "{}", error(&installed));
    let result: Value = serde_json::from_slice(&installed.stdout).expect("installed JSON");
    assert_eq!(result["package"], "community/test-game/example");
    assert_eq!(
        fs::read(&expected_file).expect("payload"),
        b"installed-by-modn"
    );

    let repeat = Command::new(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .args(["install", "example", "--yes", "--file"])
        .arg(&archive)
        .output()
        .expect("repeat");
    assert!(
        !repeat.status.success(),
        "existing destination must reject reinstallation"
    );
    assert_eq!(
        fs::read(&expected_file).expect("payload"),
        b"installed-by-modn"
    );
}

#[test]
fn incorrect_hash_and_unresolved_dependencies_do_not_touch_game() {
    let fixture = Fixture::new();
    let (archive, digest) = fixture.archive();
    fixture.bucket("wrong", "test-game", "bad", &"0".repeat(64), &[]);
    let root = fixture.temp.path().join("wrong");
    let bad_hash = Command::new(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .args(["install", "bad", "--yes", "--path"])
        .arg(&fixture.game)
        .arg("--file")
        .arg(&archive)
        .output()
        .expect("bad hash");
    assert!(!bad_hash.status.success());
    assert!(error(&bad_hash).contains("SHA-256"));
    assert!(!fixture.state.join("state.sqlite").exists());
    assert!(!fixture.game.join("addons").exists());

    // Cross-manifest dependency exists but has not been installed.
    fixture.bucket("deps", "test-game", "dependent", &digest, &["base"]);
    let base = json!({
        "schema_version":1, "id":"base","game":"test-game","name":"Base",
        "version":"1.0","archive_format":"zip","sha256":digest,
        "source":{"kind":"manual","filename":"fixture.zip"}, "dependencies":[]
    });
    fs::write(
        fixture.temp.path().join("deps/games/test-game/base.json"),
        serde_json::to_vec(&base).expect("JSON"),
    )
    .expect("dependency");
    let missing_dependency = Command::new(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .args(["install", "dependent", "--yes", "--path"])
        .arg(&fixture.game)
        .arg("--file")
        .arg(&archive)
        .output()
        .expect("dependency");
    assert!(!missing_dependency.status.success());
    assert!(error(&missing_dependency).contains("dependency not installed"));
    assert!(!fixture.game.join("addons").exists());
    assert!(!fixture.state.join("state.sqlite").exists());
    assert!(root.join("bucket.json").exists());
}

#[test]
fn mod_name_matching_game_id_is_still_a_mod_install() {
    let fixture = Fixture::new();
    fixture.bucket("collision", "test-game", "test-game", &"0".repeat(64), &[]);
    let result = fixture.run(&["install", "test-game"]);
    assert!(!result.status.success());
    assert!(error(&result).contains("--file"), "{}", error(&result));
    assert!(!fixture.state.join("state.sqlite").exists());
}

#[test]
fn local_archive_preview_without_game_registration_has_no_state_writes() {
    let fixture = Fixture::new();
    let (archive, hash) = fixture.archive();
    fixture.bucket("community", "test-game", "example", &hash, &[]);
    assert!(!fixture.state.join("state.sqlite").exists());
    let preview = Command::new(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .args(["--json", "install", "example", "--dry-run", "--path"])
        .arg(&fixture.game)
        .arg("--file")
        .arg(&archive)
        .output()
        .expect("preview");
    assert!(preview.status.success(), "{}", error(&preview));
    let data: Value = serde_json::from_slice(&preview.stdout).expect("JSON");
    assert_eq!(data["plan"]["status"], "reviewable");
    assert!(!fixture.state.join("state.sqlite").exists());
    assert!(!fixture.game.join("addons").exists());
}

#[test]
fn deleted_managed_dependency_cannot_authorize_dependent_install() {
    let fixture = Fixture::new();
    let (base_archive, base_hash) = fixture.archive();
    fixture.bucket("deps", "test-game", "base", &base_hash, &[]);
    let dependent_archive = fixture.temp.path().join("dependent.zip");
    let file = fs::File::create(&dependent_archive).expect("archive");
    let mut zip = zip::ZipWriter::new(file);
    use std::io::Write;
    zip.start_file(
        "Demo/package/plugins/dependent.plugin",
        SimpleFileOptions::default(),
    )
    .expect("entry");
    zip.write_all(b"dependent-content").expect("payload");
    zip.finish().expect("finish");
    let dependent_hash = format!(
        "{:x}",
        Sha256::digest(fs::read(&dependent_archive).expect("bytes"))
    );
    let manifest = json!({
        "schema_version":1, "id":"dependent", "game":"test-game",
        "name":"Dependent", "version":"1.0", "archive_format":"zip",
        "sha256":dependent_hash, "source":{"kind":"manual","filename":"dependent.zip"},
        "dependencies":["base"]
    });
    fs::write(
        fixture
            .temp
            .path()
            .join("deps/games/test-game/dependent.json"),
        serde_json::to_vec(&manifest).expect("JSON"),
    )
    .expect("manifest");
    fixture.register();

    let base = Command::new(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .args(["install", "base", "--yes", "--file"])
        .arg(&base_archive)
        .output()
        .expect("install base");
    assert!(base.status.success(), "{}", error(&base));
    let owned_file = fixture.game.join("addons/example.plugin");
    assert!(owned_file.is_file());
    fs::remove_file(&owned_file).expect("simulate drift");

    let dependent = Command::new(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .args(["install", "dependent", "--yes", "--file"])
        .arg(&dependent_archive)
        .output()
        .expect("install dependent");
    assert!(
        !dependent.status.success(),
        "drifted prerequisite must block"
    );
    assert!(
        error(&dependent).contains("dependency not installed or intact"),
        "{}",
        error(&dependent)
    );
    assert!(!fixture.game.join("addons/dependent.plugin").exists());
    assert_eq!(
        fs::read(fixture.game.join("bin/TestGame.exe")).expect("marker"),
        b"game-exe"
    );
}

#[test]
fn search_is_top_level_filters_name_and_game_and_never_creates_state() {
    let fixture = Fixture::new();
    let output = fixture.run(&["--json", "search", "Engine"]);
    assert!(output.status.success(), "{}", error(&output));
    let matches: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(matches.as_array().expect("array").len(), 1);
    assert_eq!(matches[0]["id"], "cet");
    let filtered = fixture.run(&["--json", "--game", "test-game", "search"]);
    assert!(filtered.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&filtered.stdout).expect("json"),
        json!([])
    );
    assert!(!fixture.state.join("state.sqlite").exists());
    assert!(!fixture.run(&["bucket", "search"]).status.success());
    assert!(!fixture.run(&["add", "cet"]).status.success());
}

#[test]
fn list_and_game_remove_preserve_game_and_refuse_managed_mods() {
    let fixture = Fixture::new();
    let (archive, _) = fixture.archive();
    fixture.register();
    let listed = fixture.run(&[
        "--json",
        "list",
        "--path",
        fixture.game.to_str().expect("path"),
    ]);
    assert!(listed.status.success(), "{}", error(&listed));
    assert_eq!(
        serde_json::from_slice::<Value>(&listed.stdout).expect("json"),
        json!([])
    );
    assert!(!fixture.run(&["installed"]).status.success());

    let installed = fixture.run_paths(&["install", "--yes"], &[&archive]);
    assert!(installed.status.success(), "{}", error(&installed));
    let package: Value =
        serde_json::from_slice(&fixture.run(&["--json", "list"]).stdout).expect("list");
    assert_eq!(package.as_array().expect("array").len(), 1);
    let owned = fixture.game.join("addons/example.plugin");
    assert!(owned.is_file());

    let blocked = fixture.run(&["game", "remove", "test-game"]);
    assert!(!blocked.status.success());
    assert!(
        error(&blocked).contains("managed packages"),
        "{}",
        error(&blocked)
    );
    assert!(owned.is_file());
    let id = package[0]["id"].as_str().expect("id");
    let removed = fixture.run(&["uninstall", id, "--yes"]);
    assert!(removed.status.success(), "{}", error(&removed));
    let unregistered = fixture.run(&["--json", "game", "remove", "test-game"]);
    assert!(unregistered.status.success(), "{}", error(&unregistered));
    let removal: Value = serde_json::from_slice(&unregistered.stdout).expect("json");
    assert_eq!(removal["game_id"], "test-game");
    let games = fixture.run(&["--json", "game", "list"]);
    assert_eq!(
        serde_json::from_slice::<Value>(&games.stdout).expect("json"),
        json!([])
    );
    assert!(!fixture
        .run(&["game", "remove", "test-game"])
        .status
        .success());
    assert_eq!(
        fs::read(fixture.game.join("bin/TestGame.exe")).expect("marker"),
        b"game-exe"
    );
    assert!(!owned.exists());
}

#[test]
fn game_remove_requires_path_with_duplicate_registrations() {
    let fixture = Fixture::new();
    fixture.register();
    let second = fixture.temp.path().join("second-game");
    fs::create_dir_all(second.join("bin")).expect("fixture");
    fs::write(second.join("bin/TestGame.exe"), b"second").expect("marker");
    let added = fixture.run_paths(&["game", "add", "test-game", "--path"], &[&second]);
    assert!(added.status.success(), "{}", error(&added));
    let ambiguous = fixture.run(&["game", "remove", "test-game"]);
    assert!(!ambiguous.status.success());
    assert!(
        error(&ambiguous).contains("multiple installations"),
        "{}",
        error(&ambiguous)
    );
    let selected = fixture.run_paths(&["game", "remove", "test-game", "--path"], &[&second]);
    assert!(selected.status.success(), "{}", error(&selected));
    let games = fixture.run(&["--json", "game", "list"]);
    let list: Value = serde_json::from_slice(&games.stdout).expect("json");
    assert_eq!(list.as_array().expect("array").len(), 1);
    assert_eq!(
        list[0]["path"],
        serde_json::to_value(fixture.game.canonicalize().expect("canonical")).expect("json")
    );
    assert_eq!(
        fs::read(second.join("bin/TestGame.exe")).expect("marker"),
        b"second"
    );
}
