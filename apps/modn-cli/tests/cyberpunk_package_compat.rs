//! Real CLI regressions for Cyberpunk ZIP layouts and generated Input Loader files.
use serde_json::Value;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

fn game() -> (TempDir, PathBuf, PathBuf) {
    let temp = tempdir().expect("temp");
    let root = temp.path().join("game");
    fs::create_dir_all(root.join("bin/x64")).expect("game");
    fs::write(root.join("bin/x64/Cyberpunk2077.exe"), b"marker").expect("marker");
    let state = temp.path().join("state");
    (temp, root, state)
}

fn archive(path: &Path, members: &[(&str, &[u8])]) {
    let mut writer = zip::ZipWriter::new(File::create(path).expect("archive"));
    for (name, bytes) in members {
        writer
            .start_file(*name, SimpleFileOptions::default())
            .expect("zip entry");
        writer.write_all(bytes).expect("contents");
    }
    writer.finish().expect("close");
}

fn cli(state: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--json", "--state-dir"])
        .arg(state)
        .args(["--game", "cyberpunk-2077"])
        .args(args)
        .output()
        .expect("modn")
}

fn json(state: &Path, args: &[&str]) -> Value {
    let output = cli(state, args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("JSON")
}

fn plan(state: &Path, game: &Path, source: &Path) -> Value {
    json(
        state,
        &[
            "install",
            source.to_str().expect("archive"),
            "--dry-run",
            "--path",
            game.to_str().expect("game"),
        ],
    )
}

fn has_blocker(plan: &Value, code: &str) -> bool {
    plan["blockers"]
        .as_array()
        .expect("blockers")
        .iter()
        .any(|blocker| blocker["code"] == code)
}

#[test]
fn windows_zip_names_roundtrip_and_keep_original_entry_identity() {
    let (temp, root, state) = game();
    let source = temp.path().join("windows.zip");
    let raw = r"bin\x64\plugins\cyber_engine_tweaks\mods\BetterLootMarkers\init.lua";
    let normalized = "bin/x64/plugins/cyber_engine_tweaks/mods/BetterLootMarkers/init.lua";
    archive(&source, &[(raw, b"return {enabled=true}")]);
    let inspected = json(&state, &["inspect", source.to_str().expect("archive")]);
    assert_eq!(inspected["files"][0]["entry"], normalized);
    assert_eq!(inspected["files"][0]["original_entry"], raw);
    let preview = plan(&state, &root, &source);
    assert_eq!(preview["status"], "reviewable", "{preview}");
    assert_eq!(preview["operations"][0]["source"], normalized);
    assert_eq!(preview["operations"][0]["destination"], normalized);

    let installed = json(
        &state,
        &[
            "install",
            source.to_str().expect("archive"),
            "--yes",
            "--path",
            root.to_str().expect("game"),
        ],
    );
    assert_eq!(
        fs::read(root.join(normalized)).expect("installed"),
        b"return {enabled=true}"
    );
    assert_eq!(installed["files"][0]["destination"], normalized);
    let package = installed["id"].as_str().expect("id");
    json(
        &state,
        &[
            "uninstall",
            package,
            "--yes",
            "--path",
            root.to_str().expect("game"),
        ],
    );
    assert!(!root.join(normalized).exists());
}

#[test]
fn rejects_unsafe_windows_names_and_normalized_case_collisions() {
    let (temp, root, state) = game();
    let source = temp.path().join("bad.zip");
    for name in [
        r"bin\x64\..\escape.lua",
        r"\\server\share\mod.lua",
        r"C:\temp\mod.lua",
        r"bin\\x64\mod.lua",
    ] {
        archive(&source, &[(name, b"bad")]);
        let inspected = cli(&state, &["inspect", source.to_str().expect("archive")]);
        assert!(!inspected.status.success(), "{name} must be rejected");
    }
    archive(
        &source,
        &[
            (
                r"bin\x64\plugins\cyber_engine_tweaks\mods\Same\init.lua",
                b"one",
            ),
            (
                "bin/x64/plugins/cyber_engine_tweaks/mods/same/INIT.lua",
                b"two",
            ),
        ],
    );
    let inspected = cli(&state, &["inspect", source.to_str().expect("archive")]);
    assert!(
        !inspected.status.success(),
        "normalized destination collision"
    );
    assert!(!root
        .join("bin/x64/plugins/cyber_engine_tweaks/mods")
        .exists());
}

#[test]
fn custom_input_xml_requires_plugin_and_rejects_invalid_input_layouts() {
    let (temp, root, state) = game();
    let source = temp.path().join("custom-quickslots.zip");
    archive(
        &source,
        &[("r6/input/CustomQuickslots.xml", b"<bindings/>")],
    );
    let result = plan(&state, &root, &source);
    assert_eq!(result["status"], "blocked");
    assert!(
        has_blocker(&result, "INPUT_LOADER_NOT_INSTALLED"),
        "{result}"
    );
    let attempted = cli(
        &state,
        &[
            "install",
            source.to_str().expect("archive"),
            "--yes",
            "--path",
            root.to_str().expect("game"),
        ],
    );
    assert!(!attempted.status.success());
    assert!(!root.join("r6/input/CustomQuickslots.xml").exists());

    let plugin = root.join("red4ext/plugins/input_loader/input_loader.dll");
    fs::create_dir_all(plugin.parent().expect("parent")).expect("plugin folder");
    fs::write(&plugin, b"installed plugin").expect("plugin");
    let result = plan(&state, &root, &source);
    assert_eq!(result["status"], "reviewable", "{result}");
    assert_eq!(result["operations"][0]["kind"], "input-xml");
    fs::create_dir_all(root.join("r6/input")).expect("input dir");
    fs::write(root.join("r6/input/CustomQuickslots.xml"), b"existing").expect("unmanaged");
    let result = plan(&state, &root, &source);
    assert!(has_blocker(&result, "EXISTING_FILE"), "{result}");
    assert_eq!(
        fs::read(root.join("r6/input/CustomQuickslots.xml")).expect("unchanged"),
        b"existing"
    );

    for name in [
        "r6/input/Bad.txt",
        "r6/Input/Bad.xml",
        "r6/input/Inner/Bad.xml",
    ] {
        archive(&source, &[(name, b"bad")]);
        let result = plan(&state, &root, &source);
        assert_eq!(result["status"], "blocked", "{name}: {result}");
        assert!(
            has_blocker(&result, "UNSUPPORTED_CONTENT")
                || has_blocker(&result, "NONCANONICAL_LAYOUT")
                || has_blocker(&result, "UNRECOGNIZED_LAYOUT"),
            "{name}: {result}"
        );
    }
}

#[test]
fn input_loader_generated_outputs_are_unowned_and_survive_uninstall() {
    let (temp, root, state) = game();
    let source = temp.path().join("input-loader.zip");
    let dll = "red4ext/plugins/input_loader/input_loader.dll";
    let input = "red4ext/plugins/input_loader/inputUserMappings.xml";
    let generated = [
        "engine/config/platform/pc/input_loader.ini",
        "r6/cache/inputContexts.xml",
        "r6/cache/inputUserMappings.xml",
    ];
    archive(
        &source,
        &[
            (dll, b"plugin"),
            (input, b"<bindings/>"),
            (generated[0], b"seed ini"),
            (generated[1], b"seed contexts"),
            (generated[2], b"seed mappings"),
        ],
    );
    let preview = plan(&state, &root, &source);
    assert_eq!(preview["status"], "reviewable", "{preview}");
    assert_eq!(
        preview["operations"].as_array().expect("operations").len(),
        2
    );
    let skipped = preview["skipped_generated_files"]
        .as_array()
        .expect("skipped");
    for name in generated {
        assert!(
            skipped.iter().any(|item| item == name),
            "missing skipped {name}: {preview}"
        );
    }
    let installed = json(
        &state,
        &[
            "install",
            source.to_str().expect("archive"),
            "--yes",
            "--path",
            root.to_str().expect("game"),
        ],
    );
    assert_eq!(installed["files"].as_array().expect("owned").len(), 2);
    assert!(root.join(dll).exists());
    for name in generated {
        assert!(
            !root.join(name).exists(),
            "manager copied runtime output: {name}"
        );
        let path = root.join(name);
        fs::create_dir_all(path.parent().expect("parent")).expect("cache dir");
        fs::write(path, b"game generated content").expect("game output");
    }
    let package = installed["id"].as_str().expect("id");
    json(
        &state,
        &[
            "uninstall",
            package,
            "--yes",
            "--path",
            root.to_str().expect("game"),
        ],
    );
    assert!(!root.join(dll).exists());
    assert!(!root.join(input).exists());
    for name in generated {
        assert_eq!(
            fs::read(root.join(name)).expect("unowned runtime data"),
            b"game generated content"
        );
    }

    // The exception is tied to the Input Loader package, never general write access.
    archive(
        &source,
        &[(dll, b"plugin"), ("r6/cache/arbitrary.xml", b"other")],
    );
    let result = plan(&state, &root, &source);
    assert_eq!(result["status"], "blocked", "{result}");
    assert!(has_blocker(&result, "UNRECOGNIZED_LAYOUT"));
    archive(&source, &[(generated[0], b"not Input Loader")]);
    let result = plan(&state, &root, &source);
    assert_eq!(result["status"], "blocked", "{result}");
}

#[test]
fn scanner_reports_input_xml_and_missing_loader_warning() {
    let (_temp, root, state) = game();
    let input = root.join("r6/input/CustomQuickslots.xml");
    fs::create_dir_all(input.parent().expect("parent")).expect("r6/input");
    fs::write(&input, b"<bindings/>").expect("input");
    let scanned = json(&state, &["scan", "--path", root.to_str().expect("game")]);
    assert!(
        scanned["mods"]
            .as_array()
            .expect("mods")
            .iter()
            .any(
                |item| item["relative_path"] == "r6/input/CustomQuickslots.xml"
                    && item["kind"] == "input-xml"
            ),
        "{scanned}"
    );
    let doctor = json(&state, &["doctor", "--path", root.to_str().expect("game")]);
    assert!(
        doctor["diagnostics"]
            .as_array()
            .expect("diagnostics")
            .iter()
            .any(|d| d["code"] == "INPUT_LOADER_NOT_INSTALLED"),
        "{doctor}"
    );
    let plugin = root.join("red4ext/plugins/input_loader/input_loader.dll");
    fs::create_dir_all(plugin.parent().expect("parent")).expect("folder");
    fs::write(plugin, b"plugin").expect("marker");
    let doctor = json(&state, &["doctor", "--path", root.to_str().expect("game")]);
    assert!(!doctor["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .any(|d| d["code"] == "INPUT_LOADER_NOT_INSTALLED"));
}

#[test]
fn nested_loader_dll_name_cannot_satisfy_input_xml_dependency() {
    let (temp, root, state) = game();
    let source = temp.path().join("spoof.zip");
    archive(
        &source,
        &[
            ("r6/input/Test.xml", b"<bindings/>"),
            (
                "red4ext/plugins/other/red4ext/plugins/input_loader/input_loader.dll",
                b"wrong location",
            ),
        ],
    );
    let preview = plan(&state, &root, &source);
    assert_eq!(preview["status"], "blocked", "{preview}");
    assert!(
        has_blocker(&preview, "INPUT_LOADER_NOT_INSTALLED"),
        "{preview}"
    );
    assert!(preview["skipped_generated_files"]
        .as_array()
        .expect("skipped")
        .is_empty());
}

#[test]
fn wrapped_input_loader_skips_only_matching_generated_runtime_outputs() {
    let (temp, root, state) = game();
    let source = temp.path().join("wrapped-input-loader.zip");
    archive(
        &source,
        &[
            (
                "InputLoader/red4ext/plugins/input_loader/input_loader.dll",
                b"dll",
            ),
            (
                "InputLoader/red4ext/plugins/input_loader/inputUserMappings.xml",
                b"<bindings/>",
            ),
            (
                "InputLoader/engine/config/platform/pc/input_loader.ini",
                b"generated ini",
            ),
            (
                "InputLoader/r6/cache/inputContexts.xml",
                b"generated contexts",
            ),
            (
                "InputLoader/r6/cache/inputUserMappings.xml",
                b"generated mappings",
            ),
        ],
    );
    let preview = plan(&state, &root, &source);
    assert_eq!(preview["status"], "reviewable", "{preview}");
    assert_eq!(
        preview["operations"].as_array().expect("operations").len(),
        2
    );
    let skipped = preview["skipped_generated_files"]
        .as_array()
        .expect("skipped");
    assert_eq!(skipped.len(), 3, "{preview}");
    for suffix in [
        "engine/config/platform/pc/input_loader.ini",
        "r6/cache/inputContexts.xml",
        "r6/cache/inputUserMappings.xml",
    ] {
        let expected = format!("InputLoader/{suffix}");
        assert!(
            skipped.iter().any(|name| name == &expected),
            "missing {expected}: {preview}"
        );
    }
    let installed = json(
        &state,
        &[
            "install",
            source.to_str().expect("zip"),
            "--yes",
            "--path",
            root.to_str().expect("game"),
        ],
    );
    assert_eq!(installed["files"].as_array().expect("owned").len(), 2);
    for suffix in [
        "engine/config/platform/pc/input_loader.ini",
        "r6/cache/inputContexts.xml",
        "r6/cache/inputUserMappings.xml",
    ] {
        assert!(
            !root.join(suffix).exists(),
            "generated path unexpectedly owned: {suffix}"
        );
    }
    json(
        &state,
        &[
            "uninstall",
            installed["id"].as_str().expect("id"),
            "--yes",
            "--path",
            root.to_str().expect("game"),
        ],
    );
}

#[test]
fn scanner_ignores_nested_input_xml_but_discovers_direct_unmanaged_files() {
    let (_temp, root, state) = game();
    let direct = root.join("r6/input/Direct.xml");
    let nested = root.join("r6/input/subdir/Unused.xml");
    fs::create_dir_all(nested.parent().expect("nested parent")).expect("folder");
    fs::write(&direct, b"direct").expect("direct");
    fs::write(&nested, b"nested").expect("nested");
    let scanned = json(&state, &["scan", "--path", root.to_str().expect("game")]);
    let mods = scanned["mods"].as_array().expect("mods");
    assert!(
        mods.iter().any(
            |mod_file| mod_file["relative_path"] == "r6/input/Direct.xml"
                && mod_file["kind"] == "input-xml"
        ),
        "{scanned}"
    );
    assert!(
        !mods
            .iter()
            .any(|mod_file| mod_file["relative_path"] == "r6/input/subdir/Unused.xml"),
        "{scanned}"
    );
    assert!(mods
        .iter()
        .all(|item| !item["managed"].as_bool().expect("managed")));
    assert_eq!(fs::read(&direct).expect("direct unchanged"), b"direct");
    assert_eq!(fs::read(nested).expect("nested unchanged"), b"nested");
}
