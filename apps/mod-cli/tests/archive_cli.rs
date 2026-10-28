use serde_json::Value;
use std::{
    fs::{self, File},
    io::Write,
    process::Command,
};
use tempfile::tempdir;

#[test]
fn inspect_and_plan_json_do_not_write_game_files() {
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

    let inspect = Command::new(env!("CARGO_BIN_EXE_mod"))
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

    let plan = Command::new(env!("CARGO_BIN_EXE_mod"))
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "--json",
            "plan",
            archive.to_str().unwrap(),
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
    assert_eq!(report["status"], "reviewable");
    assert_eq!(report["metadata_only"], true);
    assert_eq!(
        report["operations"][0]["destination"],
        "archive/pc/mod/weather.archive"
    );
    assert!(!game.join("archive").exists());

    fs::create_dir_all(game.join("archive/pc/mod")).unwrap();
    fs::write(game.join("archive/pc/mod/weather.archive"), "unmanaged").unwrap();
    let plan = Command::new(env!("CARGO_BIN_EXE_mod"))
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "--json",
            "plan",
            archive.to_str().unwrap(),
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
