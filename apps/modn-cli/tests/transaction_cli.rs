use serde_json::Value;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::tempdir;
use zip::write::SimpleFileOptions;

#[expect(clippy::unwrap_used, reason = "Fixture setup may fail fast in tests")]
fn cli(state: &Path, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_modn"));
    cmd.args(["--game", "cyberpunk-2077"]);
    cmd.args(["--state-dir", state.to_str().unwrap(), "--json"]);
    cmd.args(args).output().unwrap()
}

#[test]
fn cli_requires_opt_in_and_tracks_and_uninstalls_own_files() {
    let dir = tempdir().unwrap();
    let state = dir.path().join("state");
    let game = dir.path().join("game");
    fs::create_dir_all(game.join("bin/x64")).unwrap();
    fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").unwrap();
    let archive = dir.path().join("hd.zip");
    let mut writer = zip::ZipWriter::new(File::create(&archive).unwrap());
    writer
        .start_file(
            "archive/pc/mod/HD Reworked.archive",
            SimpleFileOptions::default(),
        )
        .unwrap();
    writer.write_all(b"archive-file").unwrap();
    writer.finish().unwrap();

    let args = [
        "install",
        archive.to_str().unwrap(),
        "--path",
        game.to_str().unwrap(),
    ];
    let denied = cli(&state, &args);
    assert!(!denied.status.success());
    assert!(String::from_utf8_lossy(&denied.stderr).contains("--yes"));
    assert!(!game.join("archive").exists());

    let result = cli(
        &state,
        &[
            "install",
            archive.to_str().unwrap(),
            "--path",
            game.to_str().unwrap(),
            "--yes",
        ],
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let value: Value = serde_json::from_slice(&result.stdout).unwrap();
    let id = value["id"].as_str().unwrap();
    assert_eq!(
        value["files"][0]["destination"],
        "archive/pc/mod/HD Reworked.archive"
    );
    assert_eq!(
        fs::read(game.join("archive/pc/mod/HD Reworked.archive")).unwrap(),
        b"archive-file"
    );

    let result = cli(&state, &["list", "--path", game.to_str().unwrap()]);
    let list: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);

    let denied = cli(&state, &["uninstall", id, "--path", game.to_str().unwrap()]);
    assert!(!denied.status.success());
    assert!(game.join("archive/pc/mod/HD Reworked.archive").is_file());

    let done = cli(
        &state,
        &["uninstall", id, "--path", game.to_str().unwrap(), "--yes"],
    );
    assert!(
        done.status.success(),
        "{}",
        String::from_utf8_lossy(&done.stderr)
    );
    assert!(!game.join("archive/pc/mod/HD Reworked.archive").exists());

    let result = cli(&state, &["list", "--path", game.to_str().unwrap()]);
    let list: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 0);

    let denied = cli(&state, &["recover"]);
    assert!(!denied.status.success());
    let recovered = cli(&state, &["recover", "--yes"]);
    assert!(recovered.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&recovered.stdout).unwrap()["recovered"],
        0
    );
}
