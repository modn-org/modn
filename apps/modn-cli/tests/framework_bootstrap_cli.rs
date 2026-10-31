//! Real CLI install/uninstall tests for complete framework distributions.
use serde_json::Value;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

fn fixture() -> (TempDir, PathBuf, PathBuf) {
    let temp = tempdir().expect("fixture");
    let game = temp.path().join("game");
    fs::create_dir_all(game.join("bin/x64")).expect("game directories");
    fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").expect("game marker");
    let state = temp.path().join("state");
    (temp, game, state)
}

fn zip(path: &Path, files: &[(&str, &[u8])], wrapper: &str) {
    let mut writer = zip::ZipWriter::new(File::create(path).expect("ZIP"));
    for &(name, content) in files {
        writer
            .start_file(format!("{wrapper}{name}"), SimpleFileOptions::default())
            .expect("ZIP member");
        writer.write_all(content).expect("ZIP content");
    }
    writer.finish().expect("ZIP complete");
}

fn cli(state: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--game", "cyberpunk-2077", "--json", "--state-dir"])
        .arg(state)
        .args(args)
        .output()
        .expect("CLI")
}

fn json(state: &Path, args: &[&str]) -> Value {
    let result = cli(state, args);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).expect("CLI JSON")
}

fn roundtrip(files: &[(&str, &[u8])], kind: &str, wrapper: &str) {
    let (temp, game, state) = fixture();
    let package = temp.path().join("framework.zip");
    zip(&package, files, wrapper);
    let game_path = game.to_str().expect("game");
    let zip_path = package.to_str().expect("ZIP");
    let plan = json(
        &state,
        &["install", zip_path, "--dry-run", "--path", game_path],
    );
    assert_eq!(plan["status"], "reviewable", "{plan}");
    let operations = plan["operations"].as_array().expect("operations");
    assert_eq!(operations.len(), files.len(), "{plan}");
    for operation in operations {
        assert_eq!(operation["kind"], kind, "{operation}");
        assert!(operation.get("owned_directory").is_none(), "{operation}");
    }
    assert!(!state.exists(), "dry run created manager state");
    let owned = json(&state, &["install", zip_path, "--yes", "--path", game_path]);
    let id = owned["id"].as_str().expect("package id");
    assert_eq!(
        owned["files"].as_array().expect("owned files").len(),
        files.len()
    );
    assert_eq!(
        owned["directories"]
            .as_array()
            .expect("owned directories")
            .len(),
        0
    );
    assert_eq!(
        owned["archive_sha256"]
            .as_str()
            .expect("ZIP provenance")
            .len(),
        64
    );
    for &(path, expected) in files {
        assert_eq!(
            fs::read(game.join(path)).expect("installed payload"),
            expected,
            "{path}"
        );
    }
    let installed = json(&state, &["installed", "--path", game_path]);
    assert_eq!(installed.as_array().expect("packages").len(), 1);
    assert_eq!(installed[0]["id"], id);
    assert!(installed[0].get("origin").is_none());

    let generated = game.join("red4ext/logs/generated.log");
    fs::create_dir_all(generated.parent().expect("parent")).expect("logs");
    fs::write(&generated, b"generated").expect("runtime log");
    json(&state, &["uninstall", id, "--yes", "--path", game_path]);
    for &(path, _) in files {
        assert!(!game.join(path).exists(), "{path} survived uninstall");
    }
    assert_eq!(
        fs::read(&generated).expect("retained generated content"),
        b"generated"
    );
    assert!(json(&state, &["installed", "--path", game_path])
        .as_array()
        .expect("empty ledger")
        .is_empty());
}

#[test]
fn cet_framework_owns_loader_and_plugin_assets_as_one_package() {
    roundtrip(
        &[
            ("bin/x64/version.dll", b"loader"),
            ("bin/x64/plugins/cyber_engine_tweaks.asi", b"extension"),
            ("bin/x64/LICENSE", b"license"),
            ("bin/x64/global.ini", b"config"),
            (
                "bin/x64/plugins/cyber_engine_tweaks/ThirdParty_LICENSES",
                b"licenses",
            ),
            (
                "bin/x64/plugins/cyber_engine_tweaks/fonts/NotoSans.ttf",
                b"font",
            ),
            (
                "bin/x64/plugins/cyber_engine_tweaks/tweakdb/usedhashes.kark",
                b"hashes",
            ),
            (
                "bin/x64/plugins/cyber_engine_tweaks/scripts/json/json.lua",
                b"json",
            ),
            (
                "bin/x64/plugins/cyber_engine_tweaks/scripts/json/README.md",
                b"docs",
            ),
        ],
        "cet-runtime",
        "wrapper/",
    );
}

#[test]
fn red4ext_framework_owns_bootstrap_files_as_one_package() {
    roundtrip(
        &[
            ("bin/x64/winmm.dll", b"loader"),
            ("red4ext/RED4ext.dll", b"framework"),
            ("red4ext/LICENSE.txt", b"license"),
            ("red4ext/THIRD_PARTY_LICENSES.txt", b"licenses"),
        ],
        "red4ext-runtime",
        "",
    );
}

#[test]
fn redscript_framework_installs_compiler_executable_only_at_its_tool_path() {
    roundtrip(
        &[
            ("engine/tools/scc.exe", b"compiler"),
            ("engine/tools/scc_lib.dll", b"library"),
            ("engine/config/base/scripts.ini", b"ini"),
            ("r6/config/cybercmd/scc.toml", b"config"),
        ],
        "redscript-runtime",
        "archive/",
    );
}

#[test]
fn partial_mixed_and_unapproved_executables_fail_without_side_effects() {
    let (temp, game, state) = fixture();
    let pkg = temp.path().join("invalid.zip");
    let root = game.to_str().expect("game");
    let zip_path = pkg.to_str().expect("zip");
    for invalid in [
        vec![("engine/tools/scc.exe", b"compiler".as_slice())],
        vec![
            ("engine/tools/scc.exe", b"compiler".as_slice()),
            ("engine/tools/scc_lib.dll", b"library"),
            ("engine/tools/evil.exe", b"evil"),
        ],
        vec![
            ("bin/x64/version.dll", b"loader".as_slice()),
            ("bin/x64/plugins/cyber_engine_tweaks.asi", b"plugin"),
            ("bin/x64/winmm.dll", b"another bootstrap"),
            ("red4ext/RED4ext.dll", b"another runtime"),
        ],
        vec![
            ("bin/x64/version.dll", b"loader".as_slice()),
            ("bin/x64/plugins/cyber_engine_tweaks.asi", b"plugin"),
            (
                "bin/x64/plugins/cyber_engine_tweaks/mods/ThirdParty/init.lua",
                b"mod",
            ),
        ],
    ] {
        zip(&pkg, &invalid, "");
        let preview = json(&state, &["install", zip_path, "--dry-run", "--path", root]);
        assert_eq!(preview["status"], "blocked", "{preview}");
        assert!(
            !cli(&state, &["install", zip_path, "--yes", "--path", root])
                .status
                .success()
        );
        assert!(
            json(&state, &["installed", "--path", root])
                .as_array()
                .expect("packages")
                .is_empty(),
            "blocked ZIP claimed ownership"
        );
    }
    assert!(!game.join("engine").exists());
    assert!(!game.join("red4ext").exists());
    assert!(!game.join("bin/x64/version.dll").exists());
}
