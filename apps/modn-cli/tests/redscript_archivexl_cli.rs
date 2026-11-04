#[path = "support/main_bucket.rs"]
mod main_bucket;
use serde_json::Value;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Output;
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

fn fixture() -> (TempDir, PathBuf) {
    let temp = tempdir().expect("fixture");
    let game = temp.path().join("game");
    fs::create_dir_all(game.join("bin/x64")).expect("game");
    fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"marker").expect("marker");
    (temp, game)
}

fn framework(game: &Path, path: &str) {
    let target = game.join(path);
    fs::create_dir_all(target.parent().expect("parent")).expect("parent");
    fs::write(target, b"framework fixture, not compatibility proof").expect("framework");
}

fn zip(path: &Path, entries: &[(&str, &[u8])]) {
    let mut archive = zip::ZipWriter::new(File::create(path).expect("ZIP"));
    for &(name, data) in entries {
        archive
            .start_file(name, SimpleFileOptions::default())
            .expect("member");
        archive.write_all(data).expect("contents");
    }
    archive.finish().expect("closed ZIP");
}

fn cli(state: &Path, args: &[&str]) -> Output {
    main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .args(["--game", "cyberpunk-2077", "--json", "--state-dir"])
        .arg(state)
        .args(args)
        .output()
        .expect("CLI")
}

fn ok(state: &Path, args: &[&str]) -> Value {
    let response = cli(state, args);
    assert!(
        response.status.success(),
        "{}",
        String::from_utf8_lossy(&response.stderr)
    );
    serde_json::from_slice(&response.stdout).expect("JSON")
}

fn preview(state: &Path, game: &Path, archive: &Path) -> Value {
    ok(
        state,
        &[
            "install",
            archive.to_str().expect("ZIP path"),
            "--dry-run",
            "--path",
            game.to_str().expect("game path"),
        ],
    )
}

fn roundtrip(entries: &[(&str, &[u8])], dependencies: &[&str]) {
    let (temp, game) = fixture();
    let state = temp.path().join("state");
    let package = temp.path().join("fixture.zip");
    for dependency in dependencies {
        framework(&game, dependency);
    }
    zip(&package, entries);
    let inspected = ok(&state, &["inspect", package.to_str().expect("ZIP path")]);
    assert_eq!(inspected["format"], "zip");
    let proposal = preview(&state, &game, &package);
    assert_eq!(proposal["status"], "reviewable", "{proposal}");
    assert_eq!(
        proposal["operations"].as_array().expect("operations").len(),
        entries.len()
    );
    assert!(!state.exists(), "preview modified manager state");
    for &(name, _) in entries {
        assert!(!game.join(name).exists(), "preview wrote {name}");
    }
    let installed = ok(
        &state,
        &[
            "install",
            package.to_str().expect("ZIP"),
            "--yes",
            "--path",
            game.to_str().expect("game"),
        ],
    );
    let id = installed["id"].as_str().expect("package id");
    assert_eq!(
        installed["files"].as_array().expect("files").len(),
        entries.len()
    );
    let listed = ok(&state, &["list", "--path", game.to_str().expect("game")]);
    assert_eq!(listed.as_array().expect("installed packages").len(), 1);
    let scanned = ok(&state, &["scan", "--path", game.to_str().expect("game")]);
    let mods = scanned["mods"].as_array().expect("mods");
    let owned = scanned["owned_files"].as_array().expect("owned files");
    assert_eq!(owned.len(), entries.len());
    for &(name, bytes) in entries {
        assert_eq!(fs::read(game.join(name)).expect("managed file"), bytes);
        let directory_owned = installed["directories"]
            .as_array()
            .expect("directory claims")
            .iter()
            .filter_map(Value::as_str)
            .any(|root| name.starts_with(&format!("{root}/")));
        let expected_status = if directory_owned { "present" } else { "intact" };
        assert!(
            owned
                .iter()
                .any(|f| { f["relative_path"] == name && f["status"] == expected_status }),
            "missing {expected_status} owned file: {name}; {scanned}"
        );
        assert!(
            mods.iter()
                .any(|m| m["relative_path"] == name && m["managed"] == true),
            "missing managed scan attribution: {name}; {scanned}"
        );
    }
    let doctor = ok(&state, &["doctor", "--path", game.to_str().expect("game")]);
    assert!(!doctor["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .any(|item| item["code"]
            .as_str()
            .unwrap_or("")
            .starts_with("OWNED_FILE_")));
    ok(
        &state,
        &[
            "uninstall",
            id,
            "--yes",
            "--path",
            game.to_str().expect("game"),
        ],
    );
    for &(name, _) in entries {
        assert!(!game.join(name).exists(), "uninstall left {name}");
    }
    for dependency in dependencies {
        assert!(
            game.join(dependency).exists(),
            "removed unmanaged framework {dependency}"
        );
    }
}

