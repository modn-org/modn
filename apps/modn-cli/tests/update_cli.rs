#[path = "support/catalog.rs"]
mod support;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Output;
use support::{error, Fixture};
use zip::write::SimpleFileOptions;

fn plugins(fixture: &Fixture, name: &str, files: &[(&str, &str)]) -> (PathBuf, String) {
    let path = fixture.temp.path().join(format!("{name}.zip"));
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).expect("archive"));
    for (file, contents) in files {
        zip.start_file(
            format!("Demo/package/plugins/{file}"),
            SimpleFileOptions::default(),
        )
        .expect("entry");
        zip.write_all(contents.as_bytes()).expect("payload");
    }
    zip.finish().expect("finish");
    let digest = Sha256::digest(fs::read(&path).expect("bytes"));
    (path, format!("{digest:x}"))
}

fn manifest(fixture: &Fixture, file: &str, id: &str, version: &str, hash: &str, extra: Value) {
    let mut manifest = json!({
        "schema_version": 1, "id": id, "game": "test-game", "name": id,
        "version": version, "archive_format": "zip", "sha256": hash,
        "source": {"kind": "manual", "filename": format!("{id}-{version}.zip")}
    });
    if let (Some(manifest), Value::Object(extra)) = (manifest.as_object_mut(), extra) {
        manifest.extend(extra);
    }
    let folder = fixture.temp.path().join("deps/games/test-game");
    fs::create_dir_all(&folder).expect("bucket folder");
    fs::write(
        folder.join(file),
        serde_json::to_vec(&manifest).expect("JSON"),
    )
    .expect("manifest");
}

fn run(fixture: &Fixture, args: &[&str], file: Option<&Path>) -> Output {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_modn"));
    command
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .args(args);
    if let Some(file) = file {
        command.arg("--file").arg(file);
    }
    command.output().expect("run CLI")
}

fn json_of(output: &Output) -> Value {
    assert!(output.status.success(), "{}", error(output));
    serde_json::from_slice(&output.stdout).expect("JSON output")
}

struct Catalog {
    fixture: Fixture,
    v1: PathBuf,
    v1_hash: String,
    v2: PathBuf,
    v2_hash: String,
}

impl Catalog {
    fn addon(&self, file: &str) -> Option<String> {
        fs::read_to_string(self.fixture.game.join("addons").join(file)).ok()
    }

    fn top_release(&self, file: &str, version: &str, contents: &str, requirement: &str) -> PathBuf {
        let (path, hash) = plugins(
            &self.fixture,
            &format!("top-{version}"),
            &[("top.plugin", contents)],
        );
        manifest(
            &self.fixture,
            file,
            "top",
            version,
            &hash,
            json!({"dependencies": ["base"], "dependency_versions": {"base": requirement}}),
        );
        path
    }

    fn blocked_update_to_2(&self, expected: &str) {
        let blocked = run(&self.fixture, &["update", "base", "--yes"], Some(&self.v2));
        assert!(!blocked.status.success());
        assert!(error(&blocked).contains(expected), "{}", error(&blocked));
        assert_eq!(self.addon("base.plugin").as_deref(), Some("v1"));
    }
}

fn installed_with_dependent() -> Catalog {
    installed_with_dependent_requiring(">=1.0")
}

fn installed_with_dependent_requiring(requirement: &str) -> Catalog {
    let fixture = Fixture::new();
    let (v1, v1_hash) = plugins(
        &fixture,
        "base-1",
        &[("base.plugin", "v1"), ("retired.plugin", "old only")],
    );
    let (v2, v2_hash) = plugins(
        &fixture,
        "base-2",
        &[("base.plugin", "v2"), ("added.plugin", "new only")],
    );
    fixture.bucket("deps", "test-game", "base", &v1_hash, &[]);
    let catalog = Catalog {
        fixture,
        v1,
        v1_hash,
        v2,
        v2_hash,
    };
    let top = catalog.top_release("top.json", "1.0", "top", requirement);
    catalog.fixture.register();
    let mut install = std::process::Command::new(env!("CARGO_BIN_EXE_modn"));
    let installed = install
        .arg("--state-dir")
        .arg(&catalog.fixture.state)
        .env("MODN_GAME_MODULES_DIR", &catalog.fixture.modules)
        .args(["install", "top", "--yes", "--file"])
        .arg(&catalog.v1)
        .arg("--file")
        .arg(&top)
        .output()
        .expect("install");
    assert!(installed.status.success(), "{}", error(&installed));
    manifest(
        &catalog.fixture,
        "base.json",
        "base",
        "2.0",
        &catalog.v2_hash,
        json!({}),
    );
    manifest(
        &catalog.fixture,
        "base@1.0.json",
        "base",
        "1.0",
        &catalog.v1_hash,
        json!({}),
    );
    catalog
}

