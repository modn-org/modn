//! Installing a catalog mod with its missing dependencies as one stack.
#[path = "support/catalog.rs"]
mod support;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use support::{error, Fixture};
use zip::write::SimpleFileOptions;

/// A ZIP whose plugin the test-game policy installs to `addons/<name>.plugin`.
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
    let digest = Sha256::digest(fs::read(&path).expect("bytes"));
    (path, format!("{digest:x}"))
}

fn manifest(fixture: &Fixture, id: &str, hash: &str, dependencies: &[&str], extra: Value) {
    let mut manifest = json!({
        "schema_version":1, "id":id, "game":"test-game", "name":id,
        "version":"1.0", "archive_format":"zip", "sha256":hash,
        "source":{"kind":"manual","filename":format!("{id}.zip")},
        "dependencies":dependencies
    });
    if let (Some(manifest), Value::Object(extra)) = (manifest.as_object_mut(), extra) {
        manifest.extend(extra);
    }
    fs::write(
        fixture
            .temp
            .path()
            .join(format!("deps/games/test-game/{id}.json")),
        serde_json::to_vec(&manifest).expect("JSON"),
    )
    .expect("manifest");
}

struct Stack {
    fixture: Fixture,
    base: PathBuf,
    middle: PathBuf,
    top: PathBuf,
}

/// `top` depends on `middle`, which depends on `base`; nothing is installed.
fn stack(middle_extra: Value) -> Stack {
    let fixture = Fixture::new();
    let (base, base_hash) = plugin_archive(&fixture, "base");
    let (middle, middle_hash) = plugin_archive(&fixture, "middle");
    let (top, top_hash) = plugin_archive(&fixture, "top");
    fixture.bucket("deps", "test-game", "base", &base_hash, &[]);
    manifest(&fixture, "middle", &middle_hash, &["base"], middle_extra);
    manifest(&fixture, "top", &top_hash, &["middle"], json!({}));
    fixture.register();
    Stack {
        fixture,
        base,
        middle,
        top,
    }
}

