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
        r#"{"api_version":1,"game_id":"demo-game","version":"1.0.0","entrypoint":"main.lua"}"#,
    )
    .expect("manifest");
    fs::write(
        module.join("main.lua"),
        "return { game_id='demo-game', api_version=mods_host.api_version }",
    )
    .expect("entrypoint");

    let output = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--game", "cyberpunk-2077"])
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
        module.join("main.lua"),
        "print('unexpected stdout'); return {game_id='demo-game',api_version=1}",
    )
    .expect("replace module with printing script");
    let printing = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--game", "cyberpunk-2077"])
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
        module.join("main.lua"),
        "assert(io == nil); error('script failure')",
    )
    .expect("replace module");
    let failed = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--game", "cyberpunk-2077"])
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

#[test]
fn scoped_cli_roots_work_without_creating_state_or_writing_to_game() {
    let temp = tempfile::tempdir().expect("fixture");
    let module = temp.path().join("module");
    let game = temp.path().join("authorized-game");
    let state = temp.path().join("state");
    fs::create_dir(&module).expect("module dir");
    fs::create_dir(&game).expect("game dir");
    fs::write(
        module.join("module.json"),
        r#"{"api_version":1,"game_id":"demo-game","version":"1.0.0","entrypoint":"main.lua"}"#,
    )
    .expect("manifest");
    fs::write(game.join("marker.txt"), b"abc").expect("marker");
    fs::write(
        module.join("main.lua"),
        r#"
            local roots = mods_host.discovery.roots()
            assert(#roots == 1 and roots[1].id == 1)
            assert(mods_host.fs.metadata(1, "marker.txt").bytes == 3)
            assert(mods_host.hash.sha256(1, "marker.txt") ==
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
            return { game_id="demo-game", api_version=mods_host.api_version }
        "#,
    )
    .expect("script");

    let command = || {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_modn"));
        cmd.args(["--game", "cyberpunk-2077"]);
        cmd.args(["--json", "--state-dir"])
            .arg(&state)
            .args(["module", "check"])
            .arg(&module);
        cmd
    };
    let missing_grant = command().output().expect("run without grant");
    assert!(!missing_grant.status.success());
    assert!(missing_grant.stdout.is_empty());

    let allowed = command()
        .arg("--allow-root")
        .arg(&game)
        .output()
        .expect("run with explicit grant");
    assert!(
        allowed.status.success(),
        "{}",
        String::from_utf8_lossy(&allowed.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&allowed.stdout).expect("valid JSON");
    assert_eq!(report["game_id"], "demo-game");
    assert_eq!(report["api_version"], 1);
    assert_eq!(
        fs::read(game.join("marker.txt")).expect("marker intact"),
        b"abc"
    );
    assert!(
        !state.exists(),
        "module capabilities must not create manager state"
    );
}

#[test]
fn standalone_module_check_ignores_existing_invalid_manager_state() {
    let root = tempfile::tempdir().expect("fixture");
    let module = root.path().join("module");
    let state = root.path().join("state");
    fs::create_dir(&module).expect("module");
    fs::create_dir(&state).expect("state fixture");
    fs::write(state.join("state.sqlite"), b"not a SQLite database").expect("corrupted state");
    fs::write(
        module.join("module.json"),
        r#"{"api_version":1,"game_id":"demo-game","version":"1.0.0","entrypoint":"main.lua"}"#,
    )
    .expect("manifest");
    fs::write(
        module.join("main.lua"),
        "return { game_id='demo-game', api_version=mods_host.api_version }",
    )
    .expect("entrypoint");
    let output = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--state-dir", state.to_str().unwrap(), "module", "check"])
        .arg(&module)
        .output()
        .expect("module check");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read(state.join("state.sqlite")).unwrap(),
        b"not a SQLite database"
    );
}
