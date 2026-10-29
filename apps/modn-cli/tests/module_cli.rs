//! End-to-end CLI checks for local module validation without state or game writes.
use std::fs;
use std::process::Command;

#[test]
fn command_reports_module_identity_without_opening_state_database() {
    let root = tempfile::tempdir().expect("fixture");
    let module = root.path().join("module");
    let state = root.path().join("state");
    fs::create_dir(&module).expect("module");
    fs::write(
        module.join("module.json"),
        r#"{"api_version":1,"game_id":"demo-game","version":"1.0.0","entrypoint":"main.luau"}"#,
    )
    .expect("manifest");
    fs::write(
        module.join("main.luau"),
        "return { game_id='demo-game', api_version=mods_host.api_version }",
    )
    .expect("entrypoint");

    let output = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--json", "--state-dir"])
        .arg(&state)
        .arg("module")
        .arg("check")
        .arg(&module)
        .output()
        .expect("run modn");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON result");
    assert_eq!(report["game_id"], "demo-game");
    assert_eq!(report["api_version"], 1);
    assert_eq!(report["version"], "1.0.0");
    assert!(
        !state.exists(),
        "module inspection must not initialize SQLite"
    );

    fs::write(
        module.join("main.luau"),
        "print('unexpected stdout'); return {game_id='demo-game',api_version=1}",
    )
    .expect("replace module with printing script");
    let printing = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--json", "--state-dir"])
        .arg(&state)
        .arg("module")
        .arg("check")
        .arg(&module)
        .output()
        .expect("run printing module");
    assert!(!printing.status.success(), "module print must be denied");
    assert!(printing.stdout.is_empty(), "untrusted stdout must be empty");
    assert!(!state.exists(), "denied print must not open state");

    fs::write(
        module.join("main.luau"),
        "assert(io == nil); error('script failure')",
    )
    .expect("replace module");
    let failed = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--state-dir"])
        .arg(&state)
        .arg("module")
        .arg("check")
        .arg(&module)
        .output()
        .expect("run invalid mod");
    assert!(!failed.status.success());
    assert!(
        String::from_utf8_lossy(&failed.stderr).contains("script failure"),
        "{}",
        String::from_utf8_lossy(&failed.stderr)
    );
    assert!(!state.exists(), "failure must not initialize SQLite");
}
