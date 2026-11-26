use super::*;

#[test]
fn unicode_casefold_collision_does_not_adopt_unmanaged_file() {
    main_bucket::seeded_roots(|roots| {
        let (root, mut store, game, _archive, roots) = fixture(roots.data());
        let zip_path = root.path().join("kelvin.zip");
        let mut writer = zip::ZipWriter::new(File::create(&zip_path).unwrap());
        writer
            .start_file("archive/pc/mod/k.archive", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"owned contents").unwrap();
        writer.finish().unwrap();
        let package = install_zip(&roots, &mut store, &game, &zip_path);

        let lookalike = game.join("archive/pc/mod/\u{212A}.archive");
        if lookalike.exists() {
            return;
        }
        fs::write(&lookalike, b"manual content").unwrap();
        let report = scan_with_ownership_for(&roots, &store, "cyberpunk-2077", &game).unwrap();
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
        assert!(
            doctor_with_ownership_for(&roots, &store, "cyberpunk-2077", &game)
                .unwrap()
                .diagnostics
                .iter()
                .any(|d| d.code == "UNMANAGED_CONTENT")
        );
        assert_eq!(fs::read(&lookalike).unwrap(), b"manual content");
    });
}

#[cfg(not(windows))]
#[test]
fn ordinary_case_sensitive_file_names_remain_distinct() {
    main_bucket::seeded_roots(|roots| {
        let (root, mut store, game, _archive, roots) = fixture(roots.data());
        let zip_path = root.path().join("case.zip");
        let mut writer = zip::ZipWriter::new(File::create(&zip_path).unwrap());
        writer
            .start_file("archive/pc/mod/k.archive", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"owned").unwrap();
        writer.finish().unwrap();
        install_zip(&roots, &mut store, &game, &zip_path);
        fs::write(game.join("archive/pc/mod/K.archive"), b"unmanaged").unwrap();
        let report = scan_with_ownership_for(&roots, &store, "cyberpunk-2077", &game).unwrap();
        assert!(report
            .mods
            .iter()
            .any(|m| m.relative_path == Path::new("archive/pc/mod/k.archive") && m.managed));
        assert!(report
            .mods
            .iter()
            .any(|m| m.relative_path == Path::new("archive/pc/mod/K.archive") && !m.managed));
    });
}

#[cfg(windows)]
#[test]
fn windows_non_ascii_case_only_rename_stays_managed() {
    main_bucket::seeded_roots(|roots| {
        let (root, mut store, game, _archive, roots) = fixture(roots.data());
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
        let package = install_zip(&roots, &mut store, &game, &zip_path);
        let original = game.join("archive/pc/mod/\u{00C9}.archive");
        let renamed = game.join("archive/pc/mod/\u{00E9}.archive");
        fs::rename(&original, &renamed).unwrap();
        let detected = scan_with_ownership_for(&roots, &store, "cyberpunk-2077", &game).unwrap();
        assert_eq!(detected.owned_files[0].status, IntegrityStatus::Intact);
        let entry = detected
            .mods
            .iter()
            .find(|m| m.relative_path == Path::new("archive/pc/mod/\u{00E9}.archive"))
            .unwrap();
        assert!(entry.managed);
        assert_eq!(entry.package_id.as_deref(), Some(package.id.as_str()));
        assert!(
            !doctor_with_ownership_for(&roots, &store, "cyberpunk-2077", &game)
                .unwrap()
                .diagnostics
                .iter()
                .any(|d| d.code == "UNMANAGED_CONTENT")
        );
    });
}

