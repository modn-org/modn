//! Declared dependency version requirements and conflicts through the CLI.
#[path = "support/catalog.rs"]
mod support;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use support::{error, Fixture};
use zip::write::SimpleFileOptions;

fn plugin_archive(fixture: &Fixture, name: &str) -> (PathBuf, String) {
    let path = fixture.temp.path().join(format!("{name}.zip"));
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).expect("archive"));
    zip.start_file(
        format!("Demo/package/plugins/{name}.plugin"),
        SimpleFileOptions::default(),
    )
    .expect("entry");
    zip.write_all(name.as_bytes()).expect("payload");
    zip.finish().expect("finish");
    let digest = format!("{:x}", Sha256::digest(fs::read(&path).expect("bytes")));
    (path, digest)
}

fn manifest(fixture: &Fixture, id: &str, version: &str, hash: &str, extra: Value) {
    let mut manifest = json!({
        "schema_version":1, "id":id, "game":"test-game", "name":id,
        "version":version, "archive_format":"zip", "sha256":hash,
        "source":{"kind":"manual","filename":format!("{id}.zip")}
    });
    if let (Some(manifest), Value::Object(extra)) = (manifest.as_object_mut(), extra) {
        manifest.extend(extra);
    }
    let dir = fixture.temp.path().join("deps/games/test-game");
    fs::write(
        dir.join(format!("{id}.json")),
        serde_json::to_vec(&manifest).expect("JSON"),
    )
    .expect("manifest");
}

fn modn(fixture: &Fixture, args: &[&str], files: &[&Path]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_modn"));
    command
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .env("MODN_CACHE", fixture.temp.path().join("cache"))
        .args(args);
    for file in files {
        command.arg("--file").arg(file);
    }
    command.output().expect("run CLI")
}

#[test]
fn a_dependency_release_outside_the_requirement_blocks_the_plan() {
    let fixture = Fixture::new();
    let (base, base_hash) = plugin_archive(&fixture, "base");
    let (top, top_hash) = plugin_archive(&fixture, "top");
    fixture.bucket("deps", "test-game", "seed", &"0".repeat(64), &[]);
    manifest(&fixture, "base", "1.30.2", &base_hash, json!({}));
    manifest(
        &fixture,
        "top",
        "1.0",
        &top_hash,
        json!({"dependencies":["base"], "dependency_versions":{"base":">=1.31.0"}}),
    );
    fixture.register();

    for args in [
        &["install", "top", "--dry-run"][..],
        &["install", "top", "--yes"][..],
    ] {
        let blocked = modn(&fixture, args, &[&base, &top]);
        assert!(!blocked.status.success(), "{args:?}");
        assert!(
            error(&blocked)
                .contains("top requires base >=1.31.0, but bucket deps publishes base 1.30.2"),
            "{}",
            error(&blocked)
        );
    }
    assert!(!fixture.game.join("addons").exists());

    manifest(&fixture, "base", "1.31.0", &base_hash, json!({}));
    let installed = modn(&fixture, &["install", "top", "--yes"], &[&base, &top]);
    assert!(installed.status.success(), "{}", error(&installed));
}

#[test]
fn a_declared_conflict_is_reported_and_blocks_before_any_write() {
    let fixture = Fixture::new();
    let (a, a_hash) = plugin_archive(&fixture, "a");
    let (b, b_hash) = plugin_archive(&fixture, "b");
    fixture.bucket("deps", "test-game", "seed", &"0".repeat(64), &[]);
    manifest(&fixture, "a", "1.0", &a_hash, json!({"conflicts":["b"]}));
    manifest(&fixture, "b", "1.0", &b_hash, json!({}));
    fixture.register();
    let installed = modn(&fixture, &["--json", "install", "a", "--yes"], &[&a]);
    assert!(installed.status.success(), "{}", error(&installed));
    let report: Value = serde_json::from_slice(&installed.stdout).expect("JSON");
    let a_id = report["installed"]["id"].as_str().expect("id").to_owned();

    // `b` declares nothing, but the installed `a` recorded its conflict.
    let preview = modn(&fixture, &["--json", "install", "b", "--dry-run"], &[&b]);
    assert!(preview.status.success(), "{}", error(&preview));
    let plan: Value = serde_json::from_slice(&preview.stdout).expect("JSON");
    assert_eq!(
        plan["conflicts"],
        json!([format!(
            "deps/test-game/b conflicts with installed {a_id} (deps/test-game/a)"
        )])
    );

    let blocked = modn(&fixture, &["install", "b", "--yes"], &[&b]);
    assert!(!blocked.status.success());
    assert!(
        error(&blocked).contains("installation blocked by declared conflicts"),
        "{}",
        error(&blocked)
    );
    assert!(!fixture.game.join("addons/b.plugin").exists());

    let removed = fixture.run_paths(&["uninstall", &a_id, "--yes", "--path"], &[&fixture.game]);
    assert!(removed.status.success(), "{}", error(&removed));
    let installed = modn(&fixture, &["install", "b", "--yes"], &[&b]);
    assert!(installed.status.success(), "{}", error(&installed));
}
