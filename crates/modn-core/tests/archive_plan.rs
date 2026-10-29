//! Integration checks for archive plan behavior.
use modn_core::archive::inspect;
use modn_core::planning::resolve;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use tempfile::tempdir;
use zip::write::SimpleFileOptions;

#[expect(clippy::unwrap_used, reason = "Fixture setup may fail fast in tests")]
fn zip_fixture(path: &Path, entries: &[(&str, &[u8])]) {
    let mut writer = zip::ZipWriter::new(File::create(path).unwrap());
    for (name, data) in entries {
        writer
            .start_file(*name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap();
}

#[expect(clippy::unwrap_used, reason = "Fixture setup may fail fast in tests")]
fn setup_game(root: &Path, redmod: bool) {
    fs::create_dir_all(root.join("bin/x64")).unwrap();
    fs::write(root.join("bin/x64/Cyberpunk2077.exe"), "game").unwrap();
    if redmod {
        fs::create_dir_all(root.join("tools/redmod/bin")).unwrap();
        fs::write(root.join("tools/redmod/bin/redMod.exe"), "tool").unwrap();
    }
}

#[expect(clippy::unwrap_used, reason = "Disposable fixture plan resolution")]
fn plan(
    inspection: &modn_core::archive::Inspection,
    game: &Path,
) -> modn_core::planning::InstallPlan {
    resolve("cyberpunk-2077", inspection, game).unwrap()
}

#[test]
fn zip_inspection_and_legacy_plan_are_read_only() {
    let dir = tempdir().unwrap();
    let game = dir.path().join("game");
    setup_game(&game, false);
    let archive = dir.path().join("mod.zip");
    zip_fixture(
        &archive,
        &[
            ("MyMod/archive/pc/mod/weather.archive", b"archive"),
            ("MyMod/README.md", b"documentation"),
        ],
    );
    let inspection = inspect(&archive).unwrap();
    assert_eq!(inspection.format, "zip");
    assert_eq!(inspection.sha256.len(), 64);
    assert_eq!(inspection.files.len(), 2);
    let report = plan(&inspection, &game);
    assert_eq!(report.status, "reviewable", "{:?}", report.blockers);
    assert_eq!(report.operations.len(), 1);
    assert_eq!(
        report.operations[0].destination,
        "archive/pc/mod/weather.archive"
    );
    assert!(!game.join("archive").exists());
    assert_eq!(
        fs::read(game.join("bin/x64/Cyberpunk2077.exe")).unwrap(),
        b"game"
    );
}

#[test]
fn unmanaged_file_and_linked_directory_block_installation_plan() {
    let dir = tempdir().unwrap();
    let game = dir.path().join("game");
    setup_game(&game, false);
    let archive = dir.path().join("mod.jar");
    zip_fixture(&archive, &[("archive/pc/mod/example.archive", b"new")]);
    let inspection = inspect(&archive).unwrap();
    let target = game.join("archive/pc/mod/example.archive");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "existing").unwrap();
    let report = plan(&inspection, &game);
    assert_eq!(report.status, "blocked");
    assert!(report
        .blockers
        .iter()
        .any(|issue| issue.code == "EXISTING_FILE"));
    assert_eq!(fs::read(&target).unwrap(), b"existing");
}

#[test]
fn redmod_package_requires_info_and_tools() {
    let dir = tempdir().unwrap();
    let game = dir.path().join("game");
    setup_game(&game, false);
    let archive = dir.path().join("redmod.zip");
    zip_fixture(
        &archive,
        &[
            ("mods/Sample/info.json", b"{}"),
            ("mods/Sample/archives/sample.archive", b"data"),
        ],
    );
    let inspection = inspect(&archive).unwrap();
    assert!(plan(&inspection, &game)
        .blockers
        .iter()
        .any(|issue| issue.code == "REDMOD_NOT_INSTALLED"));
    setup_game(&game, true);
    assert!(plan(&inspection, &game)
        .blockers
        .iter()
        .any(|issue| issue.code == "NOT_INSTALLABLE"));

    let missing = dir.path().join("missing-info.zip");
    zip_fixture(&missing, &[("mods/Sample/archives/test.archive", b"data")]);
    let report = plan(&inspect(&missing).unwrap(), &game);
    assert!(report
        .blockers
        .iter()
        .any(|issue| issue.code == "REDMOD_INFO_MISSING"));
}

#[test]
fn unsafe_paths_duplicates_and_unknown_layouts_are_refused() {
    let dir = tempdir().unwrap();
    let archive = dir.path().join("unsafe.zip");
    zip_fixture(&archive, &[("../escape.archive", b"x")]);
    assert!(inspect(&archive).is_err());

    let archive = dir.path().join("duplicate.zip");
    zip_fixture(
        &archive,
        &[
            ("archive/pc/mod/Same.archive", b"x"),
            ("archive/pc/mod/same.archive", b"y"),
        ],
    );
    assert!(inspect(&archive).is_err());

    let game = dir.path().join("game");
    setup_game(&game, true);
    let archive = dir.path().join("unknown.zip");
    zip_fixture(&archive, &[("random/location/data.bin", b"x")]);
    let report = plan(&inspect(&archive).unwrap(), &game);
    assert_eq!(report.status, "blocked");
    assert!(report
        .blockers
        .iter()
        .any(|issue| issue.code == "UNRECOGNIZED_LAYOUT"));

    let archive = dir.path().join("installer.zip");
    zip_fixture(&archive, &[("mods/Foo/setup.exe", b"exe")]);
    let report = plan(&inspect(&archive).unwrap(), &game);
    assert!(report
        .blockers
        .iter()
        .any(|issue| issue.code == "EXECUTABLE_PAYLOAD"));
}

#[test]
fn mixed_package_variants_are_blocked() {
    let dir = tempdir().unwrap();
    let game = dir.path().join("game");
    setup_game(&game, false);
    let archive = dir.path().join("variants.zip");
    zip_fixture(
        &archive,
        &[
            ("Balanced/archive/pc/mod/quality.archive", b"balanced"),
            ("Ultra/archive/pc/mod/quality.archive", b"ultra"),
        ],
    );
    let result = plan(&inspect(&archive).unwrap(), &game);
    assert_eq!(result.status, "blocked");
    assert!(result
        .blockers
        .iter()
        .any(|issue| issue.code == "DUPLICATE_DESTINATION"));
}

#[test]
fn sevenz_headers_can_be_inspected_without_extraction() {
    let dir = tempdir().unwrap();
    let input = dir.path().join("package");
    fs::create_dir_all(input.join("archive/pc/mod")).unwrap();
    fs::write(input.join("archive/pc/mod/example.archive"), "fixture").unwrap();
    let archive = dir.path().join("sample.7z");
    let mut writer = sevenz_rust2::ArchiveWriter::create(&archive).unwrap();
    writer
        .push_archive_entry(
            sevenz_rust2::ArchiveEntry::new_file("package/archive/pc/mod/example.archive"),
            Some(File::open(input.join("archive/pc/mod/example.archive")).unwrap()),
        )
        .unwrap();
    writer.finish().unwrap();
    let report = inspect(&archive).unwrap();
    assert_eq!(report.format, "7z");
    assert!(report
        .files
        .iter()
        .any(|file| file.entry.ends_with("example.archive")));
    assert!(!dir.path().join("game").exists());
}

#[test]
fn mixed_framework_layouts_are_plannable() {
    let dir = tempdir().unwrap();
    let game_dir = dir.path().join("game");
    setup_game(&game_dir, false);
    let archive = dir.path().join("mixed.zip");
    zip_fixture(
        &archive,
        &[
            ("Pack/r6/scripts/hello.reds", b"script"),
            ("Pack/r6/tweaks/settings.yaml", b"tweak"),
            (
                "Pack/bin/x64/plugins/cyber_engine_tweaks/mods/AMM/init.lua",
                b"lua",
            ),
            ("Pack/red4ext/plugins/Codeware/entry.dll", b"dll"),
        ],
    );
    let report = plan(&inspect(&archive).unwrap(), &game_dir);
    assert_eq!(report.status, "blocked", "{:?}", report.blockers);
    assert_eq!(report.operations.len(), 4);
    assert!(report
        .blockers
        .iter()
        .any(|issue| issue.code == "REDSCRIPT_NOT_INSTALLED"));
    assert!(!game_dir.join("r6").exists());
}

#[test]
fn directory_replaced_by_file_blocks_plan() {
    let dir = tempdir().unwrap();
    let game_dir = dir.path().join("game");
    setup_game(&game_dir, false);
    fs::create_dir_all(game_dir.join("archive")).unwrap();
    fs::write(game_dir.join("archive/pc"), b"not a directory").unwrap();
    let archive = dir.path().join("mod.zip");
    zip_fixture(&archive, &[("archive/pc/mod/test.archive", b"x")]);
    let report = plan(&inspect(&archive).unwrap(), &game_dir);
    assert_eq!(report.status, "blocked");
    assert!(report
        .blockers
        .iter()
        .any(|issue| issue.code == "NON_DIRECTORY_ANCESTOR"));
}

#[test]
fn executable_disguised_as_documentation_is_not_ignored() {
    let dir = tempdir().unwrap();
    let game_dir = dir.path().join("game");
    setup_game(&game_dir, false);
    let archive = dir.path().join("mod.zip");
    zip_fixture(
        &archive,
        &[
            ("archive/pc/mod/test.archive", b"x"),
            ("README.exe", b"malicious"),
        ],
    );
    let report = plan(&inspect(&archive).unwrap(), &game_dir);
    assert_eq!(report.status, "blocked");
    assert!(report
        .blockers
        .iter()
        .any(|issue| issue.code == "EXECUTABLE_PAYLOAD"));
}

#[test]
fn identical_zip_record_names_are_refused() {
    let dir = tempdir().unwrap();
    let archive = dir.path().join("duplicate-raw.zip");
    zip_fixture(
        &archive,
        &[
            ("archive/pc/mod/Same.archive", b"first"),
            ("archive/pc/mod/same.archive", b"second"),
        ],
    );
    // ZipWriter disallows identical names. Rewrite both local and central names.
    let mut bytes = fs::read(&archive).unwrap();
    let from = b"archive/pc/mod/same.archive";
    let to = b"archive/pc/mod/Same.archive";
    let mut replacements = 0;
    for i in 0..=bytes.len() - from.len() {
        if &bytes[i..i + from.len()] == from {
            bytes[i..i + from.len()].copy_from_slice(to);
            replacements += 1;
        }
    }
    assert_eq!(replacements, 2);
    fs::write(&archive, bytes).unwrap();
    assert!(inspect(&archive).is_err());
}

#[test]
fn documentation_prefix_does_not_hide_payloads() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("game");
    setup_game(&root, false);
    let archive = dir.path().join("readme.zip");
    zip_fixture(
        &archive,
        &[
            ("archive/pc/mod/readme.archive", b"a"),
            ("r6/scripts/READMEHelper.reds", b"b"),
            ("README.md", b"documentation"),
        ],
    );
    let report = plan(&inspect(&archive).unwrap(), &root);
    assert_eq!(report.status, "blocked", "{:?}", report.blockers);
    assert!(report
        .blockers
        .iter()
        .any(|issue| issue.code == "REDSCRIPT_NOT_INSTALLED"));
    assert_eq!(report.operations.len(), 2);
}