#[test]
fn redscript_nested_files_roundtrip() {
    roundtrip(
        &[
            ("r6/scripts/Fixture/Core/main.reds", b"class Main {}"),
            ("r6/scripts/Fixture/side.reds", b"class Side {}"),
        ],
        &["engine/tools/scc.exe"],
    );
}

#[test]
fn archive_xl_and_reds_user_hints_roundtrip() {
    roundtrip(
        &[
            ("archive/pc/mod/Fixture.archive", b"archive"),
            ("archive/pc/mod/Fixture.archive.xl", b"resource: {}"),
            ("r6/config/redsUserHints/Fixture.toml", b"[hints]"),
        ],
        &[
            "red4ext/plugins/ArchiveXL/ArchiveXL.dll",
            "engine/tools/scc.exe",
        ],
    );
}

#[test]
fn mixed_supported_layouts_roundtrip() {
    roundtrip(
        &[
            ("archive/pc/mod/Base.archive", b"archive"),
            ("archive/pc/mod/Base.archive.xl", b"resource: {}"),
            ("r6/scripts/Fixture/example.reds", b"class Example {}"),
            ("r6/config/redsUserHints/Fixture.toml", b"[hints]"),
            ("r6/tweaks/Fixture.yaml", b"mod: true"),
            (
                "bin/x64/plugins/cyber_engine_tweaks/mods/Fixture/init.lua",
                b"return {}",
            ),
            ("red4ext/plugins/Fixture/Fixture.dll", b"dll"),
        ],
        &[
            "engine/tools/scc.exe",
            "red4ext/plugins/ArchiveXL/ArchiveXL.dll",
            "red4ext/plugins/TweakXL/TweakXL.dll",
        ],
    );
}

#[test]
fn dependency_and_layout_failures_block_entire_package() {
    let (temp, game) = fixture();
    let state = temp.path().join("state");
    let archive = temp.path().join("blocked.zip");
    let safe = ("archive/pc/mod/Safe.archive", b"safe".as_slice());
    for (member, expected) in [
        ("r6/scripts/Foo.reds", "REDSCRIPT_NOT_INSTALLED"),
        ("archive/pc/mod/Foo.archive.xl", "ARCHIVEXL_NOT_INSTALLED"),
        ("r6/tweaks/Foo.yaml", "TWEAKXL_NOT_INSTALLED"),
        ("r6/scripts/Foo.exe", "UNSUPPORTED_CONTENT"),
        ("r6/scripts/Foo.yaml", "UNSUPPORTED_CONTENT"),
        ("r6/config/redsUserHints/Foo.txt", "UNSUPPORTED_CONTENT"),
        ("r6/Tweaks/Foo.yaml", "NONCANONICAL_LAYOUT"),
        ("r6/Scripts/Foo.reds", "NONCANONICAL_LAYOUT"),
        ("archive/PC/mod/Foo.archive.xl", "NONCANONICAL_LAYOUT"),
        ("mods/Redmod/info.json", "NOT_INSTALLABLE"),
        ("unknown/path/data.dat", "UNRECOGNIZED_LAYOUT"),
    ] {
        zip(&archive, &[safe, (member, b"bad")]);
        let proposed = preview(&state, &game, &archive);
        assert_eq!(proposed["status"], "blocked", "{member}: {proposed}");
        assert!(
            proposed["blockers"]
                .as_array()
                .expect("blockers")
                .iter()
                .any(|issue| issue["code"] == expected),
            "{member}: {proposed}"
        );
        let approved = cli(
            &state,
            &[
                "install",
                archive.to_str().expect("ZIP"),
                "--yes",
                "--path",
                game.to_str().expect("game"),
            ],
        );
        assert!(
            !approved.status.success(),
            "installed blocked layout: {member}"
        );
        assert!(
            !game.join(safe.0).exists(),
            "partial file from blocked package"
        );
    }
}

