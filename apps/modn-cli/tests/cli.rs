use serde_json::Value;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn registration_scan_and_diagnostics_are_read_only_for_game() {
    let temp = tempdir().unwrap();
    let game = temp.path().join("Cyberpunk 2077");
    let game_exe = game.join("bin/x64/Cyberpunk2077.exe");
    let archive = game.join("archive/pc/mod/test.archive");
    fs::create_dir_all(game_exe.parent().unwrap()).unwrap();
    fs::create_dir_all(archive.parent().unwrap()).unwrap();
    fs::write(&game_exe, b"game unchanged").unwrap();
    fs::write(&archive, b"mod unchanged").unwrap();
    let state = temp.path().join("outside-game-state");
    let state_str = state.to_str().unwrap();
    let game_str = game.to_str().unwrap();

    let add = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args([
            "--state-dir",
            state_str,
            "game",
            "add",
            "cyberpunk",
            "--path",
            game_str,
        ])
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "{:?}",
        String::from_utf8_lossy(&add.stderr)
    );

    let list = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--state-dir", state_str, "game", "list", "--json"])
        .output()
        .unwrap();
    assert!(list.status.success());
    let registered: Value = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(registered.as_array().unwrap().len(), 1);

    let scan = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--state-dir", state_str, "scan", "--json"])
        .output()
        .unwrap();
    assert!(scan.status.success());
    let scanned: Value = serde_json::from_slice(&scan.stdout).unwrap();
    assert_eq!(scanned["mods"][0]["kind"], "legacy-archive");
    assert_eq!(scanned["mods"][0]["managed"], false);

    let doctor = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--state-dir", state_str, "doctor", "--json"])
        .output()
        .unwrap();
    assert!(doctor.status.success());
    let diagnosed: Value = serde_json::from_slice(&doctor.stdout).unwrap();
    assert_eq!(diagnosed["scan"]["redmod_available"], false);
    assert!(diagnosed["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["code"] == "UNMANAGED_CONTENT"));

    assert_eq!(fs::read(game_exe).unwrap(), b"game unchanged");
    assert_eq!(fs::read(archive).unwrap(), b"mod unchanged");
    assert!(!game.join("state.sqlite").exists());
}

#[test]
fn refuses_to_register_a_non_game_directory() {
    let temp = tempdir().unwrap();
    let state = temp.path().join("state");
    let result = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "game",
            "add",
            "cyberpunk",
            "--path",
            temp.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Cyberpunk2077.exe"));
}

#[test]
fn refuses_state_database_inside_game_directory() {
    let temp = tempdir().unwrap();
    let game = temp.path().join("Cyberpunk 2077");
    let executable = game.join("bin/x64/Cyberpunk2077.exe");
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    fs::write(&executable, b"original").unwrap();
    let state = game.join("private/manager-state");
    let output = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "scan",
            "--path",
            game.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("state directory must be outside"));
    assert!(!game.join("private").exists());
    assert_eq!(fs::read(executable).unwrap(), b"original");
}

#[test]
fn modn_name_and_default_state_directory() {
    let temp = tempdir().unwrap();
    let exe = env!("CARGO_BIN_EXE_modn");
    let help = Command::new(exe).arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("Usage: modn"));

    let output = Command::new(exe)
        .env("LOCALAPPDATA", temp.path())
        .args(["game", "list"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(temp.path().join("modn/state.sqlite").is_file());
    assert!(!temp.path().join("mods/state.sqlite").exists());
    assert!(!temp.path().join("Mods/state.sqlite").exists());
}
