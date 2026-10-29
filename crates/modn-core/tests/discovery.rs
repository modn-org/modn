//! Integration checks for discovery behavior.
use modn_core::discovery::{discover_in_for, validate_installation_for};
use modn_core::state::Store;
use std::fs;
use tempfile::tempdir;

#[expect(clippy::unwrap_used, reason = "Fixture setup may fail fast in tests")]
fn game_at(path: &std::path::Path) {
    fs::create_dir_all(path.join("bin/x64")).unwrap();
    fs::write(path.join("bin/x64/Cyberpunk2077.exe"), "fixture").unwrap();
}

#[test]
fn steam_library_and_manifest_are_discovered() {
    let temp = tempdir().unwrap();
    let steam = temp.path().join("Steam");
    let library = temp.path().join("OtherLibrary");
    let game = library.join("steamapps/common/Custom Cyberpunk");
    game_at(&game);
    fs::create_dir_all(steam.join("steamapps")).unwrap();
    fs::write(
        steam.join("steamapps/libraryfolders.vdf"),
        format!("\"path\" \"{}\"\n", library.display()),
    )
    .unwrap();
    fs::write(
        library.join("steamapps/appmanifest_1091500.acf"),
        "\"installdir\" \"Custom Cyberpunk\"\n",
    )
    .unwrap();
    let results = discover_in_for("cyberpunk-2077", &[steam], &[], &[]);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].source, "steam");
    assert_eq!(results[0].path, game.canonicalize().unwrap());
}

#[test]
fn epic_manifest_is_detected_and_deduplicated() {
    let temp = tempdir().unwrap();
    let manifests = temp.path().join("manifests");
    let game = temp.path().join("Cyberpunk 2077");
    game_at(&game);
    fs::create_dir_all(&manifests).unwrap();
    fs::write(
        manifests.join("cyberpunk.item"),
        serde_json::json!({
            "DisplayName": "Cyberpunk 2077",
            "InstallLocation": game.to_string_lossy()
        })
        .to_string(),
    )
    .unwrap();
    let results = discover_in_for(
        "cyberpunk-2077",
        &[],
        &[manifests],
        std::slice::from_ref(&game),
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].source, "epic");
}

#[test]
fn invalid_game_is_rejected() {
    let temp = tempdir().unwrap();
    assert!(validate_installation_for("cyberpunk-2077", temp.path()).is_err());
}

#[test]
fn registration_persists_without_touching_game() {
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
}
