use serde_json::Value;
use std::fs::{self, File};
use std::io::Write;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn inspect_and_dry_run_json_do_not_write_game_files() {
    let temp = tempdir().unwrap();
    let archive = temp.path().join("weather.zip");
    let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
    zip.start_file(
        "archive/pc/mod/weather.archive",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(b"weather-fixture").unwrap();
    zip.finish().unwrap();

    let game = temp.path().join("game");
    fs::create_dir_all(game.join("bin/x64")).unwrap();
    fs::write(game.join("bin/x64/Cyberpunk2077.exe"), "game").unwrap();
    let state = temp.path().join("state");

    let inspect = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "--json",
            "inspect",
            archive.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        inspect.status.success(),
        "{}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let inventory: Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(inventory["format"], "zip");
    assert_eq!(inventory["metadata_only"], true);
    assert!(!state.exists());

    let plan = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "--json",
            "install",
            archive.to_str().unwrap(),
            "--dry-run",
            "--path",
            game.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    let report: Value = serde_json::from_slice(&plan.stdout).unwrap();
    assert_eq!(report["status"], "reviewable", "{report}");
    assert_eq!(report["metadata_only"], true);
    assert_eq!(
        report["operations"][0]["destination"],
        "archive/pc/mod/weather.archive"
    );
    assert!(!game.join("archive").exists());
    assert!(!state.exists(), "dry-run must not initialize manager state");

    fs::create_dir_all(game.join("archive/pc/mod")).unwrap();
    fs::write(game.join("archive/pc/mod/weather.archive"), "unmanaged").unwrap();
    let plan = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "--json",
            "install",
            archive.to_str().unwrap(),
            "--dry-run",
            "--path",
            game.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(plan.status.success());
    let report: Value = serde_json::from_slice(&plan.stdout).unwrap();
    assert_eq!(report["status"], "blocked");
    assert_eq!(
        fs::read(game.join("archive/pc/mod/weather.archive")).unwrap(),
        b"unmanaged"
    );
}

#[test]
fn registered_install_dry_run_reads_existing_db_without_changes() {
    let temp = tempdir().unwrap();
    let game = temp.path().join("game");
    fs::create_dir_all(game.join("bin/x64")).unwrap();
    fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").unwrap();
    let archive = temp.path().join("mod.zip");
    let mut writer = zip::ZipWriter::new(File::create(&archive).unwrap());
    writer
        .start_file(
            "archive/pc/mod/example.archive",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    writer.write_all(b"fixture").unwrap();
    writer.finish().unwrap();

    let state = temp.path().join("state");
    let registered = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "game",
            "add",
            "cyberpunk",
            "--path",
            game.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        registered.status.success(),
        "{}",
        String::from_utf8_lossy(&registered.stderr)
    );
    let database = state.join("state.sqlite");
    let before = fs::read(&database).unwrap();

    let preview = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "--json",
            "install",
            archive.to_str().unwrap(),
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    let report: Value = serde_json::from_slice(&preview.stdout).unwrap();
    assert_eq!(report["status"], "reviewable", "{report}");
    assert_eq!(fs::read(database).unwrap(), before);
    assert!(!game.join("archive").exists());
}
