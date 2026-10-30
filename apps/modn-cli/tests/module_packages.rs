//! Exercise separately distributed policy versions with the same host executable.
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use zip::write::SimpleFileOptions;

struct Fixture {
    temp: tempfile::TempDir,
    state: PathBuf,
    game: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("fixture operation");
        let state = temp.path().join("state");
        let game = temp.path().join("game");
        fs::create_dir_all(game.join("bin")).expect("fixture operation");
        fs::write(game.join("bin/TestGame.exe"), b"unchanged game").expect("fixture operation");
        Self { temp, state, game }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_modn"));
        cmd.env_remove("MODN_GAME_MODULES_DIR")
            .args(["--json", "--state-dir"])
            .arg(&self.state)
            .args(args);
        cmd
    }

    fn ok(&self, args: &[&str]) -> Value {
        json(self.command(args).output().expect("fixture operation"))
    }

    fn package(&self, version: &str, destination: &str) -> PathBuf {
        let source = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/test-game/main.luau"),
        )
        .expect("fixture operation");
        let source = source.replace(
            "destination = \"addons/\" .. suffix,",
            &format!("destination = \"{destination}/\" .. suffix,"),
        );
        let manifest = serde_json::json!({"package_format":1,"game_id":"test-game",
            "version":version,"api_version":1,"entrypoint":"main.luau"});
        let path = self.temp.path().join(format!("{version}.zip"));
        write_zip(
            &path,
            &[
                ("module.json", manifest.to_string().as_bytes()),
                ("main.luau", source.as_bytes()),
            ],
        );
        path
    }

    fn preview(&self) -> Value {
        let archive = self.temp.path().join("mod.zip");
        write_zip(&archive, &[("package/plugins/example.plugin", b"payload")]);
        json(
            self.command(&["--game", "test-game", "install"])
                .arg(archive)
                .arg("--dry-run")
                .arg("--path")
                .arg(&self.game)
                .output()
                .expect("fixture operation"),
        )
    }
}

fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
    let mut zip = zip::ZipWriter::new(fs::File::create(path).expect("fixture operation"));
    for (name, data) in entries {
        zip.start_file(
            *name,
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
        )
        .expect("fixture operation");
        zip.write_all(data).expect("fixture operation");
    }
    zip.finish().expect("fixture operation");
}

fn json(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("fixture operation")
}

#[test]
fn install_enable_update_rollback_disable_without_rebuilding_or_game_writes() {
    let f = Fixture::new();
    assert_eq!(f.ok(&["module", "list"]), serde_json::json!([]));
    assert!(!f.state.exists());
    let a = f.package("1.0", "addons");
    let b = f.package("2.0", "other-addons");
    f.ok(&[
        "module",
        "install",
        a.to_str().expect("fixture operation"),
        "--dry-run",
    ]);
    assert!(!f.state.exists());
    let denied = f
        .command(&["module", "install"])
        .arg(&a)
        .output()
        .expect("fixture operation");
    assert!(!denied.status.success());
    assert!(!f.state.exists());
    f.ok(&[
        "module",
        "install",
        a.to_str().expect("fixture operation"),
        "--yes",
    ]);
    assert!(
        !f.preview_failure().status.success(),
        "install must not enable"
    );
    f.ok(&["module", "enable", "test-game", "1.0", "--yes"]);
    assert_eq!(
        f.preview()["operations"][0]["destination"],
        "addons/example.plugin"
    );
    f.ok(&[
        "module",
        "update",
        b.to_str().expect("fixture operation"),
        "--yes",
    ]);
    assert_eq!(
        f.preview()["operations"][0]["destination"],
        "other-addons/example.plugin"
    );
    f.ok(&["module", "rollback", "test-game", "--yes"]);
    assert_eq!(
        f.preview()["operations"][0]["destination"],
        "addons/example.plugin"
    );
    assert_eq!(
        f.ok(&["module", "list"])
            .as_array()
            .expect("fixture operation")
            .len(),
        2
    );
    f.ok(&["module", "disable", "test-game", "--yes"]);
    assert!(!f.preview_failure().status.success());
    f.ok(&["module", "enable", "test-game", "2.0", "--yes"]);
    assert_eq!(
        f.preview()["operations"][0]["destination"],
        "other-addons/example.plugin"
    );
    assert!(!f.state.join("state.sqlite").exists());
    assert_eq!(
        fs::read(f.game.join("bin/TestGame.exe")).expect("fixture operation"),
        b"unchanged game"
    );
    assert_eq!(fs::read_dir(&f.game).expect("fixture operation").count(), 1);
}

