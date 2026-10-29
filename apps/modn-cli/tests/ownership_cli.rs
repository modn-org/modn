use serde_json::Value;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::tempdir;
use zip::write::SimpleFileOptions;

#[expect(clippy::unwrap_used, reason = "Test helpers may fail fast")]
fn call(state: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--game", "cyberpunk-2077"])
        .args(["--state-dir", state.to_str().unwrap(), "--json"])
        .args(args)
        .output()
        .unwrap()
}

#[expect(clippy::unwrap_used, reason = "Test helpers may fail fast")]
fn output_json(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn scan_and_doctor_reconcile_ownership_without_game_writes() {
    let root = tempdir().unwrap();
    let game = root.path().join("game");
    let state = root.path().join("state");
    fs::create_dir_all(game.join("bin/x64")).unwrap();
    fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"fixture").unwrap();
    let zip = root.path().join("one.zip");
    let mut writer = zip::ZipWriter::new(File::create(&zip).unwrap());
    writer
        .start_file("archive/pc/mod/one.archive", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"known bytes").unwrap();
    writer.finish().unwrap();

    let game_text = game.to_str().unwrap();
    let zip_text = zip.to_str().unwrap();
    let before = output_json(call(&state, &["scan", "--path", game_text]));
    assert!(before["owned_files"].as_array().unwrap().is_empty());
    let package = output_json(call(
        &state,
        &["install", zip_text, "--path", game_text, "--yes"],
    ));
    assert_eq!(package["files"].as_array().unwrap().len(), 1);

    let target = game.join("archive/pc/mod/one.archive");
    let expected = fs::read(&target).unwrap();
    let scanned = output_json(call(&state, &["scan", "--path", game_text]));
    assert_eq!(scanned["mods"][0]["managed"], true);
    assert_eq!(scanned["mods"][0]["integrity"], "intact");
    assert_eq!(scanned["owned_files"][0]["status"], "intact");
    assert_eq!(fs::read(&target).unwrap(), expected);

    fs::write(&target, b"other bytes").unwrap();
    let doctor = output_json(call(&state, &["doctor", "--path", game_text]));
    assert!(doctor["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["code"] == "OWNED_FILE_MODIFIED"));
    assert_eq!(doctor["scan"]["owned_files"][0]["status"], "modified");
    assert_eq!(fs::read(&target).unwrap(), b"other bytes");
}
