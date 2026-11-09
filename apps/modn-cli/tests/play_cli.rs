//! `modn play` refusals and launch-target validation.
//!
//! No test starts a process or opens a store URI: every success path would
//! launch, so these tests stop at a refusal or validate a module directly.

#[path = "support/main_bucket.rs"]
mod main_bucket;

use rusqlite::Connection;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use tempfile::TempDir;

fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().expect("parent")).expect("fixture folder");
    fs::write(path, bytes).expect("fixture file");
}

fn cyberpunk(temp: &TempDir) -> PathBuf {
    let game = temp.path().join("Cyberpunk 2077");
    write(&game.join("bin/x64/Cyberpunk2077.exe"), b"fixture");
    game.canonicalize().expect("canonical game")
}

fn run(state: &Path, args: &[&str]) -> Output {
    main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .env_remove("MODN_GAME_MODULES_DIR")
        .env_remove("MODN_STORE_LIBRARIES")
        .arg("--state-dir")
        .arg(state)
        .args(args)
        .output()
        .expect("run modn")
}

fn error_code(output: &Output) -> String {
    let value: Value = serde_json::from_slice(&output.stderr).expect("JSON error");
    value["error"].as_str().expect("error code").to_owned()
}

#[test]
fn refuses_play_without_a_registered_installation_or_path() {
    let temp = tempfile::tempdir().expect("fixture");
    let state_dir = temp.path().join("outside");

    let output = run(&state_dir, &["play", "cyberpunk-2077", "--json"]);

    assert!(!output.status.success());
    assert_eq!(error_code(&output), "LAUNCH_INSTALLATION_INVALID");
}

#[test]
fn refuses_play_of_an_unregistered_path() {
    let temp = tempfile::tempdir().expect("fixture");
    let game = cyberpunk(&temp);
    let state_dir = temp.path().join("outside");

    let output = run(
        &state_dir,
        &[
            "play",
            "cyberpunk-2077",
            "--path",
            game.to_str().expect("game path"),
            "--json",
        ],
    );

    assert!(!output.status.success());
    assert_eq!(error_code(&output), "LAUNCH_INSTALLATION_UNREGISTERED");
    assert!(output.stdout.is_empty());
}

#[test]
fn refuses_play_while_recovery_is_pending() {
    let temp = tempfile::tempdir().expect("fixture");
    let game = cyberpunk(&temp);
    let state_dir = temp.path().join("outside");
    let registered = run(
        &state_dir,
        &[
            "game",
            "add",
            "cyberpunk-2077",
            "--path",
            game.to_str().expect("game path"),
        ],
    );
    assert!(
        registered.status.success(),
        "{}",
        String::from_utf8_lossy(&registered.stderr)
    );
    let connection = Connection::open(state_dir.join("state.sqlite")).expect("state database");
    connection
        .execute(
            "INSERT INTO pending_transactions
             (id, action, status, game_path, package_id, archive_sha256)
             VALUES ('pending', 'install', 'pending', ?1, 'fixture', NULL)",
            [game.to_string_lossy().as_ref()],
        )
        .expect("pending transaction");

    let output = run(
        &state_dir,
        &[
            "play",
            "cyberpunk-2077",
            "--path",
            game.to_str().expect("game path"),
            "--json",
        ],
    );

    assert!(!output.status.success());
    assert_eq!(error_code(&output), "LAUNCH_RECOVERY_PENDING");
    assert!(output.stdout.is_empty());
}

#[test]
fn refuses_a_module_with_a_traversing_executable() {
    let temp = tempfile::tempdir().expect("fixture");
    let module = temp.path().join("launch-bad");
    write(
        &module.join("module.json"),
        br#"{"game_id":"launch-bad","version":"0.1.0","api_version":1,"entrypoint":"module.lua","launch":{"executable":"../Game.exe"}}"#,
    );
    write(
        &module.join("module.lua"),
        br#"return { game_id = "launch-bad", api_version = mods_host.api_version }"#,
    );

    let output = run(
        &temp.path().join("state.sqlite"),
        &[
            "module",
            "check",
            module.to_str().expect("module path"),
            "--json",
        ],
    );

    assert!(!output.status.success());
    assert_eq!(error_code(&output), "MODULE_LAUNCH_TARGET_INVALID");
}

#[test]
fn refuses_a_module_whose_store_uri_scheme_disagrees_with_its_store() {
    let temp = tempfile::tempdir().expect("fixture");
    let module = temp.path().join("launch-bad");
    write(
        &module.join("module.json"),
        br#"{"game_id":"launch-bad","version":"0.1.0","api_version":1,"entrypoint":"module.lua","stores":{"steam":"1091500"},"launch":{"steam":"goggalaxy://openGameView/1"}}"#,
    );
    write(
        &module.join("module.lua"),
        br#"return { game_id = "launch-bad", api_version = mods_host.api_version }"#,
    );

    let output = run(
        &temp.path().join("state.sqlite"),
        &[
            "module",
            "check",
            module.to_str().expect("module path"),
            "--json",
        ],
    );

    assert!(!output.status.success());
    assert_eq!(error_code(&output), "MODULE_LAUNCH_TARGET_INVALID");
}

#[test]
fn refuses_play_when_the_module_declares_no_launch_target() {
    let temp = tempfile::tempdir().expect("fixture");
    let module = temp.path().join("launch-none");
    write(
        &module.join("module.json"),
        br#"{"game_id":"launch-none","version":"0.1.0","api_version":1,"entrypoint":"module.lua"}"#,
    );
    write(
        &module.join("module.lua"),
        br#"local policy = { game_id = "launch-none", api_version = mods_host.api_version, marker = "Game.exe" }
function policy.validate()
    local ok, item = pcall(mods_host.fs.metadata, 1, "Game.exe")
    return ok and item ~= nil and item.kind == "file"
end
return policy"#,
    );
    let game = temp.path().join("Game");
    write(&game.join("Game.exe"), b"fixture");
    let modules = temp.path().join("modules");
    fs::create_dir_all(&modules).expect("modules folder");
    fs::create_dir_all(modules.join("launch-none")).expect("module folder");
    fs::copy(
        module.join("module.json"),
        modules.join("launch-none/module.json"),
    )
    .expect("module manifest");
    fs::copy(
        module.join("module.lua"),
        modules.join("launch-none/module.lua"),
    )
    .expect("module script");
    let state_dir = temp.path().join("state");
    let game_path = game.to_str().expect("game path");
    let added = main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .env("MODN_GAME_MODULES_DIR", &modules)
        .env_remove("MODN_STORE_LIBRARIES")
        .arg("--state-dir")
        .arg(&state_dir)
        .args(["game", "add", "launch-none", "--path", game_path])
        .output()
        .expect("run modn");
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );

    let output = main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .env("MODN_GAME_MODULES_DIR", &modules)
        .env_remove("MODN_STORE_LIBRARIES")
        .arg("--state-dir")
        .arg(&state_dir)
        .args(["play", "launch-none", "--path", game_path, "--json"])
        .output()
        .expect("run modn");

    assert!(!output.status.success());
    assert_eq!(error_code(&output), "LAUNCH_TARGET_UNAVAILABLE");
}