#[test]
fn manual_conflicts_and_drift_are_not_adopted_or_destroyed() {
    let (temp, game) = fixture();
    let state = temp.path().join("state");
    let archive = temp.path().join("drift.zip");
    let script = "r6/scripts/Fixture/Fixture.reds";
    framework(&game, "engine/tools/scc.exe");
    zip(&archive, &[(script, b"class Fixture {}")]);
    let installed = ok(
        &state,
        &[
            "install",
            archive.to_str().expect("ZIP"),
            "--yes",
            "--path",
            game.to_str().expect("game"),
        ],
    );
    let id = installed["id"].as_str().expect("id");
    fs::write(game.join(script), b"changed").expect("drift");
    let doctor = ok(&state, &["doctor", "--path", game.to_str().expect("game")]);
    assert!(doctor["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .any(|i| i["code"] == "OWNED_FILE_MODIFIED"));
    assert!(!cli(
        &state,
        &[
            "uninstall",
            id,
            "--yes",
            "--path",
            game.to_str().expect("game")
        ]
    )
    .status
    .success());
    assert_eq!(fs::read(game.join(script)).expect("unchanged"), b"changed");
    fs::write(game.join(script), b"class Fixture {}").expect("restore");
    ok(
        &state,
        &[
            "uninstall",
            id,
            "--yes",
            "--path",
            game.to_str().expect("game"),
        ],
    );
    fs::create_dir_all(game.join("r6/scripts/Fixture")).expect("manual folder");
    fs::write(game.join(script), b"manual").expect("manual");
    assert_eq!(preview(&state, &game, &archive)["status"], "blocked");
    assert_eq!(fs::read(game.join(script)).expect("manual"), b"manual");
}

#[test]
fn wrapper_docs_and_archive_xl_are_accounted_for_without_installing_docs() {
    let (temp, game) = fixture();
    let state = temp.path().join("state");
    let package = temp.path().join("wrapper.zip");
    framework(&game, "engine/tools/scc.exe");
    framework(&game, "red4ext/plugins/ArchiveXL/ArchiveXL.dll");
    zip(
        &package,
        &[
            (
                "Wrapped/r6/scripts/Fixture/nested.reds",
                b"class Fixture {}",
            ),
            (
                "Wrapped/r6/scripts/Fixture/CHANGELOG.txt",
                b"documentation only",
            ),
            ("Wrapped/archive/pc/mod/Fixture.archive.xl", b"resource: {}"),
            ("Wrapped/README.md", b"documentation only"),
        ],
    );
    let proposal = preview(&state, &game, &package);
    assert_eq!(proposal["status"], "reviewable", "{proposal}");
    assert_eq!(
        proposal["operations"].as_array().expect("operations").len(),
        2
    );
    let installed = ok(
        &state,
        &[
            "install",
            package.to_str().expect("ZIP"),
            "--yes",
            "--path",
            game.to_str().expect("game"),
        ],
    );
    assert!(game.join("r6/scripts/Fixture/nested.reds").exists());
    assert!(game.join("archive/pc/mod/Fixture.archive.xl").exists());
    assert!(!game.join("Wrapped").exists());
    assert!(!game.join("r6/scripts/Fixture/CHANGELOG.txt").exists());
    ok(
        &state,
        &[
            "uninstall",
            installed["id"].as_str().expect("id"),
            "--yes",
            "--path",
            game.to_str().expect("game"),
        ],
    );
}

#[test]
fn doctor_warns_on_missing_runtime_markers_without_adopting_manual_content() {
    let (_temp, game) = fixture();
    let state = game.parent().expect("parent").join("state");
    for path in [
        "r6/scripts/Manual/script.reds",
        "archive/pc/mod/Manual.archive.xl",
        "r6/config/redsUserHints/Manual.toml",
        "r6/tweaks/Manual.yaml",
    ] {
        framework(&game, path);
    }
    let report = ok(&state, &["scan", "--path", game.to_str().expect("game")]);
    assert_eq!(report["mods"].as_array().expect("mods").len(), 4);
    assert!(report["mods"]
        .as_array()
        .expect("mods")
        .iter()
        .all(|m| m["managed"] == false));
    let diagnosed = ok(&state, &["doctor", "--path", game.to_str().expect("game")]);
    for code in [
        "REDSCRIPT_NOT_INSTALLED",
        "ARCHIVEXL_NOT_INSTALLED",
        "TWEAKXL_NOT_INSTALLED",
        "UNMANAGED_CONTENT",
    ] {
        assert!(
            diagnosed["diagnostics"]
                .as_array()
                .expect("diagnostics")
                .iter()
                .any(|item| item["code"] == code),
            "missing {code}: {diagnosed}"
        );
    }
    assert!(report["owned_files"].as_array().expect("owned").is_empty());
}

#[test]
fn directory_ownership_removes_runtime_contents_but_shared_fallback_protects_files() {
    let (temp, game) = fixture();
    let state = temp.path().join("state");
    let archive = temp.path().join("directory.zip");
    framework(&game, "engine/tools/scc.exe");
    fs::create_dir_all(game.join("r6/scripts")).expect("shared parent");
    let script = "r6/scripts/NewMod/main.reds";
    zip(&archive, &[(script, b"class Main {}")]);
    let proposed = preview(&state, &game, &archive);
    assert_eq!(
        proposed["operations"][0]["owned_directory"],
        "r6/scripts/NewMod"
    );
    let installed = ok(
        &state,
        &[
            "install",
            archive.to_str().expect("ZIP"),
            "--yes",
            "--path",
            game.to_str().expect("game"),
        ],
    );
    assert_eq!(
        installed["directories"],
        serde_json::json!(["r6/scripts/NewMod"])
    );
    fs::write(game.join(script), b"runtime changed code").expect("change");
    fs::write(game.join("r6/scripts/NewMod/runtime.db"), b"runtime output").expect("output");
    let scanned = ok(&state, &["scan", "--path", game.to_str().expect("game")]);
    assert!(scanned["owned_files"]
        .as_array()
        .expect("owned files")
        .iter()
        .any(|file| file["relative_path"] == script && file["status"] == "present"));
    ok(
        &state,
        &[
            "uninstall",
            installed["id"].as_str().expect("id"),
            "--yes",
            "--path",
            game.to_str().expect("game"),
        ],
    );
    assert!(!game.join("r6/scripts/NewMod").exists());
    assert!(game.join("r6/scripts").is_dir());

    fs::create_dir(game.join("r6/scripts/Shared")).expect("existing folder");
    let manual = game.join("r6/scripts/Shared/manual.log");
    fs::write(&manual, b"manual").expect("manual file");
    let shared = "r6/scripts/Shared/mod.reds";
    zip(&archive, &[(shared, b"class Shared {}")]);
    let proposed = preview(&state, &game, &archive);
    assert_eq!(proposed["status"], "reviewable", "{proposed}");
    assert!(proposed["operations"][0].get("owned_directory").is_none());
    let installed = ok(
        &state,
        &[
            "install",
            archive.to_str().expect("ZIP"),
            "--yes",
            "--path",
            game.to_str().expect("game"),
        ],
    );
    assert_eq!(installed["directories"], serde_json::json!([]));
    fs::write(game.join(shared), b"runtime changed code").expect("change");
    let args = [
        "uninstall",
        installed["id"].as_str().expect("id"),
        "--yes",
        "--path",
        game.to_str().expect("game"),
    ];
    assert!(
        !cli(&state, &args).status.success(),
        "changed shared file must block removal"
    );
    fs::write(game.join(shared), b"class Shared {}").expect("restore");
    ok(&state, &args);
    assert_eq!(fs::read(manual).expect("unmanaged file"), b"manual");
    assert!(game.join("r6/scripts/Shared").is_dir());
}