#[test]
fn tampering_fails_closed_and_explicit_disable_is_a_repair_route() {
    let f = Fixture::new();
    let a = f.package("1.0", "addons");
    let installed = f.ok(&[
        "module",
        "install",
        a.to_str().expect("fixture operation"),
        "--yes",
    ]);
    f.ok(&["module", "enable", "test-game", "1.0", "--yes"]);
    let source = f
        .state
        .join("game-modules/versions")
        .join(installed["sha256"].as_str().expect("fixture operation"))
        .join("main.luau");
    let original = fs::read(&source).expect("fixture operation");
    fs::write(&source, b"error('TAMPERED CODE EXECUTED')").expect("fixture operation");
    for _ in 0..2 {
        let denied = f.preview_failure();
        assert!(!denied.status.success());
        let error = String::from_utf8_lossy(&denied.stderr);
        assert!(error.contains("integrity mismatch"), "{error}");
        assert!(
            !error.contains("TAMPERED CODE EXECUTED"),
            "must hash before execution"
        );
    }
    f.ok(&["module", "disable", "test-game", "--yes"]);
    fs::write(&source, original).expect("fixture operation");
    f.ok(&["module", "enable", "test-game", "1.0", "--yes"]);
    assert_eq!(
        f.preview()["operations"][0]["destination"],
        "addons/example.plugin"
    );
    assert!(!f.state.join("state.sqlite").exists());
}

#[test]
fn invalid_update_wrong_game_and_reused_version_do_not_change_selection_or_state() {
    let f = Fixture::new();
    let a = f.package("1.0", "addons");
    let b = f.package("2.0", "other-addons");
    let missing = f
        .command(&["module", "update"])
        .arg(&b)
        .arg("--yes")
        .output()
        .expect("fixture operation");
    assert!(!missing.status.success());
    assert!(!f.state.exists(), "failed update created state");
    let wrong = f
        .command(&["--game", "other-game", "module", "install"])
        .arg(&a)
        .arg("--yes")
        .output()
        .expect("fixture operation");
    assert!(!wrong.status.success());
    assert!(!f.state.exists());
    f.ok(&[
        "module",
        "install",
        a.to_str().expect("fixture operation"),
        "--yes",
    ]);
    f.ok(&["module", "enable", "test-game", "1.0", "--yes"]);
    let database = f.state.join("game-modules/modules.sqlite");
    let before = fs::read(&database).expect("fixture operation");
    f.package("1.0", "replaced");
    for args in [
        vec![
            "module",
            "install",
            a.to_str().expect("fixture path"),
            "--dry-run",
        ],
        vec![
            "module",
            "update",
            a.to_str().expect("fixture operation"),
            "--yes",
        ],
        vec!["module", "enable", "test-game", "missing", "--yes"],
        vec!["module", "rollback", "test-game", "--yes"],
    ] {
        assert!(!f
            .command(&args)
            .output()
            .expect("fixture operation")
            .status
            .success());
        assert_eq!(fs::read(&database).expect("fixture operation"), before);
    }
    assert_eq!(
        f.preview()["operations"][0]["destination"],
        "addons/example.plugin"
    );
}

#[test]
fn operator_override_outranks_enabled_user_store_and_fails_closed_when_missing() {
    let f = Fixture::new();
    let a = f.package("1.0", "addons");
    f.ok(&[
        "module",
        "install",
        a.to_str().expect("fixture operation"),
        "--yes",
    ]);
    f.ok(&["module", "enable", "test-game", "1.0", "--yes"]);
    let override_root = f.temp.path().join("override");
    let module = override_root.join("test-game");
    fs::create_dir_all(&module).expect("fixture operation");
    fs::write(
        module.join("module.json"),
        r#"{"game_id":"test-game","version":"override","api_version":1,"entrypoint":"main.luau"}"#,
    )
    .expect("fixture operation");
    fs::write(module.join("main.luau"), "error('OVERRIDE WAS SELECTED')")
        .expect("fixture operation");
    // Status validates identity files; actual policy execution rejects this source.
    let archive = f.temp.path().join("mod.zip");
    write_zip(&archive, &[("package/plugins/a.plugin", b"payload")]);
    let denied = f
        .command(&["--game", "test-game", "install"])
        .arg(&archive)
        .args(["--dry-run", "--path"])
        .arg(&f.game)
        .env("MODN_GAME_MODULES_DIR", &override_root)
        .output()
        .expect("fixture operation");
    assert!(!denied.status.success());
    assert!(String::from_utf8_lossy(&denied.stderr).contains("OVERRIDE WAS SELECTED"));
    let missing = f
        .command(&["--game", "test-game", "install"])
        .arg(&archive)
        .args(["--dry-run", "--path"])
        .arg(&f.game)
        .env("MODN_GAME_MODULES_DIR", f.temp.path().join("missing"))
        .output()
        .expect("fixture operation");
    assert!(
        !missing.status.success(),
        "explicit override must never fall back"
    );
}