#[test]
fn dry_run_shows_both_releases_the_changed_files_and_the_dependents() {
    let catalog = installed_with_dependent();
    let report = json_of(&run(
        &catalog.fixture,
        &["--json", "update", "base", "--dry-run"],
        Some(&catalog.v2),
    ));
    assert_eq!(report["installed"]["version"], "1.0");
    assert_eq!(report["target"]["version"], "2.0");
    assert_eq!(report["up_to_date"], false);
    assert_eq!(report["files"]["added"], json!(["addons/added.plugin"]));
    assert_eq!(report["files"]["replaced"], json!(["addons/base.plugin"]));
    assert_eq!(report["files"]["removed"], json!(["addons/retired.plugin"]));
    assert_eq!(report["dependents"][0]["package"], "deps/test-game/top");
    assert_eq!(report["dependents"][0]["satisfied"], true);
    assert_eq!(report["blockers"], json!([]));
    assert_eq!(report["plan"]["status"], "reviewable");
    assert_eq!(catalog.addon("base.plugin").as_deref(), Some("v1"));
    assert_eq!(catalog.addon("added.plugin"), None);

    let without_archive = json_of(&run(
        &catalog.fixture,
        &["--json", "update", "base", "--dry-run"],
        None,
    ));
    assert_eq!(without_archive["files"], Value::Null);
    assert!(without_archive["archive_needed"]
        .as_str()
        .is_some_and(|text| text.contains("base-2.0.zip")));
}

#[test]
fn update_applies_a_release_and_can_choose_an_older_one() {
    let catalog = installed_with_dependent();
    let updated = json_of(&run(
        &catalog.fixture,
        &["--json", "update", "base", "--yes"],
        Some(&catalog.v2),
    ));
    assert_eq!(updated["package"], "deps/test-game/base");
    assert_eq!(catalog.addon("base.plugin").as_deref(), Some("v2"));
    assert_eq!(catalog.addon("added.plugin").as_deref(), Some("new only"));
    assert_eq!(catalog.addon("retired.plugin"), None);
    let listed = json_of(&run(&catalog.fixture, &["--json", "list"], None));
    assert_eq!(listed.as_array().map(Vec::len), Some(2));
    let refused = run(&catalog.fixture, &["uninstall", "base", "--yes"], None);
    assert!(
        error(&refused).contains("required by"),
        "{}",
        error(&refused)
    );

    let current = json_of(&run(
        &catalog.fixture,
        &["--json", "update", "base", "--dry-run"],
        None,
    ));
    assert_eq!(current["up_to_date"], true);

    let unknown = run(
        &catalog.fixture,
        &["update", "base", "--version", "9.9", "--dry-run"],
        None,
    );
    assert!(
        error(&unknown).contains("available: 2.0, 1.0"),
        "{}",
        error(&unknown)
    );

    json_of(&run(
        &catalog.fixture,
        &["--json", "update", "base", "--version", "1.0", "--yes"],
        Some(&catalog.v1),
    ));
    assert_eq!(catalog.addon("base.plugin").as_deref(), Some("v1"));
    assert_eq!(catalog.addon("retired.plugin").as_deref(), Some("old only"));
    assert_eq!(catalog.addon("added.plugin"), None);
}

#[test]
fn the_installed_dependent_requirement_blocks_the_update_without_writing() {
    let catalog = installed_with_dependent_requiring("<2.0");
    let report = json_of(&run(
        &catalog.fixture,
        &["--json", "update", "base", "--dry-run"],
        Some(&catalog.v2),
    ));
    assert_eq!(report["dependents"][0]["requirement"], "<2.0");
    assert_eq!(report["dependents"][0]["satisfied"], false);
    catalog.blocked_update_to_2("requires base <2.0");

    catalog.top_release("top@1.0.json", "1.0", "top", "<2.0");
    catalog.top_release("top.json", "2.0", "top 2", ">=2.0");
    catalog.blocked_update_to_2("requires base <2.0");

    let database = rusqlite::Connection::open(catalog.fixture.state.join("state.sqlite"))
        .expect("state database");
    database
        .execute("DELETE FROM package_requirements", [])
        .expect("forget recorded requirements");
    drop(database);
    catalog.blocked_update_to_2("requires base <2.0");

    fs::remove_file(
        catalog
            .fixture
            .temp
            .path()
            .join("deps/games/test-game/top@1.0.json"),
    )
    .expect("retire the installed release");
    catalog.blocked_update_to_2("requirement on base is unknown");
}

#[test]
fn the_target_release_requirements_block_the_update() {
    let catalog = installed_with_dependent();
    let top_2 = catalog.top_release("top.json", "2.0", "top 2", ">=3.0");
    let report = json_of(&run(
        &catalog.fixture,
        &["--json", "update", "top", "--dry-run"],
        Some(&top_2),
    ));
    let expected = "top requires base >=3.0, but bucket deps publishes base 2.0";
    assert!(
        report["blockers"]
            .as_array()
            .is_some_and(|blockers| blockers.iter().any(|b| b == expected)),
        "{report}"
    );
    let blocked = run(&catalog.fixture, &["update", "top", "--yes"], Some(&top_2));
    assert!(error(&blocked).contains(expected), "{}", error(&blocked));
    assert_eq!(catalog.addon("top.plugin").as_deref(), Some("top"));
}
