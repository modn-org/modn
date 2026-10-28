use mod_core::{
    inventory::{doctor_with_ownership, scan_with_ownership},
    state::Store,
};
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

fn fixture() -> (TempDir, Store, PathBuf, PathBuf) {
    let root = tempdir().unwrap();
    let game = root.path().join("game");
    fs::create_dir_all(game.join("bin/x64")).unwrap();
    fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"fixture").unwrap();
    let state = Store::open(&root.path().join("state/state.sqlite")).unwrap();
    let archive = root.path().join("mod.zip");
    let mut writer = zip::ZipWriter::new(File::create(&archive).unwrap());
    writer
        .start_file(
            "archive/pc/mod/managed.archive",
            SimpleFileOptions::default(),
        )
        .unwrap();
    writer.write_all(b"expected").unwrap();
    writer.finish().unwrap();
    (root, state, game, archive)
}

fn owned<'a>(scan: &'a mod_core::scan::ScanReport, name: &str) -> &'a str {
    scan.owned_files
        .iter()
        .find(|f| f.relative_path == name)
        .unwrap()
        .status
        .as_str()
}

#[test]
fn discovers_unmanaged_frameworks_and_owned_legacy_archives() {
    let (_root, mut store, game, archive) = fixture();
    let installed = store.install_zip(&game, &archive).unwrap();
    fs::write(game.join("archive/pc/mod/unmanaged.archive"), b"manual").unwrap();
    fs::create_dir_all(game.join("red4ext/plugins/SomePlugin")).unwrap();
    fs::create_dir_all(game.join("bin/x64/plugins/cyber_engine_tweaks/mods/nativeSettings"))
        .unwrap();
    fs::create_dir_all(game.join("r6/scripts")).unwrap();
    fs::write(game.join("r6/scripts/manual.reds"), b"manual").unwrap();
    fs::create_dir_all(game.join("red4ext/plugins/ArchiveXL")).unwrap();
    fs::write(
        game.join("red4ext/plugins/ArchiveXL/ArchiveXL.dll"),
        b"fake",
    )
    .unwrap();

    let report = scan_with_ownership(&store, &game).unwrap();
    assert_eq!(report.owned_files.len(), 1);
    assert_eq!(owned(&report, "archive/pc/mod/managed.archive"), "intact");
    let managed = report
        .mods
        .iter()
        .find(|m| m.relative_path == Path::new("archive/pc/mod/managed.archive"))
        .unwrap();
    assert!(managed.managed);
    assert_eq!(managed.package_id.as_deref(), Some(installed.id.as_str()));
    assert_eq!(managed.integrity.as_deref(), Some("intact"));
    assert!(
        !report
            .mods
            .iter()
            .find(|m| m.relative_path == Path::new("archive/pc/mod/unmanaged.archive"))
            .unwrap()
            .managed
    );
    assert!(report
        .mods
        .iter()
        .any(|m| m.kind == "cet-mod" && !m.managed));
    assert!(report
        .mods
        .iter()
        .any(|m| m.kind == "red4ext-plugin" && !m.managed));
    assert!(report
        .mods
        .iter()
        .any(|m| m.kind == "redscript" && !m.managed));
    assert!(report
        .framework_signals
        .iter()
        .any(|f| f.name == "ArchiveXL" && f.kind == "file"));
    assert!(report
        .framework_signals
        .iter()
        .any(|f| f.name == "CET" && f.kind == "directory"));
    let doctor = doctor_with_ownership(&store, &game).unwrap();
    assert!(doctor
        .diagnostics
        .iter()
        .any(|d| d.code == "UNMANAGED_CONTENT"));
    assert!(!doctor
        .diagnostics
        .iter()
        .any(|d| d.code == "OWNED_FILE_MODIFIED"));
    assert_eq!(
        fs::read(game.join("archive/pc/mod/managed.archive")).unwrap(),
        b"expected"
    );
}

#[test]
fn reports_modified_missing_and_restored_files_without_changing_them() {
    let (_root, mut store, game, archive) = fixture();
    store.install_zip(&game, &archive).unwrap();
    let file = game.join("archive/pc/mod/managed.archive");
    fs::write(&file, b"changed!").unwrap(); // same byte length tests actual SHA-256, not only size.
    let report = scan_with_ownership(&store, &game).unwrap();
    assert_eq!(owned(&report, "archive/pc/mod/managed.archive"), "modified");
    assert!(report
        .mods
        .iter()
        .any(|m| m.managed && m.integrity.as_deref() == Some("modified")));
    assert!(doctor_with_ownership(&store, &game)
        .unwrap()
        .diagnostics
        .iter()
        .any(|d| d.code == "OWNED_FILE_MODIFIED"));
    assert_eq!(fs::read(&file).unwrap(), b"changed!");

    fs::remove_file(&file).unwrap();
    let report = scan_with_ownership(&store, &game).unwrap();
    assert_eq!(owned(&report, "archive/pc/mod/managed.archive"), "missing");
    assert!(doctor_with_ownership(&store, &game)
        .unwrap()
        .diagnostics
        .iter()
        .any(|d| d.code == "OWNED_FILE_MISSING"));
    assert!(!file.exists());

    fs::write(&file, b"expected").unwrap();
    assert_eq!(
        owned(
            &scan_with_ownership(&store, &game).unwrap(),
            "archive/pc/mod/managed.archive"
        ),
        "intact"
    );
}

