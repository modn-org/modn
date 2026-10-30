use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::tempdir;

#[expect(
    clippy::unwrap_used,
    reason = "Disposable fixture commands may fail fast"
)]
fn cli(state: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--game", "cyberpunk-2077"])
        .args(["--state-dir", state.to_str().unwrap(), "--json"])
        .args(args)
        .output()
        .unwrap()
}

#[expect(clippy::unwrap_used, reason = "Test assertion helper")]
fn checked(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn preview_adopt_scan_doctor_uninstall_and_drift() {
    let temp = tempdir().unwrap();
    let game = temp.path().join("game");
    let state = temp.path().join("state");
    let mod_file = game.join("archive/pc/mod/Existing.archive");
    let neighbor = game.join("archive/pc/mod/Neighbor.archive");
    fs::create_dir_all(mod_file.parent().unwrap()).unwrap();
    fs::create_dir_all(game.join("bin/x64")).unwrap();
    fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").unwrap();
    fs::write(&mod_file, b"manual contents").unwrap();
    fs::write(&neighbor, b"other manual mod").unwrap();
    let game_text = game.to_str().unwrap();
    let selector = "archive/pc/mod/Existing.archive";

    let preview = checked(cli(&state, &["adopt", "--dry-run", "--path", game_text]));
    assert!(preview["consequence"].as_str().unwrap().contains("DELETE"));
    assert_eq!(preview["packages"].as_array().unwrap().len(), 2);
    assert_eq!(preview["packages"][0]["files"][0]["bytes"], 15);
    assert_eq!(
        preview["packages"][0]["files"][0]["sha256"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    assert!(!state.exists(), "dry run must not create manager state");
    let before = checked(cli(&state, &["scan", "--path", game_text]));
    assert_eq!(before["mods"][0]["managed"], false);
    let denied = cli(&state, &["adopt", selector, "--path", game_text]);
    assert!(!denied.status.success());

    let original_metadata = fs::metadata(&mod_file).unwrap();
    let original_identity = same_file::Handle::from_path(&mod_file).unwrap();
    let adopted = checked(cli(
        &state,
        &["adopt", selector, "--yes", "--path", game_text],
    ));
    assert_eq!(adopted["origin"], "manual");
    assert!(adopted["archive_sha256"].is_null());
    assert_eq!(adopted["files"].as_array().unwrap().len(), 1);
    assert_eq!(fs::read(&mod_file).unwrap(), b"manual contents");
    assert_eq!(
        fs::metadata(&mod_file).unwrap().modified().unwrap(),
        original_metadata.modified().unwrap()
    );
    assert_eq!(
        same_file::Handle::from_path(&mod_file).unwrap(),
        original_identity
    );
    let id = adopted["id"].as_str().unwrap();
    let again = cli(&state, &["adopt", selector, "--yes", "--path", game_text]);
    assert!(!again.status.success());

    let scan = checked(cli(&state, &["scan", "--path", game_text]));
    assert_eq!(scan["owned_files"][0]["status"], "intact");
    assert_eq!(scan["mods"][0]["managed"], true);
    assert_eq!(scan["mods"][1]["managed"], false);
    let doctor = checked(cli(&state, &["doctor", "--path", game_text]));
    assert_eq!(doctor["scan"]["owned_files"][0]["status"], "intact");

    fs::write(&mod_file, b"changed contents").unwrap();
    assert!(
        !cli(&state, &["uninstall", id, "--yes", "--path", game_text])
            .status
            .success()
    );
    assert_eq!(fs::read(&mod_file).unwrap(), b"changed contents");
    fs::write(&mod_file, b"manual contents").unwrap();
    checked(cli(
        &state,
        &["uninstall", id, "--yes", "--path", game_text],
    ));
    assert!(!mod_file.exists());
    assert_eq!(fs::read(&neighbor).unwrap(), b"other manual mod");
}

#[test]
fn group_is_all_or_nothing_and_refuses_unsafe_files() {
    let temp = tempdir().unwrap();
    let game = temp.path().join("game");
    let state = temp.path().join("state");
    let folder = game.join("red4ext/plugins/Custom");
    fs::create_dir_all(&folder).unwrap();
    fs::create_dir_all(game.join("bin/x64")).unwrap();
    fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").unwrap();
    fs::write(folder.join("entry.dll"), b"plugin dll").unwrap();
    fs::write(folder.join("asset.bin"), b"asset").unwrap();
    let game_text = game.to_str().unwrap();
    let selector = "red4ext/plugins/Custom";
    let preview = checked(cli(&state, &["adopt", "--dry-run", "--path", game_text]));
    assert_eq!(preview["packages"][0]["selector"], selector);
    assert_eq!(preview["packages"][0]["files"].as_array().unwrap().len(), 2);
    fs::write(folder.join("config.json"), b"user preferences").unwrap();
    // Unknown manual content cannot silently be omitted from an adopted group.
    assert!(
        !cli(&state, &["adopt", selector, "--yes", "--path", game_text])
            .status
            .success()
    );
    assert!(
        !state.join("state.sqlite").exists()
            || checked(cli(&state, &["installed", "--path", game_text]))
                .as_array()
                .unwrap()
                .is_empty()
    );
    fs::remove_file(folder.join("config.json")).unwrap();
    let adopted = checked(cli(
        &state,
        &["adopt", selector, "--yes", "--path", game_text],
    ));
    assert_eq!(adopted["files"].as_array().unwrap().len(), 2);
    let id = adopted["id"].as_str().unwrap();
    checked(cli(
        &state,
        &["uninstall", id, "--yes", "--path", game_text],
    ));
    assert!(folder.is_dir());
}

#[cfg(unix)]
#[test]
fn refuses_symlinks_and_hardlinks_without_adopting_anything() {
    use std::os::unix::fs::symlink;
    let temp = tempdir().unwrap();
    let game = temp.path().join("game");
    let state = temp.path().join("state");
    let dir = game.join("archive/pc/mod");
    fs::create_dir_all(&dir).unwrap();
    fs::create_dir_all(game.join("bin/x64")).unwrap();
    fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").unwrap();
    let external = temp.path().join("outside");
    fs::write(&external, b"outside").unwrap();
    symlink(&external, dir.join("link.archive")).unwrap();
    assert!(!cli(
        &state,
        &["adopt", "--dry-run", "--path", game.to_str().unwrap()]
    )
    .status
    .success());
    fs::remove_file(dir.join("link.archive")).unwrap();
    fs::hard_link(&external, dir.join("linked.archive")).unwrap();
    assert!(!cli(
        &state,
        &["adopt", "--dry-run", "--path", game.to_str().unwrap()]
    )
    .status
    .success());
}

#[test]
fn refuses_registered_nested_game_root_before_adopting_any_files() {
    let temp = tempdir().unwrap();
    let game = temp.path().join("game");
    let nested = game.join("nested");
    let state = temp.path().join("state");
    fs::create_dir_all(game.join("bin/x64")).unwrap();
    fs::create_dir_all(nested.join("bin/x64")).unwrap();
    fs::create_dir_all(game.join("archive/pc/mod")).unwrap();
    fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").unwrap();
    fs::write(nested.join("bin/x64/Cyberpunk2077.exe"), b"nested game").unwrap();
    let manual = game.join("archive/pc/mod/manual.archive");
    fs::write(&manual, b"manual").unwrap();
    checked(cli(
        &state,
        &[
            "game",
            "add",
            "cyberpunk-2077",
            "--path",
            nested.to_str().unwrap(),
        ],
    ));
    let attempt = cli(
        &state,
        &[
            "adopt",
            "archive/pc/mod/manual.archive",
            "--yes",
            "--path",
            game.to_str().unwrap(),
        ],
    );
    assert!(!attempt.status.success());
    assert!(String::from_utf8_lossy(&attempt.stderr).contains("overlaps"));
    assert_eq!(fs::read(&manual).unwrap(), b"manual");
}

#[test]
fn supports_redscript_hints_but_refuses_plugin_generated_config() {
    let temp = tempdir().unwrap();
    let game = temp.path().join("game");
    let state = temp.path().join("state");
    let hint = game.join("r6/config/redsUserHints/Compatible.toml");
    let plugin = game.join("red4ext/plugins/Custom");
    fs::create_dir_all(game.join("bin/x64")).unwrap();
    fs::create_dir_all(hint.parent().unwrap()).unwrap();
    fs::create_dir_all(plugin.join("config")).unwrap();
    fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").unwrap();
    fs::write(&hint, b"hint contents").unwrap();
    fs::write(plugin.join("entry.dll"), b"plugin").unwrap();
    fs::write(plugin.join("config/user.toml"), b"generated config").unwrap();
    let root = game.to_str().unwrap();
    let selector = "r6/config/redsUserHints/Compatible.toml";
    let preview = checked(cli(&state, &["adopt", "--dry-run", "--path", root]));
    let packages = preview["packages"].as_array().unwrap();
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0]["selector"], selector);
    assert_eq!(packages[0]["kind"], "redscript-hint");
    assert!(!state.exists(), "dry run must not initialize manager state");
    let adopted = checked(cli(&state, &["adopt", selector, "--yes", "--path", root]));
    assert_eq!(adopted["files"][0]["destination"], selector);
    assert_eq!(fs::read(&hint).unwrap(), b"hint contents");
    checked(cli(
        &state,
        &[
            "uninstall",
            adopted["id"].as_str().unwrap(),
            "--yes",
            "--path",
            root,
        ],
    ));
    assert!(!hint.exists());
    assert_eq!(
        fs::read(plugin.join("config/user.toml")).unwrap(),
        b"generated config"
    );
}
