use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn removed_adopt_command_cannot_create_ownership() {
    let temp = tempdir().expect("fixture");
    let game = temp.path().join("game");
    let state = temp.path().join("state");
    fs::create_dir_all(game.join("bin/x64")).expect("game");
    fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").expect("game marker");
    let manual = game.join("r6/scripts/Manual.reds");
    fs::create_dir_all(manual.parent().expect("parent")).expect("manual folder");
    fs::write(&manual, b"manual mod").expect("manual mod");

    let binary = env!("CARGO_BIN_EXE_modn");
    let help = Command::new(binary)
        .arg("--help")
        .output()
        .expect("CLI help");
    assert!(help.status.success());
    let usage = String::from_utf8_lossy(&help.stdout);
    assert!(!usage
        .lines()
        .any(|line| line.trim_start().starts_with("adopt ")));

    let denied = Command::new(binary)
        .args(["--game", "cyberpunk-2077", "--state-dir"])
        .arg(&state)
        .args(["adopt", "--dry-run", "--path"])
        .arg(&game)
        .output()
        .expect("removed CLI command");
    assert!(!denied.status.success());
    assert!(
        String::from_utf8_lossy(&denied.stderr).contains("unrecognized subcommand"),
        "{}",
        String::from_utf8_lossy(&denied.stderr)
    );
    assert_eq!(fs::read(&manual).expect("manual preserved"), b"manual mod");
    assert!(
        !state.exists(),
        "unknown command must not create manager state"
    );
}
