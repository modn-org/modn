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
    let bundled =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main/games/cyberpunk-2077");
    main_bucket::copy_main_module(&bundled, &planted);
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(planted.join("module.json")).expect("module manifest"))
            .expect("module manifest JSON");
    let entrypoint = manifest["entrypoint"]
        .as_str()
        .expect("the manifest names its entrypoint")
        .to_owned();
    fs::write(planted.join(&entrypoint), "error('planted policy loaded')").expect("planted Lua");
    assert_eq!(
        fs::read_to_string(planted.join(&entrypoint)).expect("manifest entrypoint"),
        "error('planted policy loaded')",
        "the marker must live in the entrypoint the loader reads"
    );

    let game = root.path().join("not-a-game");
    fs::create_dir_all(&game).expect("game directory");
    main_bucket::seed_state(&root.path().join("state"));
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

    let checked = main_bucket::command(&copied)
        .env_remove("MODN_GAME_MODULES_DIR")
        .current_dir(root.path())
        .args(["module", "check"])
        .arg(&planted)
        .output()
        .expect("check planted module");
    let checked_stderr = String::from_utf8_lossy(&checked.stderr);
    assert!(
        !checked.status.success(),
        "the planted marker fails the check"
    );
    assert!(
        checked_stderr.contains("planted policy loaded"),
        "{checked_stderr}"
    );
}
