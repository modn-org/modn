#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/catalog.rs"]
mod support;
#[path = "support/tree.rs"]
mod tree;

use serde_json::Value;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Output;
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

fn fixture() -> (TempDir, PathBuf, PathBuf) {
    let temp = tempdir().expect("temporary fixture");
    let game = temp.path().join("game");
    let state = temp.path().join("state");
    write(&game, "bin/x64/Cyberpunk2077.exe", b"disposable marker");
    write(&game, "mods/Manual/info.json", br#"{"name":"manual"}"#);
    (temp, game, state)
}

fn write(root: &Path, name: &str, bytes: &[u8]) {
    let target = root.join(name);
    fs::create_dir_all(target.parent().expect("fixture parent")).expect("fixture directories");
    fs::write(target, bytes).expect("fixture bytes");
}

fn archive(path: &Path, entries: &[(&str, &[u8])]) {
    let mut zip = zip::ZipWriter::new(File::create(path).expect("fixture archive"));
    for &(name, bytes) in entries {
        zip.start_file(name, SimpleFileOptions::default())
            .expect("archive member");
        zip.write_all(bytes).expect("member bytes");
    }
    zip.finish().expect("closed archive");
}

fn cli(state: &Path, game: &Path, args: &[&str]) -> Output {
    main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .args(["--game", "cyberpunk-2077", "--json", "--state-dir"])
        .arg(state)
        .args(args)
        .arg("--path")
        .arg(game)
        .output()
        .expect("real CLI process")
}

fn ok(state: &Path, game: &Path, args: &[&str]) -> Value {
    let output = cli(state, game, args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("CLI JSON")
}

#[test]
fn adoption_requires_both_approvals_and_commits_exact_file_claims() {
    let (temp, game, state) = fixture();
    let zip = temp.path().join("manual.zip");
    let files = [
        ("archive/pc/mod/A.archive", b"one".as_slice()),
        ("archive/pc/mod/B.archive", b"two".as_slice()),
    ];
    archive(&zip, &files);
    for &(name, bytes) in &files {
        write(&game, name, bytes);
    }
    let before = tree::snapshot(&game);
    assert_eq!(tree::file_count(&before), 4);
    let path = zip.to_str().unwrap();
    let preview = ok(&state, &game, &["adopt", path, "--dry-run"]);
    assert_eq!(preview["status"], "verification_required");
    assert_eq!(preview["target_contents_verified"], false);
    assert_eq!(
        preview["plan"]["archive_sha256"],
        blake3::hash(&fs::read(&zip).unwrap()).to_hex().to_string()
    );
    assert_eq!(preview["plan"]["operations"].as_array().unwrap().len(), 2);
    assert!(!state.exists());
    assert_eq!(tree::snapshot(&game), before);
    for approvals in [vec![], vec!["--yes"], vec!["--acknowledge"]] {
        let mut args = vec!["adopt", path];
        args.extend(approvals);
        assert!(!cli(&state, &game, &args).status.success());
        assert!(!state.exists());
        assert_eq!(tree::snapshot(&game), before);
    }
    assert!(!cli(&state, &game, &["install", path, "--yes"])
        .status
        .success());
    write(&game, files[1].0, b"bad");
    assert!(
        !cli(&state, &game, &["adopt", path, "--acknowledge", "--yes"])
            .status
            .success()
    );
    assert!(ok(&state, &game, &["list"]).as_array().unwrap().is_empty());
    assert_eq!(fs::read(game.join(files[0].0)).unwrap(), files[0].1);
    write(&game, files[1].0, files[1].1);
    let package = ok(&state, &game, &["adopt", path, "--acknowledge", "--yes"]);
    assert_eq!(
        package["archive_sha256"],
        blake3::hash(&fs::read(&zip).unwrap()).to_hex().to_string()
    );
    assert_eq!(package["files"].as_array().unwrap().len(), 2);
    for &(name, bytes) in &files {
        let owned = package["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| file["destination"] == name)
            .unwrap();
        assert_eq!(owned["sha256"], tree::sha(&game.join(name)));
        assert_eq!(owned["bytes"], bytes.len());
    }
    assert!(package["directories"].as_array().unwrap().is_empty());
    assert_eq!(tree::snapshot(&game), before);
    let state_before = tree::snapshot(&state);
    ok(&state, &game, &["adopt", path, "--dry-run"]);
    assert_eq!(tree::snapshot(&state), state_before);
    assert!(
        !cli(&state, &game, &["adopt", path, "--acknowledge", "--yes"])
            .status
            .success()
    );
    assert_eq!(ok(&state, &game, &["list"]).as_array().unwrap().len(), 1);
    assert_eq!(tree::snapshot(&game), before);
    archive(
        &zip,
        &[
            (files[0].0, files[0].1),
            ("archive/pc/mod/C.archive", b"new manual"),
        ],
    );
    write(&game, "archive/pc/mod/C.archive", b"new manual");
    let collision_game = tree::snapshot(&game);
    let collision_state = tree::snapshot(&state);
    assert!(
        !cli(&state, &game, &["adopt", path, "--acknowledge", "--yes"])
            .status
            .success()
    );
    assert_eq!(tree::snapshot(&state), collision_state);
    assert_eq!(tree::snapshot(&game), collision_game);
}

#[test]
fn lifecycle_refusals_preserve_missing_files_state_and_unsafe_destinations() {
    let (temp, game, state) = fixture();
    let zip = temp.path().join("manual.zip");
    archive(
        &zip,
        &[
            ("archive/pc/mod/A.archive", b"one"),
            ("archive/pc/mod/B.archive", b"two"),
        ],
    );
    write(&game, "archive/pc/mod/A.archive", b"one");
    let path = zip.to_str().unwrap();
    assert!(
        !cli(&state, &game, &["adopt", path, "--acknowledge", "--yes"])
            .status
            .success()
    );
    assert!(ok(&state, &game, &["list"]).as_array().unwrap().is_empty());
    write(&game, "archive/pc/mod/B.archive", b"two");
    let package = ok(&state, &game, &["adopt", path, "--acknowledge", "--yes"]);
    let id = package["id"].as_str().unwrap();
    fs::remove_file(game.join("archive/pc/mod/B.archive")).unwrap();
    let before = tree::snapshot(&game);
    let state_before = tree::snapshot(&state);
    assert!(!cli(&state, &game, &["uninstall", id, "--force", "--yes"])
        .status
        .success());
    assert_eq!(tree::snapshot(&game), before);
    assert_eq!(tree::snapshot(&state), state_before);
    let unsafe_state = game.join("manager");
    assert!(!cli(&unsafe_state, &game, &["adopt", path, "--dry-run"])
        .status
        .success());
    assert!(!unsafe_state.exists());
    archive(&zip, &[("../outside.archive", b"unsafe")]);
    assert!(
        !cli(&state, &game, &["adopt", path, "--acknowledge", "--yes"])
            .status
            .success()
    );
    assert_eq!(tree::snapshot(&game), before);
    assert_eq!(tree::snapshot(&state), state_before);
}

#[test]
fn force_cleanup_keeps_unrecorded_descendants_in_exclusive_directory() {
    let (temp, game, state) = fixture();
    write(&game, "red4ext/plugins/manual.dll", b"manual marker");
    let zip = temp.path().join("exclusive.zip");
    archive(&zip, &[("red4ext/plugins/Example/plugin.dll", b"plugin")]);
    let package = ok(&state, &game, &["install", zip.to_str().unwrap(), "--yes"]);
    assert_eq!(package["directories"].as_array().unwrap().len(), 1);
    write(&game, "red4ext/plugins/Example/plugin.dll", b"changed");
    write(
        &game,
        "red4ext/plugins/Example/generated.log",
        b"unrecorded bytes",
    );
    let id = package["id"].as_str().unwrap();
    ok(&state, &game, &["uninstall", id, "--force", "--yes"]);
    assert!(!game.join("red4ext/plugins/Example/plugin.dll").exists());
    assert_eq!(
        fs::read(game.join("red4ext/plugins/Example/generated.log")).unwrap(),
        b"unrecorded bytes"
    );
    assert_eq!(
        fs::read(game.join("red4ext/plugins/manual.dll")).unwrap(),
        b"manual marker"
    );
}

#[test]
fn lifecycle_help_names_independent_approvals_and_pending_checks() {
    for (command, expected) in [
        (
            "adopt",
            vec!["--acknowledge", "--yes", "--dry-run", "byte matching"],
        ),
        ("uninstall", vec!["--force", "--yes", "--dry-run"]),
    ] {
        let output = main_bucket::command(env!("CARGO_BIN_EXE_modn"))
            .args([command, "--help"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let help = String::from_utf8(output.stdout).unwrap();
        for text in expected {
            assert!(help.contains(text), "{help}");
        }
    }
}

#[test]
fn force_cleanup_is_confirmed_owned_only_and_normal_uninstall_stays_strict() {
    let (temp, game, state) = fixture();
    let zip = temp.path().join("owned.zip");
    archive(&zip, &[("archive/pc/mod/A.archive", b"one")]);
    let package = ok(&state, &game, &["install", zip.to_str().unwrap(), "--yes"]);
    let id = package["id"].as_str().unwrap();
    let other = temp.path().join("other.zip");
    archive(&other, &[("archive/pc/mod/B.archive", b"other package")]);
    ok(
        &state,
        &game,
        &["install", other.to_str().unwrap(), "--yes"],
    );
    write(&game, "archive/pc/mod/A.archive", b"changed");
    write(&game, "archive/pc/mod/manual.archive", b"manual sibling");
    let before = tree::snapshot(&game);
    let state_before = tree::snapshot(&state);
    let preview = ok(&state, &game, &["uninstall", id, "--force", "--dry-run"]);
    assert_eq!(preview["force"], true);
    assert_eq!(preview["status"], "verification_required");
    assert_eq!(tree::snapshot(&state), state_before);
    for args in [
        vec!["uninstall", id, "--force"],
        vec!["uninstall", id, "--yes"],
        vec![
            "uninstall",
            "archive/pc/mod/manual.archive",
            "--force",
            "--yes",
        ],
        vec!["uninstall", id, "--force", "--cascade", "--yes"],
        vec!["uninstall", id, "--force", "--include-deps", "--yes"],
    ] {
        assert!(!cli(&state, &game, &args).status.success());
        assert_eq!(tree::snapshot(&game), before);
        assert_eq!(tree::snapshot(&state), state_before);
        assert_eq!(ok(&state, &game, &["list"]).as_array().unwrap().len(), 2);
    }
    ok(&state, &game, &["uninstall", id, "--force", "--yes"]);
    assert!(!game.join("archive/pc/mod/A.archive").exists());
    assert_eq!(
        fs::read(game.join("archive/pc/mod/B.archive")).unwrap(),
        b"other package"
    );
    assert_eq!(
        fs::read(game.join("archive/pc/mod/manual.archive")).unwrap(),
        b"manual sibling"
    );
    assert_eq!(
        fs::read(game.join("mods/Manual/info.json")).unwrap(),
        br#"{"name":"manual"}"#
    );
    assert_eq!(ok(&state, &game, &["list"]).as_array().unwrap().len(), 1);
}

#[test]
fn forced_cleanup_refuses_a_required_package_without_changing_state_or_bytes() {
    let fixture = support::Fixture::new();
    let (base_archive, base_hash) = fixture.archive();
    fixture.bucket("deps", "test-game", "base", &base_hash, &[]);
    let dependent_archive = fixture.temp.path().join("dependent.zip");
    archive(
        &dependent_archive,
        &[("Demo/package/plugins/dependent.plugin", b"dependent")],
    );
    fs::write(
        fixture
            .temp
            .path()
            .join("deps/mods/test-game/dependent.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 2, "id": "dependent", "game": "test-game",
            "name": "Dependent", "version": "1.0", "archive_format": "zip",
            "blake3": blake3::hash(&fs::read(&dependent_archive).unwrap())
                .to_hex()
                .to_string(),
            "sources": [{"kind": "manual", "filename": "dependent.zip"}],
            "dependencies": ["base"],
        }))
        .unwrap(),
    )
    .unwrap();
    fixture.register();
    for (name, archive) in [("base", &base_archive), ("dependent", &dependent_archive)] {
        let result = fixture.run(&[
            "install",
            name,
            "--file",
            archive.to_str().unwrap(),
            "--yes",
        ]);
        assert!(result.status.success(), "{}", support::error(&result));
    }
    fs::write(fixture.game.join("addons/example.plugin"), b"changed").unwrap();
    let game_before = tree::snapshot(&fixture.game);
    let state_before = tree::snapshot(&fixture.state);
    for mode in ["--yes", "--dry-run"] {
        let result = fixture.run(&["uninstall", "base", "--force", mode]);
        assert!(!result.status.success());
        assert!(support::error(&result).contains("required by"));
        assert_eq!(tree::snapshot(&fixture.game), game_before);
        assert_eq!(tree::snapshot(&fixture.state), state_before);
    }
}

#[test]
fn redmod_real_cli_lifecycle_preserves_manual_content_and_refuses_bad_layouts() {
    let (temp, game, state) = fixture();
    let zip = temp.path().join("redmod.zip");
    let entries = [
        (
            "mods/Example/info.json",
            br#"{"name":"Example","version":"1.0"}"#.as_slice(),
        ),
        (
            "mods/Example/archives/example.archive",
            b"payload".as_slice(),
        ),
    ];
    archive(&zip, &entries);
    let path = zip.to_str().unwrap();
    let before = tree::snapshot(&game);
    let preview = ok(&state, &game, &["install", path, "--dry-run"]);
    assert_eq!(preview["status"], "reviewable");
    assert!(!state.exists());
    assert_eq!(tree::snapshot(&game), before);
    let package = ok(&state, &game, &["install", path, "--yes"]);
    let id = package["id"].as_str().unwrap();
    assert_eq!(package["files"].as_array().unwrap().len(), 2);
    assert_eq!(ok(&state, &game, &["list"])[0]["id"], id);
    let doctor = ok(&state, &game, &["doctor"]);
    assert_eq!(doctor["scan"]["owned_files"].as_array().unwrap().len(), 2);
    for &(name, bytes) in &entries {
        assert_eq!(fs::read(game.join(name)).unwrap(), bytes);
    }
    ok(&state, &game, &["uninstall", id, "--yes"]);
    assert_eq!(tree::snapshot(&game), before);
    for invalid in [
        vec![("mods/Example/info.json", b"[]".as_slice())],
        vec![("Mods/Example/info.json", b"{}".as_slice())],
        vec![("mods/Manual/info.json", b"{}".as_slice())],
        vec![
            ("mods/A/info.json", b"{}".as_slice()),
            ("mods/B/info.json", b"{}".as_slice()),
        ],
    ] {
        archive(&zip, &invalid);
        assert!(!cli(&state, &game, &["install", path, "--yes"])
            .status
            .success());
        assert_eq!(tree::snapshot(&game), before);
        assert!(ok(&state, &game, &["list"]).as_array().unwrap().is_empty());
    }
}
