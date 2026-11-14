#[path = "support/main_bucket.rs"]
mod main_bucket;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn checked_in_manifests() -> Vec<Value> {
    let games = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main/games");
    let mut manifests = Vec::new();
    for game in fs::read_dir(games).expect("games directory") {
        for package in fs::read_dir(game.expect("game entry").path()).expect("game packages") {
            let path = package.expect("package entry").path();
            if path.file_name().expect("fixture invariant") == "module.json"
                || path.extension().expect("fixture invariant") != "json"
            {
                continue;
            }
            let bytes = fs::read(path).expect("manifest");
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
        let target = bucket.join("mods").join(game.file_name());
        fs::create_dir_all(&target).expect("record directory");
        for package in fs::read_dir(game.path()).expect("game packages") {
            let package = package.expect("package entry");
            if package.file_name() == "module.json"
                || package.path().extension().expect("fixture invariant") != "json"
            {
                continue;
            }
            let mut record: Value =
                serde_json::from_slice(&fs::read(package.path()).expect("record"))
                    .expect("record JSON");
            record["schema_version"] = serde_json::json!(2);
            if let Some(source) = record.as_object_mut().and_then(|entry| entry.remove("source"))
            {
                record["sources"] = serde_json::json!([source]);
            }
            if let Some(pin) = record.as_object_mut().and_then(|entry| entry.remove("sha256")) {
                record["blake3"] = pin;
            }
            fs::write(
                target.join(package.file_name()),
                serde_json::to_vec(&record).expect("record JSON"),
            )
            .expect("record");
        }
    }
    fs::write(
        bucket.join("bucket.json"),
        r#"{"schema_version":2,"name":"checkout","title":"Checked-in main"}"#,
    )
    .expect("bucket metadata");
    bucket
}

fn modn(state: &Path, args: &[&str]) -> Value {
    let appdata = state.parent().expect("state parent").join("appdata");
    fs::create_dir_all(&appdata).expect("appdata");
    let output = main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(state)
        .arg("--json")
        .env("APPDATA", appdata)
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
    let ids: BTreeSet<String> = main
        .iter()
        .map(|entry| entry["id"].as_str().expect("id").to_owned())
        .collect();
    let expected: BTreeSet<String> = checked_in_ids().into_iter().collect();
    assert_eq!(ids, expected);
    let mut ids_by_game: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for entry in &main {
        let game = entry["game"].as_str().expect("game");
        let id = entry["id"].as_str().expect("id");
        ids_by_game.entry(game).or_default().insert(id);
    }
    for entry in &main {
        let game = entry["game"].as_str().expect("game");
        let known = ids_by_game.get(game).expect("game");
        for dependency in entry["dependencies"].as_array().into_iter().flatten() {
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
    let state = temp.path().join("state");
    let manifests = checked_in_manifests();
    let mut game_paths: BTreeMap<String, PathBuf> = BTreeMap::new();
    for manifest in &manifests {
        let game = manifest["game"].as_str().expect("game");
        if game_paths.contains_key(game) {
            continue;
        }
        let marker = match game {
            "cyberpunk-2077" => "bin/x64/Cyberpunk2077.exe",
            "balatro" => "Balatro.exe",
            other => panic!("no installation marker for {other}"),
        };
        let game_path = temp.path().join(game);
        let marker_path = game_path.join(marker);
        fs::create_dir_all(marker_path.parent().expect("marker parent")).expect("game");
        fs::write(marker_path, b"game").expect("marker");
        game_paths.insert(game.to_owned(), game_path);
    }
    for manifest in &manifests {
        let game = manifest["game"].as_str().expect("game");
        let id = manifest["id"].as_str().expect("id");
        let reference = format!("main/{game}/{id}");
        let game_path = game_paths.get(game).expect("game path");
        let plan = modn(
            &state,
            &[
                "install",
                &reference,
                "--dry-run",
                "--allow-native-injector",
                "--path",
                game_path.to_str().expect("UTF-8 game path"),
            ],
        );
        assert_eq!(plan["package"], reference);
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
