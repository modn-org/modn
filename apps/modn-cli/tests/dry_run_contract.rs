//! Host validation is independent of game module proposals.
use serde_json::Value;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

#[expect(
    clippy::unwrap_used,
    reason = "Test fixtures fail immediately on setup errors"
)]
fn fixture(entries: &[&str]) -> (TempDir, PathBuf, PathBuf, PathBuf) {
    let temp = tempdir().unwrap();
    let game = temp.path().join("game");
    fs::create_dir_all(game.join("bin/x64")).unwrap();
    fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").unwrap();
    let archive = temp.path().join("package.zip");
    let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
    for entry in entries {
        zip.start_file(*entry, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"payload").unwrap();
    }
    zip.finish().unwrap();
    let state = temp.path().join("state");
    (temp, game, archive, state)
}

#[expect(
    clippy::unwrap_used,
    reason = "Test fixtures fail immediately on setup errors"
)]
fn module_with(temp: &Path, implementation: &str) -> PathBuf {
    let parent = temp.join("modules");
    let module = parent.join("cyberpunk-2077");
    fs::create_dir_all(&module).unwrap();
    let bundled = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/modn-core/game-modules/cyberpunk-2077");
    fs::copy(bundled.join("module.json"), module.join("module.json")).unwrap();
    let code = fs::read_to_string(bundled.join("main.luau")).unwrap();
    let before = code
        .strip_suffix("return policy\r\n")
        .or_else(|| code.strip_suffix("return policy\n"))
        .unwrap();
    fs::write(
        module.join("main.luau"),
        format!("{before}\nfunction policy.plan(files)\n{implementation}\nend\nreturn policy\n"),
    )
    .unwrap();
    parent
}

#[expect(clippy::unwrap_used, reason = "Test process setup may fail fast")]
fn run(game: &Path, archive: &Path, state: &Path, module: Option<&Path>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_modn"));
    cmd.args(["--game", "cyberpunk-2077"]);
    cmd.args(["--state-dir", state.to_str().unwrap(), "--json"]);
    cmd.args([
        "install",
        archive.to_str().unwrap(),
        "--dry-run",
        "--path",
        game.to_str().unwrap(),
    ]);
    if let Some(module) = module {
        cmd.env("MODN_GAME_MODULES_DIR", module);
    }
    cmd.output().unwrap()
}

