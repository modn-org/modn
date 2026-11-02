use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn checked_in_manifests() -> Vec<Value> {
    let games = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main/games");
    let mut manifests = Vec::new();
    for game in fs::read_dir(games).expect("games directory") {
        for package in fs::read_dir(game.expect("game entry").path()).expect("game packages") {
            let bytes = fs::read(package.expect("package entry").path()).expect("manifest");
            manifests.push(serde_json::from_slice(&bytes).expect("manifest JSON"));
        }
    }
    manifests.sort_by_key(|manifest: &Value| manifest["id"].as_str().map(str::to_owned));
    manifests
}

fn checked_in_ids() -> Vec<String> {
    checked_in_manifests()
        .iter()
        .map(|manifest| manifest["id"].as_str().expect("id").to_owned())
        .collect()
}

fn checkout_copy(destination: &Path) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main/games");
    let bucket = destination.join("checkout");
    for game in fs::read_dir(&source).expect("games directory") {
        let game = game.expect("game entry");
        let target = bucket.join("games").join(game.file_name());
        fs::create_dir_all(&target).expect("game directory");
        for package in fs::read_dir(game.path()).expect("game packages") {
            let package = package.expect("package entry");
            fs::copy(package.path(), target.join(package.file_name())).expect("copy manifest");
        }
    }
    fs::write(
        bucket.join("bucket.json"),
        r#"{"schema_version":1,"name":"checkout","title":"Checked-in main"}"#,
    )
    .expect("bucket metadata");
    bucket
}

fn modn(state: &Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(state)
        .arg("--json")
        .args(args)
        .output()
        .expect("run modn");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("JSON output")
}

fn entries_in(search: &Value, bucket: &str) -> Vec<Value> {
    search
        .as_array()
        .expect("search array")
        .iter()
        .filter(|entry| entry["bucket"] == bucket)
        .map(|entry| {
            let mut entry = entry.clone();
            entry
                .as_object_mut()
                .expect("entry object")
                .remove("bucket");
            entry
        })
        .collect()
}

#[test]
fn bundled_snapshot_lists_every_official_package() {
    let temp = TempDir::new().expect("fixture");
    let search = modn(&temp.path().join("state"), &["search"]);
    let main = entries_in(&search, "main");
    let ids: Vec<&str> = main
        .iter()
        .map(|entry| entry["id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids, checked_in_ids());
    let known: BTreeSet<&str> = ids.iter().copied().collect();
    for entry in &main {
        assert_eq!(entry["game"], "cyberpunk-2077");
        for dependency in entry["dependencies"].as_array().expect("dependencies") {
            let dependency = dependency.as_str().expect("dependency ID");
            assert!(
                known.contains(dependency),
                "{} depends on unknown {dependency}",
                entry["id"]
            );
        }
    }
}

#[test]
fn bundled_snapshot_matches_the_checked_in_bucket_directory() {
    let temp = TempDir::new().expect("fixture");
    let state = temp.path().join("state");
    let source = checkout_copy(temp.path());
    modn(
        &state,
        &[
            "bucket",
            "add",
            "checkout",
            source.to_str().expect("UTF-8 source path"),
        ],
    );
    let search = modn(&state, &["search"]);
    assert_eq!(entries_in(&search, "main"), entries_in(&search, "checkout"));
}

#[test]
fn every_official_mod_resolves_with_its_version_requirements() {
    let temp = TempDir::new().expect("fixture");
    let game = temp.path().join("game");
    fs::create_dir_all(game.join("bin/x64")).expect("game");
    fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").expect("marker");
    let state = temp.path().join("state");
    let game_path = game.to_str().expect("UTF-8 game path");
    for id in checked_in_ids() {
        let plan = modn(&state, &["install", &id, "--dry-run", "--path", game_path]);
        assert_eq!(plan["package"], format!("main/cyberpunk-2077/{id}"));
    }
    let constrained = modn(&state, &["search"])
        .as_array()
        .expect("search array")
        .iter()
        .filter(|entry| entry["dependency_versions"].is_object())
        .count();
    let expected = checked_in_manifests()
        .iter()
        .filter(|manifest| manifest["dependency_versions"].is_object())
        .count();
    assert_eq!(constrained, expected);
}
