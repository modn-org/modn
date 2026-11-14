//! `--state-dir` selects the manager profile for module trust and releases.
//!
//! The default profile and the selected profile keep separate module
//! registries. Trusting a bucket in the selected profile must not move the
//! default profile's active release, and the selected profile must resolve the
//! imported module with its own trust record.

#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/module_catalog.rs"]
mod module_catalog;

use module_catalog::{policy_lua, ModuleCatalog};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn cli(data: &Path, state_dir: Option<&Path>, args: &[&str]) -> Output {
    let mut command = Command::new(main_bucket::modn_binary());
    command
        .env("LOCALAPPDATA", data)
        .env("APPDATA", data)
        .env_remove("MODN_GAME_MODULES_DIR")
        .env_remove("MODN_APPDATA");
    if let Some(state_dir) = state_dir {
        command.arg("--state-dir").arg(state_dir);
    }
    command.args(args).output().expect("run modn")
}

fn success(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("JSON output")
}

fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, current: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let Ok(entries) = fs::read_dir(current) else {
            return;
        };
        for entry in entries {
            let entry = entry.expect("tree entry");
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .expect("relative path")
                .to_path_buf();
            if entry.file_type().expect("entry type").is_dir() {
                walk(root, &path, files);
            } else {
                files.insert(relative, fs::read(&path).expect("tree file"));
            }
        }
    }
    let mut files = BTreeMap::new();
    if root.is_dir() {
        walk(root, root, &mut files);
    }
    files
}

fn write_module(directory: &Path, game: &str, version: &str, marker: &str) {
    fs::create_dir_all(directory).expect("module directory");
    fs::write(
        directory.join("module.json"),
        json!({
            "game_id": game,
            "version": version,
            "api_version": 1,
            "entrypoint": "module.lua",
        })
        .to_string(),
    )
    .expect("module manifest");
    fs::write(directory.join("module.lua"), policy_lua(game, marker)).expect("module Lua");
}

#[test]
fn state_dir_keeps_module_trust_and_releases_in_its_profile() {
    let root = tempfile::tempdir().expect("fixture");
    let profile_a = root.path().join("profile-a");
    let profile_b = root.path().join("profile-b");
    fs::create_dir_all(&profile_a).expect("default profile");
    fs::create_dir_all(&profile_b).expect("selected profile");
    main_bucket::seed(&profile_a);

    // The default profile already has one trusted release for the game.
    let local = root.path().join("local-module");
    write_module(&local, "fixture-game", "1.0.0", "Game.exe");
    success(&cli(
        &profile_a,
        None,
        &[
            "--json",
            "module",
            "register",
            local.to_str().expect("module path"),
            "--trust",
        ],
    ));
    let default_modules = profile_a.join("modn/game-modules");
    let before = tree(&default_modules);
    assert!(!before.is_empty(), "default profile has a module registry");

    // A trusted catalog in the selected profile publishes newer releases.
    let catalog = ModuleCatalog::new("community");
    catalog.write_module(
        "fixture-game",
        "2.0.0",
        &policy_lua("fixture-game", "Game.exe"),
    );
    catalog.sign_and_commit("module v2");

    let local_bucket = root.path().join("local-bucket");
    fs::create_dir_all(&local_bucket).expect("local bucket");
    fs::write(
        local_bucket.join("bucket.json"),
        r#"{"schema_version":2,"name":"local","title":"Local fixture"}"#,
    )
    .expect("bucket metadata");
    success(&cli(
        &profile_a,
        Some(&profile_b),
        &[
            "--json",
            "bucket",
            "add",
            "local",
            local_bucket.to_str().expect("bucket path"),
        ],
    ));

    let synced = success(&cli(
        &profile_a,
        Some(&profile_b),
        &[
            "--json",
            "bucket",
            "sync",
            "community",
            "--source",
            catalog.repo().to_str().expect("repository path"),
            "--revision",
            "HEAD",
        ],
    ));
    assert_eq!(
        synced["modules"]["ignored"][0]["reason"],
        "untrusted_source"
    );

    let trusted = success(&cli(
        &profile_a,
        Some(&profile_b),
        &[
            "--json",
            "bucket",
            "trust-modules",
            "community",
            "--yes",
            "--fingerprint",
            catalog.key_id(),
        ],
    ));
    assert_eq!(trusted["imported"][0]["version"], "2.0.0");

    let game = root.path().join("game");
    fs::create_dir_all(&game).expect("game directory");
    fs::write(game.join("Game.exe"), b"").expect("game marker");
    let doctor = success(&cli(
        &profile_a,
        Some(&profile_b),
        &[
            "--json",
            "--game",
            "fixture-game",
            "doctor",
            "--path",
            game.to_str().expect("game path"),
        ],
    ));
    assert!(
        doctor["diagnostics"]
            .as_array()
            .expect("diagnostics")
            .iter()
            .all(|diagnostic| diagnostic["code"] != "MODULE_SOURCE_UNTRUSTED"),
        "selected profile must resolve its trusted module: {doctor}"
    );

    let listed = success(&cli(
        &profile_a,
        Some(&profile_b),
        &["--json", "module", "list"],
    ));
    let entries = listed.as_array().expect("module list");
    assert_eq!(entries.len(), 1, "selected profile has its own registry");
    assert_eq!(entries[0]["module"]["version"], "2.0.0");
    assert_eq!(entries[0]["active"], true);
    assert_eq!(entries[0]["source_trusted"], true);

    catalog.write_module(
        "fixture-game",
        "3.0.0",
        &policy_lua("fixture-game", "Game.exe"),
    );
    catalog.sign_and_commit("module v3");
    let updated = success(&cli(
        &profile_a,
        Some(&profile_b),
        &["--json", "module", "update", "fixture-game"],
    ));
    assert_eq!(updated["from"], "2.0.0");
    assert_eq!(updated["to"], "3.0.0");
    assert_eq!(updated["outcome"], "imported");

    let rolled = success(&cli(
        &profile_a,
        Some(&profile_b),
        &[
            "--json",
            "module",
            "rollback",
            "fixture-game",
            "--to",
            "2.0.0",
        ],
    ));
    assert_eq!(rolled["from"], "3.0.0");
    assert_eq!(rolled["to"], "2.0.0");

    assert_eq!(
        tree(&default_modules),
        before,
        "the default profile module registry must stay byte-identical"
    );
}
