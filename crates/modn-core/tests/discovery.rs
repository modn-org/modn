#[path = "support/main_bucket.rs"]
mod main_bucket;
use modn_core::detection::detect_installations;
use modn_core::discovery::validate_installation_for;
use modn_core::state::Store;
use modn_core::stores::{StoreLibraries, Storefront};
use std::fs;
use tempfile::tempdir;

#[expect(clippy::unwrap_used, reason = "Fixture setup may fail fast in tests")]
fn game_at(path: &std::path::Path) {
    fs::create_dir_all(path.join("bin/x64")).unwrap();
    fs::write(path.join("bin/x64/Cyberpunk2077.exe"), "fixture").unwrap();
}

#[test]
fn steam_library_and_manifest_are_detected() {
    main_bucket::seeded(|| {
        let temp = tempdir().unwrap();
        let steam = temp.path().join("Steam");
        let library = temp.path().join("OtherLibrary");
        let game = library.join("steamapps/common/Custom Cyberpunk");
        game_at(&game);
        fs::create_dir_all(steam.join("steamapps")).unwrap();
        fs::write(
            steam.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"path\" \"{}\"
",
                library.display().to_string().replace('\\', "\\\\")
            ),
        )
        .unwrap();
        fs::write(
            library.join("steamapps/appmanifest_1091500.acf"),
            "\"appid\" \"1091500\"
\"installdir\" \"Custom Cyberpunk\"
",
        )
        .unwrap();
        let libraries = StoreLibraries::new(vec![steam], Vec::new(), Vec::new());
        let detection = detect_installations(&libraries, &["cyberpunk-2077".to_owned()]);
        assert_eq!(detection.confirmed.len(), 1);
        assert_eq!(detection.confirmed[0].store, Storefront::Steam);
        assert_eq!(detection.confirmed[0].path, game.canonicalize().unwrap());
    });
}

#[test]
fn epic_manifest_is_detected_and_deduplicated() {
    main_bucket::seeded(|| {
        let temp = tempdir().unwrap();
        let manifests = temp.path().join("manifests");
        let game = temp.path().join("Cyberpunk 2077");
        game_at(&game);
        fs::create_dir_all(&manifests).unwrap();
        for name in ["cyberpunk.item", "copy.item"] {
            fs::write(
                manifests.join(name),
                serde_json::json!({
                    "AppName": "Ginger",
                    "InstallLocation": game.to_string_lossy()
                })
                .to_string(),
            )
            .unwrap();
        }
        let libraries = StoreLibraries::new(Vec::new(), vec![manifests], Vec::new());
        let detection = detect_installations(&libraries, &["cyberpunk-2077".to_owned()]);
        assert_eq!(detection.confirmed.len(), 1);
        assert_eq!(detection.confirmed[0].store, Storefront::Epic);
    });
}

#[test]
fn invalid_game_is_rejected() {
    main_bucket::seeded(|| {
        let temp = tempdir().unwrap();
        assert!(validate_installation_for("cyberpunk-2077", temp.path()).is_err());
    });
}

#[test]
fn registration_persists_without_touching_game() {
    main_bucket::seeded(|| {
        let temp = tempdir().unwrap();
        let game = temp.path().join("game");
        game_at(&game);
        let state = temp.path().join("state/mod.sqlite");
        let game_path = validate_installation_for("cyberpunk-2077", &game).unwrap();
        let store = Store::open(&state).unwrap();
        store
            .add_game("cyberpunk-2077", &game_path, "manual")
            .unwrap();
        store
            .add_game("cyberpunk-2077", &game_path, "manual")
            .unwrap();
        drop(store);
        let entries = Store::open(&state).unwrap().list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, game_path);
        assert_eq!(
            fs::read(game.join("bin/x64/Cyberpunk2077.exe")).unwrap(),
            b"fixture"
        );
    });
}
