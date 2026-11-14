#[path = "support/catalog.rs"]
mod support;

#[path = "support/main_bucket.rs"]
mod main_bucket;
use serde_json::{json, Value};
use std::fs;
use support::{error, Fixture};
use zip::write::SimpleFileOptions;

#[test]
fn deleted_managed_dependency_cannot_authorize_dependent_install() {
    let fixture = Fixture::new();
    let (base_archive, base_hash) = fixture.archive();
    fixture.bucket("deps", "test-game", "base", &base_hash, &[]);
    let dependent_archive = fixture.temp.path().join("dependent.zip");
    let file = fs::File::create(&dependent_archive).expect("archive");
    let mut zip = zip::ZipWriter::new(file);
    use std::io::Write;
    zip.start_file(
        "Demo/package/plugins/dependent.plugin",
        SimpleFileOptions::default(),
    )
    .expect("entry");
    zip.write_all(b"dependent-content").expect("payload");
    zip.finish().expect("finish");
    let dependent_hash = blake3::hash(&fs::read(&dependent_archive).expect("bytes"))
        .to_hex()
        .to_string();
    let manifest = json!({
        "schema_version":2, "id":"dependent", "game":"test-game",
        "name":"Dependent", "version":"1.0", "archive_format":"zip",
        "blake3":dependent_hash, "sources":[{"kind":"manual","filename":"dependent.zip"}],
        "dependencies":["base"]
    });
    fs::write(
        fixture
            .temp
            .path()
            .join("deps/mods/test-game/dependent.json"),
        serde_json::to_vec(&manifest).expect("JSON"),
    )
    .expect("manifest");
    fixture.register();

    let base = main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .args(["install", "base", "--yes", "--file"])
        .arg(&base_archive)
        .output()
        .expect("install base");
    assert!(base.status.success(), "{}", error(&base));
    let owned_file = fixture.game.join("addons/example.plugin");
    assert!(owned_file.is_file());
    fs::remove_file(&owned_file).expect("simulate drift");

    let dependent = main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .args(["install", "dependent", "--yes", "--file"])
        .arg(&dependent_archive)
        .output()
        .expect("install dependent");
    assert!(
        !dependent.status.success(),
        "drifted prerequisite must block"
    );
    assert!(
        error(&dependent).contains("dependency not installed or intact"),
        "{}",
        error(&dependent)
    );
    assert!(!fixture.game.join("addons/dependent.plugin").exists());
    assert_eq!(
        fs::read(fixture.game.join("bin/TestGame.exe")).expect("marker"),
        b"game-exe"
    );
}

#[test]
fn exclusive_directory_dependency_requires_original_payload_bytes() {
    let fixture = Fixture::new();
    let (archive, hash) = fixture.archive();
    fixture.bucket("deps", "test-game", "base", &hash, &[]);
    let dependent = json!({
        "schema_version":2, "id":"dependent", "game":"test-game",
        "name":"Dependent", "version":"1.0", "archive_format":"zip",
        "blake3":hash, "sources":[{"kind":"manual", "filename":"fixture.zip"}],
        "dependencies":["base"]
    });
    fs::write(
        fixture
            .temp
            .path()
            .join("deps/mods/test-game/dependent.json"),
        serde_json::to_vec(&dependent).expect("JSON"),
    )
    .expect("dependent manifest");
    fixture.register();

    let policy = fixture.modules.join("test-game/main.lua");
    let original = fs::read_to_string(&policy).expect("Lua");
    let exclusive = original
        .replace(
            "destination = \"addons/\" .. suffix,",
            "destination = \"addons/base/\" .. suffix,",
        )
        .replace(
            "package_root = wrapper,",
            "package_root = wrapper, owned_directory = \"addons/base\",",
        );
    assert_ne!(original, exclusive);
    fs::write(&policy, exclusive).expect("install policy");
    fs::create_dir_all(fixture.game.join("addons")).expect("exclusive parent");
    let base = main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .args(["install", "base", "--yes", "--file"])
        .arg(&archive)
        .output()
        .expect("install base");
    assert!(base.status.success(), "{}", error(&base));
    let payload = fixture.game.join("addons/base/example.plugin");
    assert!(payload.is_file());
    fs::write(&policy, original).expect("restore default mapping");

    let preview = main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .args(["--json", "install", "dependent", "--dry-run", "--file"])
        .arg(&archive)
        .output()
        .expect("preview");
    assert!(preview.status.success(), "{}", error(&preview));
    let plan: Value = serde_json::from_slice(&preview.stdout).expect("plan");
    assert_eq!(plan["plan"]["status"], "reviewable");
    assert!(!fixture.game.join("addons/example.plugin").exists());

    fs::remove_file(&payload).expect("drifted payload");
    assert!(fixture.game.join("addons/base").is_dir());
    let blocked = main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(&fixture.state)
        .env("MODN_GAME_MODULES_DIR", &fixture.modules)
        .args(["install", "dependent", "--yes", "--file"])
        .arg(&archive)
        .output()
        .expect("install dependent");
    assert!(!blocked.status.success());
    assert!(
        error(&blocked).contains("dependency not installed or intact"),
        "{}",
        error(&blocked)
    );
    assert!(!fixture.game.join("addons/example.plugin").exists());
    assert!(fixture.game.join("addons/base").is_dir());
}

