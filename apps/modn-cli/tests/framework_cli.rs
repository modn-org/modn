use serde_json::Value;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

fn fixture() -> (TempDir, PathBuf, PathBuf) {
    let temp = tempdir().expect("test directory");
    let game = temp.path().join("game");
    fs::create_dir_all(game.join("bin/x64")).expect("game directory");
    fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").expect("game marker");
    let state = temp.path().join("state");
    (temp, game, state)
}

fn archive(path: &Path, entries: &[(&str, &[u8])]) {
    let mut writer = zip::ZipWriter::new(File::create(path).expect("archive file"));
    for &(name, bytes) in entries {
        writer
            .start_file(name, SimpleFileOptions::default())
            .expect("ZIP entry");
        writer.write_all(bytes).expect("ZIP bytes");
    }
    writer.finish().expect("complete ZIP");
}

fn cli(state: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--game", "cyberpunk-2077", "--json", "--state-dir"])
        .arg(state)
        .args(args)
        .output()
        .expect("modn CLI")
}

fn success(state: &Path, args: &[&str]) -> Value {
    let output = cli(state, args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("CLI JSON")
}

fn blocked(state: &Path, args: &[&str]) -> Value {
    let output = cli(state, args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).expect("CLI preview");
    assert_eq!(result["status"], "blocked", "{result}");
    result
}

fn preview(game: &Path, state: &Path, package: &Path) -> Value {
    success(
        state,
        &[
            "install",
            package.to_str().expect("ZIP path"),
            "--dry-run",
            "--path",
            game.to_str().expect("game path"),
        ],
    )
}

#[test]
fn cet_nested_zip_is_managed_and_survives_conflicts_and_drift() {
    let (temp, game, state) = fixture();
    let zip = temp.path().join("cet.zip");
    let prefix = "bin/x64/plugins/cyber_engine_tweaks/mods/FixtureCET";
    let init = format!("{prefix}/init.lua");
    let config = format!("{prefix}/data/config.json");
    archive(
        &zip,
        &[
            (&format!("bundle/{init}"), b"return {}"),
            (&format!("bundle/{config}"), b"{}"),
            ("bundle/README.md", b"fixture"),
        ],
    );
    let review = preview(&game, &state, &zip);
    assert_eq!(review["status"], "reviewable", "{review}");
    assert_eq!(
        review["operations"].as_array().expect("operations").len(),
        2
    );
    assert!(!state.exists(), "preview must remain read-only");
    assert!(!game.join("bin/x64/plugins").exists());

    let owned = success(
        &state,
        &[
            "install",
            zip.to_str().expect("archive"),
            "--yes",
            "--path",
            game.to_str().expect("game"),
        ],
    );
    let id = owned["id"].as_str().expect("installed package id");
    assert_eq!(
        fs::read(game.join(&init)).expect("installed Lua"),
        b"return {}"
    );
    assert_eq!(
        fs::read(game.join(&config)).expect("installed config"),
        b"{}"
    );
    let scan = success(&state, &["scan", "--path", game.to_str().expect("game")]);
    assert!(scan["mods"]
        .as_array()
        .expect("mods")
        .iter()
        .any(|item| item["kind"] == "cet-mod"
            && item["managed"] == true
            && item["relative_path"] == init));
    let doctor = success(&state, &["doctor", "--path", game.to_str().expect("game")]);
    assert!(!doctor["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .any(|item| item["code"] == "UNMANAGED_CONTENT"));

    let duplicate = cli(
        &state,
        &[
            "install",
            zip.to_str().expect("archive"),
            "--yes",
            "--path",
            game.to_str().expect("game"),
        ],
    );
    assert!(
        !duplicate.status.success(),
        "existing owned files must block"
    );
    fs::write(game.join(&init), b"edited").expect("mutated managed fixture");
    let drift = success(&state, &["doctor", "--path", game.to_str().expect("game")]);
    assert!(drift["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .any(|item| item["code"] == "OWNED_FILE_MODIFIED"));
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
    assert_eq!(
        fs::read(game.join(&init)).expect("preserved changes"),
        b"edited"
    );
    fs::write(game.join(&init), b"return {}").expect("restore owned fixture");
    success(
        &state,
        &[
            "uninstall",
            id,
            "--yes",
            "--path",
            game.to_str().expect("game"),
        ],
    );
    assert!(!game.join(&init).exists());
    assert!(!game.join(&config).exists());
}

#[test]
fn red4ext_plugin_zip_accepts_dll_and_protects_manual_files() {
    let (temp, game, state) = fixture();
    let zip = temp.path().join("red4ext.zip");
    let dll = "red4ext/plugins/FixtureRed4ext/FixtureRed4ext.dll";
    let config = "red4ext/plugins/FixtureRed4ext/settings/config.yaml";
    archive(&zip, &[(dll, b"fixture-dll"), (config, b"enabled: true")]);
    let review = preview(&game, &state, &zip);
    assert_eq!(review["status"], "reviewable", "{review}");
    assert_eq!(
        review["operations"].as_array().expect("operations").len(),
        2
    );
    let owned = success(
        &state,
        &[
            "install",
            zip.to_str().expect("archive"),
            "--yes",
            "--path",
            game.to_str().expect("game"),
        ],
    );
    let id = owned["id"].as_str().expect("package ID");
    assert_eq!(fs::read(game.join(dll)).expect("dll"), b"fixture-dll");
    let scan = success(&state, &["scan", "--path", game.to_str().expect("game")]);
    assert!(scan["mods"]
        .as_array()
        .expect("mods")
        .iter()
        .any(|item| item["kind"] == "red4ext-plugin"
            && item["managed"] == true
            && item["relative_path"] == dll));
    let doctor = success(&state, &["doctor", "--path", game.to_str().expect("game")]);
    assert!(!doctor["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .any(|item| item["code"] == "UNMANAGED_CONTENT"));
    success(
        &state,
        &[
            "uninstall",
            id,
            "--yes",
            "--path",
            game.to_str().expect("game"),
        ],
    );
    assert!(!game.join(dll).exists());
    assert!(!game.join(config).exists());

    let other = "red4ext/plugins/FixtureRed4ext/FixtureRed4ext.dll";
    fs::create_dir_all(game.join("red4ext/plugins/FixtureRed4ext")).expect("manual plugin");
    fs::write(game.join(other), b"manual").expect("manual file");
    let blocked = blocked(
        &state,
        &[
            "install",
            zip.to_str().expect("archive"),
            "--dry-run",
            "--path",
            game.to_str().expect("game"),
        ],
    );
    assert!(blocked["blockers"]
        .as_array()
        .expect("blockers")
        .iter()
        .any(|entry| entry["code"] == "EXISTING_FILE"));
    assert_eq!(
        fs::read(game.join(other)).expect("manual unchanged"),
        b"manual"
    );
}

#[test]
fn compatible_mixed_layouts_install_atomically_and_unrelated_content_is_blocked() {
    let (temp, game, state) = fixture();
    let zip = temp.path().join("mixed.zip");
    let cet = "bin/x64/plugins/cyber_engine_tweaks/mods/Foo/init.lua";
    let red = "red4ext/plugins/Bar/Bar.dll";
    let legacy = "archive/pc/mod/bar.archive";
    archive(&zip, &[(cet, b"lua"), (red, b"dll"), (legacy, b"archive")]);
    assert_eq!(preview(&game, &state, &zip)["status"], "reviewable");
    let pkg = success(
        &state,
        &[
            "install",
            zip.to_str().expect("archive"),
            "--yes",
            "--path",
            game.to_str().expect("game"),
        ],
    );
    let id = pkg["id"].as_str().expect("package ID");
    for name in [cet, red, legacy] {
        assert!(game.join(name).is_file(), "not installed: {name}");
    }
    success(
        &state,
        &[
            "uninstall",
            id,
            "--yes",
            "--path",
            game.to_str().expect("game"),
        ],
    );
    for name in [cet, red, legacy] {
        assert!(!game.join(name).exists(), "not removed: {name}");
    }

    for extra in [
        "r6/scripts/new.reds",
        "mods/REDmod/info.json",
        "red4ext/plugins/Bar/uninstall.bat",
    ] {
        archive(&zip, &[(cet, b"lua"), (red, b"dll"), (extra, b"bad")]);
        let report = blocked(
            &state,
            &[
                "install",
                zip.to_str().expect("archive"),
                "--dry-run",
                "--path",
                game.to_str().expect("game"),
            ],
        );
        assert!(!report["blockers"].as_array().expect("blockers").is_empty());
        let result = cli(
            &state,
            &[
                "install",
                zip.to_str().expect("archive"),
                "--yes",
                "--path",
                game.to_str().expect("game"),
            ],
        );
        assert!(
            !result.status.success(),
            "mixed unsafe package accepted: {extra}"
        );
        assert!(
            !game.join(cet).exists(),
            "partial contents leaked from unsafe ZIP"
        );
        assert!(
            !game.join(red).exists(),
            "partial contents leaked from unsafe ZIP"
        );
    }
}

#[test]
fn preexisting_directory_and_duplicate_target_remain_blocked() {
    let (temp, game, state) = fixture();
    let zip = temp.path().join("blocked.zip");
    let path = "bin/x64/plugins/cyber_engine_tweaks/mods/Manual/init.lua";
    fs::create_dir_all(game.join("bin/x64/plugins/cyber_engine_tweaks/mods/Manual"))
        .expect("manual mod directory");
    fs::write(game.join(path), b"unmanaged").expect("manual mod file");
    archive(&zip, &[(path, b"new")]);
    assert_eq!(preview(&game, &state, &zip)["status"], "blocked");
    assert_eq!(
        fs::read(game.join(path)).expect("manual file"),
        b"unmanaged"
    );
    archive(
        &zip,
        &[
            (
                "bin/x64/plugins/cyber_engine_tweaks/mods/Duplicate/init.lua",
                b"a",
            ),
            (
                "bin/x64/plugins/cyber_engine_tweaks/mods/duplicate/INIT.LUA",
                b"b",
            ),
        ],
    );
    let result = cli(
        &state,
        &[
            "install",
            zip.to_str().expect("zip"),
            "--dry-run",
            "--path",
            game.to_str().expect("game"),
        ],
    );
    assert!(
        !result.status.success(),
        "archive duplicates must fail inspection"
    );
    assert_eq!(
        fs::read(game.join(path)).expect("manual preserved"),
        b"unmanaged"
    );
}

#[test]
fn redmod_scan_stays_directory_only_and_does_not_traverse_large_mods() {
    let (_temp, game, state) = fixture();
    let root = game.join("mods/UnmanagedREDmod");
    fs::create_dir_all(root.join("nested")).expect("REDmod directory fixture");
    fs::write(root.join("info.json"), b"{}").expect("REDmod metadata");
    fs::write(root.join("nested/asset.tweak"), b"data").expect("REDmod payload");
    let report = success(&state, &["scan", "--path", game.to_str().expect("game")]);
    let redmods: Vec<&Value> = report["mods"]
        .as_array()
        .expect("mods array")
        .iter()
        .filter(|mod_entry| mod_entry["kind"] == "redmod-directory")
        .collect();
    assert_eq!(redmods.len(), 1, "REDmod files must not be emitted as mods");
    assert_eq!(redmods[0]["relative_path"], "mods/UnmanagedREDmod");
    assert_eq!(redmods[0]["managed"], false);
    assert_eq!(report["incomplete"], false);
}

#[test]
fn executable_windows_payload_suffixes_block_framework_packages() {
    let (temp, game, state) = fixture();
    let zip = temp.path().join("unsafe-executable.zip");
    for extension in ["com", "scr", "hta", "pif", "cpl"] {
        let payload = format!("red4ext/plugins/Fixture/launch.{extension}");
        archive(&zip, &[(payload.as_str(), b"unsafe")]);
        let preview = success(
            &state,
            &[
                "install",
                zip.to_str().expect("archive"),
                "--dry-run",
                "--path",
                game.to_str().expect("game"),
            ],
        );
        assert_eq!(preview["status"], "blocked", "unsafe extension {extension}");
        assert!(preview["blockers"]
            .as_array()
            .expect("blockers")
            .iter()
            .any(|issue| issue["code"] == "EXECUTABLE_PAYLOAD"));
        let approved = cli(
            &state,
            &[
                "install",
                zip.to_str().expect("archive"),
                "--yes",
                "--path",
                game.to_str().expect("game"),
            ],
        );
        assert!(
            !approved.status.success(),
            "unsafe executable installed: {extension}"
        );
        assert!(!game.join("red4ext").exists());
    }
}

#[test]
fn incorrectly_cased_framework_paths_block_mixed_zip_without_writes() {
    let (temp, game, state) = fixture();
    let zip = temp.path().join("wrong-case.zip");
    let legacy = "archive/pc/mod/Safe.archive";
    for invalid in [
        "Bin/x64/plugins/cyber_engine_tweaks/mods/Case/init.lua",
        "bin/X64/plugins/cyber_engine_tweaks/mods/Case/init.lua",
        "bin/x64/Plugins/cyber_engine_tweaks/mods/Case/init.lua",
        "bin/x64/plugins/Cyber_Engine_Tweaks/mods/Case/init.lua",
        "bin/x64/plugins/cyber_engine_tweaks/Mods/Case/init.lua",
        "RED4ext/plugins/Case/Case.dll",
        "red4ext/Plugins/Case/Case.dll",
    ] {
        archive(&zip, &[(legacy, b"legacy"), (invalid, b"plugin")]);
        let existed_before_preview = state.exists();
        let report = preview(&game, &state, &zip);
        assert_eq!(report["status"], "blocked", "{invalid}: {report}");
        assert!(report["blockers"]
            .as_array()
            .expect("blockers")
            .iter()
            .any(|item| item["code"] == "NONCANONICAL_LAYOUT"));
        assert_eq!(
            state.exists(),
            existed_before_preview,
            "preview changed state"
        );
        let approved = cli(
            &state,
            &[
                "install",
                zip.to_str().expect("archive"),
                "--yes",
                "--path",
                game.to_str().expect("game"),
            ],
        );
        assert!(
            !approved.status.success(),
            "installed wrong-cased {invalid}"
        );
        assert!(!game.join(invalid).exists(), "stray framework files");
        assert!(!game.join(legacy).exists(), "partial legacy installation");
        let scan = success(&state, &["scan", "--path", game.to_str().expect("game")]);
        assert!(scan["mods"].as_array().expect("mods").is_empty());
        assert!(scan["owned_files"]
            .as_array()
            .expect("owned files")
            .is_empty());
    }
}