#[test]
fn keeps_inventory_separate_across_game_installations() {
    let (root, mut store, game, archive) = fixture();
    store.install_zip(&game, &archive).unwrap();
    let another = root.path().join("other-game");
    fs::create_dir_all(another.join("bin/x64")).unwrap();
    fs::write(another.join("bin/x64/Cyberpunk2077.exe"), b"game").unwrap();
    fs::create_dir_all(another.join("archive/pc/mod")).unwrap();
    fs::write(another.join("archive/pc/mod/managed.archive"), b"expected").unwrap();
    let result = scan_with_ownership(&store, &another).unwrap();
    assert!(result.owned_files.is_empty());
    assert_eq!(result.mods.len(), 1);
    assert!(!result.mods[0].managed);
}

#[test]
fn redirects_away_from_game_are_never_marked_intact() {
    let (root, mut store, game, archive) = fixture();
    store.install_zip(&game, &archive).unwrap();
    let folder = game.join("archive").join("pc").join("mod");
    let outside = root.path().join("outside");
    fs::rename(&folder, &outside).unwrap();
    #[cfg(windows)]
    {
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&folder)
            .arg(&outside)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &folder).unwrap();
    let report = scan_with_ownership(&store, &game).unwrap();
    assert_eq!(owned(&report, "archive/pc/mod/managed.archive"), "unsafe");
    assert!(report.mods.iter().all(|m| !m.managed));
    assert!(doctor_with_ownership(&store, &game)
        .unwrap()
        .diagnostics
        .iter()
        .any(|d| d.code == "OWNED_FILE_UNSAFE"));
    assert_eq!(
        fs::read(outside.join("managed.archive")).unwrap(),
        b"expected"
    );
}
#[cfg(windows)]
#[test]
fn locked_managed_file_is_unreadable_not_unmanaged() {
    use std::os::windows::fs::OpenOptionsExt;
    let (_root, mut store, game, archive) = fixture();
    store.install_zip(&game, &archive).unwrap();
    let target = game.join("archive/pc/mod/managed.archive");
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&target)
        .unwrap();
    let report = scan_with_ownership(&store, &game).unwrap();
    assert_eq!(
        owned(&report, "archive/pc/mod/managed.archive"),
        "unreadable"
    );
    assert!(doctor_with_ownership(&store, &game)
        .unwrap()
        .diagnostics
        .iter()
        .any(|d| d.code == "OWNED_FILE_UNREADABLE"));
    drop(lock);
    assert_eq!(
        owned(
            &scan_with_ownership(&store, &game).unwrap(),
            "archive/pc/mod/managed.archive"
        ),
        "intact"
    );
}

#[test]
fn unicode_casefold_collision_does_not_adopt_unmanaged_file() {
    let (root, mut store, game, _archive) = fixture();
    let zip_path = root.path().join("kelvin.zip");
    let mut writer = zip::ZipWriter::new(File::create(&zip_path).unwrap());
    writer
        .start_file("archive/pc/mod/k.archive", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"owned contents").unwrap();
    writer.finish().unwrap();
    let package = store.install_zip(&game, &zip_path).unwrap();

    let lookalike = game.join("archive/pc/mod/\u{212A}.archive");
    // Skip on a filesystem that really aliases these names: never overwrite a fixture's owned file.
    if lookalike.exists() {
        return;
    }
    fs::write(&lookalike, b"manual content").unwrap();
    let report = scan_with_ownership(&store, &game).unwrap();
    let recorded = report
        .mods
        .iter()
        .find(|m| m.relative_path == Path::new("archive/pc/mod/k.archive"))
        .unwrap();
    assert!(recorded.managed);
    assert_eq!(recorded.package_id.as_deref(), Some(package.id.as_str()));
    let unowned = report
        .mods
        .iter()
        .find(|m| m.relative_path == Path::new("archive/pc/mod/\u{212A}.archive"))
        .unwrap();
    assert!(!unowned.managed);
    assert!(unowned.package_id.is_none());
    assert!(doctor_with_ownership(&store, &game)
        .unwrap()
        .diagnostics
        .iter()
        .any(|d| d.code == "UNMANAGED_CONTENT"));
    assert_eq!(fs::read(&lookalike).unwrap(), b"manual content");
}