impl Fixture {
    fn preview_failure(&self) -> Output {
        let archive = self.temp.path().join("mod.zip");
        write_zip(&archive, &[("package/plugins/example.plugin", b"payload")]);
        self.command(&["--game", "test-game", "install"])
            .arg(archive)
            .arg("--dry-run")
            .arg("--path")
            .arg(&self.game)
            .output()
            .expect("fixture operation")
    }
}

#[test]
fn bundled_policy_is_only_shadowed_by_explicit_enable_and_corruption_never_falls_back() {
    let f = Fixture::new();
    let bundle = f.temp.path().join("standalone");
    let module = bundle.join("game-modules/test-game");
    fs::create_dir_all(&module).expect("fixture operation");
    let binary = bundle.join(if cfg!(windows) { "modn.exe" } else { "modn" });
    fs::copy(env!("CARGO_BIN_EXE_modn"), &binary).expect("fixture operation");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/test-game");
    fs::copy(fixture.join("module.json"), module.join("module.json")).expect("fixture operation");
    let bundled_source = fs::read_to_string(fixture.join("main.luau"))
        .expect("fixture operation")
        .replace(
            "destination = \"addons/\" .. suffix,",
            "destination = \"bundled-addons/\" .. suffix,",
        );
    fs::write(module.join("main.luau"), bundled_source).expect("fixture operation");
    let archive = f.temp.path().join("mod.zip");
    write_zip(&archive, &[("package/plugins/example.plugin", b"payload")]);
    let preview = || {
        Command::new(&binary)
            .env_remove("MODN_GAME_MODULES_DIR")
            .args(["--json", "--state-dir"])
            .arg(&f.state)
            .args(["--game", "test-game", "install"])
            .arg(&archive)
            .args(["--dry-run", "--path"])
            .arg(&f.game)
            .output()
            .expect("fixture operation")
    };
    assert_eq!(
        json(preview())["operations"][0]["destination"],
        "bundled-addons/example.plugin"
    );
    let a = f.package("1.0", "addons");
    let info = f.ok(&[
        "module",
        "install",
        a.to_str().expect("fixture operation"),
        "--yes",
    ]);
    assert_eq!(
        json(preview())["operations"][0]["destination"],
        "bundled-addons/example.plugin"
    );
    f.ok(&["module", "enable", "test-game", "1.0", "--yes"]);
    assert_eq!(
        json(preview())["operations"][0]["destination"],
        "addons/example.plugin"
    );
    let script = f
        .state
        .join("game-modules/versions")
        .join(info["sha256"].as_str().expect("fixture operation"))
        .join("main.luau");
    fs::write(script, b"tampered").expect("fixture operation");
    let corrupt = preview();
    assert!(!corrupt.status.success());
    assert!(String::from_utf8_lossy(&corrupt.stderr).contains("automatic fallback refused"));
    f.ok(&["module", "disable", "test-game", "--yes"]);
    assert_eq!(
        json(preview())["operations"][0]["destination"],
        "bundled-addons/example.plugin"
    );
}

#[test]
fn initial_package_cannot_create_its_store_inside_its_game() {
    let f = Fixture::new();
    let a = f.package("1.0", "addons");
    let bad_state = f.game.join("private-state");
    let denied = Command::new(env!("CARGO_BIN_EXE_modn"))
        .env_remove("MODN_GAME_MODULES_DIR")
        .arg("--state-dir")
        .arg(&bad_state)
        .args(["module", "install"])
        .arg(&a)
        .arg("--yes")
        .output()
        .expect("fixture operation");
    assert!(!denied.status.success());
    assert!(String::from_utf8_lossy(&denied.stderr).contains("outside a game installation"));
    assert!(!bad_state.exists());
    assert_eq!(fs::read_dir(&f.game).expect("fixture operation").count(), 1);
}

#[test]
fn selection_commands_require_yes_before_creating_state() {
    let f = Fixture::new();
    let a = f.package("1.0", "addons");
    for args in [
        vec!["module", "enable", "test-game", "1.0"],
        vec!["module", "disable", "test-game"],
        vec!["module", "rollback", "test-game"],
        vec!["module", "update", a.to_str().expect("fixture path")],
    ] {
        let denied = f.command(&args).output().expect("fixture command");
        assert!(!denied.status.success());
        assert!(String::from_utf8_lossy(&denied.stderr).contains("--yes"));
        assert!(!f.state.exists());
    }
}
