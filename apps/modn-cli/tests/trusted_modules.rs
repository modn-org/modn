#[path = "support/main_bucket.rs"]
mod main_bucket;

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn cli(data: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_modn"))
        .env("LOCALAPPDATA", data)
        .env_remove("MODN_GAME_MODULES_DIR")
        .args(args)
        .output()
        .expect("CLI")
}

fn success(output: &Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("JSON")
}

#[test]
fn trusted_registry_copies_checks_and_removes_runtime_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    main_bucket::seed(temp.path());
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/game-modules/sample-game");
    let source = source.to_str().unwrap();
    let denied = cli(temp.path(), &["module", "register", source]);
    assert!(!denied.status.success());
    assert!(String::from_utf8_lossy(&denied.stderr).contains("trust"));
    assert!(!temp.path().join("modn/game-modules").exists());
    let added = success(&cli(
        temp.path(),
        &["--json", "module", "register", source, "--trust"],
    ));
    assert_eq!(added["module"]["game_id"], "sample-game");
    let listed = success(&cli(temp.path(), &["--json", "module", "list"]));
    assert_eq!(listed.as_array().unwrap().len(), 1);
    fs::write(
        temp.path().join("modn/game-modules/sample-game/main.lua"),
        "return {}",
    )
    .unwrap();
    assert!(!cli(temp.path(), &["module", "list"]).status.success());
    success(&cli(
        temp.path(),
        &["--json", "module", "remove", "sample-game"],
    ));
    assert_eq!(
        success(&cli(temp.path(), &["--json", "module", "list"])),
        serde_json::json!([])
    );
    let official =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main/games/cyberpunk-2077");
    assert!(!cli(
        temp.path(),
        &["module", "register", official.to_str().unwrap(), "--trust"]
    )
    .status
    .success());
}
