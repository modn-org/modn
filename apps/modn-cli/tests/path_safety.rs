//! Regression cases for canonical game identity and manager-state path safety.
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;
use zip::write::SimpleFileOptions;

fn create_modules(root: &Path) -> PathBuf {
    let modules = root.join("modules");
    let sources = [
        ("test-game", "../../tests/fixtures/test-game"),
        (
            "cyberpunk-2077",
            "../../crates/modn-core/game-modules/cyberpunk-2077",
        ),
    ];
    for (id, source) in sources {
        let destination = modules.join(id);
        fs::create_dir_all(&destination).expect("fixture setup or assertion");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(source);
        for name in ["module.json", "main.lua"] {
            fs::copy(source.join(name), destination.join(name))
                .expect("fixture setup or assertion");
        }
    }
    modules
}

fn run(modules: &Path, state: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_modn"))
        .env("MODN_GAME_MODULES_DIR", modules)
        .arg("--state-dir")
        .arg(state)
        .args(args)
        .output()
        .expect("fixture setup or assertion")
}

#[test]
fn rejects_nonexistent_parent_then_dotdot_in_state_directory() {
    let temp = tempdir().expect("fixture setup or assertion");
    let modules = create_modules(temp.path());
    let game = temp.path().join("game");
    let marker = game.join("bin/TestGame.exe");
    fs::create_dir_all(marker.parent().expect("fixture setup or assertion"))
        .expect("fixture setup or assertion");
    fs::write(&marker, b"fixture").expect("fixture setup or assertion");
    let malformed_state = temp.path().join("missing/../game/private");
    let output = run(
        &modules,
        &malformed_state,
        &[
            "game",
            "add",
            "test-game",
            "--path",
            game.to_str().expect("fixture setup or assertion"),
        ],
    );
    assert!(
        !output.status.success(),
        "state traversal accepted: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("parent-directory"),
        "wrong failure: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !game.join("private").exists(),
        "state was created inside game"
    );
    assert!(
        !temp.path().join("missing").exists(),
        "unexpected directory created"
    );
    assert_eq!(
        fs::read(marker).expect("fixture setup or assertion"),
        b"fixture"
    );
}

#[cfg(windows)]
fn redirect_old_to_new(old: &Path, new: &Path) {
    let output = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(old)
        .arg(new)
        .output()
        .expect("fixture setup or assertion");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
fn redirect_old_to_new(old: &Path, new: &Path) {
    std::os::unix::fs::symlink(new, old).expect("fixture setup or assertion");
}

#[test]
fn relocated_registration_cannot_be_reinterpreted_through_directory_alias() {
    let temp = tempdir().expect("fixture setup or assertion");
    let modules = create_modules(temp.path());
    let old = temp.path().join("old-game");
    let new = temp.path().join("new-game");
    fs::create_dir_all(old.join("bin/x64")).expect("fixture setup or assertion");
    fs::write(old.join("bin/TestGame.exe"), b"test-marker").expect("fixture setup or assertion");
    fs::write(old.join("bin/x64/Cyberpunk2077.exe"), b"cyberpunk-marker")
        .expect("fixture setup or assertion");
    let state = temp.path().join("state");
    let registered = run(
        &modules,
        &state,
        &[
            "game",
            "add",
            "test-game",
            "--path",
            old.to_str().expect("fixture setup or assertion"),
        ],
    );
    assert!(
        registered.status.success(),
        "{}",
        String::from_utf8_lossy(&registered.stderr)
    );
    fs::rename(&old, &new).expect("fixture setup or assertion");
    redirect_old_to_new(&old, &new);
    assert_eq!(
        old.canonicalize().expect("fixture setup or assertion"),
        new.canonicalize().expect("fixture setup or assertion")
    );

    let resolved = run(
        &modules,
        &state,
        &[
            "--json",
            "scan",
            "--path",
            old.to_str().expect("fixture setup or assertion"),
        ],
    );
    assert!(
        resolved.status.success(),
        "registered alias not resolved: {}",
        String::from_utf8_lossy(&resolved.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&resolved.stdout).expect("fixture setup or assertion");
    assert_eq!(report["game_id"], "test-game");

    let zip_path = temp.path().join("cyberpunk.zip");
    let mut writer =
        zip::ZipWriter::new(File::create(&zip_path).expect("fixture setup or assertion"));
    writer
        .start_file("archive/pc/mod/wrong.archive", SimpleFileOptions::default())
        .expect("fixture setup or assertion");
    writer
        .write_all(b"not-owned")
        .expect("fixture setup or assertion");
    writer.finish().expect("fixture setup or assertion");

    for approval in ["--dry-run", "--yes"] {
        let output = run(
            &modules,
            &state,
            &[
                "--game",
                "cyberpunk-2077",
                "install",
                zip_path.to_str().expect("fixture setup or assertion"),
                approval,
                "--path",
                old.to_str().expect("fixture setup or assertion"),
            ],
        );
        assert!(
            !output.status.success()
                && String::from_utf8_lossy(&output.stderr).contains("different game"),
            "{approval} accepted different policy: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!new.join("archive").exists(), "wrong policy wrote to game");
    }
}
