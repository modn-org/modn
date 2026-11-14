#[path = "support/catalog.rs"]
mod support;

#[path = "support/main_bucket.rs"]
mod main_bucket;

#[path = "support/tree.rs"]
#[allow(dead_code, reason = "this test crate uses only the snapshot helper")]
mod tree;
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Output;
use support::{error, Fixture};
use zip::write::SimpleFileOptions;

fn run_with_files(fixture: &Fixture, args: &[&str], files: &[&Path]) -> Output {
    let mut command = main_bucket::command(env!("CARGO_BIN_EXE_modn"));
    command
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .args(args);
    for file in files {
        command.arg("--file").arg(file);
    }
    command.output().expect("run CLI")
}

fn plugin_archive(fixture: &Fixture, name: &str, contents: &str) -> (PathBuf, String) {
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

fn write_manifest(
    fixture: &Fixture,
    bucket: &str,
    id: &str,
    version: &str,
    hash: &str,
    dependencies: &[&str],
    native: bool,
) {
    let manifest = json!({
        "schema_version": 2,
        "id": id,
        "game": "native-injector-game",
        "name": "Injector",
        "version": version,
        "archive_format": "zip",
        "blake3": hash,
        "sources": [{"kind": "manual", "filename": format!("{id}-{version}.zip")}],
        "dependencies": dependencies,
        "native_injector": native,
    });
    let folder = fixture
        .temp
        .path()
        .join(bucket)
        .join("mods/native-injector-game");
    fs::create_dir_all(&folder).expect("bucket folder");
    fs::write(
        folder.join(format!("{id}.json")),
        serde_json::to_vec(&manifest).expect("JSON"),
    )
    .expect("manifest");
}

fn native_setup() -> (Fixture, PathBuf, String) {
    let fixture = Fixture::new();
    let (archive, digest) = fixture.native_archive();
    fixture.native_bucket("native", "injector", &digest, &[]);
    fixture.register_native();
    (fixture, archive, digest)
}

#[test]
fn install_refuses_native_injector_before_archive_and_yes_checks() {
    let (fixture, _archive, _digest) = native_setup();
    let game_before = tree::snapshot(&fixture.native_game);
    let state_before = tree::snapshot(&fixture.state);

    let output = fixture.run(&["install", "injector", "--yes"]);
    assert!(!output.status.success());
    let message = error(&output);
    assert!(message.contains("native injector"), "{message}");
    assert!(message.contains("--allow-native-injector"), "{message}");
    assert!(
        message.contains("native/native-injector-game/injector"),
        "{message}"
    );
    assert_eq!(tree::snapshot(&fixture.native_game), game_before);
    assert_eq!(tree::snapshot(&fixture.state), state_before);
}

#[test]
fn json_refusal_uses_the_error_envelope() {
    let (fixture, _archive, _digest) = native_setup();
    let output = fixture.run(&["--json", "install", "injector", "--yes"]);
    assert!(!output.status.success());
    let refusal: Value = serde_json::from_slice(&output.stderr).expect("structured refusal");
    assert_eq!(refusal["error"], "NATIVE_INJECTOR_NOT_ALLOWED");
    assert_eq!(refusal["mod_id"], "injector");
    assert_eq!(refusal["package"], "native/native-injector-game/injector");
    assert_eq!(refusal["flag"], "--allow-native-injector");
    assert_eq!(refusal["uncovered"]["native_injector"], true);
    assert!(!fixture.native_game.join("winmm.dll").exists());
}

#[test]
fn dry_run_reports_required_native_approval_and_writes_nothing() {
    let (fixture, archive, _digest) = native_setup();
    let output = fixture.run_paths(
        &["--json", "install", "injector", "--dry-run", "--file"],
        &[&archive],
    );
    assert!(output.status.success(), "{}", error(&output));
    let report: Value = serde_json::from_slice(&output.stdout).expect("dry-run report");
    assert_eq!(report["approval"]["required"], true);
    assert_eq!(report["approval"]["covered_by_stored_approval"], false);
    assert_eq!(report["approval"]["uncovered"]["native_injector"], true);
    assert!(!fixture.native_game.join("winmm.dll").exists());
}

#[test]
fn stored_native_approval_survives_uninstall_and_covers_reinstall() {
    let (fixture, archive, _digest) = native_setup();
    let installed = fixture.run_paths(
        &[
            "install",
            "injector",
            "--yes",
            "--allow-native-injector",
            "--file",
        ],
        &[&archive],
    );
    assert!(installed.status.success(), "{}", error(&installed));
    let uninstalled = fixture.run(&["uninstall", "injector", "--yes"]);
    assert!(uninstalled.status.success(), "{}", error(&uninstalled));

    let reinstalled = fixture.run_paths(&["install", "injector", "--yes", "--file"], &[&archive]);
    assert!(reinstalled.status.success(), "{}", error(&reinstalled));
    assert!(fixture.native_game.join("winmm.dll").is_file());
}

#[test]
fn a_missing_native_dependency_refuses_the_whole_install() {
    let fixture = Fixture::new();
    let (native, native_hash) = fixture.native_archive();
    let (top, top_hash) = plugin_archive(&fixture, "top", "top");
    fixture.bucket(
        "native",
        "native-injector-game",
        "dependency",
        &native_hash,
        &[],
    );
    write_manifest(
        &fixture,
        "native",
        "dependency",
        "1.0",
        &native_hash,
        &[],
        true,
    );
    write_manifest(
        &fixture,
        "native",
        "top",
        "1.0",
        &top_hash,
        &["dependency"],
        false,
    );
    fixture.register_native();

    let output = run_with_files(
        &fixture,
        &["--json", "install", "top", "--yes"],
        &[&top, &native],
    );
    assert!(!output.status.success());
    let refusal: Value = serde_json::from_slice(&output.stderr).expect("structured refusal");
    assert_eq!(refusal["error"], "NATIVE_INJECTOR_NOT_ALLOWED");
    assert_eq!(refusal["package"], "native/native-injector-game/dependency");
    assert!(!fixture.native_game.join("winmm.dll").exists());
    assert!(!fixture.native_game.join("addons/example.plugin").exists());
}

#[test]
fn an_installed_native_dependency_does_not_require_the_flag_again() {
    let fixture = Fixture::new();
    let (native, native_hash) = fixture.native_archive();
    let (top, top_hash) = plugin_archive(&fixture, "top", "top");
    fixture.bucket(
        "native",
        "native-injector-game",
        "dependency",
        &native_hash,
        &[],
    );
    write_manifest(
        &fixture,
        "native",
        "dependency",
        "1.0",
        &native_hash,
        &[],
        true,
    );
    write_manifest(
        &fixture,
        "native",
        "top",
        "1.0",
        &top_hash,
        &["dependency"],
        false,
    );
    fixture.register_native();

    let dependency = run_with_files(
        &fixture,
        &["install", "dependency", "--yes", "--allow-native-injector"],
        &[&native],
    );
    assert!(dependency.status.success(), "{}", error(&dependency));
    let selected = run_with_files(&fixture, &["install", "top", "--yes"], &[&top]);
    assert!(selected.status.success(), "{}", error(&selected));
    assert!(fixture.native_game.join("winmm.dll").is_file());
    assert!(fixture.native_game.join("addons/example.plugin").is_file());
}

#[test]
fn an_allowed_install_places_the_native_library_at_the_game_root() {
    let (fixture, archive, _digest) = native_setup();
    let installed = fixture.run_paths(
        &[
            "install",
            "injector",
            "--yes",
            "--allow-native-injector",
            "--file",
        ],
        &[&archive],
    );
    assert!(installed.status.success(), "{}", error(&installed));
    assert_eq!(
        fs::read(fixture.native_game.join("winmm.dll")).expect("native library"),
        b"native-injector-payload"
    );

    let uninstalled = fixture.run(&["uninstall", "injector", "--yes"]);
    assert!(uninstalled.status.success(), "{}", error(&uninstalled));
    assert!(!fixture.native_game.join("winmm.dll").exists());
    assert_eq!(
        fs::read(fixture.native_game.join("bin/TestGame.exe")).expect("marker"),
        b"game-exe"
    );
}

#[test]
fn update_refuses_a_native_target_until_the_flag_is_passed() {
    let fixture = Fixture::new();
    let (v1, v1_hash) = plugin_archive(&fixture, "injector-1", "v1");
    let (v2, v2_hash) = fixture.native_archive();
    fixture.bucket("native", "native-injector-game", "injector", &v1_hash, &[]);
    fixture.register_native();
    let installed = fixture.run_paths(&["install", "injector", "--yes", "--file"], &[&v1]);
    assert!(installed.status.success(), "{}", error(&installed));
    write_manifest(&fixture, "native", "injector", "2.0", &v2_hash, &[], true);

    let refused = fixture.run_paths(&["--json", "update", "injector", "--yes", "--file"], &[&v2]);
    assert!(!refused.status.success());
    let refusal: Value = serde_json::from_slice(&refused.stderr).expect("structured refusal");
    assert_eq!(refusal["error"], "NATIVE_INJECTOR_NOT_ALLOWED");
    assert_eq!(refusal["mod_id"], "injector");
    assert_eq!(refusal["package"], "native/native-injector-game/injector");
    assert_eq!(refusal["uncovered"]["native_injector"], true);
    assert!(!fixture.native_game.join("winmm.dll").exists());
    assert_eq!(
        fs::read(fixture.native_game.join("addons/example.plugin")).expect("old payload"),
        b"v1"
    );

    let dry_run = fixture.run_paths(
        &["--json", "update", "injector", "--dry-run", "--file"],
        &[&v2],
    );
    assert!(dry_run.status.success(), "{}", error(&dry_run));
    assert!(!fixture.native_game.join("winmm.dll").exists());

    let updated = fixture.run_paths(
        &[
            "update",
            "injector",
            "--yes",
            "--allow-native-injector",
            "--file",
        ],
        &[&v2],
    );
    assert!(updated.status.success(), "{}", error(&updated));
    assert_eq!(
        fs::read(fixture.native_game.join("winmm.dll")).expect("native library"),
        b"native-injector-payload"
    );
    assert!(!fixture.native_game.join("addons/example.plugin").exists());

    let uninstalled = fixture.run(&["uninstall", "injector", "--yes"]);
    assert!(uninstalled.status.success(), "{}", error(&uninstalled));
    let reinstalled = fixture.run_paths(&["install", "injector", "--yes", "--file"], &[&v2]);
    assert!(reinstalled.status.success(), "{}", error(&reinstalled));
}

#[test]
fn help_documents_the_flag_on_install_and_update() {
    let fixture = Fixture::new();
    for command in ["install", "update"] {
        let output = fixture.run(&[command, "--help"]);
        assert!(output.status.success(), "{}", error(&output));
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(
            text.contains("--allow-native-injector"),
            "{command} help: {text}"
        );
    }
}