#[test]
fn conflicts_between_planned_file_and_directory_are_blocked() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("game");
    setup_game(&root, false);
    let archive = dir.path().join("nested.zip");
    zip_fixture(
        &archive,
        &[
            ("r6/scripts/Foo", b"file"),
            ("r6/scripts/Foo/hello.reds", b"script"),
        ],
    );
    let report = plan(&inspect(&archive).unwrap(), &root);
    assert_eq!(report.status, "blocked");
    assert!(report
        .blockers
        .iter()
        .any(|issue| issue.code == "FILE_DIRECTORY_COLLISION"));
}

#[test]
fn redmod_manifest_must_be_at_package_root_case_insensitively() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("game");
    setup_game(&root, true);
    let archive = dir.path().join("nested-info.zip");
    zip_fixture(
        &archive,
        &[
            ("mods/Foo/assets/info.json", b"{}"),
            ("mods/Foo/archives/test.archive", b"test"),
        ],
    );
    let result = plan(&inspect(&archive).unwrap(), &root);
    assert!(result
        .blockers
        .iter()
        .any(|issue| issue.code == "REDMOD_INFO_MISSING"));

    let archive = dir.path().join("upper-info.zip");
    zip_fixture(&archive, &[("mods/Foo/INFO.JSON", b"{}")]);
    assert!(plan(&inspect(&archive).unwrap(), &root)
        .blockers
        .iter()
        .any(|issue| issue.code == "NOT_INSTALLABLE"));
}

