#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/catalog.rs"]
mod support;

use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use support::{error, Fixture};

fn contents(root: &Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    if !root.exists() {
        return files;
    }
    for entry in fs::read_dir(root).expect("fixture directory") {
        let entry = entry.expect("fixture entry");
        if entry.file_type().expect("file type").is_dir() {
            files.extend(contents(&entry.path()));
        } else {
            files.insert(entry.path(), fs::read(entry.path()).expect("fixture bytes"));
        }
    }
    files
}

#[test]
fn missing_database_and_empty_registration_errors_keep_their_precedence() {
    let fixture = Fixture::new();
    for args in [
        vec!["list"],
        vec!["scan"],
        vec!["doctor"],
        vec!["uninstall", "missing", "--dry-run"],
        vec!["uninstall", "missing"],
        vec!["pin", "missing"],
    ] {
        let output = fixture.run(&args);
        assert!(!output.status.success());
        assert!(
            error(&output).contains("no registered game database for preview"),
            "{args:?}: {}",
            error(&output)
        );
        assert!(!fixture.state.exists());
    }
    fixture.register();
    let removed = fixture.run(&["game", "remove", "test-game"]);
    assert!(removed.status.success(), "{}", error(&removed));
    let before = contents(&fixture.state);
    for command in ["list", "scan", "doctor"] {
        let output = fixture.run(&[command]);
        assert!(!output.status.success());
        assert!(
            error(&output).contains("no matching game registered"),
            "{}",
            error(&output)
        );
    }
    assert_eq!(contents(&fixture.state), before);
}

#[test]
fn explicit_archive_preview_needs_identity_and_never_creates_state() {
    let fixture = Fixture::new();
    let (archive, _) = fixture.archive();
    let archive = archive.to_str().expect("archive path");
    let path = fixture.game.to_str().expect("game path");
    let denied = fixture.run(&["install", archive, "--dry-run", "--path", path]);
    assert!(!denied.status.success());
    assert!(
        error(&denied).contains("unregistered installation"),
        "{}",
        error(&denied)
    );
    assert!(!fixture.state.exists());

    let allowed = fixture.run(&[
        "--game",
        "test-game",
        "--json",
        "install",
        archive,
        "--dry-run",
        "--path",
        path,
    ]);
    assert!(allowed.status.success(), "{}", error(&allowed));
    let report: Value = serde_json::from_slice(&allowed.stdout).expect("preview JSON");
    assert_eq!(report["status"], "reviewable");
    assert!(!fixture.state.exists());
    assert!(!fixture.game.join("addons").exists());
}

#[test]
fn registered_previews_leave_every_manager_file_unchanged() {
    let fixture = Fixture::new();
    let (archive, _) = fixture.archive();
    fixture.register();
    let before = contents(&fixture.state);
    let game_before = contents(&fixture.game);
    for explicit in [false, true] {
        let mut args = vec![
            "install",
            archive.to_str().expect("archive path"),
            "--dry-run",
        ];
        if explicit {
            args.extend(["--path", fixture.game.to_str().expect("game path")]);
        }
        let output = fixture.run(&args);
        assert!(output.status.success(), "{}", error(&output));
        assert_eq!(contents(&fixture.state), before);
        assert_eq!(contents(&fixture.game), game_before);
    }
}

#[test]
fn protected_state_error_precedes_explicit_game_selection() {
    let mut fixture = Fixture::new();
    fixture.state = fixture.game.join("private-manager");
    for command in ["list", "scan", "doctor"] {
        let output = fixture.run_paths(&[command, "--path"], &[&fixture.game]);
        assert!(!output.status.success());
        assert!(
            error(&output).contains("state directory must be outside a game installation"),
            "{command}: {}",
            error(&output)
        );
        assert!(!fixture.state.exists());
    }
}

#[test]
fn standalone_install_requires_approval_before_explicit_identity() {
    let fixture = Fixture::new();
    let (archive, _) = fixture.archive();
    let output = fixture.run(&[
        "install",
        archive.to_str().expect("archive path"),
        "--path",
        fixture.game.to_str().expect("game path"),
    ]);
    assert!(!output.status.success());
    assert!(
        error(&output).contains("explicitly approve with --yes"),
        "{}",
        error(&output)
    );
    assert!(!fixture.game.join("addons").exists());
}