fn run(fixture: &Fixture, args: &[&str], files: &[&Path]) -> std::process::Output {
    let mut all: Vec<std::ffi::OsString> = args.iter().map(Into::into).collect();
    for file in files {
        all.push("--file".into());
        all.push(file.as_os_str().to_owned());
    }
    std::process::Command::new(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .args(all)
        .output()
        .expect("run CLI")
}

fn managed(fixture: &Fixture) -> usize {
    let output = fixture.run(&["--json", "list"]);
    assert!(output.status.success(), "{}", error(&output));
    let list: Value = serde_json::from_slice(&output.stdout).expect("JSON");
    list.as_array().expect("array").len()
}

#[test]
fn missing_dependencies_install_with_the_mod_in_order_after_one_approval() {
    let Stack {
        fixture,
        base,
        middle,
        top,
    } = stack(json!({}));
    // Archives match their releases by digest, in any order.
    let files = [top.as_path(), base.as_path(), middle.as_path()];
    let preview = run(&fixture, &["--json", "install", "top", "--dry-run"], &files);
    assert!(preview.status.success(), "{}", error(&preview));
    let plan: Value = serde_json::from_slice(&preview.stdout).expect("JSON");
    assert_eq!(plan["dependencies"][0]["archive"], json!(base));
    assert_eq!(plan["dependencies"][1]["archive"], json!(middle));
    assert!(!fixture.game.join("addons").exists());

    let installed = run(&fixture, &["--json", "install", "top", "--yes"], &files);
    assert!(installed.status.success(), "{}", error(&installed));
    let report: Value = serde_json::from_slice(&installed.stdout).expect("JSON");
    let order: Vec<&str> = report["stack"]
        .as_array()
        .expect("stack")
        .iter()
        .map(|member| member["package"].as_str().expect("reference"))
        .collect();
    assert_eq!(
        order,
        [
            "deps/test-game/base",
            "deps/test-game/middle",
            "deps/test-game/top"
        ]
    );
    for name in ["base", "middle", "top"] {
        let file = fixture.game.join(format!("addons/{name}.plugin"));
        assert_eq!(fs::read(file).expect("installed"), name.as_bytes());
    }
    assert_eq!(managed(&fixture), 3);

    // Each member is its own managed package with recorded dependency links.
    let base_id = report["stack"][0]["installed"]["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let blocked = fixture.run_paths(
        &["uninstall", &base_id, "--yes", "--path"],
        &[&fixture.game],
    );
    assert!(!blocked.status.success());
    assert!(
        error(&blocked).contains("required by"),
        "{}",
        error(&blocked)
    );
}

#[test]
fn a_failed_member_rolls_back_the_whole_stack() {
    let Stack {
        fixture,
        base,
        middle,
        top,
    } = stack(json!({}));
    // An unmanaged file occupies the last member's destination.
    fs::create_dir_all(fixture.game.join("addons")).expect("addons");
    let occupied = fixture.game.join("addons/top.plugin");
    fs::write(&occupied, b"user file").expect("unmanaged file");

    let files = [base.as_path(), middle.as_path(), top.as_path()];
    let failed = run(&fixture, &["install", "top", "--yes"], &files);
    assert!(!failed.status.success());
    let message = error(&failed);
    assert!(message.contains("rolled back 2 package(s)"), "{message}");
    assert!(message.contains("deps/test-game/top"), "{message}");
    assert!(!fixture.game.join("addons/base.plugin").exists());
    assert!(!fixture.game.join("addons/middle.plugin").exists());
    assert_eq!(fs::read(&occupied).expect("unmanaged"), b"user file");
    assert_eq!(managed(&fixture), 0);

    // Nothing is left pending: the next write needs no recovery.
    let base_only = run(&fixture, &["install", "base", "--yes"], &[&base]);
    assert!(base_only.status.success(), "{}", error(&base_only));
    assert_eq!(managed(&fixture), 1);
}

#[test]
fn a_missing_dependency_archive_blocks_before_any_write() {
    let Stack {
        fixture,
        middle,
        top,
        ..
    } = stack(json!({}));
    let state = fixture.state.join("state.sqlite");
    let before = fs::read(&state).expect("registered state");
    let blocked = run(&fixture, &["install", "top", "--yes"], &[&top, &middle]);
    assert!(!blocked.status.success());
    let message = error(&blocked);
    assert!(
        message.contains(
            "dependency not installed: deps/test-game/base 1.0; download \"fixture.zip\" manually"
        ),
        "{message}"
    );
    assert!(!message.contains("deps/test-game/middle 1.0"), "{message}");
    assert!(!fixture.game.join("addons").exists());
    assert_eq!(fs::read(&state).expect("state"), before);

    // An archive that no release in this installation pins is refused.
    let (other, _) = plugin_archive(&fixture, "other");
    let unknown = run(&fixture, &["install", "top", "--yes"], &[&top, &other]);
    assert!(!unknown.status.success());
    assert!(
        error(&unknown)
            .contains("does not match catalog manifest of deps/test-game/top or its dependencies"),
        "{}",
        error(&unknown)
    );
}

#[test]
fn declared_executables_need_explicit_approval_before_the_stack_starts() {
    let Stack {
        fixture,
        base,
        middle,
        top,
    } = stack(json!({"executables": ["engine/tools/fixture.exe"]}));
    let files = [base.as_path(), middle.as_path(), top.as_path()];
    let blocked = run(&fixture, &["install", "top", "--yes"], &files);
    assert!(!blocked.status.success());
    assert!(
        error(&blocked).contains("catalog manifests declare engine/tools/fixture.exe"),
        "{}",
        error(&blocked)
    );
    assert!(!fixture.game.join("addons").exists());
    assert_eq!(managed(&fixture), 0);

    // An explicit approval lets the stack proceed; the archive has no executable.
    let approved = run(
        &fixture,
        &[
            "install",
            "top",
            "--yes",
            "--allow-executable",
            "engine/tools/fixture.exe",
        ],
        &files,
    );
    assert!(approved.status.success(), "{}", error(&approved));
    assert_eq!(managed(&fixture), 3);
}

#[test]
fn dependencies_sharing_one_archive_install_it_once() {
    let fixture = Fixture::new();
    let (shared, shared_hash) = plugin_archive(&fixture, "shared");
    let (top, top_hash) = plugin_archive(&fixture, "top");
    fixture.bucket("deps", "test-game", "a", &shared_hash, &[]);
    manifest(&fixture, "b", &shared_hash, &[], json!({}));
    manifest(&fixture, "top", &top_hash, &["a", "b"], json!({}));
    fixture.register();

    let installed = run(
        &fixture,
        &["--json", "install", "top", "--yes"],
        &[&shared, &top],
    );
    assert!(installed.status.success(), "{}", error(&installed));
    let report: Value = serde_json::from_slice(&installed.stdout).expect("JSON");
    let order: Vec<&str> = report["stack"]
        .as_array()
        .expect("stack")
        .iter()
        .map(|member| member["package"].as_str().expect("reference"))
        .collect();
    assert_eq!(order, ["deps/test-game/a", "deps/test-game/top"]);
    assert_eq!(managed(&fixture), 2);

    // The mod itself may not share its archive with a missing dependency.
    let fixture = Fixture::new();
    let (archive, hash) = plugin_archive(&fixture, "same");
    fixture.bucket("deps", "test-game", "base", &hash, &[]);
    manifest(&fixture, "top", &hash, &["base"], json!({}));
    fixture.register();
    let refused = run(&fixture, &["install", "top", "--yes"], &[&archive]);
    assert!(!refused.status.success());
    assert!(
        error(&refused).contains("also the archive of a missing dependency"),
        "{}",
        error(&refused)
    );
    assert!(!fixture.game.join("addons").exists());

    // A skipped alias may not drop requirements the kept release lacks.
    let fixture = Fixture::new();
    let (shared, shared_hash) = plugin_archive(&fixture, "shared");
    let (x, x_hash) = plugin_archive(&fixture, "x");
    let (top, top_hash) = plugin_archive(&fixture, "top");
    fixture.bucket("deps", "test-game", "a", &shared_hash, &[]);
    manifest(&fixture, "b", &shared_hash, &["x"], json!({}));
    manifest(&fixture, "x", &x_hash, &[], json!({}));
    manifest(&fixture, "top", &top_hash, &["a", "b"], json!({}));
    fixture.register();
    let refused = run(&fixture, &["install", "top", "--yes"], &[&shared, &x, &top]);
    assert!(!refused.status.success());
    assert!(
        error(&refused).contains(
            "deps/test-game/a and deps/test-game/b share one archive but declare different dependencies or conflicts"
        ),
        "{}",
        error(&refused)
    );
    assert!(!fixture.game.join("addons").exists());
    assert_eq!(managed(&fixture), 0);
}
