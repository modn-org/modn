#[path = "support/main_bucket.rs"]
mod main_bucket;

use std::fs;
use std::path::Path;
use std::process::Output;
use tempfile::TempDir;

fn run(state: &Path, appdata: &Path, args: &[&str]) -> Output {
    main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .env("APPDATA", appdata)
        .env("MODN_APPDATA", appdata)
        .env_remove("MODN_GAME_MODULES_DIR")
        .args(["--state-dir"])
        .arg(state)
        .args(["--json", "module"])
        .args(args)
        .output()
        .expect("test fixture invariant")
}

fn output(result: &Output) -> String {
    String::from_utf8_lossy(&result.stderr).to_string()
}

#[test]
fn module_registry_destination_inside_game_root_is_refused() {
    let temp = TempDir::new().expect("test fixture invariant");
    let game = temp.path().join("game");
    let marker = game.join("bin/x64/Cyberpunk2077.exe");
    fs::create_dir_all(marker.parent().expect("marker parent")).expect("fixture game");
    fs::write(&marker, b"original marker").expect("fixture game marker");
    let state = game.join("profile");
    fs::create_dir_all(&state).expect("fixture profile");
    main_bucket::seed_state(&state);
    let appdata = temp.path().join("appdata");
    fs::create_dir_all(&appdata).expect("fixture appdata");

    let result = run(&state, &appdata, &["remove", "cyberpunk-2077"]);

    assert!(!result.status.success(), "{}", output(&result));
    assert!(
        output(&result).contains("outside a game installation"),
        "{}",
        output(&result)
    );
    assert!(!state.join("game-modules").exists());
    assert!(!state.join("state.sqlite").exists());
    assert_eq!(fs::read(&marker).expect("marker"), b"original marker");
}