#[cfg(not(windows))]
#[test]
fn ordinary_case_sensitive_file_names_remain_distinct() {
    let (root, mut store, game, _archive) = fixture();
    let zip_path = root.path().join("case.zip");
    let mut writer = zip::ZipWriter::new(File::create(&zip_path).unwrap());
    writer
        .start_file("archive/pc/mod/k.archive", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"owned").unwrap();
    writer.finish().unwrap();
    store.install_zip(&game, &zip_path).unwrap();
    fs::write(game.join("archive/pc/mod/K.archive"), b"unmanaged").unwrap();
    let report = scan_with_ownership(&store, &game).unwrap();
    assert!(report
        .mods
        .iter()
        .any(|m| m.relative_path == Path::new("archive/pc/mod/k.archive") && m.managed));
    assert!(report
        .mods
        .iter()
        .any(|m| m.relative_path == Path::new("archive/pc/mod/K.archive") && !m.managed));
}

#[cfg(windows)]
#[test]
fn windows_non_ascii_case_only_rename_stays_managed() {
    let (root, mut store, game, _archive) = fixture();
    let zip_path = root.path().join("unicode-case.zip");
    let mut writer = zip::ZipWriter::new(File::create(&zip_path).unwrap());
    writer
        .start_file(
            "archive/pc/mod/\u{00C9}.archive",
            SimpleFileOptions::default(),
        )
        .unwrap();
    writer.write_all(b"owned").unwrap();
    writer.finish().unwrap();
    let package = store.install_zip(&game, &zip_path).unwrap();
    let original = game.join("archive/pc/mod/\u{00C9}.archive");
    let renamed = game.join("archive/pc/mod/\u{00E9}.archive");
    fs::rename(&original, &renamed).unwrap();
    let detected = scan_with_ownership(&store, &game).unwrap();
    assert_eq!(detected.owned_files[0].status, "intact");
    let entry = detected
        .mods
        .iter()
        .find(|m| m.relative_path == Path::new("archive/pc/mod/\u{00E9}.archive"))
        .unwrap();
    assert!(entry.managed);
    assert_eq!(entry.package_id.as_deref(), Some(package.id.as_str()));
    assert!(!doctor_with_ownership(&store, &game)
        .unwrap()
        .diagnostics
        .iter()
        .any(|d| d.code == "UNMANAGED_CONTENT"));
}

#[test]
fn different_hardlink_alias_does_not_gain_ownership() {
    let (root, mut store, game, _archive) = fixture();
    let zip_path = root.path().join("hardlink.zip");
    let mut writer = zip::ZipWriter::new(File::create(&zip_path).unwrap());
    writer
        .start_file("archive/pc/mod/k.archive", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"owned").unwrap();
    writer.finish().unwrap();
    store.install_zip(&game, &zip_path).unwrap();
    let original = game.join("archive/pc/mod/k.archive");
    let alias = game.join("archive/pc/mod/\u{212A}.archive");
    if alias.exists() {
        return;
    }
    fs::hard_link(&original, &alias).unwrap();
    let report = scan_with_ownership(&store, &game).unwrap();
    assert_eq!(report.mods.len(), 2);
    assert!(report
        .mods
        .iter()
        .any(|m| m.relative_path == Path::new("archive/pc/mod/k.archive") && m.managed));
    assert!(report
        .mods
        .iter()
        .any(|m| m.relative_path == Path::new("archive/pc/mod/\u{212A}.archive") && !m.managed));
}

#[cfg(windows)]
#[test]
fn windows_greek_sigma_contextual_case_rename_stays_managed() {
    let (root, mut store, game, _archive) = fixture();
    let zip_path = root.path().join("greek-sigma.zip");
    let mut writer = zip::ZipWriter::new(File::create(&zip_path).unwrap());
    writer
        .start_file(
            "archive/pc/mod/\u{039F}\u{03A3}1.archive",
            SimpleFileOptions::default(),
        )
        .unwrap();
    writer.write_all(b"owned").unwrap();
    writer.finish().unwrap();
    let package = store.install_zip(&game, &zip_path).unwrap();
    let original = game.join("archive/pc/mod/\u{039F}\u{03A3}1.archive");
    let renamed = game.join("archive/pc/mod/\u{03BF}\u{03C3}1.archive");
    fs::rename(&original, &renamed).unwrap();
    assert!(same_file::is_same_file(&original, &renamed).unwrap());
    let report = scan_with_ownership(&store, &game).unwrap();
    assert_eq!(report.owned_files[0].status, "intact");
    let detected = report
        .mods
        .iter()
        .find(|m| m.relative_path == Path::new("archive/pc/mod/\u{03BF}\u{03C3}1.archive"))
        .unwrap();
    assert!(detected.managed);
    assert_eq!(detected.package_id.as_deref(), Some(package.id.as_str()));
    assert!(!doctor_with_ownership(&store, &game)
        .unwrap()
        .diagnostics
        .iter()
        .any(|d| d.code == "UNMANAGED_CONTENT"));
}
