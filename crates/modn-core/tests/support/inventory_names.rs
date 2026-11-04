use super::*;

#[cfg(windows)]
#[test]
fn final_sigma_directory_rename_retains_managed_scan_attribution() {
    main_bucket::seeded(|| {
        let (root, mut store, game, _archive) = fixture();
        fs::create_dir_all(game.join("red4ext/plugins")).unwrap();
        let zip_path = root.path().join("sigma-folder.zip");
        let source = "red4ext/plugins/\u{039F}\u{03A3}/plugin.dll";
        let mut writer = zip::ZipWriter::new(File::create(&zip_path).unwrap());
        writer
            .start_file(source, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"plugin").unwrap();
        writer.finish().unwrap();
        let installed = install_zip(&mut store, &game, &zip_path);
        assert_eq!(installed.directories, ["red4ext/plugins/\u{039F}\u{03A3}"]);

        let original = game.join("red4ext/plugins/\u{039F}\u{03A3}");
        let intermediate = game.join("red4ext/plugins/sigma-tmp");
        let renamed = game.join("red4ext/plugins/\u{03BF}\u{03C2}");
        fs::rename(&original, &intermediate).unwrap();
        fs::rename(&intermediate, &renamed).unwrap();
        if !original.exists() {
            return;
        }
        fs::write(renamed.join("generated.dll"), b"runtime").unwrap();
        let report = scan_with_ownership_for(&store, "cyberpunk-2077", &game).unwrap();
        for file in ["plugin.dll", "generated.dll"] {
            let relative = PathBuf::from("red4ext/plugins/\u{03BF}\u{03C2}").join(file);
            let scanned = report
                .mods
                .iter()
                .find(|item| item.relative_path == relative)
                .unwrap();
            assert!(
                scanned.managed,
                "{relative:?} must remain managed: {report:?}"
            );
            assert_eq!(scanned.package_id.as_deref(), Some(installed.id.as_str()));
        }
        store.uninstall(&game, &installed.id).unwrap();
        assert!(!renamed.exists());
    });
}
