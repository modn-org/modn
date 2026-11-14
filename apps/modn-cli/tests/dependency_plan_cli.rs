#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/catalog.rs"]
mod support;

use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use support::{error, Fixture};

fn files_under(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for entry in walk(root) {
        files.push(
            entry
                .strip_prefix(root)
                .expect("prefix")
                .display()
                .to_string(),
        );
    }
    files.sort();
    files
}

fn walk(root: &Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    for entry in fs::read_dir(root).expect("read dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            found.extend(walk(&path));
        } else {
            found.push(path);
        }
    }
    found
}

fn summary(dependencies: &Value) -> Vec<(String, String)> {
    dependencies
        .as_array()
        .expect("dependencies")
        .iter()
        .map(|dependency| {
            (
                dependency["reference"]
                    .as_str()
                    .expect("reference")
                    .to_owned(),
                dependency["state"].as_str().expect("state").to_owned(),
            )
        })
        .collect()
}

#[test]
fn official_mod_dry_run_lists_its_dependency_stack_without_an_archive() {
    let fixture = Fixture::new();
    let game = fixture.temp.path().join("cyberpunk");
    fs::create_dir_all(game.join("bin/x64")).expect("game");
    fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").expect("marker");
    let before = files_under(&game);

    let output = fixture.run_paths(
        &[
            "--json",
            "install",
            "virtual-atelier",
            "--dry-run",
            "--path",
        ],
        &[&game],
    );
    assert!(output.status.success(), "{}", error(&output));
    let report: Value = serde_json::from_slice(&output.stdout).expect("JSON");
    assert_eq!(report["package"], "main/cyberpunk-2077/virtual-atelier");
    assert_eq!(report["plan"], Value::Null);
    assert_eq!(report["source"]["kind"], "manual");
    assert_eq!(report["dependencies_ready"], false);
    let expected: Vec<(String, String)> = [
        "red4ext",
        "redscript",
        "archivexl",
        "codeware",
        "mod-settings",
    ]
    .iter()
    .map(|id| (format!("main/cyberpunk-2077/{id}"), "missing".to_owned()))
    .collect();
    assert_eq!(summary(&report["dependencies"]), expected);
    assert_eq!(
        report["dependencies"][0]["source"],
        json!({"kind":"https","url":"https://github.com/WopsS/RED4ext/releases/download/v1.31.0/red4ext-1.31.0.zip"})
    );
    assert_eq!(report["dependencies"][0]["version"], "1.31.0");

    let text = fixture.run_paths(
        &["install", "virtual-atelier", "--dry-run", "--path"],
        &[&game],
    );
    assert!(text.status.success(), "{}", error(&text));
    let stdout = String::from_utf8_lossy(&text.stdout);
    assert!(
        stdout.contains("main/cyberpunk-2077/red4ext 1.31.0: missing; Modn downloads it from https://github.com/WopsS/RED4ext/releases/download/v1.31.0/red4ext-1.31.0.zip"),
        "{stdout}"
    );
    assert!(
        stdout.contains("archive needed; download \"Virtual Atelier"),
        "{stdout}"
    );

    assert_eq!(files_under(&game), before);
    assert!(!fixture.state.join("state.sqlite").exists());
}

#[test]
fn dry_run_reports_ready_damaged_and_other_release_dependencies() {
    let fixture = Fixture::new();
    let (base_archive, base_hash) = fixture.archive();
    fixture.bucket("deps", "test-game", "base", &base_hash, &[]);
    let dependent = json!({
        "schema_version":2, "id":"dependent", "game":"test-game",
        "name":"Dependent", "version":"2.0", "archive_format":"zip",
        "blake3":"1".repeat(64), "sources":[{"kind":"manual","filename":"dependent.zip"}],
        "dependencies":["base"]
    });
    let manifests = fixture.temp.path().join("deps/mods/test-game");
    fs::write(
        manifests.join("dependent.json"),
        serde_json::to_vec(&dependent).expect("JSON"),
    )
    .expect("manifest");
    fixture.register();
    let installed = fixture.run_paths(&["install", "base", "--yes", "--file"], &[&base_archive]);
    assert!(installed.status.success(), "{}", error(&installed));

    let plan = || {
        let output = fixture.run(&["--json", "install", "dependent", "--dry-run"]);
        assert!(output.status.success(), "{}", error(&output));
        serde_json::from_slice::<Value>(&output.stdout).expect("JSON")
    };
    let ready = plan();
    assert_eq!(
        summary(&ready["dependencies"]),
        [("deps/test-game/base".to_owned(), "ready".to_owned())]
    );
    assert_eq!(ready["dependencies_ready"], true);
    let package_id = ready["dependencies"][0]["package_ids"][0].clone();
    assert!(package_id.is_string());

    let owned = fixture.game.join("addons/example.plugin");
    fs::write(&owned, b"modified").expect("drift");
    let damaged = plan();
    assert_eq!(damaged["dependencies"][0]["state"], "damaged");
    assert_eq!(damaged["dependencies"][0]["package_ids"][0], package_id);
    assert_eq!(damaged["dependencies_ready"], false);
    assert_eq!(fs::read(&owned).expect("owned"), b"modified");

    let mut base: Value =
        serde_json::from_slice(&fs::read(manifests.join("base.json")).expect("base"))
            .expect("JSON");
    base["blake3"] = json!("2".repeat(64));
    fs::write(
        manifests.join("base.json"),
        serde_json::to_vec(&base).expect("JSON"),
    )
    .expect("updated manifest");
    let other = plan();
    assert_eq!(other["dependencies"][0]["state"], "other_release");
    assert_eq!(other["dependencies"][0]["package_ids"][0], package_id);
}

#[test]
fn dependency_cycle_fails_the_dry_run_with_its_path() {
    let fixture = Fixture::new();
    fixture.bucket("loop", "test-game", "a", &"0".repeat(64), &["b"]);
    let b = json!({
        "schema_version":2, "id":"b", "game":"test-game", "name":"B",
        "version":"1.0", "archive_format":"zip", "blake3":"0".repeat(64),
        "sources":[{"kind":"manual","filename":"b.zip"}], "dependencies":["a"]
    });
    fs::write(
        fixture.temp.path().join("loop/mods/test-game/b.json"),
        serde_json::to_vec(&b).expect("JSON"),
    )
    .expect("manifest");
    let output = fixture.run_paths(&["install", "a", "--dry-run", "--path"], &[&fixture.game]);
    assert!(!output.status.success());
    assert!(
        error(&output).contains("dependency cycle in loop/test-game: a -> b -> a"),
        "{}",
        error(&output)
    );
}