#[test]
fn windows_invalid_filename_chars_are_refused() {
    let dir = tempdir().unwrap();
    let archive = dir.path().join("invalid.zip");
    zip_fixture(&archive, &[("archive/pc/mod/bad?.archive", b"bad")]);
    assert!(inspect(&archive).is_err());
}

#[test]
fn extensionless_readme_and_changelog_do_not_block_valid_package() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("game");
    setup_game(&root, false);
    let archive = dir.path().join("docs.zip");
    zip_fixture(
        &archive,
        &[
            ("archive/pc/mod/real.archive", b"mod"),
            ("README", b"read me"),
            ("CHANGELOG", b"history"),
        ],
    );
    let report = plan(&inspect(&archive).unwrap(), &root);
    assert_eq!(report.status, "reviewable", "{:?}", report.blockers);
    assert_eq!(report.operations.len(), 1);
}

#[test]
fn stale_archive_inspection_cannot_make_module_preview_reviewable() {
    let dir = tempdir().unwrap();
    let game = dir.path().join("game");
    setup_game(&game, false);
    let archive = dir.path().join("changing.zip");
    zip_fixture(&archive, &[("archive/pc/mod/example.archive", b"before")]);
    let stale = inspect(&archive).unwrap();
    zip_fixture(&archive, &[("archive/pc/mod/example.archive", b"after")]);
    let result = modn_core::planning::resolve("cyberpunk-2077", &stale, &game).unwrap();
    assert_eq!(result.status, "blocked");
    assert!(
        result
            .blockers
            .iter()
            .any(|issue| issue.code == "SOURCE_CHANGED"),
        "{:?}",
        result.blockers
    );
    assert!(!game.join("archive").exists());
}
