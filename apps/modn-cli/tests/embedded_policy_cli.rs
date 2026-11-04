#[path = "support/main_bucket.rs"]
mod main_bucket;
use std::fs;
use std::path::Path;

#[test]
fn policy_beside_the_executable_is_ignored() {
    let root = tempfile::tempdir().expect("fixture root");
    let bin = root.path().join("bin");
    fs::create_dir_all(&bin).expect("bin directory");
    let source = Path::new(env!("CARGO_BIN_EXE_modn"));
    let copied = bin.join(source.file_name().expect("binary name"));
    fs::copy(source, &copied).expect("copy modn binary");

    let planted = bin.join("game-modules/cyberpunk-2077");
    fs::create_dir_all(&planted).expect("planted module");
    let bundled = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../buckets/main/games/cyberpunk-2077/module.json");
    fs::copy(bundled, planted.join("module.json")).expect("planted manifest");
    fs::write(planted.join("main.lua"), "error('planted policy loaded')").expect("planted Lua");

    let game = root.path().join("not-a-game");
    fs::create_dir_all(&game).expect("game directory");
    let output = main_bucket::command(&copied)
        .env_remove("MODN_GAME_MODULES_DIR")
        .current_dir(root.path())
        .arg("--state-dir")
        .arg(root.path().join("state"))
        .args(["game", "add", "cyberpunk-2077", "--path"])
        .arg(&game)
        .output()
        .expect("run copied modn");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "a non-game directory is rejected");
    assert!(!stderr.contains("planted policy loaded"), "{stderr}");
    assert!(stderr.contains("does not contain"), "{stderr}");
    assert!(!root.path().join("state/state.sqlite").exists());
}