#[test]
fn different_hardlink_alias_does_not_gain_ownership() {
    main_bucket::seeded_roots(|roots| {
        let (root, mut store, game, _archive, roots) = fixture(roots.data());
        let zip_path = root.path().join("hardlink.zip");
        let mut writer = zip::ZipWriter::new(File::create(&zip_path).unwrap());
        writer
            .start_file("archive/pc/mod/k.archive", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"owned").unwrap();
        writer.finish().unwrap();
        install_zip(&roots, &mut store, &game, &zip_path);
        let original = game.join("archive/pc/mod/k.archive");
        let alias = game.join("archive/pc/mod/\u{212A}.archive");
        if alias.exists() {
            return;
        }
        fs::hard_link(&original, &alias).unwrap();
        let report = scan_with_ownership_for(&roots, &store, "cyberpunk-2077", &game).unwrap();
        assert_eq!(report.mods.len(), 2);
        assert!(report
            .mods
            .iter()
            .any(|m| m.relative_path == Path::new("archive/pc/mod/k.archive") && m.managed));
        assert!(report.mods.iter().any(|m| m.relative_path
            == Path::new("archive/pc/mod/\u{212A}.archive")
            && !m.managed));
    });
}

#[cfg(windows)]
#[test]
fn windows_greek_sigma_contextual_case_rename_stays_managed() {
    main_bucket::seeded_roots(|roots| {
        let (root, mut store, game, _archive, roots) = fixture(roots.data());
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
        let package = install_zip(&roots, &mut store, &game, &zip_path);
        let original = game.join("archive/pc/mod/\u{039F}\u{03A3}1.archive");
        let renamed = game.join("archive/pc/mod/\u{03BF}\u{03C3}1.archive");
        fs::rename(&original, &renamed).unwrap();
        assert!(same_file::is_same_file(&original, &renamed).unwrap());
        let report = scan_with_ownership_for(&roots, &store, "cyberpunk-2077", &game).unwrap();
        assert_eq!(report.owned_files[0].status, IntegrityStatus::Intact);
        let detected = report
            .mods
            .iter()
            .find(|m| m.relative_path == Path::new("archive/pc/mod/\u{03BF}\u{03C3}1.archive"))
            .unwrap();
        assert!(detected.managed);
        assert_eq!(detected.package_id.as_deref(), Some(package.id.as_str()));
        assert!(
            !doctor_with_ownership_for(&roots, &store, "cyberpunk-2077", &game)
                .unwrap()
                .diagnostics
                .iter()
                .any(|d| d.code == "UNMANAGED_CONTENT")
        );
    });
}

#[cfg(windows)]
#[test]
fn unicode_lookalike_directory_never_inherits_exclusive_claim() {
    main_bucket::seeded_roots(|roots| {
        let (root, mut store, game, _archive, roots) = fixture(roots.data());
        fs::create_dir_all(game.join("red4ext/plugins")).unwrap();
        let zip_path = root.path().join("directory-unicode.zip");
        let mut writer = zip::ZipWriter::new(File::create(&zip_path).unwrap());
        writer
            .start_file("red4ext/plugins/k/k.dll", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"owned plugin").unwrap();
        writer.finish().unwrap();
        let package = install_zip(&roots, &mut store, &game, &zip_path);
        assert_eq!(package.directories, ["red4ext/plugins/k"]);
        let lookalike_dir = game.join("red4ext/plugins/\u{212A}");
        if lookalike_dir.exists() {
            // NTFS volumes may actually alias these names; do not modify the original.
            return;
        }
        fs::create_dir(&lookalike_dir).unwrap();
        fs::write(lookalike_dir.join("manual.dll"), b"manual").unwrap();
        let report = scan_with_ownership_for(&roots, &store, "cyberpunk-2077", &game).unwrap();
        let original = report
            .mods
            .iter()
            .find(|mod_file| mod_file.relative_path == Path::new("red4ext/plugins/k/k.dll"))
            .unwrap();
        assert!(original.managed);
        let unrelated = report
            .mods
            .iter()
            .find(|mod_file| {
                mod_file.relative_path == Path::new("red4ext/plugins/\u{212A}/manual.dll")
            })
            .unwrap();
        assert!(!unrelated.managed, "{report:?}");
        assert!(unrelated.package_id.is_none());
        store.uninstall(&game, &package.id).unwrap();
        assert_eq!(
            fs::read(lookalike_dir.join("manual.dll")).unwrap(),
            b"manual"
        );
    });
}