#[test]
fn recorded_dependency_cannot_be_uninstalled_while_dependent_is_managed() {
    let fixture = Fixture::new();
    let (base_archive, base_hash) = fixture.archive();
    fixture.bucket("deps", "test-game", "base", &base_hash, &[]);
    let dependent_archive = fixture.temp.path().join("dependent.zip");
    let mut zip = zip::ZipWriter::new(fs::File::create(&dependent_archive).expect("archive"));
    use std::io::Write;
    zip.start_file(
        "Demo/package/plugins/dependent.plugin",
        SimpleFileOptions::default(),
    )
    .expect("entry");
    zip.write_all(b"dependent-content").expect("payload");
    zip.finish().expect("finish");
    let dependent_hash = blake3::hash(&fs::read(&dependent_archive).expect("bytes"))
        .to_hex()
        .to_string();
    fs::write(
        fixture
            .temp
            .path()
            .join("deps/mods/test-game/dependent.json"),
        serde_json::to_vec(&json!({
            "schema_version":2, "id":"dependent", "game":"test-game",
            "name":"Dependent", "version":"1.0", "archive_format":"zip",
            "blake3":dependent_hash, "sources":[{"kind":"manual","filename":"dependent.zip"}],
            "dependencies":["base"]
        }))
        .expect("JSON"),
    )
    .expect("manifest");
    fixture.register();
    let install = |id: &str, archive: &std::path::Path| -> String {
        let output = fixture.run_paths(&["--json", "install", id, "--yes", "--file"], &[archive]);
        assert!(output.status.success(), "{}", error(&output));
        let value: Value = serde_json::from_slice(&output.stdout).expect("install JSON");
        value["installed"]["id"]
            .as_str()
            .expect("package id")
            .to_owned()
    };
    let base_id = install("base", &base_archive);
    let dependent_id = install("dependent", &dependent_archive);

    let blocked = fixture.run(&["uninstall", &base_id, "--yes"]);
    assert!(
        !blocked.status.success(),
        "dependency uninstall must be refused"
    );
    assert!(
        error(&blocked).contains("required by")
            && error(&blocked).contains("deps/test-game/dependent"),
        "{}",
        error(&blocked)
    );
    assert!(fixture.game.join("addons/example.plugin").is_file());

    let dependent = fixture.run(&["uninstall", &dependent_id, "--yes"]);
    assert!(dependent.status.success(), "{}", error(&dependent));
    let base = fixture.run(&["uninstall", &base_id, "--yes"]);
    assert!(base.status.success(), "{}", error(&base));
    assert!(!fixture.game.join("addons/example.plugin").exists());
    assert_eq!(
        fs::read(fixture.game.join("bin/TestGame.exe")).expect("marker"),
        b"game-exe"
    );
}