#[expect(clippy::unwrap_used, reason = "Test output must be valid JSON")]
fn preview(game: &Path, archive: &Path, state: &Path, module: Option<&Path>) -> Value {
    let output = run(game, archive, state, module);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[expect(clippy::unwrap_used, reason = "Fixture JSON conversion must succeed")]
fn single_operation(destination: &str) -> String {
    format!(
        "return {{ schema_version = 1, operations = {{{{ source = files[1].source, destination = {}, kind = 'legacy-archive', package_root = '' }}}}, blockers = {{}} }}",
        serde_json::to_string(destination).unwrap()
    )
}

#[test]
fn hostile_destinations_are_blocked_without_writes() {
    let (temp, game, archive, state) = fixture(&["archive/pc/mod/good.archive"]);
    for (destination, expected) in [
        ("../outside.archive", "INVALID_DESTINATION"),
        ("/absolute/mod.archive", "INVALID_DESTINATION"),
        ("C:/windows/file.archive", "INVALID_DESTINATION"),
        ("archive/pc/mod/CON.archive", "INVALID_DESTINATION"),
        ("r6/scripts/startup.cmd", "EXECUTABLE_PAYLOAD"),
        ("r6/scripts/foo/../bar", "INVALID_DESTINATION"),
    ] {
        let module = module_with(temp.path(), &single_operation(destination));
        let result = preview(&game, &archive, &state, Some(&module));
        assert_eq!(result["status"], "blocked", "{destination}: {result}");
        assert!(
            result["blockers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["code"] == expected),
            "{destination}: {result}"
        );
        assert!(!game.join("archive").exists());
    }
    assert_eq!(
        fs::read(game.join("bin/x64/Cyberpunk2077.exe")).unwrap(),
        b"game"
    );
    assert!(!temp.path().join("outside.archive").exists());
}

#[test]
fn duplicate_overlapping_and_multiple_package_roots_are_blocked() {
    let (temp, game, archive, state) = fixture(&[
        "One/archive/pc/mod/a.archive",
        "Two/archive/pc/mod/b.archive",
    ]);
    let proposals = [
        (
            "return { schema_version = 1, operations = { { source = files[1].source, destination = 'r6/scripts/one.reds', kind = 'redscript', package_root = '' }, { source = files[2].source, destination = 'r6/scripts/one.reds', kind = 'redscript', package_root = '' } }, blockers = {} }",
            "DUPLICATE_DESTINATION",
        ),
        (
            "return { schema_version = 1, operations = { { source = files[1].source, destination = 'r6/scripts/data', kind = 'redscript', package_root = '' }, { source = files[2].source, destination = 'r6/scripts/data/more', kind = 'redscript', package_root = '' } }, blockers = {} }",
            "FILE_DIRECTORY_COLLISION",
        ),
        (
            "return { schema_version = 1, operations = { { source = files[1].source, destination = 'r6/scripts/one.reds', kind = 'redscript', package_root = 'One' }, { source = files[2].source, destination = 'r6/scripts/two.reds', kind = 'redscript', package_root = 'Two' } }, blockers = {} }",
            "MULTIPLE_PACKAGE_ROOTS",
        ),
    ];
    for (implementation, code) in proposals {
        let module = module_with(temp.path(), implementation);
        let result = preview(&game, &archive, &state, Some(&module));
        assert_eq!(result["status"], "blocked", "{result}");
        assert!(
            result["blockers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["code"] == code),
            "{result}"
        );
    }
    assert!(!game.join("r6").exists());
}

#[test]
fn unknown_sources_and_incompatible_schema_fail_closed() {
    let (temp, game, archive, state) = fixture(&["archive/pc/mod/a.archive"]);
    let module = module_with(temp.path(),
        "return { schema_version = 1, operations = { { source = 'missing.bin', destination = 'archive/pc/mod/a.archive', kind = 'legacy-archive', package_root = '' } }, blockers = {} }");
    let result = preview(&game, &archive, &state, Some(&module));
    assert_eq!(result["status"], "blocked");
    assert!(result["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b["code"] == "UNKNOWN_SOURCE"));

    let module = module_with(
        temp.path(),
        "return { schema_version = 2, operations = {}, blockers = {} }",
    );
    let output = run(&game, &archive, &state, Some(&module));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("unsupported installation plan schema version"));
    assert!(!game.join("archive").exists());
}

#[test]
fn occupied_target_is_blocked_even_if_module_requests_it() {
    let (temp, game, archive, state) = fixture(&["archive/pc/mod/a.archive"]);
    let destination = game.join("archive/pc/mod/a.archive");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::write(&destination, b"unmanaged").unwrap();
    let module = module_with(temp.path(), &single_operation("archive/pc/mod/a.archive"));
    let report = preview(&game, &archive, &state, Some(&module));
    assert_eq!(report["status"], "blocked");
    assert!(report["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b["code"] == "EXISTING_FILE"));
    assert_eq!(fs::read(&destination).unwrap(), b"unmanaged");
}

#[test]
fn oversized_plan_is_blocked_before_luau_execution() {
    let names: Vec<String> = (0..2_049)
        .map(|i| format!("r6/scripts/fixture/{i:05}.reds"))
        .collect();
    let entries: Vec<&str> = names.iter().map(String::as_str).collect();
    let (_temp, game, archive, state) = fixture(&entries);
    let report = preview(&game, &archive, &state, None);
    assert_eq!(report["status"], "blocked", "{report}");
    assert!(
        report["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["code"] == "TOO_MANY_FILES"),
        "{report}"
    );
    assert!(!game.join("r6").exists());
}

#[test]
fn changed_module_mapping_is_the_actual_install_mapping() {
    let (temp, game, archive, state) = fixture(&["archive/pc/mod/a.archive"]);
    let module = module_with(temp.path(), &single_operation("r6/scripts/different.reds"));
    let report = preview(&game, &archive, &state, Some(&module));
    assert_eq!(report["status"], "reviewable", "{report}");
    assert_eq!(
        report["operations"][0]["destination"],
        "r6/scripts/different.reds"
    );
    assert!(!game.join("r6").exists());
    assert!(!state.exists());
    let output = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--game", "cyberpunk-2077"])
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "install",
            archive.to_str().unwrap(),
            "--path",
            game.to_str().unwrap(),
            "--yes",
        ])
        .env("MODN_GAME_MODULES_DIR", module)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read(game.join("r6/scripts/different.reds")).unwrap(),
        b"payload"
    );
    assert!(!game.join("archive/pc/mod/a.archive").exists());
}

#[test]
fn preview_blocks_non_installable_formats_and_layouts() {
    let (temp, game, archive, state) = fixture(&["r6/scripts/hello.reds"]);
    let report = preview(&game, &archive, &state, None);
    assert_eq!(report["status"], "blocked", "{report}");
    assert!(
        report["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["code"] == "REDSCRIPT_NOT_INSTALLED"),
        "{report}"
    );
    let jar = temp.path().join("payload.jar");
    fs::copy(&archive, &jar).unwrap();
    let report = preview(&game, &jar, &state, None);
    assert_eq!(report["status"], "blocked");
    assert!(
        report["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["code"] == "NOT_INSTALLABLE"),
        "{report}"
    );
    assert!(!state.exists());
    assert!(!game.join("r6").exists());
}

#[test]
fn repeated_shared_luau_strings_respect_host_allocation_budget() {
    let (temp, game, archive, state) = fixture(&["archive/pc/mod/a.archive"]);
    let module = module_with(temp.path(),
        "local source = string.rep('a', 4096)\nlocal target = string.rep('b', 1024)\nlocal root = string.rep('c', 256)\nlocal ops = {}\nfor i = 1, 2048 do\n  ops[i] = { source = source, destination = target, kind = 'legacy-archive', package_root = root }\nend\nreturn { schema_version = 1, operations = ops, blockers = {} }");
    let output = run(&game, &archive, &state, Some(&module));
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("installation plan result exceeds 8 MiB"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!state.exists());
    assert!(!game.join("archive").exists());
}

#[test]
fn oversized_luau_blocker_messages_are_denied_before_copying() {
    let (temp, game, archive, state) = fixture(&["archive/pc/mod/a.archive"]);
    let module = module_with(temp.path(),
        "return { schema_version = 1, operations = {}, blockers = { { code = 'ERROR', message = string.rep('x', 1048576) } } }");
    let output = run(&game, &archive, &state, Some(&module));
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("installation field message exceeds 512"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!state.exists());
}

#[test]
fn approved_install_rejects_untrusted_luau_mappings_without_game_writes() {
    let (temp, game, archive, state) = fixture(&["archive/pc/mod/good.archive"]);
    for implementation in [
        single_operation("../outside/escape.archive"),
        single_operation("r6/scripts/install.exe"),
        "return { schema_version = 1, operations = { { source = 'missing', destination = 'archive/pc/mod/good.archive', kind = 'legacy-archive', package_root = '' } }, blockers = {} }".into(),
        "return { schema_version = 2, operations = {}, blockers = {} }".into(),
        "return { schema_version = 1, operations = {}, blockers = { { code = 'BLOCKED', message = 'not approved' } } }".into(),
    ] {
        let module = module_with(temp.path(), &implementation);
        let output = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--game", "cyberpunk-2077"])
            .args([
                "--state-dir", state.to_str().unwrap(),
                "install", archive.to_str().unwrap(), "--path", game.to_str().unwrap(),
                "--yes",
            ])
            .env("MODN_GAME_MODULES_DIR", module)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{implementation}");
        assert!(!game.join("archive").exists());
        assert!(!game.join("r6").exists());
        assert!(!temp.path().join("outside").exists());
    }
}

#[test]
fn bundled_luau_policy_rejects_expanded_layouts_until_enabled() {
    let (_temp, game, archive, state) = fixture(&["r6/scripts/hello.reds"]);
    let output = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--game", "cyberpunk-2077"])
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "install",
            archive.to_str().unwrap(),
            "--path",
            game.to_str().unwrap(),
            "--yes",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!game.join("r6").exists());
}

#[test]
fn luau_mapped_install_can_be_uninstalled_without_original_mapping() {
    let (temp, game, archive, state) = fixture(&["archive/pc/mod/a.archive"]);
    let module = module_with(temp.path(), &single_operation("r6/scripts/new.reds"));
    let install = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--game", "cyberpunk-2077"])
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "--json",
            "install",
            archive.to_str().unwrap(),
            "--path",
            game.to_str().unwrap(),
            "--yes",
        ])
        .env("MODN_GAME_MODULES_DIR", module)
        .output()
        .unwrap();
    assert!(
        install.status.success(),
        "{}",
        String::from_utf8_lossy(&install.stderr)
    );
    let package: Value = serde_json::from_slice(&install.stdout).unwrap();
    assert_eq!(
        fs::read(game.join("r6/scripts/new.reds")).unwrap(),
        b"payload"
    );
    let remove = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--game", "cyberpunk-2077"])
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "uninstall",
            package["id"].as_str().unwrap(),
            "--path",
            game.to_str().unwrap(),
            "--yes",
        ])
        .output()
        .unwrap();
    assert!(
        remove.status.success(),
        "{}",
        String::from_utf8_lossy(&remove.stderr)
    );
    assert!(!game.join("r6/scripts/new.reds").exists());
}
